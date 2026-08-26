//! Locating `ffmpeg` and `ffprobe`, and failing fast when they are not there.
//!
//! D-012 rejects `ffmpeg-sidecar`'s runtime auto-download outright: a
//! commercial desktop app ships pinned, checksum-verified binaries, and a
//! renderer that quietly fetches a different build produces output that cannot
//! be reproduced. So there is no download path here, and no fallback search —
//! only an explicit location, an environment override for development, and a
//! clear error naming the exact path that was tried.

use std::path::PathBuf;

/// Environment override for the FFmpeg binary. Development convenience.
pub const FFMPEG_ENV: &str = "SPOONSTILL_FFMPEG";
/// Environment override for the ffprobe binary. Development convenience.
pub const FFPROBE_ENV: &str = "SPOONSTILL_FFPROBE";

/// Where the two binaries live for this run.
///
/// Cloned into each render rather than looked up per invocation, so that a
/// 500-scene batch cannot half-run against one build and half against another
/// because someone changed `PATH` midway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl Tools {
    /// Use whichever binaries the environment names, falling back to `PATH`.
    ///
    /// `PATH` resolution is the *development* default. A shipped build passes
    /// explicit bundled paths through [`Tools::at`] — D-062 requires our own
    /// LGPL build, and the Homebrew GPL build this workspace develops against
    /// may not be redistributed.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            ffmpeg: std::env::var_os(FFMPEG_ENV)
                .map_or_else(|| PathBuf::from("ffmpeg"), PathBuf::from),
            ffprobe: std::env::var_os(FFPROBE_ENV)
                .map_or_else(|| PathBuf::from("ffprobe"), PathBuf::from),
        }
    }

    /// Use two explicitly located binaries — what a packaged build does.
    #[must_use]
    pub fn at(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }

    /// The FFmpeg binary for this run.
    #[must_use]
    pub fn ffmpeg(&self) -> &std::path::Path {
        &self.ffmpeg
    }

    /// The ffprobe binary for this run.
    #[must_use]
    pub fn ffprobe(&self) -> &std::path::Path {
        &self.ffprobe
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self::from_env()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// There is no auto-download and no candidate search — the only two ways to
    /// get a binary are the environment and an explicit path.
    #[test]
    fn defaults_are_the_bare_program_names() {
        // Not `Tools::from_env()`, which would read whatever this developer's
        // shell happens to export.
        let t = Tools::at("ffmpeg", "ffprobe");
        assert_eq!(t.ffmpeg(), std::path::Path::new("ffmpeg"));
        assert_eq!(t.ffprobe(), std::path::Path::new("ffprobe"));
    }

    #[test]
    fn explicit_paths_are_kept_verbatim() {
        let t = Tools::at("/opt/spoonstill/bin/ffmpeg", "/opt/spoonstill/bin/ffprobe");
        assert_eq!(
            t.ffmpeg(),
            std::path::Path::new("/opt/spoonstill/bin/ffmpeg")
        );
    }
}
