//! Writing a file by writing another one first (D-042).
//!
//! Every artifact this crate produces — a segment, a normalized narration, a
//! generated silence, the finished film — is written to a temporary path
//! *beside* its destination and moved into place only once it has been
//! validated. Nothing ever writes directly to a path a later run might trust.
//!
//! Two details that are easy to get wrong and expensive to get wrong:
//!
//! - **Beside, not in the system temp directory.** A rename within one
//!   filesystem is atomic; a rename across two is a copy, and a copy can be
//!   interrupted halfway.
//! - **Unique per writer, not per process.** Two scenes can share one audio
//!   file, which under D-043 means one cache key and one destination. With
//!   only a process id in the temporary name, two workers rendering those two
//!   scenes at once would write the same temporary file — so there is a
//!   counter as well.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::MediaError;

/// Distinguishes two temporaries produced by one process at the same moment.
static WRITER: AtomicU64 = AtomicU64::new(0);

/// A temporary path beside `out`, keeping its extension.
///
/// The extension is kept because FFmpeg picks its muxer from it: a segment
/// written to `.partial` rather than `.partial.mp4` is not an MP4 at all.
#[must_use]
pub fn partial_path(out: &Path) -> PathBuf {
    let stem = out
        .file_name()
        .map_or_else(|| "artifact".into(), OsString::from);
    let extension = out
        .extension()
        .map_or_else(|| "tmp".to_owned(), |e| e.to_string_lossy().into_owned());

    let writer = WRITER.fetch_add(1, Ordering::Relaxed);
    let mut name = OsString::from(".");
    name.push(&stem);
    name.push(format!(
        ".partial-{}-{writer}.{extension}",
        std::process::id()
    ));
    out.parent().unwrap_or(Path::new("")).join(name)
}

/// Create the directory an artifact is about to be written into.
///
/// # Errors
///
/// [`MediaError::Io`] naming the directory, because "permission denied" with
/// no path is not a diagnosis.
pub fn ensure_parent(path: &Path) -> Result<(), MediaError> {
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|source| MediaError::Io {
        doing: "creating the output directory",
        path: parent.to_path_buf(),
        source,
    })
}

/// Move a validated artifact to its real path.
///
/// `fs::rename` replaces the destination silently on Unix but fails on Windows
/// when it already exists, so the destination is removed first. D-071 puts
/// Windows in scope from M1, and this is the kind of difference that otherwise
/// surfaces as "works on my machine" three milestones later.
///
/// # Errors
///
/// [`MediaError::Io`] naming the destination.
pub fn move_into_place(from: &Path, to: &Path) -> Result<(), MediaError> {
    if to.exists() {
        std::fs::remove_file(to).map_err(|source| MediaError::Io {
            doing: "replacing the existing file at",
            path: to.to_path_buf(),
            source,
        })?;
    }
    std::fs::rename(from, to).map_err(|source| MediaError::Io {
        doing: "moving the finished file to",
        path: to.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_temporary_sits_beside_the_destination_and_keeps_its_extension() {
        let temp = partial_path(Path::new("/renders/proj/seg-007.mp4"));
        assert_eq!(temp.parent().unwrap(), Path::new("/renders/proj"));
        assert_eq!(temp.extension().unwrap(), "mp4");
        assert_ne!(temp, PathBuf::from("/renders/proj/seg-007.mp4"));

        let temp = partial_path(Path::new("/cache/file-abc123.wav"));
        assert_eq!(temp.extension().unwrap(), "wav");
    }

    /// A bare filename has no parent, and must not produce an absolute path or
    /// a panic.
    #[test]
    fn a_bare_output_name_still_gets_a_temporary() {
        let temp = partial_path(Path::new("seg.mp4"));
        assert!(temp.is_relative(), "{}", temp.display());
        assert!(temp.to_string_lossy().contains("seg.mp4"));
    }

    /// The temporary must not look like a finished artifact to a later run.
    #[test]
    fn the_temporary_is_visibly_partial() {
        let temp = partial_path(Path::new("seg.mp4"));
        assert!(
            temp.to_string_lossy().contains("partial"),
            "{}",
            temp.display()
        );
    }

    /// The reason the counter exists: two workers writing the same destination
    /// — two scenes sharing one narration file, which D-043 gives one cache key
    /// — must not write the same temporary file.
    #[test]
    fn two_temporaries_for_one_destination_differ() {
        let a = partial_path(Path::new("/cache/file-abc123.wav"));
        let b = partial_path(Path::new("/cache/file-abc123.wav"));
        assert_ne!(a, b, "a per-process name is not enough with a worker pool");
    }

    #[test]
    fn a_missing_extension_still_produces_a_usable_name() {
        let temp = partial_path(Path::new("/renders/list"));
        assert_eq!(temp.extension().unwrap(), "tmp");
    }
}
