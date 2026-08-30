//! Where the rows come from: a CSV manifest, or the folder itself (D-050).
//!
//! D-050's rule is that mis-pairing must be *structurally impossible* rather
//! than something an operator checks. Two modes, and the manifest wins:
//!
//! ```text
//! convention   001.png  001.txt   -> image + text  -> TTS
//!              002.png  002.mp3   -> image + supplied audio
//!              003.png            -> image only    -> silent, default duration
//!
//! manifest     one CSV, one row per scene, columns:
//!              image, text, audio_file, voice, duration, zoom_direction, zoom_anchor
//! ```
//!
//! Nothing here validates a row — that is `spoonstill_core::project`'s job,
//! and it happens after both modes have produced the same
//! [`SceneDraft`](spoonstill_core::project::SceneDraft) shape. A drafting
//! stage that also judged would judge differently in each mode.
//!
//! ## Scene order is fixed, and it matters
//!
//! Order is the order scenes appear in the film, and it is also the
//! `scene_index` that seeds each scene's motion (D-035) and reaches the cache
//! key (D-043). An unstable order re-rolls every move and invalidates every
//! cache entry on a re-run. So: **manifest order in manifest mode**, and
//! **natural-sorted stems in convention mode** — natural, so `scene2` sorts
//! before `scene10` the way an operator expects, rather than after it the way
//! a byte comparison would have it.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use spoonstill_core::project::{MAX_SCRIPT_BYTES, Problem, ProblemKind, SceneDraft, SceneId};

use super::settings::Settings;

/// The manifest looked for when `project.yaml` does not name one (D-050).
pub const DEFAULT_MANIFEST: &str = "scenes.csv";

/// Extensions treated as stills.
///
/// A hint only: plan.md §M2 is explicit that extensions are not evidence, and
/// the probe in [`super`] is what actually decides. This list exists to pair
/// files by stem, not to certify them.
pub const IMAGE_EXTENSIONS: [&str; 8] =
    ["jpg", "jpeg", "png", "webp", "tif", "tiff", "bmp", "heic"];

/// Extensions treated as narration.
pub const AUDIO_EXTENSIONS: [&str; 8] = ["mp3", "m4a", "wav", "aac", "flac", "ogg", "opus", "wma"];

/// Extensions treated as a line to speak.
pub const TEXT_EXTENSIONS: [&str; 2] = ["txt", "md"];

/// Which mode a project resolved to, so the caller can say so.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// A CSV manifest, at this path.
    Manifest(PathBuf),
    /// Stem-keyed pairing over the folder.
    Convention,
}

/// Rows could not be collected at all.
#[derive(Debug)]
pub enum RowsError {
    /// The project folder could not be listed.
    Unreadable {
        /// Which folder.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// A manifest was named — in `project.yaml` or by default — and is not
    /// there. Named explicitly is an error; the *default* name being absent
    /// simply means convention mode, and never reaches this.
    ManifestMissing {
        /// Which file was expected.
        path: PathBuf,
    },
    /// The CSV will not parse, or its header is not one we accept.
    ManifestMalformed {
        /// Which file.
        path: PathBuf,
        /// What was wrong, with a line number where the parser gave one.
        detail: String,
    },
}

impl std::fmt::Display for RowsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RowsError::Unreadable { path, source } => {
                write!(f, "could not read {}: {source}", path.display())
            }
            RowsError::ManifestMissing { path } => {
                write!(f, "no manifest at {}", path.display())
            }
            RowsError::ManifestMalformed { path, detail } => {
                write!(f, "{}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for RowsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RowsError::Unreadable { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// What a collection produced.
#[derive(Debug)]
pub struct Rows {
    /// How the rows were found.
    pub mode: Mode,
    /// The rows, in render order.
    pub drafts: Vec<SceneDraft>,
    /// Anything unresolved, as typed warnings (D-050).
    pub problems: Vec<Problem>,
}

/// Collect the project's rows, from a manifest if there is one and from the
/// folder if there is not.
///
/// # Errors
///
/// [`RowsError`] when the folder cannot be listed or a manifest cannot be
/// parsed. Rows that are merely *wrong* are not errors here — they become
/// drafts, and the validation stage reports them all at once.
pub fn collect(root: &Path, settings: &Settings) -> Result<Rows, RowsError> {
    match settings.manifest.as_deref() {
        // Named explicitly: it must be there. Falling back to convention mode
        // after an operator asked for a specific manifest would render a
        // different film from the one they described, silently.
        Some(name) => {
            let path = root.join(name);
            if !path.is_file() {
                return Err(RowsError::ManifestMissing { path });
            }
            from_manifest(&path)
        }
        None => {
            let path = root.join(DEFAULT_MANIFEST);
            if path.is_file() {
                from_manifest(&path)
            } else {
                from_convention(root, settings)
            }
        }
    }
}

/// One CSV row, exactly as D-050 specifies the columns.
///
/// `deny_unknown_fields` for D-055's reason: a column named `zoom_ancor` is a
/// typo that would otherwise be dropped without a word, and every scene would
/// quietly take the seeded anchor instead.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestRow {
    image: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    audio_file: Option<String>,
    #[serde(default)]
    voice: Option<String>,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    zoom_direction: Option<String>,
    #[serde(default)]
    zoom_anchor: Option<String>,
    /// D-106. Outside D-020's exactly-one rule, because it is not a source of
    /// audio — it is what the viewer reads while the audio plays.
    #[serde(default)]
    caption: Option<String>,
}

fn from_manifest(path: &Path) -> Result<Rows, RowsError> {
    let mut reader = csv::ReaderBuilder::new()
        .trim(csv::Trim::All)
        .flexible(false)
        .from_path(path)
        .map_err(|e| RowsError::ManifestMalformed {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;

    let mut drafts = Vec::new();
    let mut problems = Vec::new();

    for (index, record) in reader.deserialize::<ManifestRow>().enumerate() {
        let row = record.map_err(|e| RowsError::ManifestMalformed {
            path: path.to_path_buf(),
            detail: e.to_string(),
        })?;

        // The scene ID is the image's stem, which is what D-050's convention
        // mode uses too — so a project can move between modes without every
        // scene changing identity, re-seeding its motion (D-035) and missing
        // its cache (D-043).
        let id = stem_of(&row.image).unwrap_or_else(|| format!("row {}", index + 1));

        // `duration` arrives as text so that a cell reading `soon` becomes a
        // reported problem rather than a parse failure that stops the run at
        // row 1 of 500.
        let mut duration = None;
        if let Some(text) = non_empty(row.duration) {
            match text.parse::<f64>() {
                Ok(seconds) => duration = Some(seconds),
                Err(_) => problems.push(match spoonstill_core::project::SceneId::new(&id) {
                    Ok(scene) => Problem::in_scene(
                        scene,
                        ProblemKind::UnusableSetting {
                            field: "duration",
                            value: text,
                            expected: "a number of seconds",
                        },
                    ),
                    Err(_) => Problem::in_project(ProblemKind::UnusableSetting {
                        field: "duration",
                        value: text,
                        expected: "a number of seconds",
                    }),
                }),
            }
        }

        drafts.push(SceneDraft {
            id,
            image: non_empty(Some(row.image)),
            text: non_empty(row.text),
            audio: non_empty(row.audio_file),
            duration,
            voice: non_empty(row.voice),
            zoom_direction: non_empty(row.zoom_direction),
            zoom_anchor: non_empty(row.zoom_anchor),
            caption: non_empty(row.caption),
        });
    }

    Ok(Rows {
        mode: Mode::Manifest(path.to_path_buf()),
        drafts,
        problems,
    })
}

/// Stem-keyed pairing over the folder (D-050).
fn from_convention(root: &Path, settings: &Settings) -> Result<Rows, RowsError> {
    let entries = std::fs::read_dir(root).map_err(|source| RowsError::Unreadable {
        path: root.to_path_buf(),
        source,
    })?;

    // Keyed by stem so pairing is a lookup rather than a search, and ordered
    // by the natural key so the render order is stable (see the module docs).
    //
    // The stem half of the key is **case-folded** (D-111). `Shot.jpg` beside
    // `shot.wav` is one scene to the operator who named them, and keying on the
    // raw spelling made it two: a silent still, and a recording reported as
    // pairing with no image.
    let mut groups: BTreeMap<(Vec<NaturalPart>, String), Group> = BTreeMap::new();
    let mut problems = Vec::new();

    // Read the whole folder, then sort it. `read_dir` order is whatever the
    // filesystem returns — unspecified by std, and different between APFS, ext4
    // and NTFS — and this loop decides which of two candidate files a scene
    // gets. A project must not resolve differently on the machine it is
    // rendered on (D-071, D-111).
    let mut names: Vec<String> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| RowsError::Unreadable {
            path: root.to_path_buf(),
            source,
        })?;
        if !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if let Some(name) = entry.file_name().to_str() {
            names.push(name.to_owned());
        }
    }
    names.sort_unstable();

    // Files an interrupted `arrange` left behind (D-121). Counted before the
    // dotfile skip below, because that skip is exactly why they were invisible:
    // a project of 2000 scenes reported "1566 scenes — no problems".
    let interrupted = names
        .iter()
        .filter(|name| name.starts_with(".arranging-"))
        .count();
    if interrupted > 0 {
        problems.push(Problem::in_project(ProblemKind::InterruptedRename {
            files: interrupted,
        }));
    }

    for name in &names {
        let name = name.as_str();
        // `.DS_Store`, `._resource forks`, editor swap files. Never scenes.
        if name.starts_with('.') {
            continue;
        }
        let path = Path::new(name);
        let (Some(stem), Some(extension)) = (
            path.file_stem().and_then(OsStr::to_str),
            path.extension().and_then(OsStr::to_str),
        ) else {
            continue;
        };
        let extension = extension.to_ascii_lowercase();

        let slot = if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
            Slot::Image
        } else if AUDIO_EXTENSIONS.contains(&extension.as_str()) {
            Slot::Audio
        } else if TEXT_EXTENSIONS.contains(&extension.as_str()) {
            Slot::Text
        } else {
            // A README, a licence, a spreadsheet. Not a warning: a project
            // folder is a working folder, and warning about every file in it
            // would train operators to ignore warnings (D-050 asks us to
            // report *unresolved rows*, not unrelated files).
            continue;
        };

        let group = groups
            .entry((natural_key(stem), stem.to_lowercase()))
            .or_insert_with(|| Group {
                stem: stem.to_owned(),
                ..Group::default()
            });
        // Every candidate is kept, not just the first. Keeping only the first
        // meant `001.jpg` beside `001.png` silently discarded one and reported
        // "no problems" (D-111).
        match slot {
            Slot::Image => group.image.push(name.to_owned()),
            Slot::Audio => group.audio.push(name.to_owned()),
            Slot::Text => group.text.push(name.to_owned()),
        }
    }

    let mut drafts = Vec::new();
    for group in groups.into_values() {
        // Ambiguity is reported before anything is chosen from it (D-111).
        // Reported per slot and against the scene, so `still validate` prints
        // the stem, the job, and every file competing for it — the operator
        // needs to know *which* file to remove, not that something is wrong.
        for (slot, candidates) in [
            ("image", &group.image),
            ("narration", &group.audio),
            ("text", &group.text),
        ] {
            if candidates.len() > 1 {
                let kind = ProblemKind::AmbiguousScene {
                    slot,
                    candidates: candidates.clone(),
                };
                problems.push(match SceneId::new(&group.stem) {
                    Ok(scene) => Problem::in_scene(scene, kind),
                    Err(_) => Problem::in_project(kind),
                });
            }
        }

        // Sorted above, so "the first" is the same file on every machine.
        let group_audio = group.audio.into_iter().next();
        let group_text = group.text.into_iter().next();

        let Some(image) = group.image.into_iter().next() else {
            // Narration with no still. Reported, never skipped in silence:
            // "scene 12 never rendered" has to be answerable (D-050).
            for orphan in [group_audio, group_text].into_iter().flatten() {
                problems.push(Problem::in_project(ProblemKind::UnpairedFile {
                    value: orphan,
                }));
            }
            continue;
        };

        // A stem with both a `.txt` and an `.mp3` used to be D-020's two-source
        // case, reported as a conflict. It is not one (D-106): a recording is
        // the narration and the writing beside it is what the narration *says*,
        // which is a caption. Nothing is guessed and nothing is ambiguous —
        // the recording is still the only audio source, so D-020 holds — and
        // the alternative was that an operator who records their own voiceover
        // could never have subtitles at all. This turns an error into a
        // working scene, so it cannot break a project that renders today.
        let text = match &group_text {
            Some(name) => match read_line(&root.join(name)) {
                Ok(text) => Some(text),
                Err(detail) => {
                    problems.push(Problem::in_project(ProblemKind::NotUsableMedia {
                        field: "text",
                        value: name.clone(),
                        detail,
                    }));
                    None
                }
            },
            None => None,
        };

        // Image alone: a silent scene of the project's default length (D-050).
        let duration = (group_audio.is_none() && text.is_none() && group_text.is_none())
            .then_some(settings.silent_seconds);

        // With a recording present the writing is the caption; without one it
        // is the script to speak.
        let (text, caption) = if group_audio.is_some() {
            (None, text)
        } else {
            (text, None)
        };

        drafts.push(SceneDraft {
            id: group.stem,
            image: Some(image),
            text,
            audio: group_audio,
            duration,
            voice: None,
            zoom_direction: None,
            zoom_anchor: None,
            caption,
        });
    }

    Ok(Rows {
        mode: Mode::Convention,
        drafts,
        problems,
    })
}

#[derive(Debug, Default)]
struct Group {
    stem: String,
    /// Every file that could be this scene's still, sorted. More than one is
    /// an ambiguity to report, not a choice to make quietly (D-111).
    image: Vec<String>,
    /// Every file that could be its narration.
    audio: Vec<String>,
    /// Every file that could be its words.
    text: Vec<String>,
}

enum Slot {
    Image,
    Audio,
    Text,
}

/// Read a narration file: UTF-8, BOM stripped, trailing newline dropped.
///
/// A `.txt` an operator exported from a Windows editor starts with a BOM, and
/// a BOM in a TTS request is a spoken artefact or a rejected call depending on
/// the provider. Strip it here, once.
fn read_line(path: &Path) -> Result<String, String> {
    // Asked how big it is before it is read (D-126). A 191 MB `.txt` used to
    // reach `still validate` as 607 MB resident — the bytes, the `String`, and
    // the trimmed copy — and be reported as a scene with *no problems*, because
    // the length refusal lives in the provider and does not run until render.
    let size = std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("could not read it: {e}"))?;
    if size > MAX_SCRIPT_BYTES {
        return Err(format!(
            "is {} MB of text — no scene can hold more than {} KB of narration, \
             so this is not a script",
            size / (1024 * 1024),
            MAX_SCRIPT_BYTES / 1024
        ));
    }

    let bytes = std::fs::read(path).map_err(|e| format!("could not read it: {e}"))?;
    let text =
        String::from_utf8(bytes).map_err(|_| "not UTF-8 text — re-save it as UTF-8".to_owned())?;
    Ok(text.trim_start_matches('\u{feff}').trim().to_owned())
}

/// The file stem of a path written in a manifest, in either separator style.
///
/// Manifests are copied between machines (D-013), so a row written on Windows
/// as `img\001.jpg` must key the same scene as `img/001.jpg` does on macOS.
fn stem_of(value: &str) -> Option<String> {
    let tail = value.rsplit(['/', '\\']).next()?;
    let stem = Path::new(tail).file_stem()?.to_str()?.trim();
    (!stem.is_empty()).then(|| stem.to_owned())
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// One run of a natural sort key: digits compare as numbers, everything else
/// as lowercased text.
///
/// **Variant order is load-bearing**, because the derived `Ord` compares
/// variants before contents: `Number` first means `001.png` sorts before
/// `intro.png`, the way ASCII digits sort before letters and the way an
/// operator numbering a folder expects. Reordering these two silently
/// reorders every convention-mode project, which re-seeds every motion
/// (D-035) and misses every cache entry (D-043).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum NaturalPart {
    /// A run of digits, as a number — which is the whole point.
    Number(u128),
    /// A run of text. Lowercased so `Scene` and `scene` file together.
    Text(String),
}

/// Split a stem into alternating text and number runs.
///
/// `scene2` before `scene10`, and `001` before `002` either way. A digit run
/// too long to be a number stays text rather than overflowing, which is the
/// only case where the order falls back to bytes.
fn natural_key(stem: &str) -> Vec<NaturalPart> {
    let mut parts = Vec::new();
    let mut rest = stem;

    while !rest.is_empty() {
        let digits = rest.find(|c: char| c.is_ascii_digit());
        match digits {
            None => {
                parts.push(NaturalPart::Text(rest.to_lowercase()));
                break;
            }
            Some(at) => {
                if at > 0 {
                    parts.push(NaturalPart::Text(rest[..at].to_lowercase()));
                }
                let run_end = rest[at..]
                    .find(|c: char| !c.is_ascii_digit())
                    .map_or(rest.len(), |offset| at + offset);
                let run = &rest[at..run_end];
                match run.parse::<u128>() {
                    Ok(number) => parts.push(NaturalPart::Number(number)),
                    Err(_) => parts.push(NaturalPart::Text(run.to_owned())),
                }
                rest = &rest[run_end..];
            }
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A scratch project folder that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(files: &[(&str, &str)]) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "spoonstill-rows-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("scratch dir");
            for (name, contents) in files {
                std::fs::write(dir.join(name), contents).expect("scratch file");
            }
            Scratch(dir)
        }

        fn collect(&self) -> Rows {
            super::collect(&self.0, &Settings::default()).expect("collects")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn ids(rows: &Rows) -> Vec<&str> {
        rows.drafts.iter().map(|d| d.id.as_str()).collect()
    }

    /// D-050's three convention cases, in one folder.
    #[test]
    fn convention_mode_pairs_by_stem() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            ("001.txt", "the first line"),
            ("002.png", ""),
            ("002.mp3", ""),
            ("003.png", ""),
        ]);
        let rows = scratch.collect();

        assert_eq!(rows.mode, Mode::Convention);
        assert_eq!(ids(&rows), vec!["001", "002", "003"]);

        assert_eq!(rows.drafts[0].text.as_deref(), Some("the first line"));
        assert_eq!(rows.drafts[0].audio, None);

        assert_eq!(rows.drafts[1].audio.as_deref(), Some("002.mp3"));
        assert_eq!(rows.drafts[1].text, None);

        assert_eq!(
            rows.drafts[2].duration,
            Some(Settings::default().silent_seconds)
        );
        assert!(rows.problems.is_empty(), "{:?}", rows.problems);
    }

    /// Order is the film's order and the motion seed (D-035). `scene2` must
    /// come before `scene10`, which a byte comparison gets backwards.
    #[test]
    fn convention_mode_orders_stems_naturally() {
        let scratch = Scratch::new(&[
            ("scene10.png", ""),
            ("scene2.png", ""),
            ("scene1.png", ""),
            ("Scene3.png", ""),
        ]);
        assert_eq!(
            ids(&scratch.collect()),
            vec!["scene1", "scene2", "Scene3", "scene10"]
        );
    }

    #[test]
    fn convention_mode_is_stable_across_runs() {
        let scratch = Scratch::new(&[("b.png", ""), ("a.png", ""), ("c.png", "")]);
        let first = ids(&scratch.collect()).join(",");
        for _ in 0..5 {
            assert_eq!(ids(&scratch.collect()).join(","), first);
        }
    }

    /// D-050: unresolved inputs are reported, never silently skipped.
    #[test]
    fn narration_with_no_still_is_a_warning() {
        let scratch = Scratch::new(&[("001.png", ""), ("002.mp3", ""), ("003.txt", "orphan")]);
        let rows = scratch.collect();

        assert_eq!(ids(&rows), vec!["001"]);
        assert_eq!(rows.problems.len(), 2, "{:?}", rows.problems);
        for problem in &rows.problems {
            assert_eq!(
                problem.severity(),
                spoonstill_core::Severity::Warn,
                "a stray take must not stop the render"
            );
        }
        let rendered: Vec<String> = rows.problems.iter().map(ToString::to_string).collect();
        assert!(
            rendered.iter().any(|p| p.contains("002.mp3")),
            "{rendered:?}"
        );
        assert!(
            rendered.iter().any(|p| p.contains("003.txt")),
            "{rendered:?}"
        );
    }

    /// A working folder holds working files. Warning about all of them is how
    /// warnings become noise.
    #[test]
    fn unrelated_files_are_not_warnings() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            ("README.md.bak", ""),
            ("notes.xlsx", ""),
            (".DS_Store", ""),
        ]);
        let rows = scratch.collect();
        assert_eq!(ids(&rows), vec!["001"]);
        assert!(rows.problems.is_empty(), "{:?}", rows.problems);
    }

    /// Both partners present is **not** D-020's two-source case (D-106).
    ///
    /// It used to be reported as a conflict, which meant an operator who
    /// records their own voiceover could never have subtitles: the only place
    /// to put the words was the cell that also means "speak these". The
    /// recording is the narration and the writing beside it is the caption, so
    /// there is still exactly one audio source and nothing is guessed.
    #[test]
    fn a_stem_with_a_recording_and_writing_captions_the_recording() {
        let scratch = Scratch::new(&[("001.png", ""), ("001.txt", "hello"), ("001.mp3", "")]);
        let rows = scratch.collect();

        assert_eq!(rows.drafts.len(), 1);
        let draft = &rows.drafts[0];
        assert_eq!(
            draft.audio.as_deref(),
            Some("001.mp3"),
            "the recording narrates"
        );
        assert_eq!(
            draft.caption.as_deref(),
            Some("hello"),
            "the writing captions"
        );
        assert_eq!(
            draft.text, None,
            "D-020 still holds: the recording is the only audio source"
        );
        assert_eq!(draft.duration, None, "not a silent scene either");
    }

    /// Writing with no recording is still a script to speak, not a caption.
    /// The two halves of the rule above, so neither can drift into the other.
    #[test]
    fn a_stem_with_writing_alone_is_still_a_script() {
        let scratch = Scratch::new(&[("001.png", ""), ("001.txt", "hello")]);
        let draft = &scratch.collect().drafts[0];
        assert_eq!(draft.text.as_deref(), Some("hello"));
        assert_eq!(draft.caption, None);
    }

    #[test]
    fn a_narration_file_is_read_without_its_bom_or_trailing_newline() {
        let scratch = Scratch::new(&[("001.png", ""), ("001.txt", "\u{feff}hello there\n\n")]);
        assert_eq!(
            scratch.collect().drafts[0].text.as_deref(),
            Some("hello there")
        );
    }

    #[test]
    fn an_empty_folder_produces_no_rows_and_no_panic() {
        let scratch = Scratch::new(&[]);
        let rows = scratch.collect();
        assert!(rows.drafts.is_empty());
        assert!(rows.problems.is_empty());
    }

    #[test]
    fn a_manifest_wins_over_the_folder() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            ("001.txt", "convention would use this"),
            ("002.png", ""),
            (
                DEFAULT_MANIFEST,
                "image,text,audio_file,voice,duration,zoom_direction,zoom_anchor\n\
                 001.png,the manifest line,,rachel,,zoom-in,top-left\n",
            ),
        ]);
        let rows = scratch.collect();

        assert_eq!(rows.mode, Mode::Manifest(scratch.0.join(DEFAULT_MANIFEST)));
        assert_eq!(ids(&rows), vec!["001"], "D-050: the manifest is the list");

        let draft = &rows.drafts[0];
        assert_eq!(draft.text.as_deref(), Some("the manifest line"));
        assert_eq!(draft.voice.as_deref(), Some("rachel"));
        assert_eq!(draft.zoom_direction.as_deref(), Some("zoom-in"));
        assert_eq!(draft.zoom_anchor.as_deref(), Some("top-left"));
    }

    #[test]
    fn manifest_rows_keep_their_order() {
        let scratch = Scratch::new(&[(
            DEFAULT_MANIFEST,
            "image,duration\n\
             zulu.png,1\n\
             alpha.png,2\n\
             mike.png,3\n",
        )]);
        assert_eq!(ids(&scratch.collect()), vec!["zulu", "alpha", "mike"]);
    }

    #[test]
    fn a_manifest_may_omit_optional_columns() {
        let scratch = Scratch::new(&[(DEFAULT_MANIFEST, "image,text\n001.png,hello\n")]);
        let rows = scratch.collect();
        assert_eq!(rows.drafts[0].text.as_deref(), Some("hello"));
        assert_eq!(rows.drafts[0].audio, None);
    }

    /// D-055 again: a misspelled column is refused, not dropped. Silently
    /// ignoring `zoom_ancor` gives every scene the seeded anchor instead of
    /// the one the operator specified, 500 times.
    #[test]
    fn a_misspelled_column_is_refused() {
        let scratch = Scratch::new(&[(DEFAULT_MANIFEST, "image,zoom_ancor\n001.png,top-left\n")]);
        let error = super::collect(&scratch.0, &Settings::default()).expect_err("unknown column");
        assert!(
            error.to_string().contains("zoom_ancor") || error.to_string().contains("unknown"),
            "{error}"
        );
    }

    /// An unparseable `duration` is one reported scene, not a stopped run —
    /// otherwise row 1 of 500 hides the other 499 problems.
    #[test]
    fn an_unparseable_duration_is_a_problem_not_a_stopped_run() {
        let scratch = Scratch::new(&[(
            DEFAULT_MANIFEST,
            "image,duration\n001.png,soon\n002.png,2.5\n",
        )]);
        let rows = scratch.collect();

        assert_eq!(ids(&rows), vec!["001", "002"]);
        assert_eq!(rows.drafts[0].duration, None);
        assert_eq!(rows.drafts[1].duration, Some(2.5));
        assert_eq!(rows.problems.len(), 1);
        assert!(rows.problems[0].to_string().contains("soon"));
        assert_eq!(
            rows.problems[0].scene.as_ref().map(|s| s.as_str()),
            Some("001"),
            "with the scene id attached (D-052)"
        );
    }

    /// A manifest an operator named must be there. Falling back to convention
    /// mode would render a different film from the one they described.
    #[test]
    fn a_named_manifest_that_is_missing_is_an_error_not_a_fallback() {
        let scratch = Scratch::new(&[("001.png", "")]);
        let settings = Settings {
            manifest: Some("rows.csv".to_owned()),
            ..Settings::default()
        };
        let error = super::collect(&scratch.0, &settings).expect_err("named but absent");
        assert!(
            matches!(error, RowsError::ManifestMissing { .. }),
            "{error}"
        );
    }

    /// A manifest is copied between machines (D-013), so a row written on
    /// Windows must key the same scene as one written on macOS.
    #[test]
    fn a_row_written_with_windows_separators_keys_the_same_scene() {
        assert_eq!(stem_of(r"img\001.jpg").as_deref(), Some("001"));
        assert_eq!(stem_of("img/001.jpg").as_deref(), Some("001"));
        assert_eq!(stem_of("001.jpg").as_deref(), Some("001"));
        assert_eq!(
            stem_of("  spaced name .jpg  ").as_deref(),
            Some("spaced name")
        );
        assert_eq!(stem_of(""), None);
    }

    #[test]
    fn natural_keys_order_numbers_as_numbers() {
        let mut stems = ["a10", "a9", "a100", "b1", "2", "10", "1", "intro"];
        stems.sort_by_key(|s| natural_key(s));
        assert_eq!(
            stems,
            ["1", "2", "10", "a9", "a10", "a100", "b1", "intro"],
            "numbered stems first, then text, and numbers compared as numbers"
        );
    }

    /// A digit run too long for `u128` must not panic or wrap — it falls back
    /// to text, which is the only case where order is byte order.
    #[test]
    fn an_absurd_digit_run_falls_back_to_text() {
        let huge = "9".repeat(60);
        assert_eq!(natural_key(&huge), vec![NaturalPart::Text(huge)]);
    }

    /// D-126. A file that is not a script is refused by its *size*, before it
    /// is read.
    ///
    /// Measured before this existed: a 191 MB `.txt` took `still validate` to
    /// **607 MB resident** — the bytes, the `String`, and the trimmed copy —
    /// and was reported as a scene with **no problems**, because the length
    /// refusal lives in the provider and does not run until render. Afterwards
    /// the same folder validates in 14 MB and says what is wrong.
    #[test]
    fn a_script_too_large_to_be_narration_is_refused_before_it_is_read() {
        let big = "x".repeat((MAX_SCRIPT_BYTES + 1) as usize);
        let scratch = Scratch::new(&[("001.png", ""), ("001.txt", &big)]);
        let rows = scratch.collect();

        let said: Vec<String> = rows.problems.iter().map(ToString::to_string).collect();
        assert!(
            said.iter().any(|p| p.contains("not a script")),
            "a file too big to be narration was accepted: {said:?}"
        );

        // And the scene does not quietly become a narrated one on the strength
        // of a file that was never read.
        assert!(rows.drafts.iter().all(|d| d.text.is_none()));
    }

    /// The ordinary case is untouched: a real script is nowhere near the cap,
    /// and the cap must not cost a `metadata` call its correctness.
    #[test]
    fn a_script_of_an_ordinary_length_is_read_as_before() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            (
                "001.txt",
                "Chu Kingdom. Haonan Province. The Floating Cloud Sect.",
            ),
        ]);
        let rows = scratch.collect();

        assert!(rows.problems.is_empty(), "{:?}", rows.problems.len());
        assert_eq!(
            rows.drafts[0].text.as_deref(),
            Some("Chu Kingdom. Haonan Province. The Floating Cloud Sect.")
        );
    }

    /// D-111. A stem is matched case-insensitively, because the operator who
    /// named `Shot.jpg` and `shot.wav` named one scene. Keying on the raw
    /// spelling made it two: a silent still, and a recording reported as
    /// pairing with no image — so a recorded voiceover rendered silent.
    ///
    /// `ingest::stem_of` already folded case for exactly this reason; this is
    /// the folder scan agreeing with the other half of its own convention.
    #[test]
    fn a_stem_is_one_scene_whatever_its_case() {
        let scratch = Scratch::new(&[("Shot.jpg", ""), ("shot.wav", "")]);
        let rows = scratch.collect();

        assert_eq!(ids(&rows), vec!["Shot"], "one scene, not two");
        assert_eq!(
            rows.drafts[0].audio.as_deref(),
            Some("shot.wav"),
            "the recording must belong to the still it was named for"
        );
        assert!(
            rows.problems.is_empty(),
            "nothing is unpaired here: {:?}",
            rows.problems
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        );
    }

    /// Two files claiming one job is reported, not resolved quietly. Before
    /// this the scan kept whichever `read_dir` yielded first, discarded the
    /// other, and printed "no problems" over a scene built from a file the
    /// operator never chose.
    #[test]
    fn two_files_claiming_one_scene_are_reported_by_name() {
        let scratch = Scratch::new(&[
            ("001.jpg", ""),
            ("001.png", ""),
            ("001.wav", ""),
            ("001.mp3", ""),
        ]);
        let rows = scratch.collect();

        let said: Vec<String> = rows.problems.iter().map(ToString::to_string).collect();
        assert_eq!(said.len(), 2, "one per ambiguous slot: {said:?}");

        let image = said.iter().find(|p| p.contains("image")).expect("image");
        assert!(
            image.contains("001.jpg") && image.contains("001.png"),
            "{image}"
        );
        let audio = said
            .iter()
            .find(|p| p.contains("narration"))
            .expect("audio");
        assert!(
            audio.contains("001.mp3") && audio.contains("001.wav"),
            "{audio}"
        );

        for problem in &rows.problems {
            assert_eq!(
                problem.scene.as_ref().map(|s| s.as_str()),
                Some("001"),
                "the operator has to be told which scene to go and fix"
            );
            assert_eq!(
                problem.kind.severity(),
                spoonstill_core::diagnostics::Severity::Error,
                "guessing which file was meant is how 500 scenes render wrong"
            );
        }
    }

    /// The choice is the same on every machine. `read_dir` order is whatever
    /// the filesystem returns — unspecified by std, and different between APFS,
    /// ext4 and NTFS — so the scan sorts before it pairs. Creation order must
    /// not decide which still a scene gets.
    #[test]
    fn the_file_chosen_does_not_depend_on_the_order_it_was_written() {
        let forwards = Scratch::new(&[("001.jpg", ""), ("001.png", "")]);
        let backwards = Scratch::new(&[("001.png", ""), ("001.jpg", "")]);

        assert_eq!(
            forwards.collect().drafts[0].image,
            backwards.collect().drafts[0].image,
        );
        assert_eq!(
            forwards.collect().drafts[0].image.as_deref(),
            Some("001.jpg"),
            "sorted, so it is nameable in a decision rather than whatever \
             the filesystem happened to hand back"
        );
    }
}
