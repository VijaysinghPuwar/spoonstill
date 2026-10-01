//! Importing a chapter: its lines become scenes waiting for pictures (D-193).
//!
//! The cut itself is `spoonstill_core::chapter`, pure and file-free. This is
//! the half that touches a project: reading a chapter file safely, and writing
//! each piece as the next numbered `.txt` — `023.txt`, `024.txt`, … — which the
//! folder scan then reads as a scene waiting for its picture.
//!
//! The rules are ingest's (D-080), for ingest's reasons:
//!
//! - **Never overwrite.** Numbering starts one past the highest scene number
//!   already in the folder — a picture's, a line's or a recording's — and each
//!   name is claimed with `create_new`. There is no input here that can replace
//!   an operator's file.
//! - **A file is never seen half-written** (D-120): each line is written beside
//!   its name and renamed onto it.
//! - **A failure part-way leaves what it wrote**, and says how far it got.
//!   Every file written is already a complete, valid scene.
//!
//! It writes nothing else: no `project.yaml`, no picture, no placeholder.

use std::ffi::OsStr;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use spoonstill_core::chapter::{Cut, Pacing};
use spoonstill_core::project::MAX_SCRIPT_BYTES;

use crate::import::rows::{AUDIO_EXTENSIONS, DEFAULT_MANIFEST, IMAGE_EXTENSIONS, TEXT_EXTENSIONS};

pub use spoonstill_core::chapter::{CHARS_PER_SECOND, cut, estimate};

/// The largest chapter file this reads: 4 MiB.
///
/// A round number, not a derivation — about four million characters, three
/// days of speech at [`CHARS_PER_SECOND`], so no real chapter is near it. It
/// exists because a file is measured before it is read (D-126): choosing a
/// video by mistake must be a sentence, not a 2 GB `String`.
pub const MAX_CHAPTER_BYTES: u64 = 4 * 1024 * 1024;

/// The smallest number of digits a scene name gets, matching `ingest`.
const MIN_WIDTH: usize = 3;

/// Why a chapter could not be read or imported.
#[derive(Debug)]
pub enum ChapterError {
    /// The chapter file could not be read.
    Unreadable {
        /// The file.
        path: PathBuf,
        /// Why, in the operator's terms.
        detail: String,
    },
    /// There were no words to import.
    Empty,
    /// One piece is longer than any scene may speak.
    TooLong {
        /// Its position in the list, counting from one.
        piece: usize,
        /// Its size.
        bytes: usize,
    },
    /// The project lists its scenes in a manifest, so numbered files in the
    /// folder are not scenes and importing would appear to do nothing.
    ManifestProject {
        /// The manifest.
        manifest: PathBuf,
    },
    /// The project folder is not there.
    NotAProject {
        /// What was asked for.
        path: PathBuf,
    },
    /// A write failed part-way. Every line before it is in the project.
    Write {
        /// The file that could not be written.
        path: PathBuf,
        /// How many lines were written before it.
        written: usize,
        /// The operating system's reason.
        detail: String,
    },
}

impl std::fmt::Display for ChapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChapterError::Unreadable { path, detail } => {
                write!(f, "could not read {}: {detail}", path.display())
            }
            ChapterError::Empty => f.write_str("there are no words to import"),
            ChapterError::TooLong { piece, bytes } => write!(
                f,
                "piece {piece} is {} of text — no scene can speak more than {} KB; \
                 split it into smaller pieces",
                crate::import::human_size(*bytes as u64),
                MAX_SCRIPT_BYTES / 1024
            ),
            ChapterError::ManifestProject { manifest } => write!(
                f,
                "this project lists its scenes in {}, so new numbered lines in the \
                 folder would not become scenes — add them to that file instead",
                manifest
                    .file_name()
                    .unwrap_or(manifest.as_os_str())
                    .to_string_lossy()
            ),
            ChapterError::NotAProject { path } => {
                write!(f, "{} is not a folder", path.display())
            }
            ChapterError::Write {
                path,
                written,
                detail,
            } => write!(
                f,
                "could not write {}: {detail} — the {written} line{} before it {} in \
                 the project",
                path.display(),
                if *written == 1 { "" } else { "s" },
                if *written == 1 { "is" } else { "are" }
            ),
        }
    }
}

impl std::error::Error for ChapterError {}

/// Read a chapter from a text file, measured before it is read (D-126).
///
/// # Errors
///
/// [`ChapterError::Unreadable`] when the file is missing, too large, or not
/// UTF-8 text.
pub fn read(path: &Path) -> Result<String, ChapterError> {
    let unreadable = |detail: String| ChapterError::Unreadable {
        path: path.to_path_buf(),
        detail,
    };
    let size = fs::metadata(path)
        .map_err(|e| unreadable(e.to_string()))?
        .len();
    if size > MAX_CHAPTER_BYTES {
        return Err(unreadable(format!(
            "it is {} — a chapter is text, and this is larger than any chapter ({} MB)",
            crate::import::human_size(size),
            MAX_CHAPTER_BYTES / (1024 * 1024)
        )));
    }
    let bytes = fs::read(path).map_err(|e| unreadable(e.to_string()))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        unreadable("it is not plain text — save the chapter as a UTF-8 .txt file".to_owned())
    })?;
    Ok(text.trim_start_matches('\u{feff}').to_owned())
}

/// Cut a chapter with [`Pacing::new`]'s ends.
#[must_use]
pub fn plan(chapter: &str, min_seconds: f64, max_seconds: f64) -> Vec<Cut> {
    cut(chapter, Pacing::new(min_seconds, max_seconds))
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    /// The scene names written, in order — `023`, `024`, ….
    pub scenes: Vec<String>,
}

impl Imported {
    /// One line, for the terminal and the window alike.
    #[must_use]
    pub fn summary(&self) -> String {
        match (self.scenes.first(), self.scenes.last()) {
            (Some(first), Some(last)) if first == last => {
                format!("1 scene added — scene {first}, waiting for its picture")
            }
            (Some(first), Some(last)) => format!(
                "{} scenes added — {first} to {last}, each waiting for its picture",
                self.scenes.len()
            ),
            _ => "nothing added".to_owned(),
        }
    }
}

/// Write each piece as the next numbered scene, after everything already in
/// the project.
///
/// Blank pieces are skipped: a line with no words is not a scene anybody asked
/// for, and the window can produce one by merging and editing.
///
/// # Errors
///
/// [`ChapterError`] — no folder, a manifest project, no words, a piece too
/// long to speak, or a write that failed part-way.
pub fn import(root: &Path, pieces: &[String]) -> Result<Imported, ChapterError> {
    if !root.is_dir() {
        return Err(ChapterError::NotAProject {
            path: root.to_path_buf(),
        });
    }
    let manifest = root.join(DEFAULT_MANIFEST);
    if manifest.exists() {
        return Err(ChapterError::ManifestProject { manifest });
    }

    let lines: Vec<String> = pieces
        .iter()
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.is_empty())
        .collect();
    if lines.is_empty() {
        return Err(ChapterError::Empty);
    }
    if let Some((index, line)) = lines
        .iter()
        .enumerate()
        .find(|(_, line)| line.len() as u64 > MAX_SCRIPT_BYTES)
    {
        return Err(ChapterError::TooLong {
            piece: index + 1,
            bytes: line.len(),
        });
    }

    // A folder half-way through a renumber holds its scenes under parked
    // names, and numbering around them would put a new line where a parked
    // scene is about to return (D-121). Put it right first.
    let _ = crate::arrange::recover(root);

    let start = next_number(root);
    let last = start + lines.len() - 1;
    let width = MIN_WIDTH.max(last.to_string().len());

    let mut written = Vec::with_capacity(lines.len());
    for (offset, line) in lines.iter().enumerate() {
        let stem = format!("{:0width$}", start + offset, width = width);
        let path = root.join(format!("{stem}.txt"));
        write_new(&path, line).map_err(|detail| ChapterError::Write {
            path: path.clone(),
            written: written.len(),
            detail,
        })?;
        written.push(stem);
    }
    Ok(Imported { scenes: written })
}

/// One past the highest scene number in the folder, counting pictures, lines
/// and recordings alike — a line waiting for its picture holds its number as
/// firmly as a finished scene does.
fn next_number(root: &Path) -> usize {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .filter(|path| {
            path.extension()
                .and_then(OsStr::to_str)
                .map(str::to_ascii_lowercase)
                .is_some_and(|ext| {
                    IMAGE_EXTENSIONS.contains(&ext.as_str())
                        || AUDIO_EXTENSIONS.contains(&ext.as_str())
                        || TEXT_EXTENSIONS.contains(&ext.as_str())
                })
        })
        .filter_map(|path| {
            path.file_stem()
                .and_then(OsStr::to_str)
                .and_then(|stem| stem.parse::<usize>().ok())
        })
        .max()
        .map_or(1, |highest| highest + 1)
}

/// Write `text` to `path`, which must not exist: beside it first, then claimed
/// with `create_new`, then renamed over the claim (D-120, ingest's `copy_in`).
fn write_new(path: &Path, text: &str) -> Result<(), String> {
    let temporary = spoonstill_media::atomic::partial_path(path);
    let written = fs::File::create(&temporary)
        .and_then(|mut file| {
            // No `sync_all`: 506 lines took 2.35 s with one, nearly all of
            // it in the kernel, and on the author's network volume each is a
            // round trip. The rename is what makes the file all-or-nothing to
            // a reader, which is the property that matters (D-120); `copy_in`
            // makes the same choice for photographs.
            file.write_all(text.as_bytes())?;
            file.write_all(b"\n")
        })
        .map_err(|e| e.to_string());
    if let Err(detail) = written {
        let _ = fs::remove_file(&temporary);
        return Err(detail);
    }

    let claim = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path);
    match claim {
        Ok(handle) => drop(handle),
        Err(e) => {
            let _ = fs::remove_file(&temporary);
            return Err(if e.kind() == std::io::ErrorKind::AlreadyExists {
                "a file of that name is already in the project".to_owned()
            } else {
                e.to_string()
            });
        }
    }
    if let Err(e) = fs::rename(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        let _ = fs::remove_file(path);
        return Err(e.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "spoonstill-chapter-{name}-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            ));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).expect("scratch");
            for (file, contents) in files {
                fs::write(dir.join(file), contents).expect("write");
            }
            Scratch(dir)
        }

        fn listing(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.0)
                .expect("list")
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }

        fn read(&self, name: &str) -> String {
            fs::read_to_string(self.0.join(name)).expect("read")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn lines(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| (*t).to_owned()).collect()
    }

    #[test]
    fn an_empty_project_gets_scenes_from_one() {
        let s = Scratch::new("empty", &[]);
        let done = import(&s.0, &lines(&["First line.", "Second line."])).expect("imports");
        assert_eq!(done.scenes, vec!["001", "002"]);
        assert_eq!(s.listing(), vec!["001.txt", "002.txt"]);
        assert_eq!(s.read("002.txt"), "Second line.\n");
    }

    #[test]
    fn lines_go_after_the_last_scene_of_any_kind() {
        let s = Scratch::new(
            "after",
            &[
                ("001.jpeg", "x"),
                ("001.txt", "one"),
                ("002.jpeg", "x"),
                // A line already waiting for its picture holds its number.
                ("007.txt", "seven"),
                ("notes.txt", "not a scene"),
            ],
        );
        let done = import(&s.0, &lines(&["new"])).expect("imports");
        assert_eq!(done.scenes, vec!["008"]);
        assert_eq!(s.read("007.txt"), "seven", "nothing existing is touched");
        assert_eq!(s.read("001.txt"), "one");
    }

    #[test]
    fn nothing_is_ever_overwritten() {
        // A file appearing at the name between choosing it and claiming it is
        // refused, not replaced.
        let s = Scratch::new("claim", &[]);
        let path = s.0.join("001.txt");
        fs::write(&path, "operator's own").expect("write");
        let error = write_new(&path, "new").expect_err("refused");
        assert!(error.contains("already"), "{error}");
        assert_eq!(s.read("001.txt"), "operator's own");
        assert_eq!(s.listing(), vec!["001.txt"], "and no temporary is left");
    }

    #[test]
    fn blank_pieces_are_skipped_and_whitespace_is_collapsed() {
        let s = Scratch::new("blank", &[]);
        let done = import(&s.0, &lines(&["  a \n  b ", "   ", ""])).expect("imports");
        assert_eq!(done.scenes, vec!["001"]);
        assert_eq!(s.read("001.txt"), "a b\n");
        assert!(matches!(
            import(&s.0, &lines(&[" ", ""])),
            Err(ChapterError::Empty)
        ));
    }

    #[test]
    fn the_width_grows_past_999() {
        let s = Scratch::new("wide", &[("998.txt", "x")]);
        let done = import(&s.0, &lines(&["a", "b", "c"])).expect("imports");
        assert_eq!(done.scenes, vec!["0999", "1000", "1001"]);
    }

    #[test]
    fn a_manifest_project_is_refused_before_anything_is_written() {
        let s = Scratch::new("manifest", &[("scenes.csv", "id,image\n")]);
        let error = import(&s.0, &lines(&["a"])).expect_err("refused");
        assert!(error.to_string().contains("scenes.csv"), "{error}");
        assert_eq!(s.listing(), vec!["scenes.csv"]);
    }

    #[test]
    fn a_piece_too_long_to_speak_is_refused_before_anything_is_written() {
        let s = Scratch::new("long", &[]);
        let giant = "word ".repeat(60_000);
        let error = import(&s.0, &[String::from("fine"), giant]).expect_err("refused");
        assert!(
            matches!(error, ChapterError::TooLong { piece: 2, .. }),
            "{error}"
        );
        assert!(s.listing().is_empty());
    }

    #[test]
    fn a_chapter_file_is_measured_before_it_is_read() {
        let s = Scratch::new("read", &[]);
        let big = s.0.join("big.txt");
        let file = fs::File::create(&big).expect("create");
        file.set_len(MAX_CHAPTER_BYTES + 1).expect("grow");
        let error = read(&big).expect_err("refused");
        assert!(
            error.to_string().contains("larger than any chapter"),
            "{error}"
        );

        let binary = s.0.join("photo.txt");
        fs::write(&binary, [0xff, 0xd8, 0xff, 0xe0]).expect("write");
        assert!(
            read(&binary)
                .unwrap_err()
                .to_string()
                .contains("not plain text")
        );

        let bom = s.0.join("bom.txt");
        fs::write(&bom, "\u{feff}Hello.").expect("write");
        assert_eq!(read(&bom).expect("reads"), "Hello.");
    }

    #[test]
    fn imported_lines_are_read_back_as_scenes_waiting_for_pictures() {
        let s = Scratch::new("roundtrip", &[]);
        let pieces: Vec<String> = plan(
            "Chu Kingdom. Haonan Province. The Floating Cloud Sect.\n\n\
             In a log cabin in the outer sect, Adrian Vale sat on the edge of a hard \
             bed, wearing an expression of utter disbelief.",
            3.0,
            5.0,
        )
        .into_iter()
        .map(|c| c.text)
        .collect();
        import(&s.0, &pieces).expect("imports");

        let rows = crate::import::rows::collect(&s.0, &crate::import::Settings::default())
            .expect("collects");
        assert!(rows.drafts.is_empty());
        let words: Vec<String> = rows
            .awaiting
            .iter()
            .map(|a| a.text.clone().unwrap_or_default())
            .collect();
        assert_eq!(words, pieces);
    }
}
