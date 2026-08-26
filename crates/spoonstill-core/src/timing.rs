//! Turning a measured audio duration into an exact frame count (D-021, D-022).
//!
//! Two rules, and everything here follows from them:
//!
//! - **Audio duration is authoritative** (D-021), and it is *measured* by
//!   `ffprobe` on the normalized artifact. Never estimated from text length,
//!   never read from a container header — `vbr_lying_header.mp3` exists in the
//!   fixtures precisely because headers lie.
//! - **Narration is padded up to the frame grid, never trimmed** (D-022).
//!   Rounding down would clip the last syllable of every scene whose duration
//!   is not a frame multiple, which is almost all of them.

/// The one sample rate every segment uses (D-040). Pinned here rather than in
/// the media crate because the frame/sample arithmetic below depends on it.
pub const SAMPLE_RATE: u32 = 48_000;

/// Frames needed to cover `seconds` of narration at `fps`, rounded **up**.
///
/// The epsilon is not decoration. `4.0 * 30.0` is not exactly `120.0` for every
/// `f64` that a probe can produce, and a bare `ceil()` on `120.00000000000001`
/// yields 121 — one extra frame of silence on every scene whose duration
/// happens to land on the grid. The epsilon is far below one sample period, so
/// it can never absorb a real frame.
#[must_use]
pub fn frames_for_duration(seconds: f64, fps: u32) -> u32 {
    if !seconds.is_finite() || seconds <= 0.0 || fps == 0 {
        return 1;
    }
    let exact = seconds * f64::from(fps);
    let frames = (exact - 1e-9).ceil();
    // Saturating rather than `as u32`, which would wrap a nonsense duration
    // into a small frame count and render a plausible-looking wrong segment.
    if frames >= f64::from(u32::MAX) {
        u32::MAX
    } else {
        (frames as u32).max(1)
    }
}

/// Exact duration of a segment of `frames` frames at `fps`.
///
/// This, not the narration length, is the duration the segment declares. The
/// difference is the pad of D-022.
#[must_use]
pub fn duration_for_frames(frames: u32, fps: u32) -> f64 {
    if fps == 0 {
        return 0.0;
    }
    f64::from(frames) / f64::from(fps)
}

/// Audio samples that exactly fill `frames` frames at `fps`.
///
/// Returned as a sample count rather than a duration in seconds so the audio
/// trim is an integer. `atrim=0:3.733333` truncates to whatever the decimal
/// happens to express; `atrim=end_sample=179200` is the number itself.
///
/// 48 kHz divides evenly by 24, 25, 30, 50 and 60, so for every frame rate an
/// operator will use this is exact. For anything else it rounds to nearest,
/// which is a sub-sample error and cannot accumulate — the concat demuxer
/// re-bases timestamps per segment (`ffmpeg-findings.md` §7).
#[must_use]
pub fn samples_for_frames(frames: u32, fps: u32) -> u64 {
    if fps == 0 {
        return 0;
    }
    let numerator = u64::from(frames) * u64::from(SAMPLE_RATE);
    let fps = u64::from(fps);
    // Round to nearest rather than truncating.
    (numerator + fps / 2) / fps
}

/// How much silence D-022 adds to reach the frame grid, in seconds.
///
/// Reported so an operator can see it, because a large pad means a narration
/// and a frame rate that disagree — not something to discover at scene 400.
#[must_use]
pub fn pad_seconds(narration_seconds: f64, frames: u32, fps: u32) -> f64 {
    (duration_for_frames(frames, fps) - narration_seconds).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference case from `ffmpeg-findings.md` §7, with a known answer:
    /// a 3.717 s narration at 30 fps is 112 frames and 3.733333 s.
    #[test]
    fn the_measured_reference_case_has_the_measured_answer() {
        let frames = frames_for_duration(3.717, 30);
        assert_eq!(frames, 112);
        assert!((duration_for_frames(frames, 30) - 3.733_333).abs() < 1e-6);
        assert_eq!(samples_for_frames(frames, 30), 179_200);
    }

    /// D-022: pad up, never trim. A duration one sample over the grid still
    /// gets a whole extra frame.
    #[test]
    fn narration_is_padded_up_never_trimmed() {
        assert_eq!(frames_for_duration(1.0 + 1e-6, 30), 31);
        assert_eq!(frames_for_duration(0.001, 30), 1);
        for (seconds, fps, want) in [
            (4.0, 30, 120),
            (10.0, 24, 240),
            (3.717, 25, 93),
            (0.5, 60, 30),
        ] {
            assert_eq!(
                frames_for_duration(seconds, fps),
                want,
                "{seconds}s @ {fps}fps"
            );
        }
    }

    /// The epsilon exists for exactly this: a duration that lands on the grid
    /// must not gain a frame of float noise.
    #[test]
    fn exact_grid_durations_gain_no_extra_frame() {
        for fps in [24_u32, 25, 30, 50, 60] {
            for whole in 1..=20_u32 {
                let seconds = f64::from(whole) / f64::from(fps);
                assert_eq!(
                    frames_for_duration(seconds, fps),
                    whole,
                    "{whole} frames @ {fps}fps round-tripped wrong"
                );
            }
        }
    }

    /// Every frame rate an operator will use divides 48 kHz exactly, so the
    /// audio trim is a whole number of samples with no residue.
    #[test]
    fn sample_counts_are_exact_at_every_supported_frame_rate() {
        for fps in [24_u32, 25, 30, 50, 60] {
            for frames in [1_u32, 7, 112, 1800] {
                let samples = samples_for_frames(frames, fps);
                assert_eq!(
                    samples * u64::from(fps),
                    u64::from(frames) * u64::from(SAMPLE_RATE),
                    "{frames} frames @ {fps}fps is not a whole number of samples"
                );
            }
        }
    }

    /// A probe that returns nonsense must produce a refusable frame count, not
    /// a wrapped one that renders a plausible-looking wrong segment.
    #[test]
    fn degenerate_durations_do_not_wrap() {
        assert_eq!(frames_for_duration(f64::NAN, 30), 1);
        assert_eq!(frames_for_duration(f64::INFINITY, 30), 1);
        assert_eq!(frames_for_duration(-5.0, 30), 1);
        assert_eq!(frames_for_duration(0.0, 30), 1);
        assert_eq!(frames_for_duration(1.0, 0), 1);
        assert_eq!(frames_for_duration(1e18, 30), u32::MAX);
    }

    #[test]
    fn the_pad_is_reported_and_never_negative() {
        let pad = pad_seconds(3.717, 112, 30);
        assert!(pad > 0.0 && pad < 1.0 / 30.0, "pad was {pad}");
        assert_eq!(pad_seconds(4.0, 120, 30), 0.0);
        assert_eq!(pad_seconds(99.0, 30, 30), 0.0);
    }
}
