//! The FFmpeg and ffprobe process boundary (D-011, D-012).
//!
//! Everything that spawns a child process lives here, and nothing else does.
//! Two rules this crate exists to enforce:
//!
//! - **Argument vectors, never shell strings.** Every invocation is built as a
//!   sequence of `OsStr`, so a filename containing a space, a quote, an emoji
//!   or a newline is data and can never become syntax (D-052).
//! - **Exit status is not validation.** FFmpeg returns 0 on a mismatched
//!   concat (D-041), so the segment profile is asserted here against real
//!   `ffprobe` output before anything downstream trusts a file.
//!
//! M1 filled this in: [`command`] is the builder and the retained child handle
//! with separate `quit()`/`kill()`/`wait()`, [`probe`] is timed `ffprobe` JSON,
//! [`profile`] is `SEGMENT_PROFILE` and its assertion, and [`scene`] wires them
//! into one segment.
//!
//! M2 added the rest of the pipeline around that segment: [`audio`] normalizes
//! every source to one profile and measures it (D-021), [`concat`] joins
//! validated segments with a stream copy (D-040), and [`atomic`] is the
//! write-beside-then-rename rule all three share (D-042).

#![warn(missing_docs)]

pub mod atomic;
pub mod audio;
pub mod caption;
pub mod command;
pub mod concat;
pub mod error;
pub mod hardware;
pub mod probe;
pub mod profile;
pub mod scene;
pub mod tools;

pub use audio::{Normalized, measure, normalize, silence};
pub use caption::{Canvas, CaptionImage, render_cue};
pub use command::{FfmpegChild, FfmpegCommand, Finished, Progress};
pub use concat::{Film, concat};
pub use error::MediaError;
pub use hardware::{Candidate, Detected, Support, detect as detect_hardware};
pub use probe::{ProbeResult, Stream, StreamKind, probe, probe_counting_frames};
pub use profile::{Mismatch, SegmentProfile, assert_matches_profile};
pub use scene::{Cancel, EncodeSettings, RenderedScene, SceneRequest, render_scene};
pub use tools::Tools;

/// This crate's package name, resolved at compile time.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// D-073 renamed every crate in this workspace. A crate whose `[package]`
    /// name drifts out of the family is a rename that was applied by hand and
    /// missed a spot — which is the exact failure that lost the name once
    /// already. Cheap to assert, so assert it.
    #[test]
    fn crate_is_part_of_the_spoonstill_family() {
        assert!(
            CRATE_NAME.starts_with("spoonstill-"),
            "expected a spoonstill-* crate, found {CRATE_NAME:?}"
        );
    }
}
