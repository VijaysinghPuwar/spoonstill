//! Putting a picture on one particular scene, and taking it off again (D-194).
//!
//! D-193 made a line a scene that waits for its picture; this is how the
//! picture arrives — on the row the operator chose, not "after the last one".
//! The author's words: *"it should be clear where they are putting the image"*
//! and, when it lands on the wrong line, *"an option to remove the image or
//! move the image"*.
//!
//! The rules are `arrange`'s and `ingest`'s, because a picture is a scene's
//! file like any other:
//!
//! - **Nothing is deleted.** A picture that is replaced or removed goes to
//!   `removed/`, under a name that never overwrites what is already there.
//! - **Copied in, never moved** from where the operator keeps it (D-080), and
//!   never seen half-written (D-120): `ingest::copy_in` does the copy.
//! - **A swap is two renames journalled the way a renumber is** (D-121): each
//!   picture is parked under a name that says where it is going, so a swap
//!   interrupted half way is finished by `arrange::recover` the next time the
//!   project is read, rather than leaving one scene with no picture and the
//!   other with a file nobody can see.
//! - **Only where scenes are numbered**, which is `arrange`'s rule: the scene
//!   *is* its number, so a picture is put on a scene by naming it after one.

use std::collections::{BTreeMap, HashMap};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};

use crate::arrange::{self, ArrangeError, REMOVED_DIR, Scene};
use crate::import::rows::{AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, TEXT_EXTENSIONS};
use crate::import::{MediaCheck, Role};

/// Why a picture could not be placed, removed or moved.
#[derive(Debug)]
pub enum PictureError {
    /// The project could not be read as numbered scenes.
    Arrange(ArrangeError),
    /// The file is not a picture this program renders.
    NotAPicture {
        /// What was offered.
        path: PathBuf,
    },
    /// The scene has no picture to remove or move.
    NoPicture {
        /// The scene.
        scene: String,
    },
    /// The file has a picture's name and is not one this program can read —
    /// a download that stopped half way, a file renamed `.jpg`. It is taken
    /// back out, and the scene is left exactly as it was.
    Unusable {
        /// The file the operator offered.
        path: PathBuf,
        /// What the probe said, already in an operator's words (D-052).
        detail: String,
    },
    /// More pictures than scenes waiting for one from the scene they were
    /// dropped on.
    NotEnoughWaiting {
        /// Where they were dropped.
        from: String,
        /// How many pictures.
        pictures: usize,
        /// How many scenes from there on are waiting.
        waiting: usize,
    },
    /// Moving a picture onto the scene it is already on.
    SameScene {
        /// The scene.
        scene: String,
    },
    /// The copy or a rename failed.
    Io {
        /// What was being done.
        doing: String,
        /// The operating system's reason.
        detail: String,
    },
}

impl std::fmt::Display for PictureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PictureError::Arrange(e) => write!(f, "{e}"),
            PictureError::NotAPicture { path } => write!(
                f,
                "{} is not a picture — use a .jpg, .jpeg, .png, .webp, .heic, .tif \
                 or .bmp file",
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            ),
            PictureError::Unusable { path, detail } => write!(
                f,
                "{} could not be used as a picture: {detail} — nothing was changed",
                path.file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy()
            ),
            PictureError::NotEnoughWaiting {
                from,
                pictures,
                waiting,
            } => write!(
                f,
                "{pictures} pictures, and only {waiting} scene{} from {from} on {} \
                 waiting for one — nothing was changed",
                if *waiting == 1 { "" } else { "s" },
                if *waiting == 1 { "is" } else { "are" }
            ),
            PictureError::NoPicture { scene } => {
                write!(f, "scene {scene} has no picture")
            }
            PictureError::SameScene { scene } => {
                write!(f, "that picture is already on scene {scene}")
            }
            PictureError::Io { doing, detail } => write!(f, "{doing}: {detail}"),
        }
    }
}

impl std::error::Error for PictureError {}

impl From<ArrangeError> for PictureError {
    fn from(e: ArrangeError) -> Self {
        PictureError::Arrange(e)
    }
}

/// The scene called `id`, found by its number so `7`, `07` and `007` agree —
/// the same rule `still move` uses.
fn find(all: &[Scene], id: &str) -> Result<usize, PictureError> {
    let wanted = id.trim().parse::<usize>().ok();
    all.iter()
        .position(|s| s.id == id || Some(s.number) == wanted)
        .ok_or_else(|| {
            PictureError::Arrange(ArrangeError::NoSuchScene {
                id: id.to_owned(),
                count: all.len(),
            })
        })
}

/// A scene's picture, if it has one.
fn picture_of(scene: &Scene) -> Option<&PathBuf> {
    scene
        .files
        .iter()
        .find(|f| arrange::has_extension(f, &IMAGE_EXTENSIONS))
}

/// Move a file into `removed/`, never over anything already there.
fn to_bin(root: &Path, file: &Path) -> Result<PathBuf, PictureError> {
    let bin = root.join(REMOVED_DIR);
    fs::create_dir_all(&bin).map_err(|e| PictureError::Io {
        doing: "making the removed folder".to_owned(),
        detail: e.to_string(),
    })?;
    let destination = arrange::unique(bin.join(file.file_name().unwrap_or(file.as_os_str())));
    fs::rename(file, &destination).map_err(|e| PictureError::Io {
        doing: format!("moving {} to {REMOVED_DIR}/", file.display()),
        detail: e.to_string(),
    })?;
    Ok(destination)
}

/// The files an affected scene must still contain before Undo is safe (D-200).
///
/// Names identify positions, not persistent scenes. Content evidence includes
/// the narration as well as the picture, so a renumber cannot substitute a
/// different scene at the same name. Only affected files are hashed, streamed
/// through the same bounded reader used for render cache keys.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UndoState {
    root: PathBuf,
    scene: String,
    files: BTreeMap<OsString, String>,
}

fn file_hash(path: &Path) -> Result<String, PictureError> {
    spoonstill_media::scene::hash_file(path).map_err(|e| PictureError::Io {
        doing: format!("checking {} before a picture change", path.display()),
        detail: e.to_string(),
    })
}

fn real_root(root: &Path) -> Result<PathBuf, PictureError> {
    fs::canonicalize(root).map_err(|e| PictureError::Io {
        doing: "reading the project folder".to_owned(),
        detail: e.to_string(),
    })
}

impl UndoState {
    fn capture(root: &Path, scene: &Scene) -> Result<Self, PictureError> {
        let files = scene
            .files
            .iter()
            .map(|path| {
                let name = path.file_name().unwrap_or(path.as_os_str()).to_os_string();
                file_hash(path).map(|hash| (name, hash))
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            root: real_root(root)?,
            scene: scene.id.clone(),
            files,
        })
    }

    fn take(&mut self, path: &Path) -> Result<String, PictureError> {
        self.files
            .remove(path.file_name().unwrap_or(path.as_os_str()))
            .ok_or_else(|| PictureError::Io {
                doing: "checking the scene before a picture change".to_owned(),
                detail: "its files changed while they were being read — try again".to_owned(),
            })
    }

    fn matches(
        &self,
        root: &Path,
        files: &HashMap<OsString, Vec<PathBuf>>,
    ) -> Result<bool, PictureError> {
        if self.root != root {
            return Ok(false);
        }
        let actual = files
            .get(OsStr::new(&self.scene))
            .map_or(&[][..], Vec::as_slice);
        if actual.len() != self.files.len() {
            return Ok(false);
        }
        for path in actual {
            let name = path.file_name().unwrap_or(path.as_os_str());
            let Some(expected) = self.files.get(name) else {
                return Ok(false);
            };
            if file_hash(path)? != *expected {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// One directory walk per Undo, even for a five-hundred-picture fill. Files
/// are hashed only when a saved state checks its affected scene below.
fn current_files(root: &Path) -> Result<HashMap<OsString, Vec<PathBuf>>, PictureError> {
    let read_error = |e: std::io::Error| PictureError::Io {
        doing: "reading the project before Undo".to_owned(),
        detail: e.to_string(),
    };
    let mut files: HashMap<OsString, Vec<PathBuf>> = HashMap::new();
    for entry in fs::read_dir(root).map_err(read_error)? {
        let path = entry.map_err(read_error)?.path();
        if (arrange::has_extension(&path, &IMAGE_EXTENSIONS)
            || arrange::has_extension(&path, &AUDIO_EXTENSIONS)
            || arrange::has_extension(&path, &TEXT_EXTENSIONS))
            && path.is_file()
            && let Some(stem) = path.file_stem()
        {
            files.entry(stem.to_os_string()).or_default().push(path);
        }
    }
    Ok(files)
}

/// What putting a picture on a scene did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// The scene, as named in the folder.
    pub scene: String,
    /// The picture's name inside the project — `024.jpeg`.
    pub file: String,
    /// The picture it replaced, now in `removed/`, if there was one.
    pub replaced: Option<PathBuf>,
    /// The name the replaced picture had in the project, so it can go back.
    pub replaced_name: Option<String>,
    after: UndoState,
    replaced_hash: Option<String>,
}

impl Placed {
    /// One line, for the terminal and the window.
    #[must_use]
    pub fn summary(&self) -> String {
        match &self.replaced {
            None => format!("Scene {} has its picture.", self.scene),
            Some(old) => format!(
                "Scene {} has a new picture — the old one is in {REMOVED_DIR}/ as {}.",
                self.scene,
                old.file_name().unwrap_or(old.as_os_str()).to_string_lossy()
            ),
        }
    }
}

/// Put the picture at `source` on scene `id`, copying it in, and check it is
/// a picture that renders before keeping it.
///
/// A scene that already has a picture has it moved to `removed/` first, and
/// put back if the copy or the check then fails — so a failed replace leaves
/// the scene as it was. The check is `media`'s, so the window and the terminal
/// ask the same question `still validate` asks; a machine that cannot run it
/// (no FFmpeg, D-103) keeps the picture, and the project's own check says why.
///
/// # Errors
///
/// [`PictureError`] — not a picture, no such scene, an unnumbered project, or
/// the copy failing.
pub fn set(
    root: &Path,
    id: &str,
    source: &Path,
    media: &dyn MediaCheck,
) -> Result<Placed, PictureError> {
    if !arrange::has_extension(source, &IMAGE_EXTENSIONS) || !source.is_file() {
        return Err(PictureError::NotAPicture {
            path: source.to_path_buf(),
        });
    }
    let all = arrange::scenes(root)?;
    let scene = &all[find(&all, id)?];

    // Capture the evidence before moving anything: an unreadable source or
    // narration must fail without leaving a successful change with no Undo.
    let mut after = UndoState::capture(root, scene)?;
    let incoming_hash = file_hash(source)?;
    let replaced_hash = picture_of(scene).map(|old| after.take(old)).transpose()?;
    let replaced = match picture_of(scene) {
        Some(old) => Some((old.clone(), to_bin(root, old)?)),
        None => None,
    };
    let replaced_name = replaced
        .as_ref()
        .and_then(|(was, _)| was.file_name().and_then(OsStr::to_str).map(str::to_owned));

    let put_back = |replaced: Option<(PathBuf, PathBuf)>| {
        if let Some((was, binned)) = replaced {
            let _ = fs::rename(&binned, &was);
        }
    };
    match crate::ingest::copy_in(root, source, &scene.id) {
        Ok(copied) => {
            let landed = root.join(&copied.name);
            if media.ready().is_ok()
                && let Err(detail) = media.check(&landed, Role::Image)
            {
                let _ = fs::remove_file(&landed);
                put_back(replaced);
                return Err(PictureError::Unusable {
                    path: source.to_path_buf(),
                    detail,
                });
            }
            after
                .files
                .insert(OsString::from(&copied.name), incoming_hash);
            Ok(Placed {
                after,
                replaced_hash,
                scene: scene.id.clone(),
                file: copied.name,
                replaced: replaced.map(|(_, binned)| binned),
                replaced_name,
            })
        }
        Err(e) => {
            // Put the old picture back: a replace that failed must not leave
            // the scene with nothing.
            put_back(replaced);
            Err(PictureError::Io {
                doing: "copying the picture in".to_owned(),
                detail: e.to_string(),
            })
        }
    }
}

/// What putting several pictures down at once did.
#[derive(Debug)]
pub struct Filled {
    /// Each picture placed, in order.
    pub placed: Vec<Placed>,
    /// The picture that stopped it, and why, if one did. Everything before it
    /// is placed; nothing after it was tried.
    pub stopped: Option<PictureError>,
}

impl Filled {
    /// One line, for the terminal and the window.
    #[must_use]
    pub fn summary(&self) -> String {
        let n = self.placed.len();
        let mut line = match (self.placed.first(), self.placed.last()) {
            (Some(one), Some(_)) if n == 1 => format!("1 picture put on scene {}.", one.scene),
            (Some(first), Some(last)) => format!(
                "{n} pictures put on scenes {} to {}.",
                first.scene, last.scene
            ),
            _ => "No picture was placed.".to_owned(),
        };
        if let Some(why) = &self.stopped {
            line.push_str(&format!(" Stopped: {why}"));
        }
        line
    }
}

/// Put several pictures down at once (D-194): the first on scene `from` if it
/// is waiting for one, and each next one on the next scene **still waiting
/// for a picture** — never over a picture already there. In the order given,
/// which is the order the operator dragged them in.
///
/// Refused before anything changes when any file is not a picture, or when
/// fewer scenes are waiting than there are pictures: a drop that half lands is
/// the mistake this exists to make hard.
///
/// # Errors
///
/// [`PictureError`] for the refusals above; a picture that fails part-way is
/// in [`Filled::stopped`] instead, with everything before it placed.
pub fn fill(
    root: &Path,
    from: &str,
    sources: &[PathBuf],
    media: &dyn MediaCheck,
) -> Result<Filled, PictureError> {
    if let Some(bad) = sources
        .iter()
        .find(|s| !arrange::has_extension(s, &IMAGE_EXTENSIONS) || !s.is_file())
    {
        return Err(PictureError::NotAPicture { path: bad.clone() });
    }
    let all = arrange::scenes(root)?;
    let start = find(&all, from)?;
    let targets: Vec<String> = all[start..]
        .iter()
        .filter(|scene| picture_of(scene).is_none())
        .take(sources.len())
        .map(|scene| scene.id.clone())
        .collect();
    if targets.len() < sources.len() {
        return Err(PictureError::NotEnoughWaiting {
            from: all[start].id.clone(),
            pictures: sources.len(),
            waiting: targets.len(),
        });
    }

    let mut placed = Vec::with_capacity(sources.len());
    for (scene, source) in targets.iter().zip(sources) {
        match set(root, scene, source, media) {
            Ok(done) => placed.push(done),
            Err(why) => {
                return Ok(Filled {
                    placed,
                    stopped: Some(why),
                });
            }
        }
    }
    Ok(Filled {
        placed,
        stopped: None,
    })
}

/// Take scene `id`'s picture off it, into `removed/`. The scene's words stay,
/// so it becomes a scene waiting for a picture again (D-193).
///
/// Answers with the [`Change`] it made, which says where the picture went —
/// what the caller prints, and what [`undo`] needs.
///
/// # Errors
///
/// [`PictureError::NoPicture`] if it has none, or the filesystem refusing.
pub fn remove(root: &Path, id: &str) -> Result<Change, PictureError> {
    let all = arrange::scenes(root)?;
    let scene = &all[find(&all, id)?];
    let picture = picture_of(scene).ok_or_else(|| PictureError::NoPicture {
        scene: scene.id.clone(),
    })?;
    let name = picture
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_owned();
    let mut after = UndoState::capture(root, scene)?;
    let binned_hash = after.take(picture)?;
    let binned = to_bin(root, picture)?;
    Ok(Change::Removed {
        scene: scene.id.clone(),
        binned,
        name,
        after,
        binned_hash,
    })
}

/// One picture change, as it can be taken back (D-194).
///
/// Undo is how a fast operator recovers from a drop on the wrong row without
/// hunting through `removed/` — and like every other change here it deletes
/// nothing: a picture an undo takes off goes to `removed/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Pictures were put on scenes.
    Placed(Vec<Placed>),
    /// A picture was taken off a scene.
    Removed {
        /// The scene.
        scene: String,
        /// Where it went.
        binned: PathBuf,
        /// The name it had.
        name: String,
        /// What the affected scene must still contain.
        after: UndoState,
        /// Content evidence for the picture kept in `removed/`.
        binned_hash: String,
    },
    /// A picture moved, or two swapped.
    Moved(Moved),
}

/// Take back one [`Change`].
///
/// Refuses — and changes nothing — when the folder no longer looks the way the
/// change left it: a picture undone after something else replaced it would
/// otherwise put back a picture over the operator's newer choice.
///
/// # Errors
///
/// [`PictureError::Io`] naming what changed since, or the filesystem refusing.
pub fn undo(root: &Path, change: &Change) -> Result<String, PictureError> {
    let changed = |what: String| PictureError::Io {
        doing: "undoing".to_owned(),
        detail: format!("{what} changed since, so nothing was undone"),
    };
    let canonical = real_root(root)?;
    let files = current_files(root)?;
    match change {
        Change::Placed(placed) => {
            for one in placed {
                if !one.after.matches(&canonical, &files)? {
                    return Err(changed(format!("scene {}'s picture", one.scene)));
                }
                if let Some(name) = &one.replaced_name
                    && !match (&one.replaced, &one.replaced_hash) {
                        (Some(path), Some(hash)) => path.is_file() && file_hash(path)? == *hash,
                        _ => false,
                    }
                {
                    return Err(changed(format!("the old picture {name}")));
                }
            }
            for one in placed.iter().rev() {
                to_bin(root, &root.join(&one.file))?;
                if let (Some(binned), Some(name)) = (&one.replaced, &one.replaced_name) {
                    fs::rename(binned, root.join(name)).map_err(|e| PictureError::Io {
                        doing: format!("putting {name} back"),
                        detail: e.to_string(),
                    })?;
                }
            }
            Ok(match placed.as_slice() {
                [one] => format!("Undone — scene {} is as it was.", one.scene),
                many => format!("Undone — {} pictures taken back off.", many.len()),
            })
        }
        Change::Removed {
            scene,
            binned,
            name,
            after,
            binned_hash,
        } => {
            let back = root.join(name);
            if !after.matches(&canonical, &files)?
                || !binned.is_file()
                || back.exists()
                || file_hash(binned)? != *binned_hash
            {
                return Err(changed(format!("scene {scene}")));
            }
            fs::rename(binned, &back).map_err(|e| PictureError::Io {
                doing: format!("putting {name} back"),
                detail: e.to_string(),
            })?;
            Ok(format!("Undone — scene {scene} has its picture back."))
        }
        Change::Moved(moved) => {
            for after in &moved.after {
                if !after.matches(&canonical, &files)? {
                    return Err(changed(format!("scene {}", after.scene)));
                }
            }
            move_to(root, &moved.to, &moved.from)?;
            Ok(format!(
                "Undone — the picture is back on scene {}.",
                moved.from
            ))
        }
    }
}

/// What moving a picture did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    /// Where it came from.
    pub from: String,
    /// Where it went.
    pub to: String,
    /// Whether the scene it went to had a picture, which went the other way.
    pub swapped: bool,
    after: [UndoState; 2],
}

impl Moved {
    /// One line, for the terminal and the window.
    #[must_use]
    pub fn summary(&self) -> String {
        if self.swapped {
            format!("Scenes {} and {} swapped pictures.", self.from, self.to)
        } else {
            format!(
                "The picture moved from scene {} to scene {}.",
                self.from, self.to
            )
        }
    }
}

/// Move scene `from`'s picture onto scene `to`. If `to` already has one, the
/// two swap — the one correction an operator who put two pictures on each
/// other's lines needs, in one step rather than three.
///
/// # Errors
///
/// [`PictureError`] — no picture to move, the same scene, no such scene, or
/// the filesystem refusing.
pub fn move_to(root: &Path, from: &str, to: &str) -> Result<Moved, PictureError> {
    let all = arrange::scenes(root)?;
    let source = &all[find(&all, from)?];
    let target = &all[find(&all, to)?];
    if source.id == target.id {
        return Err(PictureError::SameScene {
            scene: source.id.clone(),
        });
    }
    let moving = picture_of(source)
        .ok_or_else(|| PictureError::NoPicture {
            scene: source.id.clone(),
        })?
        .clone();
    let other = picture_of(target).cloned();

    let extension = |p: &Path| {
        p.extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_owned()
    };
    let rename = |a: &Path, b: &Path| {
        fs::rename(a, b).map_err(|e| PictureError::Io {
            doing: format!("renaming {}", a.display()),
            detail: e.to_string(),
        })
    };

    let ours = extension(&moving);
    let mut after_source = UndoState::capture(root, source)?;
    let mut after_target = UndoState::capture(root, target)?;
    let moving_hash = after_source.take(&moving)?;
    if let Some(theirs) = &other {
        let other_hash = after_target.take(theirs)?;
        after_source.files.insert(
            OsString::from(format!("{}.{}", source.id, extension(theirs))),
            other_hash,
        );
    }
    after_target
        .files
        .insert(OsString::from(format!("{}.{ours}", target.id)), moving_hash);

    // Parked first, under names that say where each is going (D-121), then
    // into place. D-200 makes recovery treat another image extension as an
    // occupied picture slot too, so an interrupted first park rolls back.
    let parked_ours =
        arrange::unique(root.join(arrange::parked_name(&source.id, &target.id, &ours)));
    rename(&moving, &parked_ours)?;
    let parked_theirs = match &other {
        Some(theirs) => {
            let ext = extension(theirs);
            let parked =
                arrange::unique(root.join(arrange::parked_name(&target.id, &source.id, &ext)));
            rename(theirs, &parked)?;
            Some((parked, ext))
        }
        None => None,
    };
    // Both parked: from here an interruption is finished rather than undone,
    // and the disk alone cannot say so once neither name is taken (D-201).
    let marker = root.join(arrange::PLACING_MARKER);
    fs::write(&marker, b"").map_err(|e| PictureError::Io {
        doing: format!("marking {} half done", marker.display()),
        detail: e.to_string(),
    })?;
    rename(&parked_ours, &root.join(format!("{}.{ours}", target.id)))?;
    if let Some((parked, ext)) = &parked_theirs {
        rename(parked, &root.join(format!("{}.{ext}", source.id)))?;
    }
    let _ = fs::remove_file(&marker);

    Ok(Moved {
        from: source.id.clone(),
        to: target.id.clone(),
        swapped: other.is_some(),
        after: [after_source, after_target],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "spoonstill-picture-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(dir.join("in")).expect("scratch");
            for (file, contents) in files {
                fs::write(dir.join(file), contents).expect("write");
            }
            Scratch(dir)
        }
        fn root(&self) -> PathBuf {
            self.0.join("p")
        }
        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.root().join(name)).expect(name)
        }
        fn listing(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.root())
                .expect("list")
                .flatten()
                .filter(|e| e.path().is_file())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A check that accepts anything whose bytes do not say `BROKEN`.
    struct Looks;
    impl MediaCheck for Looks {
        fn check(
            &self,
            path: &Path,
            _role: Role,
        ) -> Result<Option<spoonstill_core::SourceGeometry>, String> {
            match fs::read_to_string(path) {
                Ok(text) if text.contains("BROKEN") => Err("not an image".to_owned()),
                _ => Ok(None),
            }
        }
    }

    fn project(name: &str) -> Scratch {
        let s = Scratch::new(name, &[]);
        fs::create_dir_all(s.root()).expect("p");
        for (file, body) in [
            ("001.jpeg", "PIC-1"),
            ("001.txt", "one"),
            ("002.txt", "two"),
            ("003.txt", "three"),
            ("004.png", "PIC-4"),
            ("004.txt", "four"),
        ] {
            fs::write(s.root().join(file), body).expect("write");
        }
        fs::write(s.0.join("in/flow-image.jpeg"), "FLOW").expect("write");
        s
    }

    #[test]
    fn a_picture_lands_on_the_scene_it_was_put_on() {
        let s = project("set");
        let done = set(&s.root(), "003", &s.0.join("in/flow-image.jpeg"), &Looks).expect("sets");
        assert_eq!(done.file, "003.jpeg");
        assert_eq!(done.replaced, None);
        assert_eq!(s.read("003.jpeg"), "FLOW");
        assert_eq!(s.read("003.txt"), "three", "its line is untouched");
        assert!(
            !s.root().join("002.jpeg").exists(),
            "not the first waiting one"
        );
        assert!(
            s.0.join("in/flow-image.jpeg").exists(),
            "copied, never moved"
        );
    }

    #[test]
    fn replacing_a_picture_keeps_the_old_one() {
        let s = project("replace");
        let done = set(&s.root(), "4", &s.0.join("in/flow-image.jpeg"), &Looks).expect("sets");
        assert_eq!(done.file, "004.jpeg");
        assert_eq!(s.read("004.jpeg"), "FLOW");
        assert!(!s.root().join("004.png").exists());
        assert_eq!(
            fs::read_to_string(s.root().join("removed/004.png")).expect("kept"),
            "PIC-4"
        );
        assert!(done.summary().contains("removed/"), "{}", done.summary());
    }

    #[test]
    fn something_that_is_not_a_picture_is_refused_and_nothing_moves() {
        let s = project("refuse");
        fs::write(s.0.join("in/notes.txt"), "x").expect("write");
        let before = s.listing();
        assert!(matches!(
            set(&s.root(), "002", &s.0.join("in/notes.txt"), &Looks),
            Err(PictureError::NotAPicture { .. })
        ));
        assert!(matches!(
            set(&s.root(), "002", &s.0.join("in/missing.jpeg"), &Looks),
            Err(PictureError::NotAPicture { .. })
        ));
        assert!(set(&s.root(), "099", &s.0.join("in/flow-image.jpeg"), &Looks).is_err());
        assert_eq!(s.listing(), before);
    }

    /// A file with a picture's name that does not read as one is taken back
    /// out, and a picture it was replacing is put back where it was.
    #[test]
    fn a_picture_that_does_not_read_is_refused_and_the_old_one_returns() {
        let s = project("unusable");
        fs::write(s.0.join("in/half.jpeg"), "BROKEN").expect("write");
        let before = s.listing();

        let error = set(&s.root(), "004", &s.0.join("in/half.jpeg"), &Looks).expect_err("refused");
        assert!(matches!(error, PictureError::Unusable { .. }), "{error}");
        assert!(error.to_string().contains("nothing was changed"), "{error}");
        assert_eq!(s.listing(), before, "the folder is as it was");
        assert_eq!(s.read("004.png"), "PIC-4", "the old picture is back");

        let error = set(&s.root(), "002", &s.0.join("in/half.jpeg"), &Looks).expect_err("refused");
        assert!(matches!(error, PictureError::Unusable { .. }), "{error}");
        assert_eq!(s.listing(), before);
    }

    #[test]
    fn several_pictures_fill_the_waiting_scenes_from_the_one_dropped_on() {
        let s = project("fill");
        for name in ["a.jpeg", "b.jpeg"] {
            fs::write(s.0.join("in").join(name), name).expect("write");
        }
        let sources = [s.0.join("in/a.jpeg"), s.0.join("in/b.jpeg")];
        // Dropped on 002: 002 and 003 are waiting, 004 has a picture.
        let done = fill(&s.root(), "002", &sources, &Looks).expect("fills");
        assert!(done.stopped.is_none());
        assert_eq!(s.read("002.jpeg"), "a.jpeg", "in the order dragged");
        assert_eq!(s.read("003.jpeg"), "b.jpeg");
        assert_eq!(
            s.read("004.png"),
            "PIC-4",
            "a scene with a picture is never touched"
        );
        assert!(done.summary().contains("002 to 003"), "{}", done.summary());
    }

    #[test]
    fn a_picture_already_there_is_skipped_not_replaced() {
        let s = project("fill-skip");
        fs::write(s.root().join("005.txt"), "five").expect("write");
        for name in ["a.jpeg", "b.jpeg"] {
            fs::write(s.0.join("in").join(name), name).expect("write");
        }
        let sources = [s.0.join("in/a.jpeg"), s.0.join("in/b.jpeg")];
        // From 003: 003 waits, 004 has a picture, 005 waits.
        fill(&s.root(), "003", &sources, &Looks).expect("fills");
        assert_eq!(s.read("003.jpeg"), "a.jpeg");
        assert_eq!(s.read("004.png"), "PIC-4");
        assert_eq!(s.read("005.jpeg"), "b.jpeg");
    }

    #[test]
    fn too_many_pictures_or_one_that_is_not_a_picture_changes_nothing() {
        let s = project("fill-refuse");
        for name in ["a.jpeg", "b.jpeg", "c.jpeg"] {
            fs::write(s.0.join("in").join(name), name).expect("write");
        }
        fs::write(s.0.join("in/notes.txt"), "x").expect("write");
        let before = s.listing();

        // From 002 only 002 and 003 wait.
        let three = [
            s.0.join("in/a.jpeg"),
            s.0.join("in/b.jpeg"),
            s.0.join("in/c.jpeg"),
        ];
        let error = fill(&s.root(), "002", &three, &Looks).expect_err("refused");
        assert!(
            matches!(error, PictureError::NotEnoughWaiting { waiting: 2, .. }),
            "{error}"
        );
        assert!(error.to_string().contains("nothing was changed"));

        let mixed = [s.0.join("in/a.jpeg"), s.0.join("in/notes.txt")];
        assert!(matches!(
            fill(&s.root(), "002", &mixed, &Looks),
            Err(PictureError::NotAPicture { .. })
        ));
        assert_eq!(s.listing(), before, "nothing changed");
    }

    #[test]
    fn undo_refuses_a_placed_picture_after_scene_reordering() {
        let s = project("undo-reordered");
        fs::write(s.root().join("002.jpeg"), "SECOND").unwrap();
        let placed = set(&s.root(), "001", &s.0.join("in/flow-image.jpeg"), &Looks).unwrap();
        arrange::move_to(&s.root(), "001", 2).unwrap();
        assert!(undo(&s.root(), &Change::Placed(vec![placed])).is_err());
        assert_eq!(s.read("001.jpeg"), "SECOND");
        assert_eq!(s.read("001.txt"), "two");
        assert_eq!(s.read("002.jpeg"), "FLOW");
        assert_eq!(s.read("002.txt"), "one");
        assert_eq!(s.read("removed/001.jpeg"), "PIC-1");
    }

    #[test]
    fn undo_refuses_a_removed_picture_after_scene_reordering() {
        let s = project("undo-removed-reordered");
        let removed = remove(&s.root(), "001").unwrap();
        arrange::move_to(&s.root(), "001", 2).unwrap();
        assert!(undo(&s.root(), &removed).is_err());
        assert!(!s.root().join("001.jpeg").exists());
        assert_eq!(s.read("001.txt"), "two");
        assert_eq!(s.read("removed/001.jpeg"), "PIC-1");
    }

    #[test]
    fn undo_refuses_a_swap_after_scene_reordering() {
        let s = project("undo-swap-reordered");
        let moved = move_to(&s.root(), "001", "004").unwrap();
        arrange::move_to(&s.root(), "001", 4).unwrap();
        assert!(undo(&s.root(), &Change::Moved(moved)).is_err());
        assert_eq!(s.read("004.png"), "PIC-4");
        assert_eq!(s.read("004.txt"), "one");
        assert_eq!(s.read("003.jpeg"), "PIC-1");
        assert_eq!(s.read("003.txt"), "four");
        assert!(!s.root().join("001.png").exists());
    }

    #[test]
    fn undo_refuses_a_picture_replaced_under_the_same_name() {
        let s = project("undo-same-name");
        let placed = set(&s.root(), "001", &s.0.join("in/flow-image.jpeg"), &Looks).unwrap();
        fs::write(s.root().join("001.jpeg"), "EDIT").unwrap();
        assert!(undo(&s.root(), &Change::Placed(vec![placed])).is_err());
        assert_eq!(s.read("001.jpeg"), "EDIT");
        assert_eq!(s.read("removed/001.jpeg"), "PIC-1");
    }

    #[test]
    fn undo_refuses_a_changed_backup_before_touching_the_current_picture() {
        let s = project("undo-backup-edited");
        let placed = set(&s.root(), "001", &s.0.join("in/flow-image.jpeg"), &Looks).unwrap();
        fs::write(placed.replaced.as_ref().unwrap(), "EDITED BACKUP").unwrap();
        assert!(undo(&s.root(), &Change::Placed(vec![placed])).is_err());
        assert_eq!(s.read("001.jpeg"), "FLOW");
        assert_eq!(s.read("removed/001.jpeg"), "EDITED BACKUP");
    }

    #[test]
    fn undo_checks_every_picture_in_a_fill_before_changing_any() {
        let s = project("undo-fill-edited");
        let sources = [
            s.0.join("in/flow-image.jpeg"),
            s.0.join("in/flow-image.jpeg"),
        ];
        let filled = fill(&s.root(), "002", &sources, &Looks).unwrap();
        fs::write(s.root().join("002.jpeg"), "EDIT").unwrap();
        assert!(undo(&s.root(), &Change::Placed(filled.placed)).is_err());
        assert_eq!(s.read("002.jpeg"), "EDIT");
        assert_eq!(s.read("003.jpeg"), "FLOW");
    }

    #[test]
    fn successive_replacements_can_still_be_undone_in_order() {
        let s = project("undo-chain");
        let first = set(&s.root(), "001", &s.0.join("in/flow-image.jpeg"), &Looks).unwrap();
        fs::write(s.0.join("in/next.png"), "NEXT").unwrap();
        let second = set(&s.root(), "001", &s.0.join("in/next.png"), &Looks).unwrap();
        undo(&s.root(), &Change::Placed(vec![second])).unwrap();
        assert_eq!(s.read("001.jpeg"), "FLOW");
        undo(&s.root(), &Change::Placed(vec![first])).unwrap();
        assert_eq!(s.read("001.jpeg"), "PIC-1");
    }

    #[test]
    fn every_swap_interruption_recovers_one_complete_pair() {
        for ext in ["jpeg", "png"] {
            for stop in 0..=4 {
                let s = project(&format!("swap-every-step-{ext}-{stop}"));
                if ext == "jpeg" {
                    fs::rename(s.root().join("004.png"), s.root().join("004.jpeg")).unwrap();
                }
                let ours = arrange::parked_name("001", "004", "jpeg");
                let theirs = arrange::parked_name("004", "001", ext);
                let steps = [
                    ("001.jpeg".to_owned(), ours.clone()),
                    (format!("004.{ext}"), theirs.clone()),
                    (ours, "004.jpeg".to_owned()),
                    (theirs, format!("001.{ext}")),
                ];
                for (from, to) in steps.iter().take(stop) {
                    fs::rename(s.root().join(from), s.root().join(to)).unwrap();
                }
                arrange::recover(&s.root()).unwrap();
                let first: Vec<_> = s
                    .listing()
                    .into_iter()
                    .filter(|n| n.starts_with("001.") && !n.ends_with(".txt"))
                    .collect();
                let last: Vec<_> = s
                    .listing()
                    .into_iter()
                    .filter(|n| n.starts_with("004.") && !n.ends_with(".txt"))
                    .collect();
                assert_eq!(first.len(), 1, "{ext}, stop {stop}: {:?}", s.listing());
                assert_eq!(last.len(), 1, "{ext}, stop {stop}: {:?}", s.listing());
                let pair = (s.read(&first[0]), s.read(&last[0]));
                assert!(
                    pair == ("PIC-1".into(), "PIC-4".into())
                        || pair == ("PIC-4".into(), "PIC-1".into()),
                    "{ext}, stop {stop}: {pair:?}"
                );
                assert_eq!(s.read("001.txt"), "one");
                assert_eq!(s.read("004.txt"), "four");
                assert!(!s.listing().iter().any(|n| n.starts_with('.')));
                assert_eq!(
                    arrange::recover(&s.root()).unwrap(),
                    0,
                    "recovery is idempotent"
                );
            }
        }
    }

    #[test]
    fn every_change_can_be_taken_back_and_nothing_is_deleted() {
        let s = project("undo");
        let original = s.listing();

        // Put a picture on a waiting scene, then undo.
        let placed = set(&s.root(), "002", &s.0.join("in/flow-image.jpeg"), &Looks).expect("set");
        undo(&s.root(), &Change::Placed(vec![placed])).expect("undo");
        assert_eq!(s.listing(), original);
        assert!(
            s.root().join("removed/002.jpeg").exists(),
            "kept, not deleted"
        );

        // Replace a picture, then undo: the old one is back under its name.
        let placed = set(&s.root(), "004", &s.0.join("in/flow-image.jpeg"), &Looks).expect("set");
        undo(&s.root(), &Change::Placed(vec![placed])).expect("undo");
        assert_eq!(s.read("004.png"), "PIC-4");
        assert!(!s.root().join("004.jpeg").exists());

        // Remove, then undo.
        let removed = remove(&s.root(), "001").expect("remove");
        undo(&s.root(), &removed).expect("undo");
        assert_eq!(s.read("001.jpeg"), "PIC-1");

        // Swap, then undo.
        let moved = move_to(&s.root(), "001", "004").expect("swap");
        undo(&s.root(), &Change::Moved(moved)).expect("undo");
        assert_eq!(s.read("001.jpeg"), "PIC-1");
        assert_eq!(s.read("004.png"), "PIC-4");
        assert_eq!(s.listing(), original);
    }

    #[test]
    fn an_undo_after_the_folder_changed_changes_nothing() {
        let s = project("undo-stale");
        let placed = set(&s.root(), "002", &s.0.join("in/flow-image.jpeg"), &Looks).expect("set");
        // Something else took that picture off since.
        remove(&s.root(), "002").expect("remove");
        let before = s.listing();
        let error = undo(&s.root(), &Change::Placed(vec![placed])).expect_err("refused");
        assert!(error.to_string().contains("nothing was undone"), "{error}");
        assert_eq!(s.listing(), before);
    }

    #[test]
    fn removing_a_picture_keeps_it_and_the_scene_waits_again() {
        let s = project("remove");
        let Change::Removed { binned, .. } = remove(&s.root(), "001").expect("removes") else {
            panic!("a removal");
        };
        assert!(binned.ends_with("removed/001.jpeg"));
        assert_eq!(fs::read_to_string(binned).expect("kept"), "PIC-1");
        assert_eq!(s.read("001.txt"), "one");
        let all = arrange::scenes(&s.root()).expect("scenes");
        assert_eq!(all.len(), 4, "the scene is still there, waiting");
        assert!(matches!(
            remove(&s.root(), "002"),
            Err(PictureError::NoPicture { .. })
        ));
    }

    #[test]
    fn removing_twice_never_overwrites_what_was_removed_before() {
        let s = project("remove-twice");
        remove(&s.root(), "001").expect("removes");
        set(&s.root(), "001", &s.0.join("in/flow-image.jpeg"), &Looks).expect("sets");
        remove(&s.root(), "001").expect("removes again");
        assert_eq!(
            fs::read_to_string(s.root().join("removed/001.jpeg")).expect("first"),
            "PIC-1"
        );
        assert_eq!(
            fs::read_to_string(s.root().join("removed/001-2.jpeg")).expect("second"),
            "FLOW"
        );
    }

    #[test]
    fn a_picture_moves_to_a_waiting_scene() {
        let s = project("move");
        let done = move_to(&s.root(), "001", "003").expect("moves");
        assert!(!done.swapped);
        assert_eq!(s.read("003.jpeg"), "PIC-1");
        assert!(!s.root().join("001.jpeg").exists());
        assert_eq!(s.read("001.txt"), "one", "lines stay where they were");
        assert_eq!(s.read("003.txt"), "three");
    }

    #[test]
    fn two_pictures_on_each_others_lines_swap_in_one_step() {
        let s = project("swap");
        let done = move_to(&s.root(), "001", "004").expect("swaps");
        assert!(done.swapped);
        assert_eq!(s.read("004.jpeg"), "PIC-1");
        assert_eq!(s.read("001.png"), "PIC-4");
        assert!(!s.root().join("001.jpeg").exists());
        assert!(!s.root().join("004.png").exists());
        assert!(
            !s.listing().iter().any(|n| n.starts_with('.')),
            "nothing left parked: {:?}",
            s.listing()
        );
    }

    #[test]
    fn a_swap_interrupted_half_way_is_finished_the_next_time_anyone_looks() {
        let s = project("swap-interrupted");
        // What a process killed between the two parks and the two renames
        // leaves: both pictures parked, neither in place, and the marker that
        // says the parks were finished (D-201).
        fs::rename(
            s.root().join("001.jpeg"),
            s.root().join(arrange::parked_name("001", "004", "jpeg")),
        )
        .expect("park");
        fs::rename(
            s.root().join("004.png"),
            s.root().join(arrange::parked_name("004", "001", "png")),
        )
        .expect("park");
        fs::write(s.root().join(arrange::PLACING_MARKER), "").expect("mark");

        arrange::scenes(&s.root()).expect("reads, and recovers");
        assert_eq!(s.read("004.jpeg"), "PIC-1");
        assert_eq!(s.read("001.png"), "PIC-4");
        assert!(!s.root().join(arrange::PLACING_MARKER).exists());
    }

    #[test]
    fn a_swap_stopped_before_its_marker_is_undone() {
        let s = project("swap-unmarked");
        // Killed after both parks and before the marker: nothing was placed,
        // so both pictures go back where they were (D-201).
        fs::rename(
            s.root().join("001.jpeg"),
            s.root().join(arrange::parked_name("001", "004", "jpeg")),
        )
        .expect("park");
        fs::rename(
            s.root().join("004.png"),
            s.root().join(arrange::parked_name("004", "001", "png")),
        )
        .expect("park");

        arrange::scenes(&s.root()).expect("reads, and recovers");
        assert_eq!(s.read("001.jpeg"), "PIC-1");
        assert_eq!(s.read("004.png"), "PIC-4");
    }

    #[test]
    fn moving_onto_itself_or_with_nothing_to_move_is_refused() {
        let s = project("move-refused");
        assert!(matches!(
            move_to(&s.root(), "001", "1"),
            Err(PictureError::SameScene { .. })
        ));
        assert!(matches!(
            move_to(&s.root(), "002", "003"),
            Err(PictureError::NoPicture { .. })
        ));
    }
}
