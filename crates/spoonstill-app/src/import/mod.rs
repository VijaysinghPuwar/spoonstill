//! Turning a folder into a project (D-013, D-050, D-056).
//!
//! Nothing here renders. This is the stage that reads what an operator wrote —
//! a `project.yaml`, a CSV manifest, or nothing but a folder of images — and
//! produces either scenes the renderer will accept or a list of everything
//! wrong, each item carrying the scene it belongs to.
//!
//! ```text
//! settings::load   project.yaml, or the defaults if there is none
//! rows::collect    the CSV manifest, or stem-keyed pairing (D-050)
//! core::validate   exactly one audio source, motion parses, ids are usable
//! resolve          every path contained in the project root (D-054)
//! MediaCheck       and every file really is the media it claims to be
//! ```
//!
//! **Every stage contributes to one problem list.** That is the whole design:
//! plan.md §M2 asks `still validate` to report every problem at once, and a
//! pipeline that fails at the first stage reports its own stage's problems and
//! hides the rest. Only two things abort — a `project.yaml` that will not
//! parse and a manifest that will not parse — because after either of those
//! there is nothing left to check.
//!
//! ## The probe is not optional, and it is not an extension check
//!
//! plan.md §M2: *extensions are a hint, not evidence*. A `.jpg` that is really
//! a PDF, a zero-byte `.mp3`, and `truncated.jpg` all pass any name-based test
//! and all fail a render (D-052). So the last stage asks `ffprobe`. It is
//! behind [`MediaCheck`] so the tests here need no FFmpeg and no fixtures —
//! and so that M3's queue can swap in a cached implementation when it probes
//! 500 files rather than 3.

pub mod rows;
pub mod settings;

use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_core::path_safety::{PathError, RealPath, resolve_within};
use spoonstill_core::project::{
    AudioSource, Problem, ProblemKind, SceneSpec, Validation, validate_drafts,
};
use spoonstill_core::remedy::Remedy;
use spoonstill_media::{Tools, probe};

pub use rows::{Mode, Rows, RowsError};
pub use settings::{Settings, SettingsError};

/// How long a single validation probe may take.
///
/// Shorter than the render-time probe budget on purpose: `still validate` is
/// meant to be quick, and a file that takes ten seconds to identify is a file
/// worth reporting rather than waiting for.
pub const VALIDATE_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// What a file is supposed to be.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The still.
    Image,
    /// The narration.
    Audio,
}

impl Role {
    /// The manifest column this role comes from, for the problem message.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Role::Image => "image",
            Role::Audio => "audio_file",
        }
    }
}

/// Deciding whether a file really is the media it claims to be.
///
/// A trait because the answer needs a subprocess, and because the tests for
/// everything *around* it should not.
pub trait MediaCheck {
    /// `Ok(())` if the file is usable in that role, or a human cause (D-052).
    ///
    /// # Errors
    ///
    /// A sentence an operator can act on — already mapped from FFmpeg's
    /// stderr, never raw.
    fn check(&self, path: &Path, role: Role) -> Result<(), String>;

    /// Whether this check can run at all on this machine (D-103).
    ///
    /// Asked **once, before any file is looked at**. A check whose subprocess
    /// is not installed fails identically for every file, and answering that
    /// per file states one installable fact five hundred times while burying
    /// it under the operator's own filenames. Answered here, it is one
    /// project-level problem that names the missing tool.
    ///
    /// The default is "it can", so a check that needs nothing — every
    /// in-memory one in the tests below — implements only [`MediaCheck::check`].
    ///
    /// # Errors
    ///
    /// A [`Remedy`]: the plain sentence an operator reads, the tool id the
    /// window turns into an Install button, and the technical detail kept
    /// apart from both (D-105).
    fn ready(&self) -> Result<(), Remedy> {
        Ok(())
    }
}

/// The real check: ask `ffprobe`.
#[derive(Debug, Clone)]
pub struct ProbeCheck {
    tools: Tools,
    timeout: Duration,
}

impl ProbeCheck {
    /// Use the `ffmpeg` and `ffprobe` on this machine.
    #[must_use]
    pub fn from_env() -> Self {
        ProbeCheck {
            tools: Tools::from_env(),
            timeout: VALIDATE_PROBE_TIMEOUT,
        }
    }
}

impl Default for ProbeCheck {
    fn default() -> Self {
        Self::from_env()
    }
}

impl MediaCheck for ProbeCheck {
    fn ready(&self) -> Result<(), Remedy> {
        self.tools.ready()
    }

    fn check(&self, path: &Path, role: Role) -> Result<(), String> {
        let probed = probe(&self.tools, path, self.timeout).map_err(|e| e.to_string())?;
        match role {
            // `source_geometry` already refuses a file with no video stream
            // and one that probes as 0x0 — `truncated.jpg` is the fixture for
            // exactly that, and it must be named rather than divided by.
            Role::Image => probed
                .source_geometry()
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Role::Audio => {
                if probed.audio().is_none() {
                    return Err("carries no audio stream".to_owned());
                }
                // Duration is *not* trusted here — D-021 measures it on the
                // normalized artifact, later. This only refuses the file that
                // has no usable audio at all, `zero_byte.mp3` being the
                // fixture.
                match probed.audio_duration() {
                    Some(_) => Ok(()),
                    None => Err("has an audio stream but no usable duration".to_owned()),
                }
            }
        }
    }
}

/// A scene whose paths have been resolved and whose files have been seen.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedScene {
    /// The row as validated, still carrying the operator's own spelling.
    pub spec: SceneSpec,
    /// Canonical path to the still, inside the project (D-054).
    pub image: PathBuf,
    /// Canonical path to the supplied narration, for
    /// [`AudioSource::File`] scenes only.
    pub audio: Option<PathBuf>,
}

/// A project, as far as it can be known without rendering anything.
#[derive(Debug)]
pub struct Project {
    /// Canonical project root.
    pub root: PathBuf,
    /// Project-level settings, defaulted where the file was silent.
    pub settings: Settings,
    /// Which mode the rows came from (D-050).
    pub mode: Mode,
    /// Scenes that resolved, in render order.
    pub scenes: Vec<ResolvedScene>,
    /// Everything wrong, from every stage, in one list.
    pub problems: Vec<Problem>,
}

impl Project {
    /// Whether anything found stops a render.
    ///
    /// The render gate. Warnings — an unpaired take, an image no manifest row
    /// mentions — are reported and do not block (D-050).
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.problems
            .iter()
            .any(|p| p.severity() >= spoonstill_core::Severity::Error)
    }

    /// Problems that stop the render.
    pub fn errors(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity() >= spoonstill_core::Severity::Error)
    }

    /// Problems that do not.
    pub fn warnings(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity() < spoonstill_core::Severity::Error)
    }
}

/// The project could not be read at all.
#[derive(Debug)]
pub enum ImportError {
    /// The root does not exist, or is not a directory.
    NoProject {
        /// What was asked for.
        path: PathBuf,
    },
    /// `project.yaml` will not load.
    Settings(SettingsError),
    /// The rows will not load.
    Rows(RowsError),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NoProject { path } => {
                write!(f, "{} is not a project folder", path.display())
            }
            ImportError::Settings(e) => write!(f, "{e}"),
            ImportError::Rows(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ImportError::NoProject { .. } => None,
            ImportError::Settings(e) => Some(e),
            ImportError::Rows(e) => Some(e),
        }
    }
}

/// `std::fs::canonicalize`, which is the one thing `spoonstill-core` cannot do
/// for itself (D-010, D-054).
struct StdFs;

impl RealPath for StdFs {
    fn real_path(&self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }
}

/// Read a project folder and say everything that is true about it.
///
/// # Errors
///
/// [`ImportError`] only for the three cases where there is nothing left to
/// check: no folder, an unparseable `project.yaml`, an unparseable manifest.
/// Everything else is a [`Problem`] in the returned project.
pub fn load(root: &Path, media: &dyn MediaCheck) -> Result<Project, ImportError> {
    let root = std::fs::canonicalize(root).map_err(|_| ImportError::NoProject {
        path: root.to_path_buf(),
    })?;
    if !root.is_dir() {
        return Err(ImportError::NoProject { path: root });
    }

    let (settings, mut problems) = settings::load(&root).map_err(ImportError::Settings)?;
    let rows = rows::collect(&root, &settings).map_err(ImportError::Rows)?;
    problems.extend(rows.problems);

    // Whether the probe can run at all is asked once, here, and not once per
    // file (D-103). When it cannot, the rows still resolve — the operator sees
    // their scenes, and one problem tells them what to install — rather than
    // every image being reported as unusable media, which is both untrue and
    // unactionable.
    let probes = media.ready();
    if let Err(remedy) = &probes {
        problems.push(Problem::in_project(ProblemKind::ToolingMissing {
            remedy: remedy.clone(),
        }));
    }
    let probes = probes.is_ok();

    let Validation {
        scenes,
        problems: validation_problems,
    } = validate_drafts(&rows.drafts, &settings.provider, &settings.voice);
    problems.extend(validation_problems);

    // Resolution runs over the **drafts**, not over the rows that validated.
    // A row with a missing `duration` and a mistyped image path has two
    // mistakes, and reporting only the first means two runs to find two
    // problems — which is the thing plan.md §M2 asks us not to do.
    let files: Vec<ResolvedFiles> = rows
        .drafts
        .iter()
        .map(|draft| resolve_files(&root, draft, media, probes, &mut problems))
        .collect();

    let mut resolved = Vec::with_capacity(scenes.len());
    for spec in scenes {
        // The first draft with this id is the one that validated: duplicates
        // are dropped by `validate_drafts`, first occurrence kept.
        let Some(index) = rows
            .drafts
            .iter()
            .position(|d| d.id.trim() == spec.id.as_str())
        else {
            continue;
        };
        let Some(image) = files[index].image.clone() else {
            continue;
        };
        let audio = match &spec.source {
            AudioSource::File { .. } => match files[index].audio.clone() {
                Some(path) => Some(path),
                // The file is named and unusable; its problem is already
                // recorded, and a scene with no narration is not renderable.
                None => continue,
            },
            // TTS and silence have no file to find yet — the artifact they
            // resolve to is produced at render time and probed then (D-021).
            AudioSource::Tts { .. } | AudioSource::Silent { .. } => None,
        };
        resolved.push(ResolvedScene { spec, image, audio });
    }

    if let Mode::Manifest(_) = rows.mode {
        problems.extend(unlisted_images(&root, &rows.drafts, &files));
    }

    order_by_scene(&mut problems, &rows.drafts);

    Ok(Project {
        root,
        settings,
        mode: rows.mode,
        scenes: resolved,
        problems,
    })
}

/// Group the problem list by scene, in render order.
///
/// The stages produce problems stage by stage, which puts a scene's validation
/// problem and its missing-file problem pages apart in a 500-row report. An
/// operator fixes a project row by row, so the report is ordered row by row —
/// project-level problems first, because a bad `aspect` is often why the rest
/// of the list looks the way it does.
///
/// A **stable** sort, so the order stages ran in is still the order within one
/// scene: what the row says, then whether its files are there, then whether
/// they are what they claim.
fn order_by_scene(problems: &mut [Problem], drafts: &[spoonstill_core::SceneDraft]) {
    let position = |problem: &Problem| -> (u8, usize) {
        match &problem.scene {
            None => (0, 0),
            Some(id) => (
                1,
                drafts
                    .iter()
                    .position(|d| d.id.trim() == id.as_str())
                    // A scene the drafts do not name cannot happen today; if
                    // it ever does, it sorts last rather than vanishing.
                    .unwrap_or(usize::MAX),
            ),
        }
    };
    problems.sort_by_key(position);
}

/// What one row's paths resolved to.
#[derive(Debug, Default)]
struct ResolvedFiles {
    image: Option<PathBuf>,
    audio: Option<PathBuf>,
}

/// Resolve one row's files, recording a problem for each that will not do.
///
/// Both cells are attempted even when the first fails, so an operator who
/// mistyped both paths is told about both.
fn resolve_files(
    root: &Path,
    draft: &spoonstill_core::SceneDraft,
    media: &dyn MediaCheck,
    probes: bool,
    problems: &mut Vec<Problem>,
) -> ResolvedFiles {
    // A row whose id is unusable has already been reported as such; its path
    // problems have nothing to attach to, and repeating them under a made-up
    // id would be worse than leaving them for the re-run.
    let Ok(id) = spoonstill_core::SceneId::new(&draft.id) else {
        return ResolvedFiles::default();
    };

    let cell = |value: &Option<String>| -> Option<PathBuf> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };

    ResolvedFiles {
        image: cell(&draft.image)
            .and_then(|path| resolve_file(root, &id, &path, Role::Image, media, probes, problems)),
        audio: cell(&draft.audio)
            .and_then(|path| resolve_file(root, &id, &path, Role::Audio, media, probes, problems)),
    }
}

fn resolve_file(
    root: &Path,
    id: &spoonstill_core::SceneId,
    requested: &Path,
    role: Role,
    media: &dyn MediaCheck,
    probes: bool,
    problems: &mut Vec<Problem>,
) -> Option<PathBuf> {
    let value = requested.display().to_string();

    let resolved = match resolve_within(root, requested, &StdFs) {
        Ok(path) => path,
        Err(error) => {
            problems.push(Problem::in_scene(
                id.clone(),
                ProblemKind::Path {
                    field: role.field(),
                    value,
                    error,
                },
            ));
            return None;
        }
    };

    // A directory resolves and contains fine; it is still not an image.
    // Caught here rather than in path_safety, because "is this a file" is a
    // question about media, not about safety (D-054).
    if !resolved.is_file() {
        problems.push(Problem::in_scene(
            id.clone(),
            ProblemKind::Path {
                field: role.field(),
                value,
                error: PathError::Missing,
            },
        ));
        return None;
    }

    // `probes` is false only when the tooling itself is missing, which is
    // already one problem on this project. The file is taken at its extension
    // for now — the render cannot start regardless, and the operator gets to
    // see the film they have been building instead of an empty window.
    if !probes {
        return Some(resolved);
    }

    if let Err(detail) = media.check(&resolved, role) {
        problems.push(Problem::in_scene(
            id.clone(),
            ProblemKind::NotUsableMedia {
                field: role.field(),
                value,
                detail,
            },
        ));
        return None;
    }

    Some(resolved)
}

/// Images sitting in the project root that no manifest row mentions (D-056).
///
/// A warning, not an error, and only in manifest mode: the manifest is the
/// complete list of scenes when it exists, so an image not in it will not be
/// rendered. That is correct for a source asset and wrong for a row somebody
/// forgot to add, and only the operator can tell those apart.
///
/// Compared against every row's **image cell**, not against the scenes that
/// resolved — an image whose row failed validation is still listed, and
/// warning that it is unlisted would be simply untrue.
///
/// Top level only. Recursing would turn every `originals/` folder into a wall
/// of warnings, which is how warnings stop being read.
fn unlisted_images(
    root: &Path,
    drafts: &[spoonstill_core::SceneDraft],
    files: &[ResolvedFiles],
) -> Vec<Problem> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };

    // Canonical where the path resolved, and the raw cell otherwise, so a row
    // pointing at a file that does not exist still counts as listing it.
    let mut used: Vec<PathBuf> = files.iter().filter_map(|f| f.image.clone()).collect();
    used.extend(drafts.iter().filter_map(|d| {
        let cell = d.image.as_deref()?.trim();
        (!cell.is_empty()).then(|| root.join(cell))
    }));

    let mut names: Vec<String> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            if name.starts_with('.') {
                return None;
            }
            let extension = Path::new(&name).extension()?.to_str()?.to_ascii_lowercase();
            if !rows::IMAGE_EXTENSIONS.contains(&extension.as_str()) {
                return None;
            }
            // Compare canonically: the row may spell it `./001.PNG` on a
            // case-insensitive volume and mean this very file (D-054).
            let path = entry.path();
            let canonical = std::fs::canonicalize(&path).unwrap_or(path);
            (!used.contains(&canonical)).then_some(name)
        })
        .collect();
    names.sort();

    names
        .into_iter()
        .map(|value| Problem::in_project(ProblemKind::UnlistedImage { value }))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Every file is whatever it says it is. Lets these tests exercise the
    /// stages around the probe without needing FFmpeg or real media.
    struct Accepting;

    impl MediaCheck for Accepting {
        fn check(&self, _path: &Path, _role: Role) -> Result<(), String> {
            Ok(())
        }
    }

    /// A machine with no FFmpeg on it: the probe cannot start at all.
    struct Uninstalled;

    impl MediaCheck for Uninstalled {
        fn ready(&self) -> Result<(), Remedy> {
            Err(Remedy::installable(
                "Spoonstill needs FFmpeg to turn your photos into video, and it is not \
                 installed yet.",
                "ffmpeg",
                "tried /nonexistent/ffprobe",
            ))
        }

        fn check(&self, _path: &Path, _role: Role) -> Result<(), String> {
            unreachable!("no file is probed when the probe cannot run")
        }
    }

    /// The opposite: a stand-in for `truncated.jpg` and `zero_byte.mp3`.
    struct Refusing(&'static str);

    impl MediaCheck for Refusing {
        fn check(&self, path: &Path, _role: Role) -> Result<(), String> {
            if path.to_string_lossy().contains(self.0) {
                Err("probes as 0x0 — the file is truncated".to_owned())
            } else {
                Ok(())
            }
        }
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(files: &[(&str, &str)]) -> Self {
            static COUNTER: AtomicU32 = AtomicU32::new(0);
            let dir = std::env::temp_dir().join(format!(
                "spoonstill-import-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).expect("scratch dir");
            for (name, contents) in files {
                let path = dir.join(name);
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).expect("scratch subdir");
                }
                std::fs::write(&path, contents).expect("scratch file");
            }
            Scratch(dir)
        }

        fn load(&self) -> Project {
            super::load(&self.0, &Accepting).expect("loads")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn messages(problems: impl Iterator<Item = impl ToString>) -> Vec<String> {
        problems.map(|p| p.to_string()).collect()
    }

    /// The M2 shape, in miniature: one TTS scene, one supplied-audio scene,
    /// one silent scene, no problems.
    #[test]
    fn a_mixed_project_resolves_all_three_sources() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            ("001.txt", "the first line"),
            ("002.png", ""),
            ("002.mp3", ""),
            ("003.png", ""),
        ]);
        let project = scratch.load();

        assert_eq!(project.mode, Mode::Convention);
        assert_eq!(project.scenes.len(), 3);
        assert!(project.problems.is_empty(), "{:?}", project.problems);
        assert!(!project.has_errors());

        let kinds: Vec<&str> = project
            .scenes
            .iter()
            .map(|s| s.spec.source.kind())
            .collect();
        assert_eq!(kinds, vec!["tts", "file", "silent"]);

        assert!(project.scenes[0].image.is_absolute(), "paths are canonical");
        assert_eq!(project.scenes[0].audio, None, "TTS has no file yet");
        assert!(project.scenes[1].audio.is_some(), "supplied audio resolved");
        assert_eq!(project.scenes[2].audio, None);
    }

    /// D-054 through the whole pipeline: a row pointing outside the project is
    /// refused, and the message is the generic one.
    #[test]
    fn a_row_pointing_outside_the_project_is_refused() {
        let scratch = Scratch::new(&[(
            rows::DEFAULT_MANIFEST,
            "image,duration\n../../../etc/passwd,3\n",
        )]);
        let project = scratch.load();

        assert!(project.scenes.is_empty());
        let found = messages(project.errors());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("outside the project folder"), "{found:?}");
    }

    #[test]
    fn a_missing_file_inside_the_project_is_named_precisely() {
        let scratch = Scratch::new(&[(rows::DEFAULT_MANIFEST, "image,duration\n001.png,3\n")]);
        let project = scratch.load();

        let found = messages(project.errors());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("no such file"), "{found:?}");
        assert!(found[0].contains("001"), "with the scene id: {found:?}");
    }

    /// plan.md §M2: extensions are a hint, not evidence.
    #[test]
    fn a_file_that_is_not_what_it_claims_is_refused_by_the_probe() {
        let scratch = Scratch::new(&[("001.png", "this is not a png"), ("001.txt", "hello")]);
        let project = super::load(&scratch.0, &Refusing("001.png")).expect("loads");

        assert!(project.scenes.is_empty());
        let found = messages(project.errors());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("truncated"), "{found:?}");
    }

    /// The headline of this whole stage. One folder, four different mistakes,
    /// one run (plan.md §M2).
    #[test]
    fn problems_from_every_stage_arrive_in_one_list() {
        let scratch = Scratch::new(&[
            ("project.yaml", "aspect: 4:3\n"),
            ("001.png", ""),
            (
                rows::DEFAULT_MANIFEST,
                "image,text,duration\n\
                 001.png,hello,3\n\
                 002.png,,4\n\
                 003.png,,\n",
            ),
        ]);
        let project = scratch.load();

        let found = messages(project.errors());
        assert_eq!(found.len(), 5, "every stage, every row: {found:?}");
        // Project first, then scene by scene in render order — not stage by
        // stage, which would scatter one row's problems across the report.
        assert!(found[0].contains("aspect"), "settings: {found:?}");
        assert!(
            found[1].contains("001") && found[1].contains("exactly one"),
            "{found:?}"
        );
        assert!(
            found[2].contains("002") && found[2].contains("no such file"),
            "{found:?}"
        );
        // Row 003 has two mistakes — no source *and* no such image — and both
        // are reported. Resolution runs over every row, not only the rows that
        // passed validation, so one run finds both.
        assert!(
            found[3].contains("003") && found[3].contains("no audio source"),
            "{found:?}"
        );
        assert!(
            found[4].contains("003") && found[4].contains("no such file"),
            "{found:?}"
        );
    }

    /// D-056. The manifest is the complete list of scenes, so an image that is
    /// not in it will not be rendered — and the operator is told, because only
    /// they know whether it is a source asset or a forgotten row.
    #[test]
    fn an_image_no_manifest_row_mentions_is_a_warning() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            ("002.png", ""),
            (rows::DEFAULT_MANIFEST, "image,duration\n001.png,3\n"),
        ]);
        let project = scratch.load();

        assert_eq!(project.scenes.len(), 1);
        assert!(!project.has_errors(), "a warning does not stop the render");

        let found = messages(project.warnings());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("002.png"), "{found:?}");
        assert!(found[0].contains("will not be rendered"), "{found:?}");
    }

    /// An image whose row failed validation is still *listed* — warning that
    /// it is not would be false, and would send the operator looking for a
    /// missing row that is right there.
    #[test]
    fn an_image_whose_row_failed_validation_is_not_called_unlisted() {
        let scratch = Scratch::new(&[
            ("001.png", ""),
            (
                rows::DEFAULT_MANIFEST,
                "image,text,duration\n001.png,hi,3\n",
            ),
        ]);
        let project = scratch.load();

        let warnings = messages(project.warnings());
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(project.errors().count(), 1, "only the two-source error");
    }

    /// Convention mode has no such thing as an unlisted image — every image is
    /// a scene — so the warning must not appear there.
    #[test]
    fn convention_mode_never_warns_about_unlisted_images() {
        let scratch = Scratch::new(&[("001.png", ""), ("002.png", "")]);
        let project = scratch.load();

        assert_eq!(project.scenes.len(), 2);
        assert!(project.problems.is_empty(), "{:?}", project.problems);
    }

    /// D-103. The bug this test exists for opened a folder of six good
    /// photographs as **zero scenes and six errors**, one per photograph, each
    /// saying `\`image\` "001.jpeg": ffprobe could not be executed` — while the
    /// operator's real problem was that the window, launched from Finder,
    /// never had Homebrew on its `PATH`.
    ///
    /// Two things have to be true afterwards. The scenes are still there, so
    /// the window has something to show. And the missing tool is said **once**,
    /// about the project, naming what to install.
    #[test]
    fn a_machine_with_no_ffmpeg_says_so_once_and_still_shows_the_scenes() {
        let scratch = Scratch::new(&[("001.jpeg", ""), ("002.jpeg", ""), ("003.jpeg", "")]);
        let project = super::load(&scratch.0, &Uninstalled).expect("the folder still loads");

        assert_eq!(
            project.scenes.len(),
            3,
            "the photographs are not the problem"
        );

        let found = messages(project.errors());
        assert_eq!(found.len(), 1, "once, not once per photograph: {found:?}");
        assert!(found[0].contains("FFmpeg"), "{found:?}");

        // D-105: and it arrives as something the window can press, not as a
        // command line the operator is left to run. A control surface that
        // could only read `found[0]` would be back where the Voice screen was.
        let tooling = project
            .problems
            .iter()
            .find_map(|problem| match &problem.kind {
                ProblemKind::ToolingMissing { remedy } => Some(remedy),
                _ => None,
            })
            .expect("the missing tool is a ToolingMissing problem");
        assert_eq!(tooling.install.as_deref(), Some("ffmpeg"), "{tooling:?}");
        assert!(
            !tooling.need.contains("brew "),
            "a command line is not an instruction an operator can follow: {}",
            tooling.need
        );
        assert!(
            project.problems.iter().all(|p| p.scene.is_none()),
            "it is a fact about the machine, so it belongs to no scene"
        );

        // And it still stops the render: a film cannot be made without FFmpeg,
        // and D-002 says so now rather than forty minutes in.
        assert!(project.has_errors());
    }

    #[test]
    fn a_folder_that_is_not_there_is_an_error_not_an_empty_project() {
        let error = super::load(Path::new("/no/such/project"), &Accepting).expect_err("no folder");
        assert!(matches!(error, ImportError::NoProject { .. }), "{error}");
    }

    #[test]
    fn an_empty_folder_reports_that_it_has_no_scenes() {
        let scratch = Scratch::new(&[]);
        let project = scratch.load();

        assert!(project.scenes.is_empty());
        let found = messages(project.errors());
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("no scenes"), "{found:?}");
    }

    /// An unparseable settings file is one of the only two aborts, because
    /// after it there is nothing to validate against.
    #[test]
    fn an_unparseable_settings_file_aborts_rather_than_defaulting() {
        let scratch = Scratch::new(&[("project.yaml", "apsect: 9:16\n"), ("001.png", "")]);
        let error = super::load(&scratch.0, &Accepting).expect_err("unknown key");
        assert!(matches!(error, ImportError::Settings(_)), "{error}");
    }

    /// D-052's normal case, end to end.
    #[test]
    fn spaces_and_unicode_in_filenames_resolve() {
        let scratch = Scratch::new(&[
            ("ünïcode spaced 名前.png", ""),
            ("ünïcode spaced 名前.txt", "hello"),
        ]);
        let project = scratch.load();

        assert_eq!(project.scenes.len(), 1);
        assert_eq!(project.scenes[0].spec.id.as_str(), "ünïcode spaced 名前");
        assert!(project.problems.is_empty(), "{:?}", project.problems);
    }

    /// A row may point into a subfolder; containment allows it, and the scene
    /// id still comes from the stem.
    #[test]
    fn a_row_may_point_into_a_subfolder() {
        let scratch = Scratch::new(&[
            ("img/001.png", ""),
            ("audio/001.mp3", ""),
            (
                rows::DEFAULT_MANIFEST,
                "image,audio_file\nimg/001.png,audio/001.mp3\n",
            ),
        ]);
        let project = scratch.load();

        assert!(project.problems.is_empty(), "{:?}", project.problems);
        assert_eq!(project.scenes.len(), 1);
        assert_eq!(project.scenes[0].spec.id.as_str(), "001");
        assert!(project.scenes[0].audio.is_some());
    }

    /// Both of a scene's paths are checked, so one run reports both mistakes.
    #[test]
    fn both_of_a_scenes_files_are_reported_not_just_the_first() {
        let scratch = Scratch::new(&[(
            rows::DEFAULT_MANIFEST,
            "image,audio_file\ngone.png,gone.mp3\n",
        )]);
        let project = scratch.load();

        let found = messages(project.errors());
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].contains("image"), "{found:?}");
        assert!(found[1].contains("audio_file"), "{found:?}");
    }
}
