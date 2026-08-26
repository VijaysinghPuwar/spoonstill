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
//! M1 fills this in: the `FfmpegCommand` builder, the retained child handle
//! with separate `quit()`/`kill()`/`wait()`, timed `ffprobe` JSON, and
//! `SEGMENT_PROFILE`.

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
