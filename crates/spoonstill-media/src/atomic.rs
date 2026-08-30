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

/// Move a validated artifact to its real path — in one step (D-119).
///
/// **One `rename`, and deliberately nothing before it.** This used to remove
/// the destination first, on the belief that `fs::rename` "replaces silently on
/// Unix but fails on Windows when it already exists". That is true of the raw
/// `MoveFile` API and of several other languages; it is **not** true of Rust,
/// whose `std::fs::rename` is documented as *"replacing the original file if
/// `to` already exists"* and which calls
/// `MoveFileExW(.., MOVEFILE_REPLACE_EXISTING)` on Windows. So the removal
/// bought nothing on either platform and cost two things:
///
/// - **A window with no artifact in it.** Between the unlink and the rename the
///   destination did not exist, so a crash there destroyed the previous good
///   file — and this function moves the finished *film* as well as cache
///   entries. Re-rendering over yesterday's film could lose yesterday's film.
/// - **A race that had to be handled.** Two workers finish one cache entry
///   whenever two scenes share a narration, which is the ordinary case at the
///   design point. Both saw `exists()`, both removed, and the loser failed the
///   render with "No such file or directory" about a file it had just been told
///   was there. That was patched during D-106; now it cannot arise, because
///   neither worker unlinks anything and `rename` is last-writer-wins.
///
/// The one behaviour to preserve deliberately: replacing is *atomic*, so a
/// reader of `to` sees the old artifact or the new one and never nothing.
///
/// # Errors
///
/// [`MediaError::Io`] naming the destination.
pub fn move_into_place(from: &Path, to: &Path) -> Result<(), MediaError> {
    std::fs::rename(from, to).map_err(|source| MediaError::Io {
        doing: "moving the finished file to",
        path: to.to_path_buf(),
        source,
    })
}

#[cfg(test)]
mod tests {
    /// Two workers finishing the same cache entry at the same moment must both
    /// succeed. Simulated rather than raced, because a real race reproduces
    /// only sometimes and a test that fails one run in twenty is a test people
    /// learn to re-run.
    ///
    /// **Kept, though the race it was written for can no longer happen**
    /// (D-119). It was added during D-106 for a loser that saw `exists()` and
    /// then lost the unlink; there is no unlink now, so both workers simply
    /// rename and the last one wins. What it still proves is the pair of cases
    /// that must both work either way — moving onto a path with nothing there,
    /// and moving onto one that already holds a file.
    #[test]
    fn a_destination_removed_by_another_worker_is_not_a_failure() {
        let dir = std::env::temp_dir().join(format!("spoonstill-move-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let destination = dir.join("shared.wav");
        let first = dir.join("first.partial");
        let second = dir.join("second.partial");

        std::fs::write(&destination, b"an earlier copy").expect("seed the destination");
        std::fs::write(&first, b"one").expect("write");
        std::fs::write(&second, b"two").expect("write");

        // Nothing at the destination — which used to be how the loser of the
        // race found it, and is now simply the first write of a cache entry.
        assert!(destination.exists());
        std::fs::remove_file(&destination).expect("the other worker got there first");
        move_into_place(&first, &destination).expect("a lost race is still a success");
        assert_eq!(std::fs::read(&destination).expect("read"), b"one");

        // And the ordinary case still replaces.
        move_into_place(&second, &destination).expect("replacing works");
        assert_eq!(std::fs::read(&destination).expect("read"), b"two");

        let _ = std::fs::remove_dir_all(&dir);
    }

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

    /// D-119. The destination is **never** absent while it is being replaced.
    ///
    /// This is the property the word "atomic" in this module's name claims, and
    /// it was not true: the old implementation unlinked `to` and then renamed
    /// over it, so anything looking at that path in between saw nothing. A
    /// crash there lost the previous artifact — and this function moves the
    /// finished film, not only cache entries.
    ///
    /// Watched rather than reasoned about: one thread replaces the file three
    /// hundred times while another does nothing but ask whether it is there.
    /// Against the old implementation this catches the gap almost immediately.
    #[test]
    fn a_replacement_never_leaves_the_destination_missing() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let dir = std::env::temp_dir().join(format!("spoonstill-atomic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let destination = dir.join("film.mp4");
        std::fs::write(&destination, b"the previous good artifact").expect("seed");

        let done = AtomicBool::new(false);
        let vanished = AtomicBool::new(false);

        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if !destination.exists() {
                        vanished.store(true, Ordering::Relaxed);
                        break;
                    }
                }
            });

            for i in 0..300 {
                let temp = dir.join(format!("next-{i}.partial"));
                std::fs::write(&temp, b"the replacement").expect("write");
                move_into_place(&temp, &destination).expect("replace");
            }
            done.store(true, Ordering::Relaxed);
        });

        assert!(
            !vanished.load(Ordering::Relaxed),
            "the destination disappeared during a replacement — a crash in that \
             window destroys the artifact that was already there"
        );
        assert!(destination.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
