//! Making a project, and filling it — the step that used to be the operator's
//! filing job (D-080).
//!
//! ## The problem this module exists to delete
//!
//! A project is a folder, and convention mode pairs `001.jpg` with `001.wav`
//! by stem (D-050, D-056). That rule is excellent for the renderer and awful
//! for the person: their camera produced `IMG_2931.HEIC` and their recorder
//! produced `Voice 014.m4a`, so before anything could render they had to rename
//! 120 files by hand. We removed the timeline and handed back a filing job —
//! twenty to forty minutes of tedium for a sixty-scene film, which is most of
//! the time we claimed to save.
//!
//! So the convention is ours to satisfy, not theirs. The operator hands over
//! whatever they have, in whatever order the finder gave it, and this module
//! copies it in under the names the rest of the system already understands.
//!
//! ## The three rules that make it safe to press twice
//!
//! 1. **Copy, never move.** The sources are someone's photo library. A tool
//!    that empties it because the operator dropped the wrong folder is a tool
//!    they use once. Every original is left exactly where it was.
//! 2. **Never overwrite.** Numbering continues from the highest number already
//!    in the folder, so a second drop appends rather than replaces. There is no
//!    input to this module that can destroy a file.
//! 3. **Nothing unusable is an error.** A `.DS_Store`, a README, a PDF among
//!    the photos — these are skipped and *counted*, not fatal. The operator
//!    dropped a folder, not a curated list, and the answer to "one of those was
//!    a spreadsheet" is a line in the report, not a refusal to work.
//!
//! ## Pairing, and why it is positional
//!
//! Photos sort into an order; recordings sort into an order; the nth of one
//! belongs to the nth of the other. That is the only assumption available
//! without reading the audio or the image, and it is the assumption the
//! operator already made when they recorded narration for their photos in
//! order. It is also *visible* — the review grid shows every pair before a
//! frame is encoded (D-051), which is where a wrong guess gets caught.
//!
//! The sort is natural rather than lexicographic: `IMG_2.jpg` precedes
//! `IMG_10.jpg`, because the operator counts in decimal and `sort` does not.
//!
//! ## What this module never does
//!
//! It does not write `project.yaml`. An absent settings file is a valid project
//! and every default is already a working default (D-056), so writing one would
//! put a file in the folder that the operator did not ask for and that this
//! program is otherwise forbidden to touch. Settings arrive when someone
//! changes one.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use crate::import::rows::{AUDIO_EXTENSIONS, IMAGE_EXTENSIONS, TEXT_EXTENSIONS};

/// The smallest number of digits a scene name gets: `001`, not `1`.
///
/// Three digits sorts correctly to 999 scenes, which is above the n=500 design
/// point; past that the width grows rather than the order breaking.
const MIN_WIDTH: usize = 3;

/// What one file turned out to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Image,
    Audio,
    Text,
}

fn kind_of(path: &Path) -> Option<Kind> {
    let extension = path
        .extension()
        .and_then(OsStr::to_str)?
        .to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(Kind::Image)
    } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
        Some(Kind::Audio)
    } else if TEXT_EXTENSIONS.contains(&extension.as_str()) {
        Some(Kind::Text)
    } else {
        None
    }
}

/// One file that was copied in, named both ways so the report can say what
/// became what.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    /// Where it came from. Untouched.
    pub source: PathBuf,
    /// What it is called inside the project, relative to the root.
    pub name: String,
}

/// One file that was not copied, and the reason in the operator's terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The file that was passed over.
    pub source: PathBuf,
    /// Why — a sentence, not a code.
    pub reason: String,
}

/// What a drop did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ingested {
    /// Stills, in the order they became scenes.
    pub images: Vec<Copied>,
    /// Narrations, each sharing a stem with the still it paired with.
    pub audio: Vec<Copied>,
    /// Lines to speak, each sharing a stem with its still (D-050).
    pub scripts: Vec<Copied>,
    /// Files that were not media, or could not be used.
    pub skipped: Vec<Skipped>,
    /// Recordings with no photo left to pair with, so they were not copied.
    ///
    /// This is the one mismatch worth naming: more narration than stills means
    /// the operator is missing photos, and silently dropping the tail would
    /// produce a film that ends early with no explanation.
    pub audio_without_image: usize,
    /// Scripts with no photo left to pair with, so they were not copied.
    pub script_without_image: usize,
    /// Stills that paired with neither a recording nor a script, and will hold
    /// for the project's default duration as silent scenes (D-050).
    pub image_without_audio: usize,
}

impl Ingested {
    /// Whether anything at all landed in the project.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.audio.is_empty() && self.scripts.is_empty()
    }

    /// The recording that came in with this still, if one did.
    ///
    /// Matched by stem rather than by index: `audio` holds only the stills that
    /// got one, so the nth recording is not the nth still's. Getting that wrong
    /// prints a pairing the operator does not have, which is worse than
    /// printing none — the report is the only place they can check our guess.
    #[must_use]
    pub fn narration_for(&self, image: &Copied) -> Option<&Copied> {
        with_stem(&self.audio, &image.name)
    }

    /// The script that came in with this still, if one did.
    #[must_use]
    pub fn script_for(&self, image: &Copied) -> Option<&Copied> {
        with_stem(&self.scripts, &image.name)
    }

    /// One line, in the shape both the terminal and the window want.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut parts = vec![format!(
            "{} photo{}",
            self.images.len(),
            plural(self.images.len())
        )];
        if !self.audio.is_empty() {
            parts.push(format!(
                "{} narration{} paired",
                self.audio.len(),
                plural(self.audio.len())
            ));
        }
        if !self.scripts.is_empty() {
            parts.push(format!(
                "{} script{} to speak",
                self.scripts.len(),
                plural(self.scripts.len())
            ));
        }
        if self.image_without_audio > 0 {
            parts.push(format!("{} silent", self.image_without_audio));
        }
        let orphans = self.audio_without_image + self.script_without_image;
        if orphans > 0 {
            parts.push(format!("{orphans} file{} with no photo", plural(orphans)));
        }
        if !self.skipped.is_empty() {
            parts.push(format!("{} skipped", self.skipped.len()));
        }
        parts.join(", ")
    }
}

/// The file in `pool` whose stem matches `name`'s.
fn with_stem<'a>(pool: &'a [Copied], name: &str) -> Option<&'a Copied> {
    let stem = stem_of(Path::new(name));
    pool.iter()
        .find(|candidate| stem_of(Path::new(&candidate.name)) == stem)
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Why a folder could not be made, or filled.
#[derive(Debug)]
pub enum IngestError {
    /// The folder could not be created.
    CannotCreate {
        /// Where it was going.
        path: PathBuf,
        /// The operating system's reason.
        detail: String,
    },
    /// A project already lives there, and creating over it would mix two films.
    AlreadyAProject {
        /// The folder in question.
        path: PathBuf,
        /// How many stills are already in it.
        stills: usize,
    },
    /// The destination is not a folder we can fill.
    NotAProject {
        /// What was asked for.
        path: PathBuf,
    },
    /// A copy failed part-way. Everything before it is already in place.
    Copy {
        /// The source file.
        from: PathBuf,
        /// Where it was going.
        to: PathBuf,
        /// The operating system's reason.
        detail: String,
    },
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::CannotCreate { path, detail } => {
                write!(f, "could not create {}: {detail}", path.display())
            }
            IngestError::AlreadyAProject { path, stills } => write!(
                f,
                "{} already holds {stills} still{} — open it instead of creating it",
                path.display(),
                plural(*stills)
            ),
            IngestError::NotAProject { path } => {
                write!(f, "{} is not a folder", path.display())
            }
            IngestError::Copy { from, to, detail } => write!(
                f,
                "could not copy {} to {}: {detail}",
                from.display(),
                to.display()
            ),
        }
    }
}

impl std::error::Error for IngestError {}

/// Make an empty project folder.
///
/// An existing *empty* folder is accepted, because "New project" pointed at a
/// folder the operator just made in the file dialog is the common case and
/// refusing it would be pedantry. An existing folder that already holds stills
/// is refused by name: that is a project, and the verb for it is open.
///
/// Nothing is written inside — see the module note on `project.yaml`.
///
/// # Errors
///
/// [`IngestError::AlreadyAProject`] if stills are already there,
/// [`IngestError::CannotCreate`] if the folder cannot be made.
pub fn create_project(path: &Path) -> Result<PathBuf, IngestError> {
    if path.is_dir() {
        let stills = media_in(path, Kind::Image).len();
        if stills > 0 {
            return Err(IngestError::AlreadyAProject {
                path: path.to_path_buf(),
                stills,
            });
        }
    } else {
        fs::create_dir_all(path).map_err(|e| IngestError::CannotCreate {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;
    }
    Ok(fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()))
}

/// Copy media into a project folder, numbered and paired.
///
/// `sources` may name files or folders; a folder contributes the media
/// directly inside it, which is what dropping a camera's export folder onto the
/// window means. Numbering continues from whatever is already in the project,
/// so this is safe to call repeatedly.
///
/// # Errors
///
/// [`IngestError::NotAProject`] if the root is not a folder, or
/// [`IngestError::Copy`] if a file cannot be copied. A copy error stops the
/// batch; everything copied before it stays, and the project is still valid —
/// it simply has fewer scenes than the operator asked for.
pub fn add_media(root: &Path, sources: &[PathBuf]) -> Result<Ingested, IngestError> {
    if !root.is_dir() {
        return Err(IngestError::NotAProject {
            path: root.to_path_buf(),
        });
    }

    let mut report = Ingested::default();
    let (mut images, mut audio, mut scripts) = (Vec::new(), Vec::new(), Vec::new());

    for source in expand(sources, &mut report) {
        match kind_of(&source) {
            Some(Kind::Image) => images.push(source),
            Some(Kind::Audio) => audio.push(source),
            // A `.txt` beside a photo is a line to speak (D-050): the operator
            // writes the words instead of recording them, and a provider says
            // them at render time.
            Some(Kind::Text) => scripts.push(source),
            None => report.skipped.push(Skipped {
                reason: "not a photo, a recording or a script".to_owned(),
                source,
            }),
        }
    }

    images.sort_by_key(|path| natural_key(path));
    audio.sort_by_key(|path| natural_key(path));
    scripts.sort_by_key(|path| natural_key(path));

    let (narration, spare_audio) = assign(&images, audio);
    let (lines, spare_scripts) = assign(&images, scripts);
    report.audio_without_image = spare_audio;
    report.script_without_image = spare_scripts;
    report.image_without_audio = narration
        .iter()
        .zip(&lines)
        .filter(|(a, t)| a.is_none() && t.is_none())
        .count();

    let start = next_index(root);
    let width = MIN_WIDTH.max(digits(start + images.len().saturating_sub(1)));

    for (offset, image) in images.iter().enumerate() {
        let stem = format!("{:0width$}", start + offset, width = width);
        report.images.push(copy_in(root, image, &stem)?);
        if let Some(narration) = narration[offset].as_ref() {
            report.audio.push(copy_in(root, narration, &stem)?);
        }
        if let Some(line) = lines[offset].as_ref() {
            report.scripts.push(copy_in(root, line, &stem)?);
        }
    }

    Ok(report)
}

/// Decide which recording, or which script, belongs to which still.
///
/// **By stem first, then by position.** If the operator wrote
/// `IMG_2931.txt` next to `IMG_2931.HEIC` they have already stated the pairing
/// and guessing again would only be a chance to get it wrong. Whatever is left
/// over falls into the stills that matched nothing, in order — the assumption
/// from the module note, applied only where no better answer exists.
///
/// Returns the assignment and the count that had no still left to belong to.
fn assign(images: &[PathBuf], pool: Vec<PathBuf>) -> (Vec<Option<PathBuf>>, usize) {
    let mut assigned = vec![None; images.len()];
    let mut pool: Vec<Option<PathBuf>> = pool.into_iter().map(Some).collect();

    for (slot, image) in assigned.iter_mut().zip(images) {
        let stem = stem_of(image);
        if let Some(found) = pool
            .iter_mut()
            .find(|candidate| candidate.as_deref().map(stem_of) == Some(stem.clone()))
        {
            *slot = found.take();
        }
    }

    let mut spare = pool.into_iter().flatten();
    for slot in assigned.iter_mut().filter(|slot| slot.is_none()) {
        match spare.next() {
            Some(file) => *slot = Some(file),
            None => break,
        }
    }

    (assigned, spare.count())
}

/// A file's stem, folded for comparison: a volume that preserves case but does
/// not distinguish it would otherwise pair `Shot.JPG` with nothing (D-052).
fn stem_of(path: &Path) -> String {
    path.file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// Turn the dropped list into a flat list of candidate files.
///
/// Folders contribute their immediate children only. Deeper recursion would
/// mean a dropped home directory quietly enumerating a hundred thousand files,
/// and a photo folder with a `thumbnails/` subfolder importing every thumbnail
/// as a scene.
fn expand(sources: &[PathBuf], report: &mut Ingested) -> Vec<PathBuf> {
    let mut flat = Vec::new();
    for source in sources {
        if source.is_dir() {
            let mut inside: Vec<PathBuf> = fs::read_dir(source)
                .into_iter()
                .flatten()
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_file())
                .collect();
            inside.sort_by_key(|path| natural_key(path));
            flat.extend(inside);
        } else if source.is_file() {
            flat.push(source.clone());
        } else {
            report.skipped.push(Skipped {
                source: source.clone(),
                reason: "no such file".to_owned(),
            });
        }
    }
    flat
}

/// Copy one file in under a scene name, keeping its extension.
fn copy_in(root: &Path, source: &Path, stem: &str) -> Result<Copied, IngestError> {
    let extension = source
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();
    let name = format!("{stem}.{extension}");
    let destination = root.join(&name);

    // Copied beside the destination first, never into it (D-120). This used to
    // `create_new` the real name and then fill it, so the operator's own scene
    // file held incomplete media for the whole of the copy. Measured on a
    // FAT32 volume — an external drive, which is where video media lives — an
    // interrupted `still add` of a 400 MB photo left **232 MB at `001.jpg`**:
    // a broken scene, a consumed scene number, and something to delete by hand.
    // The copy is O(1) on APFS, so this is invisible on the developer's own
    // disk and seconds long on the operator's.
    let temporary = spoonstill_media::atomic::partial_path(&destination);
    fs::copy(source, &temporary).map_err(|e| {
        let _ = fs::remove_file(&temporary);
        IngestError::Copy {
            from: source.to_path_buf(),
            to: destination.clone(),
            detail: e.to_string(),
        }
    })?;

    // Rule 2 of the module note: never overwrite. `create_new` is the check and
    // the claim in one syscall, so two adds cannot both take the name — and it
    // is done *after* the copy, so the name is claimed for as long as it takes
    // to rename rather than for as long as it takes to copy.
    let claim = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&destination);
    if let Err(e) = claim {
        let _ = fs::remove_file(&temporary);
        return Err(IngestError::Copy {
            from: source.to_path_buf(),
            to: destination,
            detail: if e.kind() == std::io::ErrorKind::AlreadyExists {
                "a file of that name is already in the project".to_owned()
            } else {
                e.to_string()
            },
        });
    }

    // Over our own empty claim, which `rename` replaces atomically (D-119).
    if let Err(e) = fs::rename(&temporary, &destination) {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(&destination);
        return Err(IngestError::Copy {
            from: source.to_path_buf(),
            to: destination.clone(),
            detail: e.to_string(),
        });
    }

    Ok(Copied {
        source: source.to_path_buf(),
        name,
    })
}

/// Every file of one kind directly inside a folder.
fn media_in(root: &Path, wanted: Kind) -> Vec<PathBuf> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && kind_of(path) == Some(wanted))
        .collect()
}

/// The number a new scene gets: one past the highest already there.
///
/// Folders whose stills are named `opening`/`middle` rather than numerically
/// start at 1, which is correct — those pair by their own stems and the numbers
/// we add never collide with them.
fn next_index(root: &Path) -> usize {
    media_in(root, Kind::Image)
        .iter()
        .filter_map(|path| path.file_stem().and_then(OsStr::to_str))
        .filter_map(|stem| stem.parse::<usize>().ok())
        .max()
        .map_or(1, |highest| highest + 1)
}

fn digits(n: usize) -> usize {
    n.to_string().len()
}

/// A sort key that counts the way the operator counts.
///
/// `IMG_2` before `IMG_10`: runs of digits compare as numbers, everything else
/// compares as lowercased text. Returned as an owned key so the comparison
/// itself is a plain `Vec` compare — n=500 sorts once, and clarity is worth
/// more here than allocation.
fn natural_key(path: &Path) -> Vec<Chunk> {
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_ascii_lowercase();

    let mut chunks = Vec::new();
    let mut rest = name.as_str();
    while !rest.is_empty() {
        let digit = rest.starts_with(|c: char| c.is_ascii_digit());
        let end = rest
            .find(|c: char| c.is_ascii_digit() != digit)
            .unwrap_or(rest.len());
        let (head, tail) = rest.split_at(end);
        chunks.push(if digit {
            // A number longer than u128 is not a counter, it is a hash; compare
            // it as text rather than saturating every such name to one value.
            head.parse::<u128>()
                .map_or_else(|_| Chunk::Text(head.to_owned()), Chunk::Number)
        } else {
            Chunk::Text(head.to_owned())
        });
        rest = tail;
    }
    chunks
}

/// One run of a natural sort key. Numbers sort before text at the same
/// position, which only decides `1a` vs `a1` and needs no better reason.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Chunk {
    Number(u128),
    Text(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder that cleans up after itself.
    struct Temp(PathBuf);

    impl Temp {
        fn new(tag: &str) -> Temp {
            let path = std::env::temp_dir()
                .join(format!("spoonstill-ingest-{tag}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("a temp folder");
            Temp(path)
        }

        fn file(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("a parent folder");
            }
            fs::write(&path, bytes).expect("a temp file");
            path
        }

        fn names(&self, sub: &str) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(self.0.join(sub))
                .expect("the folder")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }
    }

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn photos_and_recordings_pair_by_position_under_numbered_names() {
        let temp = Temp::new("pair");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("IMG_10.jpg", b"j"),
            temp.file("IMG_2.jpg", b"j"),
            temp.file("Voice 2.m4a", b"a"),
            temp.file("Voice 10.m4a", b"a"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        // Natural order, so IMG_2 is scene 001 and IMG_10 is scene 002 — and
        // each keeps the recording that sorted to the same place.
        assert_eq!(
            temp.names("film"),
            ["001.jpg", "001.m4a", "002.jpg", "002.m4a"]
        );
        assert_eq!(report.images[0].source, temp.0.join("IMG_2.jpg"));
        assert_eq!(report.images[0].name, "001.jpg");
        assert_eq!(report.audio[0].source, temp.0.join("Voice 2.m4a"));
        assert_eq!(report.image_without_audio, 0);
        assert_eq!(report.audio_without_image, 0);
    }

    #[test]
    fn the_originals_are_copied_and_never_moved() {
        let temp = Temp::new("copy");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let source = temp.file("holiday.png", b"pixels");

        add_media(&root, std::slice::from_ref(&source)).expect("the drop");

        assert!(
            source.is_file(),
            "the operator's own file must still be there"
        );
        assert_eq!(fs::read(root.join("001.png")).expect("the copy"), b"pixels");
    }

    #[test]
    fn a_second_drop_appends_rather_than_overwriting() {
        let temp = Temp::new("append");
        let root = create_project(&temp.0.join("film")).expect("a new project");

        add_media(&root, &[temp.file("a.jpg", b"1")]).expect("first drop");
        let second = add_media(&root, &[temp.file("b.jpg", b"2")]).expect("second drop");

        assert_eq!(second.images[0].name, "002.jpg");
        assert_eq!(temp.names("film"), ["001.jpg", "002.jpg"]);
        assert_eq!(fs::read(root.join("001.jpg")).expect("the first"), b"1");
    }

    #[test]
    fn more_photos_than_recordings_leaves_silent_scenes() {
        let temp = Temp::new("silent");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("1.jpg", b"j"),
            temp.file("2.jpg", b"j"),
            temp.file("3.jpg", b"j"),
            temp.file("only.wav", b"a"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        assert_eq!(report.image_without_audio, 2);
        assert_eq!(report.audio.len(), 1);
        assert_eq!(
            temp.names("film"),
            ["001.jpg", "001.wav", "002.jpg", "003.jpg"]
        );
    }

    /// The mismatch that means "you are missing photos", reported rather than
    /// silently truncating the film.
    #[test]
    fn more_recordings_than_photos_is_counted_and_not_copied() {
        let temp = Temp::new("orphan");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("1.jpg", b"j"),
            temp.file("1.wav", b"a"),
            temp.file("2.wav", b"a"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        assert_eq!(report.audio_without_image, 1);
        assert_eq!(temp.names("film"), ["001.jpg", "001.wav"]);
    }

    #[test]
    fn a_dropped_folder_contributes_the_media_inside_it() {
        let temp = Temp::new("folder");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        temp.file("export/one.jpg", b"j");
        temp.file("export/two.jpg", b"j");
        temp.file("export/notes.pdf", b"p");
        temp.file("export/thumbs/small.jpg", b"j");

        let report = add_media(&root, &[temp.0.join("export")]).expect("the drop");

        assert_eq!(
            report.images.len(),
            2,
            "the subfolder is not descended into"
        );
        assert_eq!(report.skipped.len(), 1);
        assert!(report.skipped[0].reason.contains("not a photo"));
    }

    #[test]
    fn junk_is_skipped_by_name_rather_than_refused() {
        let temp = Temp::new("junk");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("good.jpg", b"j"),
            temp.file(".DS_Store", b"x"),
            temp.file("script.txt", b"hello"),
            temp.0.join("gone.jpg"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        assert_eq!(report.images.len(), 1);
        assert_eq!(report.scripts.len(), 1, "a script is media, not junk");
        assert_eq!(report.skipped.len(), 2);
        assert!(
            report
                .skipped
                .iter()
                .any(|s| s.reason.contains("no such file")),
            "a named file that is not there is reported, not fatal"
        );
    }

    /// The pairing the operator stated themselves, honoured before the one we
    /// would have guessed.
    #[test]
    fn a_script_named_after_its_photo_pairs_with_that_photo() {
        let temp = Temp::new("stem");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("IMG_2931.jpg", b"j"),
            temp.file("IMG_2930.jpg", b"j"),
            // Named for the *second* photo in sort order, not the first.
            temp.file("IMG_2931.txt", b"the words"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        assert_eq!(report.images[1].name, "002.jpg");
        assert_eq!(report.scripts[0].name, "002.txt");
        assert_eq!(
            fs::read_to_string(root.join("002.txt")).expect("the script"),
            "the words"
        );
        assert_eq!(report.image_without_audio, 1, "only 001 is silent");
    }

    /// The bug this method exists to prevent: `audio` holds only the stills
    /// that got one, so indexing it by scene number pairs the wrong files in
    /// the report.
    #[test]
    fn the_report_pairs_by_stem_and_not_by_position() {
        let temp = Temp::new("report");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("a.jpg", b"j"),
            temp.file("b.jpg", b"j"),
            temp.file("c.jpg", b"j"),
            temp.file("b.wav", b"a"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        assert_eq!(report.images[1].name, "002.jpg");
        assert_eq!(
            report
                .narration_for(&report.images[1])
                .map(|c| c.name.as_str()),
            Some("002.wav")
        );
        assert!(report.narration_for(&report.images[0]).is_none());
        assert!(report.narration_for(&report.images[2]).is_none());
    }

    #[test]
    fn a_photo_can_arrive_with_both_a_recording_and_a_script() {
        let temp = Temp::new("both");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        let sources = vec![
            temp.file("a.jpg", b"j"),
            temp.file("a.mp3", b"m"),
            temp.file("a.txt", b"words"),
        ];

        let report = add_media(&root, &sources).expect("the drop");

        // Both land; which one speaks is the import layer's rule (D-050), not
        // this module's — and an mp3 is as good a recording as a wav.
        assert_eq!(temp.names("film"), ["001.jpg", "001.mp3", "001.txt"]);
        assert_eq!(report.image_without_audio, 0);
    }

    #[test]
    fn creating_over_an_existing_project_is_refused_by_name() {
        let temp = Temp::new("occupied");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        add_media(&root, &[temp.file("a.jpg", b"1")]).expect("the drop");

        let again = create_project(&root);

        assert!(
            matches!(again, Err(IngestError::AlreadyAProject { stills: 1, .. })),
            "expected a refusal naming the stills, found {again:?}"
        );
    }

    #[test]
    fn an_existing_empty_folder_is_a_fine_place_for_a_new_project() {
        let temp = Temp::new("empty");
        let path = temp.0.join("blank");
        fs::create_dir_all(&path).expect("the folder");

        assert!(create_project(&path).is_ok());
    }

    #[test]
    fn nothing_written_here_is_a_settings_file() {
        let temp = Temp::new("nosettings");
        let root = create_project(&temp.0.join("film")).expect("a new project");
        add_media(&root, &[temp.file("a.jpg", b"1")]).expect("the drop");

        assert!(
            !root.join("project.yaml").exists(),
            "an absent settings file is a valid project (D-056); \
             writing one is not this module's business"
        );
    }

    #[test]
    fn natural_order_counts_in_decimal() {
        let mut names: Vec<PathBuf> = ["IMG_10.jpg", "IMG_9.jpg", "img_2.JPG", "b.jpg"]
            .iter()
            .map(PathBuf::from)
            .collect();
        names.sort_by_key(|path| natural_key(path));

        // `b` first because text compares as text; then 2, 9, 10 rather than
        // 10, 2, 9 — and the case of `img_2.JPG` changes nothing.
        let order: Vec<&str> = names.iter().filter_map(|p| p.to_str()).collect();
        assert_eq!(order, ["b.jpg", "img_2.JPG", "IMG_9.jpg", "IMG_10.jpg"]);
    }

    /// D-120. A scene file appears complete or not at all.
    ///
    /// `copy_in` used to `create_new` the **real** name and then fill it, so
    /// the operator's own `001.jpg` held incomplete media for the whole of the
    /// copy. That is invisible on APFS, where `fs::copy` is a copy-on-write
    /// clone and finishes in microseconds, and seconds long on the filesystems
    /// video actually lives on. Measured on a FAT32 volume: an interrupted
    /// `still add` of a 400 MB photo left **232 MB at `001.jpg`** — a broken
    /// scene, a consumed scene number, and something to delete by hand. After
    /// this, the same kill leaves only a hidden `.partial` and the retry takes
    /// `001` as if nothing had happened.
    #[test]
    fn a_scene_file_is_never_seen_half_copied() {
        let temp = Temp::new("halfcopy");
        let source = temp.file("src/photo.jpg", b"the whole picture");
        let project = temp.0.join("project");
        fs::create_dir_all(&project).expect("project");

        let copied = copy_in(&project, &source, "001").expect("copies");
        assert_eq!(copied.name, "001.jpg");
        assert_eq!(
            fs::read(project.join("001.jpg")).expect("read"),
            b"the whole picture",
            "the destination must hold every byte"
        );
        assert!(
            leftovers(&project).is_empty(),
            "a temporary was left behind: {:?}",
            leftovers(&project)
        );
    }

    /// A copy that fails leaves the folder exactly as it found it — no scene
    /// file at the real name for `validate` to report, and no temporary either.
    #[test]
    fn a_failed_copy_leaves_nothing_behind() {
        let temp = Temp::new("failedcopy");
        let project = temp.0.join("project");
        fs::create_dir_all(&project).expect("project");
        let missing = temp.0.join("src/not-here.jpg");

        copy_in(&project, &missing, "001").expect_err("a missing source cannot be copied");

        assert!(
            !project.join("001.jpg").exists(),
            "the real name was created for a copy that never happened"
        );
        assert!(leftovers(&project).is_empty(), "{:?}", leftovers(&project));
    }

    /// Rule 2 of the module note still holds, and now the claim is made *after*
    /// the copy — so a refused name must not leave the copy's temporary behind.
    #[test]
    fn an_existing_name_is_refused_and_leaves_no_temporary() {
        let temp = Temp::new("existing");
        let source = temp.file("src/photo.jpg", b"new");
        let project = temp.0.join("project");
        fs::create_dir_all(&project).expect("project");
        fs::write(project.join("001.jpg"), b"the operator's own").expect("seed");

        let error = copy_in(&project, &source, "001").expect_err("must not overwrite");
        assert!(
            error.to_string().contains("already in the project"),
            "{error}"
        );
        assert_eq!(
            fs::read(project.join("001.jpg")).expect("read"),
            b"the operator's own",
            "an existing file was overwritten"
        );
        assert!(leftovers(&project).is_empty(), "{:?}", leftovers(&project));
    }

    /// Every hidden temporary in a folder, which should be none once a call
    /// has returned either way.
    fn leftovers(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .expect("the folder")
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n.contains(".partial-"))
            .collect()
    }

    /// The test that actually distinguishes the two implementations (D-120).
    ///
    /// The three above assert good invariants — complete content, no litter,
    /// no clobber — and **all three pass against the broken code**, because
    /// they inspect the end state and the defect is a *window*. Recorded
    /// because it is the trap D-116 named: a test that checks the right thing
    /// where the bug cannot show is not coverage.
    ///
    /// What is asserted here is the property the fix actually buys, stated
    /// exactly: **the scene file is never seen holding part of a photograph.**
    /// Not "never seen at all" — claiming the name still creates an empty file
    /// for the two syscalls between the claim and the rename, and on a
    /// filesystem without hard links (FAT32 answers `Operation not supported`,
    /// and that is the external drive this matters on) there is no way to both
    /// refuse to overwrite and appear atomically. An empty file for two
    /// syscalls is a different thing from 232 MB of a 400 MB photo for the
    /// length of a copy.
    #[test]
    fn the_destination_is_never_seen_holding_part_of_a_photograph() {
        use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

        const SIZE: u64 = 64 * 1024 * 1024;

        let temp = Temp::new("window");
        let project = temp.0.join("project");
        fs::create_dir_all(&project).expect("project");
        let source = temp.file("src/big.jpg", &vec![7u8; SIZE as usize]);

        let destination = project.join("001.jpg");
        let done = AtomicBool::new(false);
        let partial = AtomicU64::new(0);

        std::thread::scope(|scope| {
            scope.spawn(|| {
                while !done.load(Ordering::Relaxed) {
                    if let Ok(meta) = fs::metadata(&destination) {
                        let len = meta.len();
                        if len > 0 && len < SIZE {
                            partial.store(len, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            });
            copy_in(&project, &source, "001").expect("copies");
            done.store(true, Ordering::Relaxed);
        });

        let seen = partial.load(Ordering::Relaxed);
        assert_eq!(
            seen, 0,
            "the scene file held {seen} of {SIZE} bytes while the copy ran — an \
             interrupted `still add` leaves exactly that behind, at the real name"
        );
        assert_eq!(fs::metadata(&destination).expect("stat").len(), SIZE);
    }
}
