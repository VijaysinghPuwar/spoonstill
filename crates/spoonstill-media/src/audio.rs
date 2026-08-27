//! Ingest normalization and generated silence (D-020, D-021).
//!
//! Every audio source — a file the operator supplied, a line spoken by TTS, a
//! declared silent hold — becomes the *same kind of artifact* before the
//! renderer sees it: 48 kHz stereo 16-bit PCM in a WAV container, written into
//! the cache, and then probed. That pair, `(path, measured duration)`, is all
//! the renderer consumes, and it is why nothing downstream branches on which
//! source a scene had (D-020).
//!
//! ## Two rules that this module exists to enforce
//!
//! - **The operator's file is never touched** (D-021). Normalization reads it
//!   and writes somewhere else. There is no in-place path here, and no code
//!   path that opens a source for writing.
//! - **Duration is measured on the normalized artifact, never on the
//!   original** (D-021). A VBR MP3 header can be wrong by seconds, and a
//!   scene's frame count is derived from this number — so it is read back from
//!   the file we just wrote, with `ffprobe`, after the write.
//!
//! ## Why PCM rather than AAC
//!
//! The segment encodes AAC anyway, so a compressed intermediate would put two
//! lossy generations between the operator's recording and the film. PCM costs
//! disk instead: 48 kHz stereo 16-bit is 192 KB/s, so a 500-scene project
//! averaging 30 s a scene holds about 2.9 GB of normalized audio. The cache is
//! disposable and rebuildable; a generation of AAC loss is not.
//!
//! ## Silence is a real track
//!
//! [`silence`] writes an actual PCM file of an exact sample count. It is not a
//! flag threaded through the renderer, which is what D-020 refuses: a silent
//! title card takes exactly the same path through this crate as a narrated
//! scene, and it is probed exactly as sceptically.

use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_core::SAMPLE_RATE;
use spoonstill_core::diagnostics::{Diagnostics, Event};

use crate::atomic::{ensure_parent, move_into_place, partial_path};
use crate::command::FfmpegCommand;
use crate::error::MediaError;
use crate::probe::{self, DEFAULT_PROBE_TIMEOUT};
use crate::profile::CHANNELS;
use crate::tools::Tools;

/// Codec of a normalized artifact, as ffprobe names it.
pub const NORMALIZED_CODEC: &str = "pcm_s16le";
/// Sample format of a normalized artifact, as ffprobe names it.
pub const NORMALIZED_SAMPLE_FMT: &str = "s16";
/// File extension of a normalized artifact.
pub const NORMALIZED_EXT: &str = "wav";

/// Short, stable name of the normalization profile.
///
/// Part of every audio cache key (D-043): changing the profile must miss the
/// cache rather than silently reuse artifacts made under the old one. Bump the
/// version suffix whenever anything above changes.
pub const NORMALIZED_PROFILE: &str = "pcm_s16le/48000/2/-16lufs/v2";

/// Programme loudness every scene is brought to, in LUFS (EBU R128).
///
/// **Why this exists at all.** Without it a film's loudness is whatever each
/// source happened to be: Edge TTS lands around -24 LUFS, a phone recording
/// around -12, and a project that mixes them makes the operator ride the volume
/// knob scene by scene. At n=500 that is not a polish issue, it is the
/// difference between a deliverable and a re-do.
///
/// -16 is the streaming convention for stereo speech — quiet enough to leave
/// headroom, loud enough to sit beside anything else on a phone.
pub const LOUDNESS_TARGET_LUFS: f64 = -16.0;

/// True-peak ceiling in dBTP. The gain is reduced if reaching the loudness
/// target would push the peak past this.
pub const TRUE_PEAK_CEILING_DBTP: f64 = -1.5;

/// Most gain we will apply, in dB.
///
/// A cap rather than a preference: without it, a track that is *almost* silent
/// — a recording made with the microphone muted, the classic operator mistake —
/// gets 40 dB of gain and arrives as a wall of hiss at full volume. Clamped, it
/// arrives quiet, which is what it is.
pub const MAX_GAIN_DB: f64 = 24.0;

/// Level below which audio counts as silence when trimming, in dB.
const SILENCE_FLOOR_DB: f64 = -45.0;

/// Shortest run that counts as silence when trimming, in seconds.
const SILENCE_MIN_SECONDS: f64 = 0.05;

/// How much of a provider's leading and trailing silence to keep.
///
/// Applied **only** to synthesized speech, never to a recording the operator
/// supplied — trimming someone's own file is the "we fixed this for you"
/// behaviour that plan.md §M2 rules out. A provider's padding is not content;
/// the operator did not choose it and usually cannot turn it off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trim {
    /// Seconds of silence to keep before the first word.
    pub head_seconds: f64,
    /// Seconds to keep after the last — a beat, so the cut does not clip the
    /// final consonant.
    pub tail_seconds: f64,
}

impl Trim {
    /// Whether these values would change anything.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        !self.head_seconds.is_finite()
            || !self.tail_seconds.is_finite()
            || (self.head_seconds < 0.0 && self.tail_seconds < 0.0)
    }
}

/// What to do to a source on the way to a normalized artifact.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Shape {
    /// Trim provider padding. `None` for anything the operator supplied.
    pub trim: Option<Trim>,
}

impl Shape {
    /// Leave the ends exactly as they are — a supplied recording.
    #[must_use]
    pub fn as_supplied() -> Self {
        Shape { trim: None }
    }

    /// Trim a provider's padding down to `trim`.
    #[must_use]
    pub fn spoken(trim: Trim) -> Self {
        Shape { trim: Some(trim) }
    }
}

/// What the analysis pass found out about a source.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Analysis {
    /// Integrated loudness in LUFS, or `None` when the file is silent enough
    /// that the measurement is meaningless.
    loudness: Option<f64>,
    /// True peak in dBTP.
    peak: Option<f64>,
    /// Where the first sound starts, in seconds.
    speech_starts: f64,
    /// Where the last sound ends, in seconds. `None` means "at the end".
    speech_ends: Option<f64>,
    /// The source's own length, when it was measured.
    duration: Option<f64>,
}

/// Ceiling for one normalization or silence generation.
///
/// Generous for a long narration on a slow volume, short against a hang. There
/// is no interactive cancellation here because these are sub-second operations
/// on realistic input — the deadline is the safety net, not the UI.
pub const NORMALIZE_TIMEOUT: Duration = Duration::from_secs(300);

/// Longest silent track we will generate, as a sample count.
///
/// Mirrors [`spoonstill_core::project::MAX_SCENE_SECONDS`] — a domain rule the
/// process boundary re-checks, because this is the layer that would otherwise
/// hand FFmpeg an hours-long `atrim` derived from a typo.
pub const MAX_SILENCE_SAMPLES: u64 =
    (spoonstill_core::project::MAX_SCENE_SECONDS as u64) * SAMPLE_RATE as u64;

/// A normalized audio artifact and what it actually turned out to be.
#[derive(Debug, Clone, PartialEq)]
pub struct Normalized {
    /// Where the artifact is.
    pub path: PathBuf,
    /// Its duration in seconds, measured on this file (D-021).
    pub duration: f64,
}

/// Normalize a supplied recording into `dest`.
///
/// `source` is opened read-only and is not modified, moved, or renamed.
///
/// # Errors
///
/// [`MediaError`] from the process boundary, or [`MediaError::UnusableInput`]
/// when the artifact we just wrote does not measure as usable audio.
pub fn normalize(
    tools: &Tools,
    source: &Path,
    dest: &Path,
    shape: &Shape,
    log: &dyn Diagnostics,
) -> Result<Normalized, MediaError> {
    ensure_parent(dest)?;
    let temporary = partial_path(dest);

    // One pass to find out what this file is, one to write what it should be.
    // Both are sub-second on realistic input and the result is cached forever
    // under a content key (D-043), so the second pass is paid once per source
    // and the first is what makes the third — the operator's ears — unnecessary.
    let analysis = analyse(tools, source, shape, log)?;
    let filters = filter_chain(&analysis, shape);

    let mut command = FfmpegCommand::new(tools.ffmpeg());
    command
        .args(["-hide_banner", "-nostats", "-loglevel", "warning", "-y"])
        .input(source)
        // Only the first audio stream, and no video: an MP3 with cover art has
        // a video stream, and copying it into the normalized artifact would
        // make every later probe of this file ambiguous about what it is.
        .args(["-map", "0:a:0", "-vn"]);
    if !filters.is_empty() {
        command.arg("-af").arg(&filters);
    }
    command
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("-ac")
        .arg(CHANNELS.to_string())
        .args(["-c:a", NORMALIZED_CODEC])
        // The muxer named explicitly rather than inferred from the temporary
        // file's extension, so a rename of the partial-file convention can
        // never quietly change the container.
        .args(["-f", NORMALIZED_EXT])
        .arg(&temporary);

    run(&command, log, &temporary)?;
    finish(tools, &temporary, dest, log, "normalize")
}

/// The filter chain that turns `source` into the artifact, given what it is.
///
/// Built as a pure function of the analysis so that it can be tested without a
/// process, and pinned by a test — the same discipline D-030's filter string
/// gets, for the same reason.
///
/// **Gain is linear.** `loudnorm`'s single-pass mode would also hit the target,
/// by compressing, which changes how the speech sounds and makes the result
/// depend on FFmpeg's version. A measured constant through `volume` moves every
/// sample by the same amount, sounds like the original, and is byte-identical
/// run to run — which D-077 requires.
fn filter_chain(analysis: &Analysis, shape: &Shape) -> String {
    let mut parts: Vec<String> = Vec::new();

    if let Some(trim) = shape.trim.filter(|trim| !trim.is_noop()) {
        // The settings say how much padding to *keep*, not where to cut, so
        // each edge moves toward the speech by whatever it has to spare. A
        // provider that already pads by less than we would keep is left alone
        // on that edge rather than having silence invented for it.
        let start = (analysis.speech_starts - trim.head_seconds.max(0.0)).max(0.0);
        let end = analysis.speech_ends.map(|end| {
            let padded = end + trim.tail_seconds.max(0.0);
            analysis.duration.map_or(padded, |whole| padded.min(whole))
        });

        let cuts_head = start > 0.0005;
        let cuts_tail = end
            .zip(analysis.duration)
            .is_some_and(|(end, whole)| end < whole - 0.0005);
        match (cuts_head, cuts_tail, end) {
            (_, true, Some(end)) => parts.push(format!("atrim=start={start:.3}:end={end:.3}")),
            (true, _, _) => parts.push(format!("atrim=start={start:.3}")),
            _ => {}
        }
        if !parts.is_empty() {
            // Without this the trimmed stream keeps the timestamps it had
            // inside the original, and the artifact starts at 0.14 rather than
            // at 0 — which every later measurement would then inherit.
            parts.push("asetpts=N/SR/TB".to_owned());
        }
    }

    if let Some(gain) = gain_db(analysis) {
        parts.push(format!("volume={gain:.2}dB"));
    }

    parts.join(",")
}

/// How much to move this file, in dB, or `None` when it is already right.
///
/// The smaller of "what the loudness target asks for" and "what the peak
/// ceiling allows", so a track that is quiet *and* already peaking is brought
/// up only as far as it can go without clipping.
fn gain_db(analysis: &Analysis) -> Option<f64> {
    let loudness = analysis.loudness?;
    let wanted = LOUDNESS_TARGET_LUFS - loudness;
    let allowed = analysis
        .peak
        .map_or(f64::INFINITY, |peak| TRUE_PEAK_CEILING_DBTP - peak);
    let gain = wanted.min(allowed).clamp(-MAX_GAIN_DB, MAX_GAIN_DB);

    // A twentieth of a decibel is inaudible, and skipping it keeps the filter
    // chain — and therefore the command line in the log — empty for a file that
    // is already where it should be.
    (gain.abs() >= 0.05).then_some(gain)
}

/// Measure a source: how loud it is, and where its sound actually starts and
/// stops.
///
/// One process, decoding to nowhere. `silencedetect` passes audio through
/// unchanged, so `loudnorm`'s analysis sees exactly the same samples it would
/// have seen alone — and R128 gating already ignores the silence, so measuring
/// before the trim gives the same answer as measuring after it.
fn analyse(
    tools: &Tools,
    source: &Path,
    shape: &Shape,
    log: &dyn Diagnostics,
) -> Result<Analysis, MediaError> {
    let mut chain = format!(
        "loudnorm=I={LOUDNESS_TARGET_LUFS}:TP={TRUE_PEAK_CEILING_DBTP}:LRA=11:print_format=json"
    );
    if shape.trim.is_some() {
        chain.push_str(&format!(
            ",silencedetect=n={SILENCE_FLOOR_DB}dB:d={SILENCE_MIN_SECONDS}"
        ));
    }

    let mut command = FfmpegCommand::new(tools.ffmpeg());
    command
        // `info` rather than `warning`: both measurements are printed at info
        // level, so quietening this would silently remove the whole point.
        .args(["-hide_banner", "-nostats", "-loglevel", "info"])
        .input(source)
        .args(["-map", "0:a:0", "-vn"])
        .arg("-af")
        .arg(&chain)
        .args(["-f", "null", "-"]);

    let display = command.display();
    log.record(&Event::info("ffmpeg", "measuring audio").with("command", display.clone()));
    let finished = command
        .spawn()?
        .wait_until(NORMALIZE_TIMEOUT)?
        .ok()
        .inspect_err(|_| {
            // A source FFmpeg cannot decode fails here rather than producing a
            // silent artifact two steps later.
            log.record(&Event::error("ffmpeg", "could not measure audio").with("command", display));
        })?;

    let duration = if shape.trim.is_some() {
        probe::probe(tools, source, DEFAULT_PROBE_TIMEOUT)?
            .audio()
            .and_then(|a| a.duration)
    } else {
        None
    };

    Ok(read_analysis(&finished.stderr, duration))
}

/// Pull the four numbers out of FFmpeg's stderr.
///
/// A hand-written reader rather than a JSON dependency: `spoonstill-media` has
/// none, `loudnorm`'s report is five flat string fields, and the alternative is
/// a crate in the dependency tree of a renderer for the sake of one object.
/// Anything unreadable comes back as `None` and the file is left alone, which
/// is the safe direction — an unmeasured file is quiet, not clipped.
fn read_analysis(stderr: &str, duration: Option<f64>) -> Analysis {
    let mut silences: Vec<(f64, Option<f64>)> = Vec::new();
    for line in stderr.lines() {
        if let Some(value) = after(line, "silence_start:") {
            silences.push((value, None));
        } else if let Some(value) = after(line, "silence_end:")
            && let Some(last) = silences.last_mut()
            && last.1.is_none()
        {
            last.1 = Some(value);
        }
    }

    // A leading silence is one that starts at the very beginning.
    let speech_starts = silences
        .first()
        .filter(|(start, _)| *start <= SILENCE_MIN_SECONDS)
        .and_then(|(_, end)| *end)
        .unwrap_or(0.0);

    // A trailing one is the last silence that runs to the end of the file.
    let speech_ends = duration.and_then(|duration| {
        silences
            .last()
            .filter(|(start, _)| *start > speech_starts)
            .filter(|(_, end)| end.is_none_or(|end| end >= duration - SILENCE_MIN_SECONDS))
            .map(|(start, _)| *start)
    });

    Analysis {
        loudness: json_number(stderr, "input_i"),
        peak: json_number(stderr, "input_tp"),
        speech_starts,
        speech_ends,
        duration,
    }
}

/// `key: 1.234` at the end of a line.
fn after(line: &str, key: &str) -> Option<f64> {
    let rest = line.split(key).nth(1)?;
    rest.split_whitespace().next()?.parse().ok()
}

/// `"key" : "-23.4"` from loudnorm's report. `-inf` and `nan` read as absent,
/// which is what they mean: there was nothing here to measure.
fn json_number(stderr: &str, key: &str) -> Option<f64> {
    let rest = stderr.split(&format!("\"{key}\"")).nth(1)?;
    let value = rest.split('"').nth(1)?;
    value.parse::<f64>().ok().filter(|n| n.is_finite())
}

/// Write a silent track of exactly `samples` samples into `dest`.
///
/// Exact rather than `-t seconds`: `-t` rounds, and a scene whose declared
/// three seconds becomes 2.999979 s has moved the frame boundary for no reason
/// anyone can see.
///
/// # Errors
///
/// [`MediaError::UnusableInput`] when `samples` is zero or beyond
/// [`MAX_SILENCE_SAMPLES`], and anything the process boundary reports.
pub fn silence(
    tools: &Tools,
    samples: u64,
    dest: &Path,
    log: &dyn Diagnostics,
) -> Result<Normalized, MediaError> {
    if samples == 0 || samples > MAX_SILENCE_SAMPLES {
        return Err(MediaError::UnusableInput {
            path: dest.to_path_buf(),
            detail: format!(
                "a silent scene of {samples} samples is not renderable — it must be \
                 between 1 and {MAX_SILENCE_SAMPLES} (D-021)"
            ),
        });
    }

    ensure_parent(dest)?;
    let temporary = partial_path(dest);

    let mut command = FfmpegCommand::new(tools.ffmpeg());
    command
        .args(["-hide_banner", "-nostats", "-loglevel", "warning", "-y"])
        // `anullsrc` is infinite; `atrim` is what ends it, and it ends it on a
        // sample rather than on a rounded decimal.
        .args([
            "-f",
            "lavfi",
            "-i",
            &format!("anullsrc=r={SAMPLE_RATE}:cl=stereo"),
        ])
        .arg("-af")
        .arg(format!("atrim=end_sample={samples}"))
        .args(["-c:a", NORMALIZED_CODEC])
        .args(["-f", NORMALIZED_EXT])
        .arg(&temporary);

    run(&command, log, &temporary)?;
    finish(tools, &temporary, dest, log, "silence")
}

/// Measure an already-normalized artifact, refusing one that is not.
///
/// Used on a cache hit: the artifact is trusted only as far as this function
/// can confirm it, so a truncated or half-written cache entry is caught here
/// rather than becoming a scene of the wrong length.
///
/// # Errors
///
/// [`MediaError::UnusableInput`] when the file is not 48 kHz stereo PCM, or
/// when its duration is zero, negative, or not a number (D-021).
pub fn measure(tools: &Tools, path: &Path) -> Result<Normalized, MediaError> {
    let probed = probe::probe(tools, path, DEFAULT_PROBE_TIMEOUT)?;

    let audio = probed.audio().ok_or_else(|| MediaError::UnusableInput {
        path: path.to_path_buf(),
        detail: "carries no audio stream".into(),
    })?;

    // The normalized profile, asserted. Channel *layout* is deliberately not
    // among these: WAV does not store one, so ffprobe reports `unknown` for a
    // perfectly good stereo file.
    let mut wrong: Vec<String> = Vec::new();
    if audio.codec_name != NORMALIZED_CODEC {
        wrong.push(format!(
            "codec is {} rather than {NORMALIZED_CODEC}",
            audio.codec_name
        ));
    }
    if audio.sample_rate != Some(SAMPLE_RATE) {
        wrong.push(format!(
            "sample rate is {:?} rather than {SAMPLE_RATE}",
            audio.sample_rate
        ));
    }
    if audio.channels != Some(CHANNELS) {
        wrong.push(format!(
            "channel count is {:?} rather than {CHANNELS}",
            audio.channels
        ));
    }
    if audio.sample_fmt.as_deref() != Some(NORMALIZED_SAMPLE_FMT) {
        wrong.push(format!(
            "sample format is {:?} rather than {NORMALIZED_SAMPLE_FMT}",
            audio.sample_fmt
        ));
    }
    if !wrong.is_empty() {
        return Err(MediaError::UnusableInput {
            path: path.to_path_buf(),
            detail: format!("is not a normalized artifact: {}", wrong.join("; ")),
        });
    }

    let duration = probed
        .audio_duration()
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or_else(|| MediaError::UnusableInput {
            path: path.to_path_buf(),
            detail: "carries no usable audio duration — an empty, truncated, or \
                     half-written artifact cannot drive a scene (D-021)"
                .into(),
        })?;

    Ok(Normalized {
        path: path.to_path_buf(),
        duration,
    })
}

/// Run one short FFmpeg invocation, cleaning up its temporary on failure.
fn run(command: &FfmpegCommand, log: &dyn Diagnostics, temporary: &Path) -> Result<(), MediaError> {
    let child = command.spawn()?;
    let display = child.display().to_string();
    log.record(&Event::info("ffmpeg", "running").with("command", display.clone()));

    let finished = match child.wait_until(NORMALIZE_TIMEOUT) {
        Ok(finished) => finished,
        Err(error) => {
            let _ = std::fs::remove_file(temporary);
            return Err(error);
        }
    };

    if !finished.status.success() {
        log.record(
            &Event::error("ffmpeg", "exited non-zero")
                .with("command", display)
                .with(
                    "code",
                    finished
                        .status
                        .code()
                        .map_or_else(|| "signal".to_string(), |c| c.to_string()),
                )
                .with("stderr", finished.stderr.clone()),
        );
        let _ = std::fs::remove_file(temporary);
        finished.ok()?;
    }
    Ok(())
}

/// Measure the temporary, and only then let it become the artifact.
///
/// The same order as a segment (D-042): a file that fails its measurement is
/// deleted rather than left where a later run would find it and believe it.
fn finish(
    tools: &Tools,
    temporary: &Path,
    dest: &Path,
    log: &dyn Diagnostics,
    what: &'static str,
) -> Result<Normalized, MediaError> {
    let measured = match measure(tools, temporary) {
        Ok(measured) => measured,
        Err(error) => {
            let _ = std::fs::remove_file(temporary);
            return Err(error);
        }
    };

    move_into_place(temporary, dest)?;

    log.record(
        &Event::info("audio", what)
            .with("path", dest.display().to_string())
            .with("duration_s", format!("{:.6}", measured.duration)),
    );

    Ok(Normalized {
        path: dest.to_path_buf(),
        duration: measured.duration,
    })
}

#[cfg(test)]
mod tests {
    /// A real `loudnorm` report and a real `silencedetect` run, recorded from
    /// this machine on 2026-08-26 against an Edge TTS line. Recorded rather
    /// than described: FFmpeg's report format is not ours to assume, and a
    /// change to it must fail here rather than become a film with no gain
    /// applied and nobody the wiser.
    const RECORDED_STDERR: &str = r#"
[Parsed_silencedetect_1 @ 0x600002] silence_start: 0
[Parsed_silencedetect_1 @ 0x600002] silence_end: 0.235187 | silence_duration: 0.235187
[Parsed_silencedetect_1 @ 0x600002] silence_start: 8.944187
[Parsed_silencedetect_1 @ 0x600002] silence_end: 9.288 | silence_duration: 0.343812
[Parsed_loudnorm_0 @ 0x600003]
{
	"input_i" : "-23.22",
	"input_tp" : "-6.38",
	"input_lra" : "5.30",
	"input_thresh" : "-33.94",
	"output_i" : "-16.00",
	"normalization_type" : "dynamic",
	"target_offset" : "0.00"
}
"#;

    fn spoken_trim() -> Shape {
        Shape::spoken(Trim {
            head_seconds: 0.10,
            tail_seconds: 0.25,
        })
    }

    #[test]
    fn the_recorded_report_yields_the_measurements_it_contains() {
        let analysis = read_analysis(RECORDED_STDERR, Some(9.288));

        assert_eq!(analysis.loudness, Some(-23.22));
        assert_eq!(analysis.peak, Some(-6.38));
        assert!((analysis.speech_starts - 0.235187).abs() < 1e-6);
        assert_eq!(analysis.speech_ends, Some(8.944187));
    }

    /// The bug this whole path exists to fix: Edge TTS pads every line, so
    /// every cut landed about six tenths of a second after the speech stopped.
    #[test]
    fn a_provider_padded_line_is_trimmed_to_the_padding_we_asked_for() {
        let analysis = read_analysis(RECORDED_STDERR, Some(9.288));
        let chain = filter_chain(&analysis, &spoken_trim());

        // 0.235 of lead, of which we keep 0.100 -> cut at 0.135; the tail
        // ends at 8.944 and keeps 0.250 -> 9.194, inside the 9.288 file.
        assert!(chain.starts_with("atrim=start=0.135:end=9.194"), "{chain}");
        assert!(chain.contains("asetpts=N/SR/TB"), "{chain}");
    }

    /// D-084: a recording the operator made is theirs, ends included.
    #[test]
    fn a_supplied_recording_is_never_trimmed() {
        let analysis = read_analysis(RECORDED_STDERR, Some(9.288));
        let chain = filter_chain(&analysis, &Shape::as_supplied());

        assert!(!chain.contains("atrim"), "{chain}");
        assert!(chain.contains("volume="), "it is still levelled: {chain}");
    }

    #[test]
    fn the_gain_brings_the_recorded_line_to_the_target() {
        let analysis = read_analysis(RECORDED_STDERR, Some(9.288));

        // -23.22 LUFS wants +7.22 dB; the peak at -6.38 dBTP allows +4.88
        // before -1.5 dBTP. The peak wins.
        let gain = gain_db(&analysis).expect("a gain");
        assert!((gain - 4.88).abs() < 0.01, "{gain}");
    }

    #[test]
    fn a_file_already_at_the_target_is_left_alone() {
        let analysis = Analysis {
            loudness: Some(LOUDNESS_TARGET_LUFS),
            peak: Some(-6.0),
            speech_starts: 0.0,
            speech_ends: None,
            duration: None,
        };
        assert_eq!(gain_db(&analysis), None);
        assert!(filter_chain(&analysis, &Shape::as_supplied()).is_empty());
    }

    /// The muted-microphone case. Without the clamp this becomes 40 dB of
    /// amplified hiss at full volume, in a film someone is about to deliver.
    #[test]
    fn a_nearly_silent_file_is_not_amplified_without_limit() {
        let analysis = Analysis {
            loudness: Some(-70.0),
            peak: Some(-60.0),
            speech_starts: 0.0,
            speech_ends: None,
            duration: None,
        };
        assert_eq!(gain_db(&analysis), Some(MAX_GAIN_DB));
    }

    /// Digital silence measures as `-inf`, which is not a number and not a
    /// reason to do anything.
    #[test]
    fn an_unmeasurable_file_is_passed_through_untouched() {
        let analysis = read_analysis(
            "{ \"input_i\" : \"-inf\", \"input_tp\" : \"-inf\" }",
            Some(3.0),
        );
        assert_eq!(analysis.loudness, None);
        assert_eq!(gain_db(&analysis), None);
        assert!(filter_chain(&analysis, &spoken_trim()).is_empty());
    }

    /// A silence in the middle of a line is a pause between sentences. Trimming
    /// it would rewrite the delivery.
    #[test]
    fn an_internal_pause_is_not_a_trailing_silence() {
        let stderr = "silence_start: 0\nsilence_end: 0.20\n\
                      silence_start: 4.00\nsilence_end: 4.30\n";
        let analysis = read_analysis(stderr, Some(9.0));

        assert!((analysis.speech_starts - 0.20).abs() < 1e-9);
        assert_eq!(
            analysis.speech_ends, None,
            "the last silence ends well before the file does"
        );
    }

    /// A line that starts talking immediately has no leading silence to remove,
    /// and must not have its first syllable removed instead.
    #[test]
    fn a_line_with_no_leading_silence_keeps_its_first_syllable() {
        let stderr = "silence_start: 7.50\nsilence_end: 8.00\n";
        let analysis = read_analysis(stderr, Some(8.0));

        assert_eq!(analysis.speech_starts, 0.0);
        assert_eq!(analysis.speech_ends, Some(7.50));
        assert!(filter_chain(&analysis, &spoken_trim()).starts_with("atrim=start=0.000:end=7.750"),);
    }

    use super::*;

    /// D-043: the profile string reaches the cache key, so it has to change
    /// whenever the profile does. Pinned here so that editing one without the
    /// other fails a test rather than silently reusing stale artifacts.
    #[test]
    fn the_profile_string_names_the_actual_profile() {
        assert!(NORMALIZED_PROFILE.contains(NORMALIZED_CODEC));
        assert!(NORMALIZED_PROFILE.contains(&SAMPLE_RATE.to_string()));
        assert!(NORMALIZED_PROFILE.contains(&CHANNELS.to_string()));
    }

    /// An absurd duration is refused before FFmpeg is asked to allocate it.
    #[test]
    fn a_silent_scene_of_impossible_length_is_refused_without_spawning() {
        let tools = Tools::at("/nonexistent/ffmpeg", "/nonexistent/ffprobe");
        let log = spoonstill_core::diagnostics::Noop;
        let dest = std::env::temp_dir().join("spoonstill-never-written.wav");

        for samples in [0, MAX_SILENCE_SAMPLES + 1] {
            let error = silence(&tools, samples, &dest, &log).expect_err("refused");
            // Not BinaryMissing: the refusal happened before the spawn.
            assert!(
                matches!(error, MediaError::UnusableInput { .. }),
                "{samples} samples gave {error}"
            );
        }
        assert!(!dest.exists());
    }

    /// One hour, the domain's own ceiling, expressed in samples.
    #[test]
    fn the_silence_ceiling_is_the_domain_ceiling() {
        assert_eq!(
            MAX_SILENCE_SAMPLES,
            3_600 * u64::from(SAMPLE_RATE),
            "MAX_SILENCE_SAMPLES must track MAX_SCENE_SECONDS"
        );
    }
}
