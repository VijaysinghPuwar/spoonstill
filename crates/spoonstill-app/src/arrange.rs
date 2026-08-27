//! Removing a scene, and moving one somewhere else (D-099).
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
pub fn move_to(root: &Path, id: &str, to: usize) -> Result<Vec<Scene>, ArrangeError> {
    let all = scenes(root)?;
    let from = position_of(&all, id)?;

    // A position past the end means the end, rather than an error: "move it to
    // the bottom" is a thing an operator means, and off-by-one is a thing an
    // operator does.
    let target = to.max(1).min(all.len()) - 1;

    let mut order = all.clone();
    let moving = order.remove(from);
    order.insert(target, moving);

    renumber(root, &order)?;
    scenes(root)
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
    let mut staged: Vec<(PathBuf, String, String)> = Vec::new();
    for (index, scene) in order.iter().enumerate() {
        let wanted = format!("{:0width$}", index + 1, width = width);
        for file in &scene.files {
            let extension = file
                .extension()
                .and_then(OsStr::to_str)
                .unwrap_or_default()
                .to_owned();
            let parked = root.join(format!(".arranging-{}-{}.{extension}", scene.id, extension));
            let parked = unique(parked);
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
}
