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
//! - **Nothing is deleted**, and `still remove` says so in as many words — so
//!   it has to be true of every rename here, not only of the ones aimed at
//!   `removed/`. It was not: pass one parks every file a *scene* owns, and a
//!   scene is its still, so a narration whose photograph was deleted in Finder
//!   belonged to nothing, was never parked, sat at a name a later scene wanted,
//!   and was replaced in silence by an ordinary `still move` (D-170). A
//!   renumber now refuses an occupied destination, before it touches anything.
//! - **Renaming is two passes**, and so is removing. Renumbering in place means
//!   `002` becoming `001` while `001` still exists. Everything moves to a
//!   temporary name first, then to its final one, so no rename ever lands on a
//!   file that is still wanted. A removal parks the same way for the same
//!   reason — renaming each file straight into `removed/` meant a failure on
//!   the second one left half a scene in the project and half in the bin, with
//!   the renumber never reached (D-170). A parked file carries what it was
//!   doing, so `recover` finishes the job (D-121).
//! - **The whole scene moves together.** A still and its script and its
//!   recording share a stem, so they are renamed as a set. Renaming the image
//!   alone would silently unpair the narration — a scene that still renders,
//!   with the wrong voice on it.

use std::collections::{BTreeMap, HashSet};
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
    /// A file nobody parked is sitting where a renumbered scene must go
    /// (D-170).
    ///
    /// Pass one vacates every name a *scene* owns, and a scene is its still —
    /// so a narration whose photograph is gone belongs to nothing, is never
    /// parked, and would be replaced in silence. A parked file is recoverable
    /// (D-121); an overwritten photograph is not.
    Occupied {
        /// The file that is in the way.
        path: PathBuf,
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
            ArrangeError::Occupied { path } => write!(
                f,
                "{} is not part of any scene, so renumbering this project \
                 would write over it. Move it out of the folder, or give it a \
                 still so it becomes a scene, and try again — `still validate` \
                 lists every file in that state.",
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
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

    // A numbered scene with words or a recording and no still is a scene
    // waiting for its picture (D-193), and it is arranged like any other. It
    // used to belong to nothing — D-170's orphan — which was right for a
    // narration whose photograph had been deleted and made every move refuse
    // in a project whose lines were imported before their pictures. A stem
    // that is not a number is still not a scene, and still not renumbered.
    for file in &everything {
        if !(has_extension(file, &AUDIO_EXTENSIONS) || has_extension(file, &TEXT_EXTENSIONS)) {
            continue;
        }
        let Some(stem) = file.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        let Ok(number) = stem.parse::<usize>() else {
            continue;
        };
        if let Some(scene) = scenes.iter_mut().find(|scene| scene.id == stem) {
            if !scene.files.contains(file) {
                scene.files.push(file.clone());
            }
            continue;
        }
        scenes.push(Scene {
            id: stem.to_owned(),
            number,
            files: vec![file.clone()],
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
    let kept: Vec<Scene> = all
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != at)
        .map(|(_, scene)| scene.clone())
        .collect();

    // Asked before a single file moves (D-170). `renumber` asks the same
    // question, but by the time it runs this scene is already in `removed/` —
    // and a command that refuses ought to refuse having done nothing.
    if let Some(blocking) = occupied_destination(root, &kept, &scene.files) {
        return Err(ArrangeError::Occupied { path: blocking });
    }

    // Two passes, for the reason `renumber` has two (D-170). A removal used to
    // rename each file straight into `removed/`, so a failure on the second one
    // left half a scene in the project and half in the bin, with `renumber`
    // never reached — a still with no narration, rendering in silence. Parking
    // first is a rename within one folder, and a parked file says what it was
    // doing, so `recover` finishes the removal rather than leaving a scene in
    // two places.
    let parked = park_for_removal(root, scene)?;
    for path in &parked {
        finish_removal(root, path)?;
    }

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
    // Nothing to do, and doing it anyway is not free (D-187). `still move
    // p 001 1` on a project already in that order parked and renamed all 500
    // files — measured at 0.09 s, and every `ctime` moved, which is what a
    // backup tool reads.
    //
    // **The condition is the names, not the position**, and that distinction
    // is the whole of it: a project of twenty scenes stemmed `0001`..`0020`
    // is in the right *order* and the wrong *width*, and the renumber is what
    // repairs it. An early return on "the position did not change" would skip
    // that repair in silence.
    //
    // `occupied_destination` is not reached from here, and provably need not
    // be: every destination is one of `order`'s own files, so every one of
    // them is in the `leaving` set that check skips. It cannot fire.
    if already_in_place(order) {
        return Ok(());
    }

    // Before anything moves (D-170). Pass one vacates every name a scene owns
    // and nothing else, so a file belonging to no scene sitting at a
    // destination would be replaced in pass two without a word. Refusing here
    // leaves the folder exactly as it was; refusing later would leave it
    // parked.
    if let Some(blocking) = occupied_destination(root, order, &[]) {
        return Err(ArrangeError::Occupied { path: blocking });
    }

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

    // Pass one is complete, and that is written down before anything is placed
    // (D-201): from here an interrupted run is finished, before it rolled back.
    // If this write fails, nothing has been placed and rolling back is right.
    let marker = root.join(PLACING_MARKER);
    fs::write(&marker, b"").map_err(|source| ArrangeError::Io {
        doing: "marking a renumber half done",
        path: marker.clone(),
        source,
    })?;

    // Pass two: into place. The check above makes an occupied destination
    // unreachable; this is the net under it, because the one thing this module
    // must never do is write over a photograph. A parked file is recoverable
    // (D-121) and an overwritten one is not, so it stays parked.
    for (parked, wanted, extension) in staged {
        let destination = root.join(format!("{wanted}.{extension}"));
        if destination.exists() {
            return Err(ArrangeError::Occupied { path: destination });
        }
        rename(&parked, &destination)?;
    }
    // Left behind only by a run that stops here, and `recover` removes it then.
    let _ = fs::remove_file(&marker);
    Ok(())
}

/// Whether every file already has the name [`renumber`] would give it.
///
/// Compared by **file name** rather than by whole path: `scenes` builds these
/// from the root it was handed, and a comparison that also had to agree about
/// how that root was spelled would be answering a second question.
fn already_in_place(order: &[Scene]) -> bool {
    let width = MIN_WIDTH.max(order.len().to_string().len());
    order.iter().enumerate().all(|(index, scene)| {
        let wanted = format!("{:0width$}", index + 1, width = width);
        scene.files.iter().all(|file| {
            let extension = file.extension().and_then(OsStr::to_str).unwrap_or_default();
            file.file_name().and_then(OsStr::to_str) == Some(&format!("{wanted}.{extension}"))
        })
    })
}

/// The first file that is sitting where a renumbered scene must go, if there is
/// one (D-170).
///
/// `vacating` names files that are about to leave their current names on top of
/// the ones `order` holds — a scene on its way to `removed/`, which is still in
/// the folder when the caller asks.
fn occupied_destination(root: &Path, order: &[Scene], vacating: &[PathBuf]) -> Option<PathBuf> {
    let width = MIN_WIDTH.max(order.len().to_string().len());

    let leaving: Vec<&PathBuf> = order
        .iter()
        .flat_map(|scene| scene.files.iter())
        .chain(vacating.iter())
        .collect();

    for (index, scene) in order.iter().enumerate() {
        let wanted = format!("{:0width$}", index + 1, width = width);
        for file in &scene.files {
            let extension = file.extension().and_then(OsStr::to_str).unwrap_or_default();
            let destination = root.join(format!("{wanted}.{extension}"));
            if destination.exists() && !leaving.iter().any(|path| **path == destination) {
                return Some(destination);
            }
        }
    }
    None
}

/// Move a scene's files out of the way, inside the project folder (D-170).
///
/// Returns where they were parked, in the order they must go on to `removed/`.
fn park_for_removal(root: &Path, scene: &Scene) -> Result<Vec<PathBuf>, ArrangeError> {
    let mut parked = Vec::with_capacity(scene.files.len());
    for file in &scene.files {
        let extension = file
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_owned();
        let stem = file
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or("scene")
            .to_owned();
        let target = unique(root.join(removing_name(&stem, &extension)));
        rename(file, &target)?;
        parked.push(target);
    }
    Ok(parked)
}

/// Put one parked file into `removed/`, under a name nothing else is using.
///
/// There is no rollback arm here, and that is the decision: the files that
/// travelled before the interruption are already in the bin under their final
/// names and carry no record of having been part of this scene, so finishing is
/// the only reading that leaves the scene whole in one place. Nothing is lost
/// either way — `removed/` is what the operator drags back from (D-100).
fn finish_removal(root: &Path, parked: &Path) -> Result<(), ArrangeError> {
    let bin = root.join(REMOVED_DIR);
    fs::create_dir_all(&bin).map_err(|source| ArrangeError::Io {
        doing: "making the removed folder",
        path: bin.clone(),
        source,
    })?;

    let name = parked
        .file_name()
        .and_then(OsStr::to_str)
        .and_then(removing_parts)
        .map_or_else(
            || ("scene".to_owned(), String::new()),
            |(stem, extension)| (stem, extension),
        );
    let (stem, extension) = name;

    // Removing scene 003 twice, after a renumber, means two different
    // photographs both called `003.jpeg`. The second keeps its own copy rather
    // than replacing the first.
    let mut destination = bin.join(format!("{stem}.{extension}"));
    let mut nth = 2;
    while destination.exists() {
        destination = bin.join(format!("{stem}-{nth}.{extension}"));
        nth += 1;
    }
    rename(parked, &destination)
}

/// The name a file wears on its way out of the project.
///
/// `.removing-<stem>.<ext>` — a dot so the folder scan ignores it (D-050), and
/// the stem it is leaving so `recover` can name it in the bin.
fn removing_name(stem: &str, extension: &str) -> String {
    format!(".removing-{stem}.{extension}")
}

/// What a file parked for removal says about itself: its stem, and its
/// extension.
fn removing_parts(name: &str) -> Option<(String, String)> {
    let rest = name.strip_prefix(".removing-")?;
    let (stem, extension) = rest.rsplit_once('.')?;
    // `unique` may have appended `-2`, `-3`… to avoid a collision with another
    // parked file; the bin has its own collision rule and applies it again.
    let stem = stem.rsplit_once('-').map_or(stem, |(head, tail)| {
        if tail.chars().all(|c| c.is_ascii_digit()) && !head.is_empty() {
            head
        } else {
            stem
        }
    });
    Some((stem.to_owned(), extension.to_owned()))
}

/// The name a file wears while it is between two numbers.
///
/// `.arranging-<from>-to-<wanted>.<ext>` — a dot so the folder scan ignores it
/// (D-050), and both ids so an interrupted run can be finished or undone
/// without guessing (D-121).
pub(crate) fn parked_name(from: &str, wanted: &str, extension: &str) -> String {
    format!(".arranging-{from}-to-{wanted}.{extension}")
}

/// Present while [`renumber`]'s pass two is placing files (D-201).
///
/// A parked name says where a file came from and where it was going, but not
/// which pass stopped. In pass one a destination is often free only because
/// its owner was parked a moment earlier, and finishing onto it strands that
/// owner with both of its names taken. So the phase is written down: no marker
/// means nothing was placed and every origin is still free, so recovery rolls
/// back; the marker means pass two had begun, so recovery finishes.
///
/// A dot so the folder scan ignores it, and not `.arranging-`, which
/// `still validate` counts as a file part-way through a rename.
pub(crate) const PLACING_MARKER: &str = ".arranging";

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
/// - **If the file's destination is free, put it there.** For a picture,
///   another image extension on that scene also occupies the slot (D-200).
///   That is pass two
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
/// A removal parks too (D-170), under `.removing-…`, and has only the first
/// branch: its files go on to `removed/`, which is also a name the operator can
/// see and is where the rest of that scene already is.
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
    let mut removing: Vec<PathBuf> = Vec::new();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if name.starts_with(".removing-") {
            removing.push(entry.path());
        } else if let Some((from, wanted, extension)) = parked_parts(&name) {
            parked.push((entry.path(), from, wanted, extension));
        }
    }
    // Deterministic, so two runs of a recovery resolve the same way.
    parked.sort_by(|a, b| a.0.cmp(&b.0));
    removing.sort();

    let mut restored = 0;

    // A removal that stopped half way. It is always finished rather than undone
    // — see `finish_removal` for why that is the reading that keeps the scene
    // in one place.
    for path in removing {
        finish_removal(root, &path)?;
        restored += 1;
    }

    // D-200: recovery asks whether the scene has a picture, not whether it
    // has this exact extension. Only scan for occupied slots when a picture
    // actually needs recovering: healthy reads pay no extra metadata probes.
    let mut picture_slots = HashSet::new();
    if parked
        .iter()
        .any(|(path, ..)| has_extension(path, &IMAGE_EXTENSIONS))
    {
        let entries = fs::read_dir(root).map_err(|source| ArrangeError::Io {
            doing: "reading picture slots for recovery",
            path: root.to_path_buf(),
            source,
        })?;
        for entry in entries {
            let path = entry
                .map_err(|source| ArrangeError::Io {
                    doing: "reading a picture slot for recovery",
                    path: root.to_path_buf(),
                    source,
                })?
                .path();
            if has_extension(&path, &IMAGE_EXTENSIONS)
                && path.is_file()
                && let Some(stem) = path.file_stem().and_then(OsStr::to_str)
                && stem.parse::<usize>().is_ok()
            {
                picture_slots.insert(stem.to_owned());
            }
        }
    }

    let mut groups: BTreeMap<_, Vec<_>> = BTreeMap::new();
    for (path, from, wanted, extension) in parked {
        groups
            .entry((from, wanted))
            .or_default()
            .push((path, extension));
    }
    // D-201: which pass stopped decides the direction for every file, so it is
    // decided once, before anything moves. Pass one only vacates names, so
    // while it runs every parked file's origin is free; an origin already
    // filled means a file was placed there. The marker says the same for a
    // renumber that wrote one. Deciding file by file finished onto names that
    // were free only because their owners had been parked.
    let marked = root.join(PLACING_MARKER);
    let placing = marked.exists()
        || groups
            .iter()
            .any(|((from, _), files)| slot_taken(root, from, files, &picture_slots));
    for ((from, wanted), mut files) in groups {
        let pictures = files
            .iter()
            .filter(|(path, _)| has_extension(path, &IMAGE_EXTENSIONS))
            .count();
        // Finish when pass two had begun; otherwise go back. The other name is
        // the fallback, for a file whose first choice is taken.
        let (first, second) = if placing {
            (&wanted, &from)
        } else {
            (&from, &wanted)
        };
        let together = if pictures > 0 {
            // A whole-scene renumber uses this journal too. Its narration
            // must follow its picture, even when the destination is silent.
            let occupied = |id: &String| slot_taken(root, id, &files, &picture_slots);
            let target_id = if occupied(first) { second } else { first };
            // Conflicting parked copies cannot both occupy one scene. Leave
            // them visible to validation instead of overwriting either one.
            let extensions: HashSet<_> = files
                .iter()
                .map(|(_, ext)| ext.to_ascii_lowercase())
                .collect();
            if pictures > 1 || extensions.len() != files.len() || occupied(target_id) {
                continue;
            }
            Some(target_id)
        } else {
            None
        };
        // Keep the picture parked until its companions are restored. If
        // recovery itself stops, the next call can still decide their route.
        files.sort_by_key(|(path, _)| has_extension(path, &IMAGE_EXTENSIONS));
        for (path, extension) in files {
            let target_id = together.unwrap_or_else(|| {
                if root.join(format!("{first}.{extension}")).exists() {
                    second
                } else {
                    first
                }
            });
            let target = root.join(format!("{target_id}.{extension}"));
            let picture = has_extension(&path, &IMAGE_EXTENSIONS);
            // Recheck before each rename; never overwrite a picture.
            if target.exists() || (picture && picture_slots.contains(target_id)) {
                continue;
            }
            rename(&path, &target)?;
            if picture {
                picture_slots.insert(target_id.clone());
            }
            restored += 1;
        }
    }

    // The marker outlives pass two only if a run stopped before removing it.
    // Once nothing is parked it describes nothing; while something still is,
    // it keeps telling the next recovery which way that file was going.
    if marked.exists() && !has_parked(root) {
        let _ = fs::remove_file(&marked);
    }
    Ok(restored)
}

/// Whether scene `id` already holds a file these parked files would land on:
/// the same name, or for a picture any picture on that scene (D-200).
fn slot_taken(
    root: &Path,
    id: &str,
    files: &[(PathBuf, String)],
    picture_slots: &HashSet<String>,
) -> bool {
    files.iter().any(|(path, extension)| {
        root.join(format!("{id}.{extension}")).exists()
            || (has_extension(path, &IMAGE_EXTENSIONS) && picture_slots.contains(id))
    })
}

/// Whether any file in the folder is still parked by a renumber.
fn has_parked(root: &Path) -> bool {
    fs::read_dir(root).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|entry| entry.file_name().to_str().and_then(parked_parts).is_some())
    })
}

/// A path nothing is using yet.
pub(crate) fn unique(candidate: PathBuf) -> PathBuf {
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

pub(crate) fn has_extension(path: &Path, allowed: &[&str]) -> bool {
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
        // The marker is what says pass two had begun (D-201).
        let project = Project::new("resume", &[]);
        fs::write(
            project.0.join(parked_name("003", "002", "jpg")),
            "the third photograph",
        )
        .expect("park");
        fs::write(project.0.join(PLACING_MARKER), "").expect("mark");

        let restored = recover(&project.0).expect("recovers");

        assert_eq!(restored, 1);
        assert_eq!(
            fs::read_to_string(project.0.join("002.jpg")).expect("read"),
            "the third photograph",
            "the file should have gone on to where it was headed"
        );
        assert!(parked(&project.0).is_empty());
        assert!(!project.0.join(PLACING_MARKER).exists());
    }

    #[test]
    fn an_unmarked_renumber_with_both_names_free_goes_back() {
        // No marker and nothing in either name: pass one may not have ended,
        // so the only reading that is right in both cases is "put it back"
        // (D-201). Either way the photograph is visible again.
        let project = Project::new("unmarked", &[]);
        fs::write(
            project.0.join(parked_name("003", "002", "jpg")),
            "the third photograph",
        )
        .expect("park");

        assert_eq!(recover(&project.0).expect("recovers"), 1);
        assert_eq!(
            fs::read_to_string(project.0.join("003.jpg")).expect("read"),
            "the third photograph"
        );
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

    /// Found by killing `still move` at random on the author's 431 scenes
    /// (D-201). Moving 005 to the front parks 005→001, 001→002, 002→003 … in
    /// film order. Stopped after the third park, 001 and 002 are free because
    /// their owners were parked, not because pass two had begun. Deciding each
    /// file alone, recovery moved 001 forward onto 002 and then found 002's own
    /// file with both of its names taken: parked for ever, and every arrange
    /// command refused the project.
    #[test]
    fn a_renumber_stopped_in_pass_one_rolls_back_whatever_is_free() {
        let project = Project::new(
            "pass-one-shift",
            &[("003", &["jpg", "txt"]), ("004", &["jpg", "txt"])],
        );
        for (from, wanted) in [("005", "001"), ("001", "002"), ("002", "003")] {
            for extension in ["jpg", "txt"] {
                fs::write(
                    project.0.join(parked_name(from, wanted, extension)),
                    format!("{from}.{extension}"),
                )
                .unwrap();
            }
        }

        recover(project.path()).unwrap();

        assert!(parked(project.path()).is_empty(), "{:?}", project.listing());
        for stem in ["001", "002", "003", "004", "005"] {
            for extension in ["jpg", "txt"] {
                assert_eq!(
                    fs::read_to_string(project.0.join(format!("{stem}.{extension}"))).unwrap(),
                    format!("{stem}.{extension}"),
                    "{stem}.{extension} is not where it started"
                );
            }
        }
    }

    /// Pass two is marked, so once anything has been placed recovery finishes
    /// rather than rolling back over a file that pass two put there.
    #[test]
    fn a_renumber_stopped_in_pass_two_is_finished() {
        // Order [005, 001, 002, 003, 004]; pass two has placed 005 at 001.
        let project = Project::new("pass-two-shift", &[]);
        fs::write(project.0.join("001.jpg"), "005.jpg").unwrap();
        fs::write(project.0.join("001.txt"), "005.txt").unwrap();
        fs::write(project.0.join(PLACING_MARKER), "").unwrap();
        for (from, wanted) in [
            ("001", "002"),
            ("002", "003"),
            ("003", "004"),
            ("004", "005"),
        ] {
            for extension in ["jpg", "txt"] {
                fs::write(
                    project.0.join(parked_name(from, wanted, extension)),
                    format!("{from}.{extension}"),
                )
                .unwrap();
            }
        }

        recover(project.path()).unwrap();

        assert!(parked(project.path()).is_empty(), "{:?}", project.listing());
        assert!(!project.0.join(PLACING_MARKER).exists());
        for (stem, was) in [
            ("001", "005"),
            ("002", "001"),
            ("003", "002"),
            ("004", "003"),
            ("005", "004"),
        ] {
            for extension in ["jpg", "txt"] {
                assert_eq!(
                    fs::read_to_string(project.0.join(format!("{stem}.{extension}"))).unwrap(),
                    format!("{was}.{extension}")
                );
            }
        }
    }

    #[test]
    fn a_parked_picture_keeps_its_narration_when_recovery_rolls_back() {
        // A whole-scene renumber uses the same journal as a picture swap.
        // The destination can be a silent scene with another image format:
        // its free .txt name must not draw this picture's narration away.
        for already_restored in [false, true] {
            let project = Project::new(
                &format!("paired-rollback-{already_restored}"),
                &[("002", &["png"])],
            );
            fs::write(project.0.join(parked_name("001", "002", "jpg")), "FIRST").unwrap();
            let narration = if already_restored {
                "001.txt".to_owned()
            } else {
                parked_name("001", "002", "txt")
            };
            fs::write(project.0.join(narration), "FIRST NARRATION").unwrap();

            recover(project.path()).unwrap();

            assert_eq!(
                fs::read_to_string(project.0.join("001.jpg")).unwrap(),
                "FIRST"
            );
            assert_eq!(
                fs::read_to_string(project.0.join("001.txt")).unwrap(),
                "FIRST NARRATION"
            );
            assert_eq!(
                fs::read_to_string(project.0.join("002.png")).unwrap(),
                "002.png"
            );
            assert!(!project.0.join("002.txt").exists());
            assert!(parked(project.path()).is_empty());
        }
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

    /// D-170. A renumber must never land on a file it did not park.
    ///
    /// Pass one parks every file that belongs to a *scene*, and a scene is
    /// defined by its still — so a narration whose photograph was deleted in
    /// Finder is never parked, sits at a name a later scene wants, and used to
    /// be replaced without a word. An orphan needs no failed removal to exist;
    /// one deleted photograph is enough.
    ///
    /// Reproduced before this existed: `still move proj 005 4` over the folder
    /// below left `004.txt` reading "005.txt" and the operator's own narration
    /// gone, while `still remove` printed "Nothing was deleted".
    #[test]
    fn a_renumber_never_overwrites_a_file_it_did_not_park() {
        let project = Project::new(
            "orphan",
            &[
                ("001", &["jpeg", "txt"]),
                ("002", &["jpeg", "txt"]),
                ("003", &["jpeg", "txt"]),
                ("005", &["jpeg", "txt"]),
            ],
        );
        // Something at a name a renumbered scene wants that belongs to no
        // scene. Since D-193 a numbered `.txt` *is* a scene — one waiting for
        // its picture — so the obstacle here is a folder of that name, which
        // nothing can make part of a scene and nothing may rename over.
        fs::create_dir(project.0.join("004.txt")).expect("mkdir");
        fs::write(project.0.join("004.txt").join("keep"), "SURVIVOR").expect("write");

        let error = move_to(project.path(), "005", 4).expect_err("refused, not silently");

        assert!(
            matches!(error, ArrangeError::Occupied { .. }),
            "it names the obstacle: {error}"
        );
        let said = error.to_string();
        assert!(said.contains("004.txt"), "{said}");
        assert_eq!(
            fs::read_to_string(project.0.join("004.txt").join("keep")).expect("still there"),
            "SURVIVOR",
        );

        // And nothing was touched on the way to refusing — asserted on the raw
        // listing, because `contents()` reads the folder through `scenes()`,
        // which runs `recover` and would put a parked folder right before the
        // assertion ever saw it (D-116).
        assert_eq!(
            project.listing(),
            vec![
                "001.jpeg", "001.txt", "002.jpeg", "002.txt", "003.jpeg", "003.txt", "005.jpeg",
                "005.txt",
            ],
            "the folder is exactly as it was (the listing is files only)"
        );
    }

    /// D-193. A numbered narration with no picture is a scene waiting for one,
    /// and a move carries it like any other scene — it used to be D-170's
    /// orphan, which made every move in a freshly imported chapter refuse.
    #[test]
    fn a_scene_waiting_for_its_picture_moves_with_the_others() {
        let project = Project::new(
            "waiting",
            &[
                ("001", &["jpeg", "txt"]),
                ("002", &["jpeg", "txt"]),
                ("003", &["jpeg", "txt"]),
                ("005", &["jpeg", "txt"]),
            ],
        );
        fs::write(project.0.join("004.txt"), "WAITING").expect("write");

        let all = scenes(project.path()).expect("numbered");
        assert_eq!(all.len(), 5, "the waiting line is a scene: {all:?}");

        move_to(project.path(), "005", 4).expect("moves");

        assert_eq!(
            project.listing(),
            vec![
                "001.jpeg", "001.txt", "002.jpeg", "002.txt", "003.jpeg", "003.txt", "004.jpeg",
                "004.txt", "005.txt",
            ],
        );
        assert_eq!(
            fs::read_to_string(project.0.join("005.txt")).expect("moved, not lost"),
            "WAITING"
        );
        assert_ne!(
            fs::read_to_string(project.0.join("004.txt")).expect("read"),
            "WAITING",
            "the picture's own line travelled with it"
        );
    }

    /// The same through `still remove`, which prints "Nothing was deleted".
    #[test]
    fn a_removal_carries_a_scene_waiting_for_its_picture() {
        let project = Project::new(
            "waiting-remove",
            &[
                ("001", &["jpeg", "txt"]),
                ("002", &["jpeg", "txt"]),
                ("003", &["jpeg", "txt"]),
                ("005", &["jpeg", "txt"]),
                ("006", &["jpeg", "txt"]),
            ],
        );
        fs::write(project.0.join("004.txt"), "WAITING").expect("write");

        let removed = remove(project.path(), "001").expect("removes");

        assert_eq!(removed.remaining, 5);
        assert_eq!(
            fs::read_to_string(project.0.join("003.txt")).expect("renumbered, not lost"),
            "WAITING"
        );
        assert!(project.path().join(REMOVED_DIR).join("001.jpeg").exists());
    }

    /// A picture-less file whose name is not a scene number is still nobody's
    /// scene, and is never renamed.
    #[test]
    fn an_unnumbered_line_is_not_a_scene() {
        let project = three();
        fs::write(project.0.join("notes.txt"), "mine").expect("write");
        let all = scenes(project.path()).expect("numbered");
        assert_eq!(all.len(), 3);
        move_to(project.path(), "003", 1).expect("moves");
        assert_eq!(
            fs::read_to_string(project.0.join("notes.txt")).expect("untouched"),
            "mine"
        );
    }

    /// D-170. A removal that stops half way must not leave a scene in two
    /// places.
    ///
    /// The interrupted state is built by calling the first phase and stopping,
    /// which is exactly what a killed process leaves — and is portable, unlike
    /// making a rename fail, which on this platform needs `chflags uchg` and on
    /// Windows needs something else entirely (D-090, D-155).
    #[test]
    fn an_interrupted_removal_is_finished_rather_than_left_in_two_places() {
        let project = three();
        let all = scenes(project.path()).expect("numbered");

        let staged = park_for_removal(project.path(), &all[1]).expect("parked");
        assert_eq!(staged.len(), 2, "the still and its script");

        // This is the state a kill leaves: nothing in `removed/`, nothing at
        // `002.*`, and two files under names the folder scan skips.
        assert!(!project.path().join(REMOVED_DIR).exists());
        assert_eq!(
            project.listing(),
            vec![
                ".removing-002.jpeg",
                ".removing-002.txt",
                "001.jpeg",
                "001.txt",
                "003.jpeg",
            ]
        );

        assert_eq!(recover(project.path()).expect("recovers"), 2);

        let bin = project.path().join(REMOVED_DIR);
        let mut kept: Vec<String> = fs::read_dir(&bin)
            .expect("the removed folder")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        kept.sort();
        assert_eq!(kept, vec!["002.jpeg", "002.txt"], "whole, and in one place");
        assert!(
            project.listing().iter().all(|n| !n.contains("removing")),
            "and nothing is left parked: {:?}",
            project.listing()
        );
    }

    /// The half that already reached the bin stays there, and the parked half
    /// follows it — the scene ends whole, under the bin's own collision rule.
    #[test]
    fn a_removal_finishes_around_files_already_in_the_bin() {
        let project = three();
        let bin = project.path().join(REMOVED_DIR);
        fs::create_dir_all(&bin).expect("bin");
        // A photograph removed earlier, wearing the name this one wants.
        fs::write(bin.join("002.jpeg"), "an earlier photograph").expect("write");
        fs::write(
            project.0.join(removing_name("002", "jpeg")),
            "the interrupted one",
        )
        .expect("park");

        assert_eq!(recover(project.path()).expect("recovers"), 1);

        assert_eq!(
            fs::read_to_string(bin.join("002.jpeg")).expect("read"),
            "an earlier photograph",
            "the one that was already there is untouched"
        );
        assert_eq!(
            fs::read_to_string(bin.join("002-2.jpeg")).expect("read"),
            "the interrupted one"
        );
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

    // --- D-187: a renumber that would change nothing does nothing ---------

    /// The rule itself, which is about **names** and not about positions.
    ///
    /// Every row here is in the right order; only the third is in the right
    /// order *and* already named correctly. An early return keyed on the
    /// position would answer `true` to all three.
    #[test]
    fn already_in_place_is_about_the_names() {
        let project = Project::new("in-place", &[("001", &["jpeg"]), ("002", &["jpeg"])]);
        let order = scenes(project.path()).expect("scenes");
        assert!(
            already_in_place(&order),
            "a project that is already numbered 001, 002 needs no renumber"
        );

        // The same scenes in the same order, stemmed four wide. `MIN_WIDTH`
        // is three, so these are *wrong* and the renumber is the repair.
        let wide = Project::new("in-place-wide", &[("0001", &["jpeg"]), ("0002", &["jpeg"])]);
        let order = scenes(wide.path()).expect("scenes");
        assert!(
            !already_in_place(&order),
            "four-digit stems in a two-scene project are not in place"
        );

        // And a gap: 001, 003 is the right order and the wrong numbering.
        let gap = Project::new("in-place-gap", &[("001", &["jpeg"]), ("003", &["jpeg"])]);
        let order = scenes(gap.path()).expect("scenes");
        assert!(!already_in_place(&order), "001, 003 is not 001, 002");
    }

    /// D-187. A move that changes nothing renames nothing.
    ///
    /// The whole scene travels together, so a renumber of a 500-scene project
    /// is a thousand renames — measured at **0.06 s**, with every `ctime`
    /// moved, which is what a backup tool reads. Asserting that requires
    /// `ctime`, which only unix exposes; the rule itself is tested above on
    /// every platform, and what is checked here is that `renumber` is wired
    /// to it.
    ///
    /// **The instrument matters more than the assertion.** `stat -f %c` is
    /// whole seconds and the effect is about ten milliseconds wide, so the
    /// audit's own first attempt at this reported no change and read as a
    /// clean refutation. `st_ctime_nsec` has the resolution the effect needs.
    #[cfg(unix)]
    #[test]
    fn a_move_that_changes_nothing_touches_no_file() {
        use std::os::unix::fs::MetadataExt;

        let project = Project::new(
            "noop-move",
            &[
                ("001", &["jpeg", "txt"]),
                ("002", &["jpeg", "txt"]),
                ("003", &["jpeg", "txt"]),
            ],
        );

        let stamps = |root: &Path| -> Vec<(PathBuf, i64, i64)> {
            let mut all: Vec<_> = fs::read_dir(root)
                .expect("read")
                .flatten()
                .map(|entry| {
                    let meta = entry.metadata().expect("metadata");
                    (entry.path(), meta.ctime(), meta.ctime_nsec())
                })
                .collect();
            all.sort();
            all
        };

        let before = stamps(project.path());
        let moved =
            super::move_to(project.path(), "001", 1).expect("a move to where it already is");
        let after = stamps(project.path());

        assert_eq!(
            before, after,
            "a move that changes nothing renamed every file in the project"
        );
        // And it still answers the question it was asked.
        assert_eq!(moved.was, "001");
        assert_eq!(moved.now, "001");
        assert_eq!((moved.from, moved.to), (1, 1));
        assert_eq!(moved.scenes.len(), 3);
    }

    /// And the repair an early return must not skip.
    ///
    /// Codex's own run of `still move` on a fresh fixture normalised
    /// four-digit stems to three, which is the reason this decision is keyed
    /// on the names: *"the position did not change"* is true here, and doing
    /// nothing would leave the folder wrong. Run against an early return on
    /// position alone and seen to fail.
    #[test]
    fn a_move_that_changes_nothing_still_repairs_the_numbering() {
        let project = Project::new(
            "noop-repairs",
            &[
                ("0001", &["jpeg", "txt"]),
                ("0002", &["jpeg", "txt"]),
                ("0003", &["jpeg"]),
            ],
        );

        super::move_to(project.path(), "0001", 1).expect("a move to where it already is");

        assert_eq!(
            project.listing(),
            vec!["001.jpeg", "001.txt", "002.jpeg", "002.txt", "003.jpeg"],
            "the stems were left four wide"
        );
        // `contents` is provenance, not names: each file still says what it
        // was created as, which is how this proves photographs moved rather
        // than names being rewritten over other photographs.
        assert_eq!(
            project.contents(),
            vec!["0001.jpeg", "0002.jpeg", "0003.jpeg"]
        );
        // The whole scene travelled together, so the pairing survived.
        assert_eq!(
            fs::read_to_string(project.path().join("002.txt")).unwrap(),
            "0002.txt"
        );
    }

    /// What this module does **not** check on Windows, said out loud.
    ///
    /// D-179's rule: a test that simply compiles to nothing on a platform is
    /// a silent hole, and this project has been bitten by exactly that. No
    /// platform exposes a portable "this file was renamed" signal — a rename
    /// changes neither mtime nor size — so the end-to-end assertion needs
    /// `ctime` and `ctime` is unix's. The rule itself runs everywhere.
    #[cfg(not(unix))]
    #[test]
    fn the_untouched_file_check_is_unix_only_and_says_so() {
        eprintln!(
            "D-187's `a_move_that_changes_nothing_touches_no_file` needs ctime, which \
             Windows does not expose. The rule it is wired to is covered here by \
             already_in_place_is_about_the_names and \
             a_move_that_changes_nothing_still_repairs_the_numbering, which run on \
             every platform."
        );
    }

    /// A removal still renumbers, which is the other caller of the rule.
    ///
    /// Taking a scene out makes every scene after it non-canonical, so the
    /// early return must never fire there — and if it did, the folder would
    /// be left with a gap.
    #[test]
    fn taking_a_scene_out_still_closes_the_gap() {
        let project = Project::new(
            "noop-remove",
            &[("001", &["jpeg"]), ("002", &["jpeg"]), ("003", &["jpeg"])],
        );

        super::remove(project.path(), "002").expect("removed");

        assert_eq!(
            project.listing(),
            vec!["001.jpeg", "002.jpeg"],
            "the gap was left open"
        );
        assert_eq!(
            fs::read_to_string(project.path().join("002.jpeg")).unwrap(),
            "003.jpeg",
            "the scene after the gap did not move up"
        );
    }
}
