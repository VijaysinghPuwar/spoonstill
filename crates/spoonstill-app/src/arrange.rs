//! Removing a scene, and moving one somewhere else (D-100).
//!
//! The window could make a project and fill it, and then could do nothing to it
//! but render. A photo imported twice stayed twice; a scene that belonged third
//! stayed eleventh. Both are one gesture in every tool an operator has ever
//! used, and neither existed here — so the answer to "I put that in the wrong
//! place" was to open Finder, rename eleven files by hand, and hope the
//! pairing survived. That is the same twenty minutes D-080 was written to
//! delete.
//!
//! ## A scene is a number, and the order *is* the numbers
//!
//! Under D-050's folder convention a scene is every file sharing a numeric
//! stem: `007.jpeg`, `007.txt`, `007.m4a`. Render order is the natural order of
//! those numbers. So there is nowhere to record "this one is third now" — the
//! name is the position, and moving a scene means renaming files.
//!
//! Which is why this module exists rather than a `position:` column: the
//! convention is load-bearing, ingest depends on it, and a second source of
//! truth for order would be a second thing to disagree.
//!
//! ## Rules
//!
//! - **Only where the convention holds.** If a still's stem is not a number,
//!   this refuses rather than renumbering a project whose order came from
//!   somewhere else. Manifest mode has its own order — the CSV's — and that
//!   file is the operator's to edit (D-050).
//! - **Nothing is deleted.** A removed scene's files move to `removed/` inside
//!   the project, which the folder scan never looks in (it reads one level and
//!   takes files only). The operator drags them back if they were wrong, and
//!   nothing this program does can lose their photograph.
//! - **Renaming is two passes.** Renumbering in place means `002` becoming
//!   `001` while `001` still exists. Everything moves to a temporary name
//!   first, then to its final one, so no rename ever lands on a file that is
//!   still wanted.
//! - **The whole scene moves together.** A still and its script and its
//!   recording share a stem, so they are renamed as a set. Renaming the image
//!   alone would silently unpair the narration — a scene that still renders,
//!   with the wrong voice on it.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::import::rows::{AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, TEXT_EXTENSIONS};

/// Where a removed scene goes. Inside the project, beside the stills, and
/// never scanned — the folder walk reads one level and takes only files.
pub const REMOVED_DIR: &str = "removed";

/// The smallest number of digits a renumbered scene gets, matching `ingest`.
const MIN_WIDTH: usize = 3;

/// Why an arrangement did not happen.
#[derive(Debug)]
pub enum ArrangeError {
    /// A still in this project is not numbered, so the order is not ours to
    /// rewrite.
    NotNumbered {
        /// The file that is not `001.jpg`-shaped.
        file: PathBuf,
    },
    /// There is no scene at that position.
    NoSuchScene {
        /// What was asked for.
        id: String,
        /// How many there are.
        count: usize,
    },
    /// The folder could not be read or written.
    Io {
        /// What we were doing.
        doing: &'static str,
        /// Which path.
        path: PathBuf,
        /// The operating system's reason.
        source: std::io::Error,
    },
}

impl std::fmt::Display for ArrangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArrangeError::NotNumbered { file } => write!(
                f,
                "{} is not a numbered scene, so this project's order is not \
                 spoonstill's to rewrite. Rename its stills 001, 002, … (or use \
                 `still add`, which does), and the order becomes editable.",
                file.file_name()
                    .unwrap_or(file.as_os_str())
                    .to_string_lossy()
            ),
            ArrangeError::NoSuchScene { id, count } => write!(
                f,
                "there is no scene {id} — this project has {count} of them"
            ),
            ArrangeError::Io {
                doing,
                path,
                source,
            } => write!(f, "{doing} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for ArrangeError {}

/// What one scene is made of: the still, and whatever shares its stem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scene {
    /// The numeric stem, as written — `007`.
    pub id: String,
    /// The number it sorts by.
    pub number: usize,
    /// Every file that belongs to it, still first.
    pub files: Vec<PathBuf>,
}

/// What a removal moved out of the way.
#[derive(Debug, Clone)]
pub struct Removed {
    /// The scene as it was named before.
    pub id: String,
    /// Where its files went, so the caller can say so.
    pub moved_to: PathBuf,
    /// How many files went with it.
    pub files: usize,
    /// How many scenes are left.
    pub remaining: usize,
}

/// Every scene in the folder, in render order.
///
/// # Errors
///
/// [`ArrangeError::Io`] if the folder cannot be read, or
/// [`ArrangeError::NotNumbered`] if a still does not follow the convention.
pub fn scenes(root: &Path) -> Result<Vec<Scene>, ArrangeError> {
    // Before anything is read, not only before anything is written (D-121).
    // Every arrange operation starts here, so a folder left half-renamed by an
    // interrupted run is put right the next time anyone so much as looks at it.
    recover(root)?;

    let mut everything: Vec<PathBuf> = fs::read_dir(root)
        .map_err(|source| ArrangeError::Io {
            doing: "reading the project folder",
            path: root.to_path_buf(),
            source,
        })?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    everything.sort();

    let mut scenes: Vec<Scene> = Vec::new();
    for still in everything
        .iter()
        .filter(|path| has_extension(path, &IMAGE_EXTENSIONS))
    {
        let stem = still
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_owned();
        let number = stem
            .parse::<usize>()
            .map_err(|_| ArrangeError::NotNumbered {
                file: still.clone(),
            })?;

        // The still leads; its script and recording follow. Anything else that
        // shares the stem comes too — it is part of the scene by the same rule.
        let mut files = vec![still.clone()];
        files.extend(
            everything
                .iter()
                .filter(|path| *path != still)
                .filter(|path| path.file_stem().and_then(OsStr::to_str) == Some(stem.as_str()))
                .filter(|path| {
                    has_extension(path, &AUDIO_EXTENSIONS) || has_extension(path, &TEXT_EXTENSIONS)
                })
                .cloned(),
        );

        scenes.push(Scene {
            id: stem,
            number,
            files,
        });
    }

    scenes.sort_by_key(|scene| scene.number);
    Ok(scenes)
}

/// Take one scene out of the project, keeping its files.
///
/// # Errors
///
/// [`ArrangeError`] — an unnumbered project, an id that is not there, or the
/// filesystem refusing.
pub fn remove(root: &Path, id: &str) -> Result<Removed, ArrangeError> {
    let all = scenes(root)?;
    let at = position_of(&all, id)?;

    let bin = root.join(REMOVED_DIR);
    fs::create_dir_all(&bin).map_err(|source| ArrangeError::Io {
        doing: "making the removed folder",
        path: bin.clone(),
        source,
    })?;

    let scene = &all[at];
    for file in &scene.files {
        let name = file.file_name().unwrap_or_default();
        let mut destination = bin.join(name);
        // Removing scene 003 twice, after a renumber, means two different
        // photographs both called `003.jpeg`. The second keeps its own copy
        // rather than replacing the first.
        let mut nth = 2;
        while destination.exists() {
            let stem = file.file_stem().and_then(OsStr::to_str).unwrap_or("scene");
            let extension = file.extension().and_then(OsStr::to_str).unwrap_or("");
            destination = bin.join(format!("{stem}-{nth}.{extension}"));
            nth += 1;
        }
        rename(file, &destination)?;
    }

    let kept: Vec<Scene> = all
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != at)
        .map(|(_, scene)| scene.clone())
        .collect();
    renumber(root, &kept)?;

    Ok(Removed {
        id: scene.id.clone(),
        moved_to: bin,
        files: scene.files.len(),
        remaining: kept.len(),
    })
}

/// Move the scene at `id` so that it ends up in position `to` (1-based).
///
/// # Errors
///
/// [`ArrangeError`] — an unnumbered project, an id that is not there, or the
/// filesystem refusing.
pub fn move_to(root: &Path, id: &str, to: usize) -> Result<Moved, ArrangeError> {
    let all = scenes(root)?;
    let from = position_of(&all, id)?;

    // A position past the end means the end, rather than an error: "move it to
    // the bottom" is a thing an operator means, and off-by-one is a thing an
    // operator does.
    let target = to.max(1).min(all.len()) - 1;

    let mut order = all.clone();
    let moving = order.remove(from);
    let was = moving.id.clone();
    order.insert(target, moving);

    renumber(root, &order)?;
    let after = scenes(root)?;

    // The scene the operator moved, under whatever number it now has. Named by
    // *position* rather than by id, because the renumber is what makes the id
    // meaningless as an answer to "where did it go" (D-150).
    let landed = after.get(target);
    Ok(Moved {
        was,
        now: landed.map_or_else(String::new, |scene| scene.id.clone()),
        from: from + 1,
        to: target + 1,
        files: landed.map(|scene| scene.files.clone()).unwrap_or_default(),
        scenes: after,
    })
}

/// Where a scene went, and what travelled with it (D-150).
///
/// `move_to` used to return only the scene list, and after a renumber every row
/// of that list reads `00N  00N.jpg` — so the one thing the operator wanted
/// confirmed, that the move happened, was the one thing it could not show.
/// `still remove`'s summary, by contrast, names what it did.
#[derive(Debug, Clone)]
pub struct Moved {
    /// The number the scene had before the move.
    pub was: String,
    /// The number it has now.
    pub now: String,
    /// Its position before, counting from one.
    pub from: usize,
    /// Its position after, counting from one.
    pub to: usize,
    /// Its files, under their new names — the whole scene travels together.
    pub files: Vec<PathBuf>,
    /// Every scene after the move, in film order.
    pub scenes: Vec<Scene>,
}

/// Where in the list a scene id sits.
fn position_of(all: &[Scene], id: &str) -> Result<usize, ArrangeError> {
    // Matched on the number rather than the text, so `7`, `07` and `007` all
    // name the same scene — an operator typing an id should not have to count
    // our leading zeros.
    let wanted = id.trim().parse::<usize>().ok();
    all.iter()
        .position(|scene| scene.id == id || Some(scene.number) == wanted)
        .ok_or_else(|| ArrangeError::NoSuchScene {
            id: id.to_owned(),
            count: all.len(),
        })
}

/// Rename every scene so the folder reads 001, 002, … in the order given.
///
/// Two passes, and the first one is not optional: renumbering in place means
/// writing `001` while the old `001` is still there, and a rename that lands on
/// a live file destroys it on Unix and fails on Windows (D-071 — the two
/// platforms disagree, and neither is acceptable).
fn renumber(root: &Path, order: &[Scene]) -> Result<(), ArrangeError> {
    let width = MIN_WIDTH.max(order.len().to_string().len());

    // Pass one: out of the way. The prefix is one no scene can have, because a
    // scene's stem must parse as a number.
    //
    // The parked name carries **where the file came from and where it is
    // going** (D-121). That is the whole journal: an interrupted renumber used
    // to leave files under names the folder scan ignores, so the scenes simply
    // disappeared and nothing could work out where they belonged. Measured on a
    // 2000-scene project, killed 120 ms in: 433 files parked, 434 scenes gone,
    // and `still validate` reporting "1566 scenes — no problems".
    let mut staged: Vec<(PathBuf, String, String)> = Vec::new();
    for (index, scene) in order.iter().enumerate() {
        let wanted = format!("{:0width$}", index + 1, width = width);
        for file in &scene.files {
            let extension = file
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned();
            let parked = unique(root.join(parked_name(&scene.id, &wanted, &extension)));
            rename(file, &parked)?;
            staged.push((parked, wanted.clone(), extension));
        }
    }

    // Pass two: into place.
    for (parked, wanted, extension) in staged {
        let destination = root.join(format!("{wanted}.{extension}"));
        rename(&parked, &destination)?;
    }
    Ok(())
}

/// The name a file wears while it is between two numbers.
///
/// `.arranging-<from>-to-<wanted>.<ext>` — a dot so the folder scan ignores it
/// (D-050), and both ids so an interrupted run can be finished or undone
/// without guessing (D-121).
fn parked_name(from: &str, wanted: &str, extension: &str) -> String {
    format!(".arranging-{from}-to-{wanted}.{extension}")
}

/// What a parked file says about itself: where it came from, where it was
/// going, and its extension.
fn parked_parts(name: &str) -> Option<(String, String, String)> {
    let rest = name.strip_prefix(".arranging-")?;
    let (stems, extension) = rest.rsplit_once('.')?;

    // `unique` may have appended `-2`, `-3`… to avoid a collision; the marker
    // is still the first `-to-`.
    if let Some((from, wanted)) = stems.split_once("-to-") {
        let wanted = wanted.split('-').next().unwrap_or(wanted);
        return Some((from.to_owned(), wanted.to_owned(), extension.to_owned()));
    }

    // The shape a build before D-121 wrote: `.arranging-<from>-<ext>.<ext>`,
    // which records where the file came from and not where it was going. There
    // are real projects carrying these — a folder damaged by the shipped
    // version has to be repairable by the version that fixes it — and the only
    // safe reading is "put it back", which is what `wanted == from` asks for.
    let from = stems.strip_suffix(&format!("-{extension}"))?;
    Some((from.to_owned(), from.to_owned(), extension.to_owned()))
}

/// Finish or undo a renumber that was interrupted (D-121).
///
/// Run before every operation, and before the project is read, so a folder is
/// never *shown* to anybody in the half-renamed state. The rule is one line and
/// it is decidable from the disk alone:
///
/// - **If the file's destination is free, put it there.** That is pass two
///   resuming. After pass one every numbered name has been vacated, so this is
///   always the branch taken when the interruption happened during pass two.
/// - **Otherwise put it back where it came from.** Its old name must be free,
///   because pass one moved it away and pass two had not yet begun to fill
///   anything in. That is the branch for an interruption during pass one, and
///   it is a rollback.
///
/// Either way the file ends up under a name the operator can see, which is the
/// property that actually matters: a photograph must never be invisible.
///
/// Returns how many files it put back.
///
/// # Errors
///
/// [`ArrangeError::Io`] if the filesystem refuses a rename.
pub fn recover(root: &Path) -> Result<usize, ArrangeError> {
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(0);
    };

    let mut parked: Vec<(PathBuf, String, String, String)> = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if let Some((from, wanted, extension)) = parked_parts(&name) {
            parked.push((entry.path(), from, wanted, extension));
        }
    }
    // Deterministic, so two runs of a recovery resolve the same way.
    parked.sort_by(|a, b| a.0.cmp(&b.0));

    let mut restored = 0;
    for (path, from, wanted, extension) in parked {
        let destination = root.join(format!("{wanted}.{extension}"));
        let target = if destination.exists() {
            // Occupied: pass one had not finished, so this is a rollback.
            root.join(format!("{from}.{extension}"))
        } else {
            destination
        };
        // If even that is taken there is nothing safe to do; leaving the file
        // parked is better than overwriting one of the operator's photographs.
        if target.exists() {
            continue;
        }
        rename(&path, &target)?;
        restored += 1;
    }
    Ok(restored)
}

/// A path nothing is using yet.
fn unique(candidate: PathBuf) -> PathBuf {
    if !candidate.exists() {
        return candidate;
    }
    let stem = candidate
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("parked")
        .to_owned();
    let extension = candidate
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_owned();
    let parent = candidate.parent().unwrap_or(Path::new("")).to_path_buf();
    for nth in 2..10_000 {
        let next = parent.join(format!("{stem}-{nth}.{extension}"));
        if !next.exists() {
            return next;
        }
    }
    candidate
}

fn rename(from: &Path, to: &Path) -> Result<(), ArrangeError> {
    fs::rename(from, to).map_err(|source| ArrangeError::Io {
        doing: "renaming",
        path: from.to_path_buf(),
        source,
    })
}

fn has_extension(path: &Path, allowed: &[&str]) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .map(str::to_ascii_lowercase)
        .is_some_and(|extension| allowed.contains(&extension.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project folder that looks like one `still add` made.
    struct Project(PathBuf);

    impl Project {
        fn new(name: &str, scenes: &[(&str, &[&str])]) -> Self {
            let root = std::env::temp_dir().join(format!(
                "spoonstill-arrange-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).expect("a project folder");
            for (stem, extensions) in scenes {
                for extension in *extensions {
                    // The content names the file it started as, so a test can
                    // prove a photograph moved rather than merely that a name
                    // exists.
                    fs::write(
                        root.join(format!("{stem}.{extension}")),
                        format!("{stem}.{extension}"),
                    )
                    .expect("write");
                }
            }
            Project(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// Every file in the folder, sorted — what `ls` would show.
        fn listing(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("readable")
                .flatten()
                .filter(|entry| entry.path().is_file())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }

        /// What each still is, by the content it was created with — the check
        /// that a renumber moved photographs and not just names.
        fn contents(&self) -> Vec<String> {
            scenes(&self.0)
                .expect("numbered")
                .iter()
                .map(|scene| fs::read_to_string(&scene.files[0]).expect("readable"))
                .collect()
        }
    }

    impl Drop for Project {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn three() -> Project {
        Project::new(
            "three",
            &[
                ("001", &["jpeg", "txt"]),
                ("002", &["jpeg", "txt"]),
                ("003", &["jpeg"]),
            ],
        )
    }

    #[test]
    fn a_scene_is_its_still_and_everything_sharing_its_stem() {
        let project = three();
        let all = scenes(project.path()).expect("numbered");

        assert_eq!(all.len(), 3);
        assert_eq!(all[0].id, "001");
        assert_eq!(all[0].files.len(), 2, "the still and its script");
        assert_eq!(all[2].files.len(), 1, "a silent scene is just the still");
        assert!(
            all[0].files[0].extension().is_some_and(|e| e == "jpeg"),
            "the still leads"
        );
    }

    /// The order is the numbers, so it has to come back in numeric order and
    /// not in whatever order the filesystem hands out.
    #[test]
    fn scenes_come_back_in_render_order() {
        let project = Project::new(
            "order",
            &[("010", &["jpeg"]), ("002", &["jpeg"]), ("1", &["jpeg"])],
        );
        let all = scenes(project.path()).expect("numbered");
        let ids: Vec<&str> = all.iter().map(|scene| scene.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["1", "002", "010"],
            "2 before 10, not '10' before '2'"
        );
    }

    #[test]
    fn removing_a_scene_keeps_its_files_and_closes_the_gap() {
        let project = three();
        let removed = remove(project.path(), "002").expect("a scene that is there");

        assert_eq!(removed.id, "002");
        assert_eq!(removed.files, 2, "the script went with the still");
        assert_eq!(removed.remaining, 2);

        assert_eq!(
            project.listing(),
            vec!["001.jpeg", "001.txt", "002.jpeg"],
            "renumbered contiguously, and the survivors keep their own scripts"
        );
        assert_eq!(
            project.contents(),
            vec!["001.jpeg", "003.jpeg"],
            "scene 2 is gone and scene 3 moved up — by content, not by name"
        );

        // Nothing was deleted.
        let bin = project.path().join(REMOVED_DIR);
        let mut kept: Vec<String> = fs::read_dir(&bin)
            .expect("the removed folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert_eq!(kept, vec!["002.jpeg", "002.txt"]);
    }

    /// The trap in "move to the removed folder": remove scene 3, everything
    /// renumbers, remove the new scene 3 — two different photographs both
    /// called `003.jpeg`. The second must not overwrite the first.
    #[test]
    fn removing_twice_never_overwrites_what_was_removed_before() {
        let project = Project::new(
            "twice",
            &[
                ("001", &["jpeg"]),
                ("002", &["jpeg"]),
                ("003", &["jpeg"]),
                ("004", &["jpeg"]),
            ],
        );
        remove(project.path(), "003").expect("first");
        remove(project.path(), "003").expect("second, a different photograph");

        let bin = project.path().join(REMOVED_DIR);
        let kept: Vec<String> = fs::read_dir(&bin)
            .expect("the removed folder")
            .flatten()
            .map(|entry| fs::read_to_string(entry.path()).expect("readable"))
            .collect();
        assert_eq!(kept.len(), 2, "both are kept");
        assert!(kept.contains(&"003.jpeg".to_owned()));
        assert!(kept.contains(&"004.jpeg".to_owned()), "{kept:?}");
    }

    #[test]
    fn a_scene_moves_to_a_new_position_and_takes_its_script_with_it() {
        let project = Project::new(
            "move",
            &[
                ("001", &["jpeg"]),
                ("002", &["jpeg"]),
                ("003", &["jpeg", "txt"]),
                ("004", &["jpeg"]),
            ],
        );

        move_to(project.path(), "003", 1).expect("to the front");

        assert_eq!(
            project.contents(),
            vec!["003.jpeg", "001.jpeg", "002.jpeg", "004.jpeg"],
            "the third photograph is now first"
        );
        // Its script came with it: scene 001 must now have a .txt and 002 must not.
        let all = scenes(project.path()).expect("numbered");
        assert_eq!(all[0].files.len(), 2, "the script followed its still");
        assert_eq!(
            fs::read_to_string(&all[0].files[1]).expect("readable"),
            "003.txt",
            "and it is the right script"
        );
        assert_eq!(all[1].files.len(), 1);
    }

    /// F-07. A move renumbers, so the scene list it used to print read
    /// `001  001.jpeg`, `002  002.jpeg`, … — the one thing an operator wanted
    /// confirmed was the one thing it could not show. `Moved` says which scene
    /// went where, and what travelled with it.
    #[test]
    fn a_move_reports_which_scene_went_where_and_what_went_with_it() {
        let project = Project::new(
            "reported",
            &[
                ("001", &["jpeg"]),
                ("002", &["jpeg"]),
                ("003", &["jpeg", "txt"]),
                ("004", &["jpeg"]),
            ],
        );

        let moved = move_to(project.path(), "003", 1).expect("to the front");

        assert_eq!(moved.was, "003", "the number it had");
        assert_eq!(moved.now, "001", "the number it has");
        assert_eq!((moved.from, moved.to), (3, 1), "counting from one");
        assert_eq!(moved.scenes.len(), 4);
        assert_eq!(moved.files.len(), 2, "the whole scene travelled");
        assert!(
            moved.files.iter().all(|f| f
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("001."))),
            "the files are named for where it landed: {:?}",
            moved.files
        );
        // And it is the right scene, not a different one that happens to be
        // first now: 003 was the only one with a script.
        assert_eq!(
            fs::read_to_string(&moved.files[1]).expect("readable"),
            "003.txt"
        );
    }

    /// Moving to the end reports the end, not the position that was asked for.
    #[test]
    fn a_move_past_the_end_reports_where_it_actually_landed() {
        let project = three();
        let moved = move_to(project.path(), "001", 99).expect("clamped");
        assert_eq!((moved.from, moved.to), (1, 3));
        assert_eq!(moved.was, "001");
        assert_eq!(moved.now, "003");
    }

    #[test]
    fn moving_backwards_and_to_the_end_both_work() {
        let project = Project::new(
            "both-ways",
            &[("001", &["jpeg"]), ("002", &["jpeg"]), ("003", &["jpeg"])],
        );

        move_to(project.path(), "001", 3).expect("to the end");
        assert_eq!(project.contents(), vec!["002.jpeg", "003.jpeg", "001.jpeg"]);

        move_to(project.path(), "003", 1).expect("the one now third, to the front");
        assert_eq!(project.contents(), vec!["001.jpeg", "002.jpeg", "003.jpeg"]);
    }

    /// Off-by-one is a thing an operator does, and "put it at the bottom"
    /// is a thing they mean. Neither is an error.
    #[test]
    fn a_position_past_the_end_means_the_end() {
        let project = three();
        move_to(project.path(), "001", 99).expect("clamped, not refused");
        assert_eq!(project.contents(), vec!["002.jpeg", "003.jpeg", "001.jpeg"]);

        move_to(project.path(), "003", 0).expect("clamped the other way");
        assert_eq!(project.contents(), vec!["001.jpeg", "002.jpeg", "003.jpeg"]);
    }

    /// Moving a scene to where it already is must be a no-op, not a shuffle.
    #[test]
    fn moving_a_scene_to_its_own_position_changes_nothing() {
        let project = three();
        let before = project.contents();
        move_to(project.path(), "002", 2).expect("no-op");
        assert_eq!(project.contents(), before);
        assert_eq!(project.listing().len(), 5);
    }

    /// An id is a number, however the operator spells it.
    #[test]
    fn an_id_is_matched_by_its_number_not_its_zeros() {
        let project = three();
        assert!(move_to(project.path(), "2", 1).is_ok(), "no leading zeros");
        assert_eq!(project.contents()[0], "002.jpeg");
    }

    #[test]
    fn a_scene_that_is_not_there_says_how_many_there_are() {
        let project = three();
        let error = remove(project.path(), "009").expect_err("no scene 9");
        let said = error.to_string();
        assert!(said.contains("009"), "{said}");
        assert!(said.contains("3 of them"), "{said}");
    }

    /// A project whose stills are named `opening.jpg` pairs by its own stems
    /// and has an order we did not give it. Renumbering it would be rewriting
    /// something the operator arranged.
    #[test]
    fn a_project_that_does_not_use_numbers_is_refused_rather_than_renumbered() {
        let project = Project::new("named", &[("opening", &["jpeg"]), ("middle", &["jpeg"])]);
        let error = scenes(project.path()).expect_err("not numbered");
        assert!(matches!(error, ArrangeError::NotNumbered { .. }), "{error}");
        let said = error.to_string();
        assert!(
            said.contains("still add"),
            "it says how to make it editable: {said}"
        );
        assert_eq!(project.listing().len(), 2, "and nothing was touched");
    }

    /// The reason renumbering is two passes. One pass would rename `002` onto
    /// the live `001` — which silently destroys a photograph on Unix and fails
    /// on Windows (D-071).
    #[test]
    fn renumbering_never_renames_onto_a_file_that_is_still_wanted() {
        let project = Project::new(
            "collide",
            &[
                ("001", &["jpeg"]),
                ("002", &["jpeg"]),
                ("003", &["jpeg"]),
                ("004", &["jpeg"]),
            ],
        );
        remove(project.path(), "001").expect("the first one");

        assert_eq!(
            project.contents(),
            vec!["002.jpeg", "003.jpeg", "004.jpeg"],
            "every photograph survived a full shift-by-one"
        );
        assert!(
            project
                .listing()
                .iter()
                .all(|name| !name.contains("arranging")),
            "no staging file is left behind: {:?}",
            project.listing()
        );
    }

    /// Past 999 scenes the width grows rather than the order breaking — the
    /// same rule `ingest` follows.
    #[test]
    fn the_width_grows_rather_than_the_order_breaking() {
        let names: Vec<String> = (1..=11).map(|n| format!("{n:03}")).collect();
        let scenes_spec: Vec<(&str, &[&str])> =
            names.iter().map(|n| (n.as_str(), &["jpeg"][..])).collect();
        let project = Project::new("eleven", &scenes_spec);

        move_to(project.path(), "011", 1).expect("the last one, to the front");
        let listing = project.listing();
        assert_eq!(listing.len(), 11);
        assert!(listing.contains(&"001.jpeg".to_owned()));
        assert!(listing.contains(&"011.jpeg".to_owned()));
        assert_eq!(project.contents()[0], "011.jpeg");
    }

    /// D-121. An interrupted renumber is finished or undone, never left.
    ///
    /// Reproduced end to end before this existed: a 2000-scene project, `still
    /// remove` killed 120 ms in, left **433 files parked and 434 scenes gone**,
    /// with `still validate` reporting "1566 scenes — no problems". The files
    /// were never deleted — D-100 held — but they were invisible, and no
    /// command could put them back.
    ///
    /// The states are built by hand rather than by racing a kill, because a
    /// test that reproduces one run in eight is a test people learn to re-run.
    #[test]
    fn an_interrupted_renumber_is_finished_when_its_destination_is_free() {
        // Pass two was under way: everything has been parked, so every
        // numbered name is vacant and the parked file can simply be placed.
        let project = Project::new("resume", &[]);
        fs::write(
            project.0.join(parked_name("003", "002", "jpg")),
            "the third photograph",
        )
        .expect("park");

        let restored = recover(&project.0).expect("recovers");

        assert_eq!(restored, 1);
        assert_eq!(
            fs::read_to_string(project.0.join("002.jpg")).expect("read"),
            "the third photograph",
            "the file should have gone on to where it was headed"
        );
        assert!(parked(&project.0).is_empty());
    }

    #[test]
    fn an_interrupted_renumber_is_undone_when_its_destination_is_taken() {
        // Pass one was under way: this file was parked, but the scene that
        // holds its destination has not been moved out of the way yet. Placing
        // it would overwrite a photograph, so it goes back where it came from.
        let project = Project::new("rollback", &[("002", &["jpg"])]);
        fs::write(
            project.0.join(parked_name("003", "002", "jpg")),
            "the third photograph",
        )
        .expect("park");

        let restored = recover(&project.0).expect("recovers");

        assert_eq!(restored, 1);
        assert_eq!(
            fs::read_to_string(project.0.join("002.jpg")).expect("read"),
            "002.jpg",
            "the photograph that was already there must be untouched"
        );
        assert_eq!(
            fs::read_to_string(project.0.join("003.jpg")).expect("read"),
            "the third photograph",
            "the parked file must go back to its own name"
        );
        assert!(parked(&project.0).is_empty());
    }

    /// A folder damaged by the build that shipped this bug has to be repairable
    /// by the build that fixes it. Those names carry only where the file came
    /// from, so the only safe reading is "put it back".
    #[test]
    fn leftovers_from_the_older_name_format_are_still_recovered() {
        let project = Project::new("legacy", &[]);
        fs::write(
            project.0.join(".arranging-0007-jpg.jpg"),
            "the seventh photograph",
        )
        .expect("park");

        assert_eq!(recover(&project.0).expect("recovers"), 1);
        assert_eq!(
            fs::read_to_string(project.0.join("0007.jpg")).expect("read"),
            "the seventh photograph"
        );
    }

    /// When neither name is free there is nothing safe to do, and leaving the
    /// file parked beats overwriting one of the operator's photographs. It is
    /// still reported, by `validate`, as an unfinished rename.
    #[test]
    fn a_file_with_nowhere_safe_to_go_is_left_alone_rather_than_overwriting() {
        let project = Project::new("stuck", &[("002", &["jpg"]), ("003", &["jpg"])]);
        fs::write(
            project.0.join(parked_name("003", "002", "jpg")),
            "a third copy",
        )
        .expect("park");

        assert_eq!(recover(&project.0).expect("recovers"), 0);
        assert_eq!(
            fs::read_to_string(project.0.join("002.jpg")).expect("read"),
            "002.jpg"
        );
        assert_eq!(
            fs::read_to_string(project.0.join("003.jpg")).expect("read"),
            "003.jpg"
        );
        assert_eq!(
            parked(&project.0).len(),
            1,
            "and it is still there to report"
        );
    }

    /// Nothing parked, nothing done — recovery runs before every operation, so
    /// it must be free and must not disturb a healthy folder.
    #[test]
    fn recovery_does_nothing_to_a_folder_that_is_not_mid_rename() {
        let project = Project::new("healthy", &[("001", &["jpg", "txt"]), ("002", &["jpg"])]);
        assert_eq!(recover(&project.0).expect("recovers"), 0);
        assert_eq!(scenes(&project.0).expect("scenes").len(), 2);
    }

    /// Every parked file in a folder.
    fn parked(root: &Path) -> Vec<String> {
        fs::read_dir(root)
            .expect("the folder")
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n.starts_with(".arranging-"))
            .collect()
    }
}
