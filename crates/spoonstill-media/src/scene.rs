//! Rendering one scene: still + narration -> one profile-conforming segment.
//!
//! This is M1's whole product in miniature. The ordering below is D-042 and it
//! is not negotiable:
//!
//! ```text
//! render to a temporary path
//!   -> ffprobe validate (frames, duration, dimensions, codec, SAR, colour)
//!     -> atomic move into the real segment path
//!       -> [the caller commits state]
//! ```
//!
//! A crash at any point leaves either a complete valid segment or nothing that
//! looks valid. Nothing ever writes directly to the destination, so an
//! interrupted render cannot leave a plausible-looking stub for a later run to
//! trust and skip.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use spoonstill_core::diagnostics::{Diagnostics, Event};
use spoonstill_core::{MotionSpec, OutputSpec, SAMPLE_RATE, build_filter, timing};

use crate::atomic::{ensure_parent, move_into_place, partial_path};
use crate::command::{FfmpegCommand, Progress};
use crate::error::MediaError;
use crate::probe::{self, DEFAULT_PROBE_TIMEOUT, ProbeResult};
use crate::profile::{self, SegmentProfile};
use crate::tools::Tools;

/// How long a cancelled render may take to finalize before it is killed.
///
/// FFmpeg needs a moment to flush and close the MP4 after `q`. Two seconds is
/// generous for that and short enough that a user pressing Ctrl-C does not
/// wonder whether it worked.
pub const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// Encoder settings that affect the output, and therefore the cache key (D-043).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodeSettings {
    /// x264 preset. D-036: `medium` is the default.
    pub preset: String,
    /// x264 CRF. D-036: 18.
    pub crf: u32,
}

impl Default for EncodeSettings {
    fn default() -> Self {
        // D-036. Hardware encode is an opt-in draft mode, never the default:
        // VideoToolbox and NVENC band visibly on slow pans across large smooth
        // gradients, which is exactly this content.
        Self {
            preset: "medium".to_string(),
            crf: 18,
        }
    }
}

/// Everything needed to render one scene.
#[derive(Debug, Clone)]
pub struct SceneRequest {
    /// The still.
    pub image: PathBuf,
    /// The narration, already normalized.
    pub audio: PathBuf,
    /// Where the finished segment belongs.
    pub out: PathBuf,
    /// Output geometry and frame rate.
    pub output: OutputSpec,
    /// The move. `None` means derive it deterministically (D-035).
    pub motion: Option<MotionSpec>,
    /// Stable project identity, part of the motion seed.
    pub project_id: String,
    /// Scene index within the project, part of the motion seed.
    pub scene_index: u32,
    /// Encoder settings.
    pub encode: EncodeSettings,
}

impl SceneRequest {
    /// A request with the measured defaults, for the single-scene CLI path.
    #[must_use]
    pub fn new(image: PathBuf, audio: PathBuf, out: PathBuf, output: OutputSpec) -> Self {
        Self {
            image,
            audio,
            out,
            output,
            motion: None,
            project_id: "single-scene".to_string(),
            scene_index: 0,
            encode: EncodeSettings::default(),
        }
    }
}

/// What a finished scene turned out to be.
#[derive(Debug, Clone)]
pub struct RenderedScene {
    /// Where the segment now lives.
    pub path: PathBuf,
    /// Frame count — structural, and asserted against the file (D-030).
    pub frames: u32,
    /// The segment's exact duration in seconds.
    pub duration: f64,
    /// The narration duration as measured, before padding (D-021).
    pub narration: f64,
    /// Silence added to reach the frame grid (D-022).
    pub pad: f64,
    /// The move that was rendered.
    pub motion: MotionSpec,
    /// The validating probe, retained so a caller can checkpoint from it.
    pub probe: ProbeResult,
}

/// A cancellation flag shared with whoever handles Ctrl-C (D-045).
///
/// Deliberately trivial: the interesting part of cancellation is the ladder in
/// [`crate::command::FfmpegChild::cancel`] and the cleanup below, not the
/// signalling.
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A fresh, unset flag.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Request cancellation. Safe to call from a signal handler.
    pub fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Render one scene, validate it, and move it into place.
///
/// # Errors
///
/// Any [`MediaError`]. In particular [`MediaError::ProfileMismatch`] when the
/// rendered segment does not match [`SegmentProfile`] — which is the D-041
/// gate, and the reason the destination path stays empty until it passes.
pub fn render_scene(
    tools: &Tools,
    request: &SceneRequest,
    cancel: &Cancel,
    log: &dyn Diagnostics,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<RenderedScene, MediaError> {
    match render_scene_inner(tools, request, cancel, log, on_progress) {
        Ok(rendered) => Ok(rendered),
        Err(error) => {
            // Recorded here, once, at the boundary — so that every failure
            // reaches the diagnostics bundle whatever path produced it, and no
            // individual call site has to remember to log.
            log.record(
                &Event::error("render", "scene failed")
                    .with("scene_index", request.scene_index.to_string())
                    .with("image", request.image.display().to_string())
                    .with("audio", request.audio.display().to_string())
                    .with("out", request.out.display().to_string())
                    .with("detail", error.to_string()),
            );
            Err(error)
        }
    }
}

fn render_scene_inner(
    tools: &Tools,
    request: &SceneRequest,
    cancel: &Cancel,
    log: &dyn Diagnostics,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<RenderedScene, MediaError> {
    // 1. Measure. Audio duration is authoritative and it is measured on the
    //    normalized artifact, never estimated and never read from a header
    //    (D-021). Both probes are time-bounded from the first call.
    let image_probe = probe::probe(tools, &request.image, DEFAULT_PROBE_TIMEOUT)?;
    let source = image_probe.source_geometry()?;

    let audio_probe = probe::probe(tools, &request.audio, DEFAULT_PROBE_TIMEOUT)?;
    let narration = audio_probe
        .audio_duration()
        .ok_or_else(|| MediaError::UnusableInput {
            path: request.audio.clone(),
            detail: "carries no usable audio duration — an empty, truncated, or \
                     non-audio file cannot drive a scene (D-021)"
                .into(),
        })?;

    let fps = request.output.fps();
    let frames = timing::frames_for_duration(narration, fps);
    let duration = timing::duration_for_frames(frames, fps);
    let pad = timing::pad_seconds(narration, frames, fps);

    // 2. Decide the move. Seeded from stable identity so a re-render is
    //    byte-identical and the cache stays warm (D-035, D-043).
    let motion = match request.motion {
        Some(motion) => motion,
        None => {
            let content_hash = hash_file(&request.image)?;
            MotionSpec::seeded(&request.project_id, request.scene_index, &content_hash)
        }
    };

    let filter = build_filter(source, request.output, motion, frames);
    let expected = SegmentProfile::for_output(request.output);

    // The resolved scene, written down before anything expensive happens. When
    // a render is wrong rather than failed, this is the record that says
    // whether the wrongness entered at measurement or at encoding.
    log.record(
        &Event::info("render", "scene resolved")
            .with("scene_index", request.scene_index.to_string())
            .with("image", request.image.display().to_string())
            .with("source", format!("{}x{}", source.width(), source.height()))
            .with("audio", request.audio.display().to_string())
            .with("narration_s", format!("{narration:.6}"))
            .with("frames", frames.to_string())
            .with("duration_s", format!("{duration:.6}"))
            .with("pad_s", format!("{pad:.6}"))
            .with(
                "output",
                format!(
                    "{}x{}@{}",
                    request.output.width(),
                    request.output.height(),
                    fps
                ),
            )
            .with("motion", motion.descriptor())
            .with("motion_seed", format!("{:016x}", motion.seed))
            .with("filter", filter.clone()),
    );

    // 3. Render to a temporary path beside the destination — same directory, so
    //    the move at the end is a rename within one filesystem and therefore
    //    atomic. A temp file in the system temp directory would make it a copy.
    let temporary = partial_path(&request.out);
    ensure_parent(&temporary)?;

    let result = run_ffmpeg(
        tools,
        request,
        &filter,
        frames,
        &expected,
        &temporary,
        cancel,
        log,
        on_progress,
    );

    // 4. Validate before anything downstream can see the file. On any failure
    //    the temporary is removed, so a partial render never survives as
    //    something a later run could mistake for a finished segment.
    let probe = match result.and_then(|()| validate(tools, &temporary, &expected, frames, log)) {
        Ok(probe) => probe,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };

    // 5. Only now does the destination path come into existence.
    move_into_place(&temporary, &request.out)?;

    Ok(RenderedScene {
        path: request.out.clone(),
        frames,
        duration,
        narration,
        pad,
        motion,
        probe,
    })
}

/// Build and run the FFmpeg invocation. Argument vectors only.
#[allow(clippy::too_many_arguments)]
fn run_ffmpeg(
    tools: &Tools,
    request: &SceneRequest,
    filter: &str,
    frames: u32,
    expected: &SegmentProfile,
    temporary: &Path,
    cancel: &Cancel,
    log: &dyn Diagnostics,
    on_progress: &mut dyn FnMut(Progress),
) -> Result<(), MediaError> {
    let fps = request.output.fps();
    let samples = timing::samples_for_frames(frames, fps);

    // D-022: pad the narration up to the frame grid, never trim the video down
    // to the audio. `apad` extends with silence, `atrim` cuts at an exact
    // sample count rather than a rounded decimal, and `asetpts` rebases the
    // timestamps so the segment starts at zero.
    let audio_chain =
        format!("[1:a]aresample={SAMPLE_RATE},apad,atrim=end_sample={samples},asetpts=N/SR/TB[a]");
    let graph = format!("[0:v]{filter}[v];{audio_chain}");

    let mut command = FfmpegCommand::new(tools.ffmpeg());
    command
        .args(["-hide_banner", "-nostats", "-loglevel", "warning", "-y"])
        // D-030: a single still, with no `-loop`. The unbounded `-loop 1` form
        // is the five-hour "hang", and it is unreachable from here because
        // nothing in this function can add that flag.
        .input(&request.image)
        .input(&request.audio)
        .arg("-filter_complex")
        .arg(&graph)
        .args(["-map", "[v]", "-map", "[a]"])
        // The frame count, structurally, for the second time.
        .arg("-frames:v")
        .arg(frames.to_string())
        .args(["-c:v", "libx264"])
        .arg("-preset")
        .arg(&request.encode.preset)
        .arg("-crf")
        .arg(request.encode.crf.to_string())
        .args(["-profile:v", "high"])
        .arg("-level:v")
        .arg(expected.video_level.to_string())
        .args(["-c:a", "aac", "-b:a", profile::AUDIO_BITRATE])
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("-ac")
        .arg(profile::CHANNELS.to_string())
        // Pins the time base so it is a constant rather than a function of the
        // frame rate the operator happened to choose.
        .arg("-video_track_timescale")
        .arg(profile::VIDEO_TIMESCALE.to_string())
        .with_progress()
        .arg(temporary);

    let mut child = command.spawn()?;
    let display = child.display().to_string();

    // Every command executed, recorded in a form an operator can paste into a
    // terminal. lossless-cut's LastCommands panel is the pattern: when a render
    // fails at scene 147, the first useful move is to run that exact command by
    // hand, and making someone reconstruct it means they reconstruct it wrong.
    log.record(&Event::info("ffmpeg", "running").with("command", display.clone()));

    loop {
        // Drain progress without blocking, so a cancellation is noticed
        // promptly even on a scene that reports rarely.
        while let Ok(progress) = child.progress().try_recv() {
            on_progress(progress);
        }

        if cancel.is_requested() {
            // D-045: ask, wait, force. Whatever it wrote is a temporary file
            // and gets removed by the caller.
            let finished = child.cancel(CANCEL_GRACE);
            log.record(
                &Event::warn("ffmpeg", "cancelled")
                    .with("command", display.clone())
                    .with("stderr", finished.stderr),
            );
            return Err(MediaError::Cancelled { command: display });
        }

        match child.try_status() {
            Ok(Some(_)) => break,
            Ok(None) => {}
            Err(source) => {
                return Err(MediaError::Spawn {
                    command: display,
                    source,
                });
            }
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let finished = child.wait()?;
    if !finished.status.success() {
        // Retained raw stderr, not a parser's summary of it: the FFmpeg
        // failures that matter do not always announce themselves at error
        // level, and this is the only copy that will exist later.
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
    }
    finished.ok()?;
    Ok(())
}

/// The D-040/D-041 gate. Everything the profile pins, plus the frame count.
fn validate(
    tools: &Tools,
    path: &Path,
    expected: &SegmentProfile,
    frames: u32,
    log: &dyn Diagnostics,
) -> Result<ProbeResult, MediaError> {
    // Counted, not declared. A container's frame count is metadata; this
    // decodes the file and reports what is actually in it.
    let probe = probe::probe_counting_frames(tools, path, DEFAULT_PROBE_TIMEOUT)?;

    if let Err(mismatches) = profile::assert_matches_profile(expected, &probe) {
        // Each field on its own record, so a bundle can be scanned for "which
        // field went wrong" across a whole batch rather than read line by line.
        for mismatch in &mismatches {
            log.record(
                &Event::error("profile", "segment field does not match")
                    .with("path", path.display().to_string())
                    .with("field", mismatch.field)
                    .with("expected", mismatch.expected.clone())
                    .with("actual", mismatch.actual.clone()),
            );
        }
        return Err(MediaError::ProfileMismatch {
            path: path.to_path_buf(),
            mismatches,
        });
    }

    let actual = probe.video().and_then(|v| v.nb_read_frames);
    if actual != Some(u64::from(frames)) {
        return Err(MediaError::ProfileMismatch {
            path: path.to_path_buf(),
            mismatches: vec![profile::Mismatch {
                field: "nb_read_frames",
                expected: frames.to_string(),
                actual: actual.map_or_else(|| "<uncounted>".to_string(), |v| v.to_string()),
            }],
        });
    }

    Ok(probe)
}

/// Stable content hash of a file, for the motion seed and the cache key.
///
/// Content, never the path (D-043): moving a file must not re-roll its motion,
/// and `Automated-Video-Generator`'s path-keyed asset cache is the mistake this
/// avoids. With BYOK a cache miss costs the operator money, so this is a
/// correctness requirement rather than an optimisation.
fn hash_file(path: &Path) -> Result<String, MediaError> {
    let bytes = std::fs::read(path).map_err(|source| MediaError::Io {
        doing: "reading",
        path: path.to_path_buf(),
        source,
    })?;
    Ok(format!("{:016x}", spoonstill_core::hash::fnv1a(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-036 defaults, pinned. Changing these invalidates every cached segment
    /// and belongs in decisions.md.
    #[test]
    fn encoder_defaults_are_the_recorded_ones() {
        let e = EncodeSettings::default();
        assert_eq!(e.preset, "medium");
        assert_eq!(e.crf, 18);
    }

    #[test]
    fn cancellation_is_observable_across_clones() {
        let cancel = Cancel::new();
        let handle = cancel.clone();
        assert!(!cancel.is_requested());
        handle.request();
        assert!(cancel.is_requested());
    }

    /// D-043: identical bytes at different paths must hash identically, and
    /// different bytes must not.
    #[test]
    fn content_hashing_ignores_the_path() {
        let dir = std::env::temp_dir().join(format!("spoonstill-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.bin");
        let b = dir.join("b.bin");
        let c = dir.join("c.bin");
        std::fs::write(&a, b"same bytes").unwrap();
        std::fs::write(&b, b"same bytes").unwrap();
        std::fs::write(&c, b"other bytes").unwrap();

        assert_eq!(hash_file(&a).unwrap(), hash_file(&b).unwrap());
        assert_ne!(hash_file(&a).unwrap(), hash_file(&c).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing file must name itself rather than surface as a bare io error.
    #[test]
    fn hashing_a_missing_file_names_it() {
        let err = hash_file(Path::new("/nonexistent/scene-42.jpg")).unwrap_err();
        assert!(err.to_string().contains("scene-42.jpg"), "{err}");
    }
}
