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
pub const NORMALIZED_PROFILE: &str = "pcm_s16le/48000/2/v1";

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
    log: &dyn Diagnostics,
) -> Result<Normalized, MediaError> {
    ensure_parent(dest)?;
    let temporary = partial_path(dest);

    let mut command = FfmpegCommand::new(tools.ffmpeg());
    command
        .args(["-hide_banner", "-nostats", "-loglevel", "warning", "-y"])
        .input(source)
        // Only the first audio stream, and no video: an MP3 with cover art has
        // a video stream, and copying it into the normalized artifact would
        // make every later probe of this file ambiguous about what it is.
        .args(["-map", "0:a:0", "-vn"])
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
