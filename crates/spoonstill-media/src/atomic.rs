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

/// Whether `name` is a temporary this module wrote for `destination`.
///
/// The exact shape [`partial_path`] produces and nothing else:
/// `.<destination>.partial-<digits>-<digits>.<extension>`. It is deliberately
/// this strict, because the one place it is applied is the **operator's own
/// folder** — the film's destination is `~/Downloads`, not a cache directory
/// this program owns. A looser rule of the "starts with a dot and contains
/// `.partial-`" kind is safe inside `.spoonstill/` and is not safe here: it
/// would delete a stranger's `.notes.partial-backup.txt`.
///
/// Scoping it to one destination filename also keeps two renders into one
/// folder from touching each other's scaffolding.
#[must_use]
pub fn is_partial_of(name: &str, destination: &str) -> bool {
    let Some(rest) = name.strip_prefix('.') else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(destination) else {
        return false;
    };
    let Some(rest) = rest.strip_prefix(".partial-") else {
        return false;
    };
    // `<pid>-<writer>.<extension>`, where the extension is whatever
    // `partial_path` kept. Both numbers must be present and non-empty.
    let Some((pid, rest)) = rest.split_once('-') else {
        return false;
    };
    let writer = rest.split_once('.').map_or(rest, |(w, _)| w);
    !pid.is_empty()
        && !writer.is_empty()
        && pid.bytes().all(|b| b.is_ascii_digit())
        && writer.bytes().all(|b| b.is_ascii_digit())
}

/// Remove temporaries left beside `destination` by a run that never finished.
///
/// **Why this exists, in one sentence: an invisible leftover in the operator's
/// own folder is worse than a visible one**, because they cannot find it,
/// cannot delete it, and it is a whole 4K film's worth of bytes.
///
/// [`partial_path`] names its temporary with a leading dot so that a
/// half-written film is not mistaken for a finished one, is not indexed, and
/// is not picked up by a backup mid-write. That is right while the run is
/// alive and wrong the moment it is not: every path inside this process is
/// covered by [`Partial`], but a crash, a force quit or a power cut is not,
/// and nothing else ever looked in this directory. D-109's sweep covers
/// `.spoonstill/`; the film's destination is not in `.spoonstill/`.
///
/// **A partial something is still writing is left alone.** The render lock is
/// per *project* (D-113), and a destination folder is not a project — two
/// projects can be exporting to one path, at which point this would delete the
/// other run's film mid-write and turn a last-writer-wins overwrite into a
/// failed rename. A file being written has just been written to, so a partial
/// whose modification time is inside [`STILL_BEING_WRITTEN`] is skipped and
/// collected by a later run instead.
///
/// Returns the bytes reclaimed. **Never fails**: being unable to tidy is not a
/// reason to withhold a film.
pub fn sweep_partials(destination: &Path) -> u64 {
    sweep_partials_settled_before(destination, std::time::SystemTime::now())
}

/// How recently a partial must have been touched to be assumed alive.
///
/// A join writes continuously — it is a stream copy — so an in-flight partial's
/// modification time is always seconds old, and the only moment it is not is
/// between the last write and the rename, which is one probe apart. A minute is
/// far outside that and far inside "this was abandoned".
///
/// Being wrong in the generous direction costs one render's delay before the
/// litter goes; being wrong the other way deletes a film somebody is making.
const STILL_BEING_WRITTEN: std::time::Duration = std::time::Duration::from_secs(60);

/// [`sweep_partials`] with the clock supplied, so the age rule is testable
/// without sleeping for a minute (D-144's shape: the input that decides is a
/// parameter, not something a test has to arrange the world to produce).
fn sweep_partials_settled_before(destination: &Path, now: std::time::SystemTime) -> u64 {
    let (Some(dir), Some(name)) = (destination.parent(), destination.file_name()) else {
        return 0;
    };
    let Some(name) = name.to_str() else {
        return 0;
    };
    let dir = if dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        dir
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    let mut freed = 0;
    for entry in entries.flatten() {
        let Some(found) = entry.file_name().to_str().map(ToOwned::to_owned) else {
            continue;
        };
        if !is_partial_of(&found, name) {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        // Something is still writing this one.
        let recently_written = meta
            .modified()
            .ok()
            .and_then(|m| now.duration_since(m).ok())
            .is_none_or(|age| age < STILL_BEING_WRITTEN);
        if recently_written {
            continue;
        }
        if std::fs::remove_file(entry.path()).is_ok() {
            freed += meta.len();
        }
    }
    freed
}

/// A temporary that removes itself unless it is explicitly kept.
///
/// Every failure path in a write-beside-then-rename has to remove the
/// temporary, and there is always one more failure path than the author
/// remembered — the one that shipped was a failed `rename`, which left a
/// gigabyte of invisible MP4 in the operator's Downloads folder with nothing
/// in the product able to see it again. A guard cannot be skipped by a `?`
/// and runs while a panic unwinds, which is D-123's reasoning applied to the
/// one artifact an operator actually looks for.
#[derive(Debug)]
pub struct Partial {
    path: PathBuf,
    keep: bool,
}

impl Partial {
    /// Claim a temporary path beside `out`.
    #[must_use]
    pub fn beside(out: &Path) -> Self {
        Self {
            path: partial_path(out),
            keep: false,
        }
    }

    /// The path to write to.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Move the finished artifact into place, after which there is nothing to
    /// clean up.
    ///
    /// # Errors
    ///
    /// Whatever [`move_into_place`] reports — and the temporary is still
    /// removed, because a rename that failed leaves it exactly where the
    /// operator cannot see it.
    pub fn move_into_place(mut self, to: &Path) -> Result<(), MediaError> {
        let outcome = move_into_place(&self.path, to);
        self.keep = outcome.is_ok();
        outcome
    }
}

impl Drop for Partial {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_file(&self.path);
        }
    }
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

    /// D-178. The reported defect, at the unit it happens in.
    ///
    /// A failed rename used to leave the temporary sitting in the operator's
    /// own folder — dot-prefixed, so invisible in Finder, so impossible for
    /// them to find or delete. Renaming onto a path whose parent does not
    /// exist is the cheapest real failure to provoke.
    #[test]
    fn a_failed_rename_does_not_leave_an_invisible_file_behind() {
        let dir = scratch("failed-rename");
        let destination = dir.join("film.mp4");

        let partial = Partial::beside(&destination);
        let path = partial.path().to_path_buf();
        std::fs::write(&path, b"half a film").expect("write");
        assert!(path.exists());

        let nowhere = dir.join("no-such-folder").join("film.mp4");
        partial
            .move_into_place(&nowhere)
            .expect_err("renaming into a folder that does not exist must fail");

        assert!(
            !path.exists(),
            "a failed rename left {} where the operator cannot see it",
            path.display()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The guard runs on every exit, not only the ones somebody remembered —
    /// including an early `?` return and a panic unwinding through it.
    #[test]
    fn dropping_the_guard_removes_the_temporary() {
        let dir = scratch("dropped");
        let destination = dir.join("film.mp4");

        let path = {
            let partial = Partial::beside(&destination);
            std::fs::write(partial.path(), b"half a film").expect("write");
            partial.path().to_path_buf()
        };
        assert!(!path.exists(), "{}", path.display());

        // And the success case keeps it, or a finished film would delete
        // itself.
        let partial = Partial::beside(&destination);
        let path = partial.path().to_path_buf();
        std::fs::write(&path, b"a whole film").expect("write");
        partial.move_into_place(&destination).expect("rename");
        assert!(!path.exists());
        assert_eq!(std::fs::read(&destination).expect("read"), b"a whole film");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D-178. Litter from a run this process never saw — a crash, a force
    /// quit, a power cut — which nothing in the product could reach before.
    /// Four of these were in the reporting operator's folders at once.
    #[test]
    fn litter_from_a_dead_run_is_swept_and_a_strangers_file_is_not() {
        let dir = scratch("sweep");
        let destination = dir.join("ch 4.mp4");

        // Two abandoned films, exactly as the log shows them.
        let ours = [
            dir.join(".ch 4.mp4.partial-10767-183.mp4"),
            dir.join(".ch 4.mp4.partial-10767-185.mp4"),
        ];
        for path in &ours {
            std::fs::write(path, b"0123456789").expect("write");
        }

        // Everything a sweep of this folder must not touch: another render's
        // scaffolding, a stranger's dotfile that merely says "partial", the
        // operator's own film, and a file with our shape but no numbers.
        let spared = [
            dir.join(".other.mp4.partial-10767-1.mp4"),
            dir.join(".notes.partial-backup.txt"),
            dir.join("ch 4.mp4"),
            dir.join(".ch 4.mp4.partial-draft.mp4"),
        ];
        for path in &spared {
            std::fs::write(path, b"keep me").expect("write");
        }

        // A clock an hour on, so these count as settled rather than in flight.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3_600);
        let freed = sweep_partials_settled_before(&destination, later);

        assert_eq!(freed, 20, "the reclaimed bytes are reported");
        for path in &ours {
            assert!(!path.exists(), "left behind: {}", path.display());
        }
        for path in &spared {
            assert!(path.exists(), "deleted somebody else's: {}", path.display());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A partial another run is still writing is not litter (D-178).
    ///
    /// The render lock is per **project**, and a destination folder is not a
    /// project: two projects exporting to one path would have this delete the
    /// other's film mid-join, turning a last-writer-wins overwrite into a
    /// failed rename with an ENOENT nobody could explain. Found by reviewing
    /// this sweep rather than by it going wrong, which is the only reason it
    /// is not in the log with everything else here.
    #[test]
    fn a_film_another_run_is_still_writing_is_left_alone() {
        let dir = scratch("in-flight");
        let destination = dir.join("film.mp4");

        let in_flight = dir.join(".film.mp4.partial-4242-0.mp4");
        std::fs::write(&in_flight, b"being written right now").expect("write");

        // The real clock: the file was touched a moment ago, so something has
        // it open.
        let freed = sweep_partials(&destination);
        assert_eq!(freed, 0, "a film being written was swept");
        assert!(
            in_flight.exists(),
            "deleted a partial another render is still writing to"
        );

        // Once it has gone quiet for long enough, it is litter after all — so
        // the guard delays collection rather than preventing it.
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3_600);
        assert!(sweep_partials_settled_before(&destination, later) > 0);
        assert!(!in_flight.exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The predicate is the whole safety argument for deleting inside a folder
    /// this program does not own, so it is stated directly as well.
    #[test]
    fn only_our_own_scaffolding_for_this_destination_matches() {
        assert!(is_partial_of(".ch 4.mp4.partial-10767-183.mp4", "ch 4.mp4"));
        assert!(is_partial_of(".film.mp4.partial-1-0.mp4", "film.mp4"));

        for (name, destination) in [
            // Another destination in the same folder.
            (".other.mp4.partial-1-0.mp4", "film.mp4"),
            // A stranger's dotfile.
            (".notes.partial-backup.txt", "film.mp4"),
            // Ours in shape, but the counters are not numbers.
            (".film.mp4.partial-draft.mp4", "film.mp4"),
            (".film.mp4.partial--0.mp4", "film.mp4"),
            (".film.mp4.partial-1-.mp4", "film.mp4"),
            // The finished film itself, which is the one file that must never
            // match.
            ("film.mp4", "film.mp4"),
            // A prefix match that is not the whole name.
            (".film.mp4.backup.mp4", "film.mp4"),
        ] {
            assert!(
                !is_partial_of(name, destination),
                "{name:?} must not be swept as scaffolding for {destination:?}"
            );
        }
    }

    /// The name this sweep has to recognise is the name this module writes, so
    /// the two are pinned to each other rather than to a literal that could
    /// drift apart from it.
    #[test]
    fn what_partial_path_writes_is_what_the_sweep_recognises() {
        for destination in ["film.mp4", "ch 4.mp4", "a.b.c.mp4", "no-extension"] {
            let out = Path::new("/somewhere").join(destination);
            let temporary = partial_path(&out);
            let name = temporary.file_name().unwrap().to_str().unwrap();
            assert!(
                is_partial_of(name, destination),
                "{name:?} is written for {destination:?} and not recognised"
            );
        }
    }

    fn scratch(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spoonstill-{what}-{}-{}",
            std::process::id(),
            WRITER.load(Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

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
