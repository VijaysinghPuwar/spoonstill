//! The spoonstill desktop shell — the review grid of D-051, and nothing more.
//!
//! **This window owns no business logic** (D-010). Every command below is a
//! translation: arguments in, a call into `spoonstill-app`, a serializable view
//! out. There is no filter string here, no FFmpeg argument, no duration
//! arithmetic and no cache key — if a question can be answered here rather than
//! in `spoonstill-app`, it is being answered in the wrong place, and the CLI
//! would then be able to do something the shell cannot (or the reverse, which
//! is worse).
//!
//! ## What the frontend is allowed to do
//!
//! It passes **folders and files the operator picked** into a Rust command and
//! receives rows of already-resolved facts. It never constructs a path from
//! parts, never names a binary, and never sees an FFmpeg invocation. The
//! capability file grants three things — a file and folder dialog, opening a
//! finished file in the system player, and the core defaults — which is the
//! audit plan.md §M4 asks for, kept short enough to actually read.
//!
//! Dropped files are the one path that does not come from a dialog. They
//! arrive as an OS-level drag-drop event carrying real paths, are copied by
//! `spoonstill_app::ingest` — which refuses to overwrite anything and never
//! moves an original (D-080) — and are then read back through the same
//! `validate_project` call as any other folder.
//!
//! ## Ordering, and why the render runs where it does
//!
//! `render_project` blocks for as long as the film takes, so it runs on a
//! blocking thread and reports through an ordered [`Channel`]. The cancel flag
//! lives in managed Rust state, never in frontend state (plan.md §M4) — a
//! webview that reloads must not be able to lose track of a running render.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![warn(missing_docs)]

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use spoonstill_app::film::{FilmEvent, RenderProjectOptions};
use spoonstill_app::surface::Cancel;
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tauri_plugin_opener::OpenerExt;

/// One scene, as the review grid shows it.
#[derive(Debug, Clone, Serialize)]
struct SceneView {
    index: usize,
    /// The scene's own name — in convention mode, the shared file stem.
    id: String,
    /// `tts`, `file` or `silent` — D-051's source badge.
    source: String,
    /// The still, relative to the project root.
    image: String,
    /// The still's real path, for the thumbnail. The window turns this into an
    /// `asset:` URL; the directory is granted to the asset protocol when the
    /// project opens, and never before.
    image_path: String,
    /// The words to be spoken, for a `tts` scene. Empty otherwise.
    narration: String,
    /// The supplied recording's filename, for a `file` scene. Empty otherwise.
    audio: String,
    /// The declared length of a `silent` scene. `None` for everything else,
    /// because a real duration is *measured* and is not known until the render
    /// resolves the narration (D-021) — a number here would be a guess.
    seconds: Option<f64>,
    /// The voice this scene would be spoken in, for a `tts` scene.
    voice: String,
    /// What this scene would put on screen if subtitles are on (D-106).
    ///
    /// Empty for a scene with no words — a supplied recording with no `.txt`
    /// beside it and no `caption` column. The Subtitles screen counts these,
    /// because "subtitles are on and four of my nine scenes have none" is a
    /// fact an operator should meet before the render rather than after it.
    caption: String,
    /// Whether this scene can render. Kept as a field rather than assumed,
    /// because a provider that is not installed is still a row the grid must
    /// be able to mark.
    renderable: bool,
}

/// One problem, exactly as `still validate` would print it — plus, when the
/// window can do something about it, the thing to press (D-105).
#[derive(Debug, Clone, Serialize)]
struct ProblemView {
    severity: String,
    scene: Option<String>,
    /// What an operator reads. For an installable problem this is the plain
    /// sentence alone: the paths and exit codes are in `detail`, which is
    /// behind a disclosure and in the bundle.
    message: String,
    /// The tool id to hand to `install_tool`, when this is a problem the
    /// window can fix by pressing something. `None` for every problem that is
    /// about the operator's own files, which is nearly all of them.
    install: Option<String>,
    /// The technical half, when there is one. Empty otherwise.
    detail: String,
}

/// A project, as far as it can be known without rendering anything.
#[derive(Debug, Clone, Serialize)]
struct ProjectView {
    root: String,
    name: String,
    mode: String,
    /// True when scenes come from the folder itself rather than a CSV. Only
    /// then can the window write a scene's narration: in manifest mode the
    /// operator's CSV is the source of truth and we do not edit it for them.
    convention: bool,
    output: String,
    /// The folder the film will actually be written into, absolute. The window
    /// shows this rather than making the operator work it out from `output`
    /// and the project root — and it never joins the two itself (D-052).
    output_dir: String,
    /// The film's own file name.
    output_name: String,
    /// The two of them, joined by Rust.
    output_path: String,
    geometry: String,
    /// The project's own aspect, as `16:9` — the shape chooser opens on it,
    /// and the scenes grid crops its thumbnails to it.
    ///
    /// Sent rather than derived from `geometry`: the grid used to decide
    /// "portrait" by testing whether that string started with `1080x1920`,
    /// which is one size of one aspect, so a 4K Short showed landscape
    /// thumbnails (D-143).
    aspect: String,
    /// The project's own short edge, which the size chooser opens on.
    short_edge: u32,
    /// Whether `project.yaml` asks for burned-in subtitles (D-106).
    subtitles: bool,
    /// Which theme it names, resolved to a real one.
    subtitle_theme: String,
    /// Which edge they sit against.
    subtitle_placement: String,
    scenes: Vec<SceneView>,
    problems: Vec<ProblemView>,
    has_errors: bool,
    /// Whether this folder is genuinely empty — no scenes, and nothing wrong
    /// with it beyond having none yet.
    ///
    /// The window's "Choose photos…" screen is for exactly that folder, and
    /// **only** that folder. Deciding it from `scenes.is_empty()` alone was
    /// D-103's second half: a project whose every image failed its probe also
    /// has no scenes, so the window offered to add photos to a folder that was
    /// already full of them and threw away the problem list that said why. The
    /// answer is computed here rather than in the webview because "is this
    /// project empty" is a fact about the project (D-010).
    empty: bool,
    /// The default voice a spoken scene gets, from `project.yaml`.
    voice: String,
    /// The default provider.
    provider: String,
}

/// One project the operator has opened before, as the home screen lists it.
#[derive(Debug, Clone, Serialize)]
struct RecentView {
    path: String,
    /// The same path with the home directory written `~`, which is how a
    /// person reads it. Shortened **here**: it is a path operation, and the
    /// window is not allowed to take one apart (D-052).
    pretty: String,
    name: String,
    /// Seconds since the epoch. Formatted in the window, because "2 minutes
    /// ago" is a presentation decision and this is a translation layer.
    at: u64,
    /// Whether the folder is still there. A project that has been moved or
    /// deleted is **shown and marked**, not silently dropped — vanishing from
    /// the list is how an operator concludes the program lost their work.
    exists: bool,
}

/// The stored form. Kept in the OS's config directory rather than in a project,
/// because "which projects has this person opened" is not a fact about any one
/// of them (D-013 puts *machine state for a project* under `.spoonstill/`; this
/// is machine state for the operator).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct RecentProject {
    path: String,
    name: String,
    at: u64,
}

/// Where the window remembers the operator's projects.
const RECENT_FILE: &str = "recent-projects.json";

/// Where the window remembers the answers that are about this machine rather
/// than about any one project (D-092). Same reasoning as `RECENT_FILE`: a
/// fallback voice is not a fact about a project, so it does not live in one.
const SETTINGS_FILE: &str = "app-settings.json";

/// The machine's own answers. Every field is optional, because every field has
/// a working default and a first run has none of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AppSettings {
    /// The voice a project that names none falls back to.
    ///
    /// **A fallback, never a write.** `project.yaml` is an input (D-013), so
    /// choosing here changes what the *next render* asks for and nothing on
    /// disk. A project that names its own voice still wins, and the Voice
    /// screen's per-run override wins over both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_voice: Option<String>,
}

fn settings_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(SETTINGS_FILE))
}

/// Read them, or the defaults. As with the recent list, every failure here is
/// "there are none yet", which is normal and never an error in front of anyone.
#[tauri::command]
fn app_settings(app: tauri::AppHandle) -> AppSettings {
    settings_file(&app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// Set the fallback voice, or clear it by passing nothing.
#[tauri::command]
fn set_default_voice(app: tauri::AppHandle, voice: Option<String>) -> Result<AppSettings, String> {
    let mut settings = app_settings(app.clone());
    settings.default_voice = voice.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());

    let path = settings_file(&app).ok_or("this machine has no config directory")?;
    let text = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(settings)
}

/// Install whichever tool the window was asked about, and say what was run.
///
/// The window used to print `pip install edge-tts` and leave the operator to
/// find a terminal, which is the program declining to do the thing it just
/// asked for (D-092). D-105 finished that thought: **every** tool a
/// [`spoonstill_core::Remedy`] marks installable is installable from wherever
/// the problem was shown, and that now includes FFmpeg, which every render
/// needs and which until now offered a string and no button.
///
/// One entry point rather than one per tool, because the caller is a webview
/// echoing back a `install` field it was handed — it should not have to know
/// which subsystem owns which program, and adding a third tool should not add
/// a third command.
#[tauri::command]
async fn install_tool(tool: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // D-010: the shell asks spoonstill-app and stops there. Which
        // subsystem owns which binary is `tooling`'s business, not a
        // webview's.
        spoonstill_app::tooling::install(&tool).map_err(|remedy| remedy.to_string())
    })
    .await
    .map_err(|e| format!("the install thread failed: {e}"))?
}

/// Whether FFmpeg is on this machine, asked the same way the voice service is.
///
/// This exists so the Render screen can refuse *before* the operator presses
/// Render, rather than after — D-089's rule that a disabled control explains
/// itself where it is, applied to the one dependency that every single render
/// needs. Without it, a machine with no FFmpeg answers "Render" with a wall of
/// per-file probe failures, which is D-103's original report.
#[tauri::command]
async fn ffmpeg_status() -> Result<ProviderStatus, String> {
    tauri::async_runtime::spawn_blocking(|| status_of(spoonstill_app::tooling::ffmpeg()))
        .await
        .map_err(|e| format!("the check failed: {e}"))
}

/// How many to keep. Long enough to cover a working period, short enough that
/// the home screen stays a list rather than an archive.
const RECENT_LIMIT: usize = 30;

fn recent_file(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_config_dir().ok()?;
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(RECENT_FILE))
}

fn read_recent(app: &tauri::AppHandle) -> Vec<RecentProject> {
    // Every failure here is "there is no list yet", which is a normal state on
    // a first run and never worth an error in front of the operator.
    recent_file(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Vec<RecentProject>>(&text).ok())
        .unwrap_or_default()
}

fn write_recent(app: &tauri::AppHandle, list: &[RecentProject]) {
    if let (Some(path), Ok(text)) = (recent_file(app), serde_json::to_string_pretty(list)) {
        let _ = std::fs::write(path, text);
    }
}

fn seconds_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Record that a project was opened. Called from `validate_project` rather than
/// exposed as its own command: there is then no way to open a project without
/// it appearing on the home screen, which is the whole promise of that screen.
fn remember(app: &tauri::AppHandle, root: &std::path::Path) {
    let path = root.display().to_string();
    let mut list = read_recent(app);
    list.retain(|entry| entry.path != path);
    list.insert(
        0,
        RecentProject {
            name: root
                .file_name()
                .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned()),
            path,
            at: seconds_now(),
        },
    );
    list.truncate(RECENT_LIMIT);
    write_recent(app, &list);
}

fn views_of(app: &tauri::AppHandle, list: Vec<RecentProject>) -> Vec<RecentView> {
    let home = app
        .path()
        .home_dir()
        .ok()
        .map(|home| home.display().to_string())
        .filter(|home| !home.is_empty());

    list.into_iter()
        .map(|entry| RecentView {
            exists: std::path::Path::new(&entry.path).is_dir(),
            pretty: match &home {
                Some(home) if entry.path.starts_with(home.as_str()) => {
                    format!("~{}", &entry.path[home.len()..])
                }
                _ => entry.path.clone(),
            },
            path: entry.path,
            name: entry.name,
            at: entry.at,
        })
        .collect()
}

/// Every project this operator has opened, newest first — the home screen.
#[tauri::command]
fn recent_projects(app: tauri::AppHandle) -> Vec<RecentView> {
    let list = read_recent(&app);
    views_of(&app, list)
}

/// Take one off the home screen. The folder is untouched; this is the list
/// forgetting a project, never the program deleting one.
#[tauri::command]
fn forget_project(app: tauri::AppHandle, path: String) -> Vec<RecentView> {
    let mut list = read_recent(&app);
    list.retain(|entry| entry.path != path);
    write_recent(&app, &list);
    views_of(&app, list)
}

/// One tool report, as the window receives it.
///
/// The same shape for FFmpeg and for a voice service, because the window draws
/// them identically: a sentence, a button when there is one, and the technical
/// half folded away (D-105).
fn status_of(tool: spoonstill_app::tooling::ToolReport) -> ProviderStatus {
    let remedy = tool
        .remedy
        .unwrap_or_else(|| spoonstill_core::Remedy::manual("", ""));
    ProviderStatus {
        id: tool.id,
        ready: tool.ready,
        need: remedy.need,
        install: remedy.install,
        detail: remedy.detail,
        default_voice: String::new(),
    }
}

/// Whether a voice service can be reached, and what to do if not.
///
/// Three fields rather than one sentence, because the window has a button and
/// a terminal does not (D-105). The screenshot that prompted this showed
/// `need`, `install` and `detail` mashed into one line of grey text above an
/// empty list — the operator was told to open a terminal by a program that
/// could have done it for them.
#[derive(Debug, Clone, Serialize)]
struct ProviderStatus {
    id: String,
    ready: bool,
    /// The plain sentence. Empty when ready.
    need: String,
    /// The tool id for `install_tool`, when the window can fetch it. `None`
    /// means there is genuinely nothing to press and `need` carries the whole
    /// answer.
    install: Option<String>,
    /// The technical half — the path tried, the exit status. Behind a
    /// disclosure, and in the diagnostics bundle.
    detail: String,
    /// What this provider speaks when a project names no voice. `project.yaml`
    /// says the word `default`; the window has to be able to say whose voice
    /// that actually is.
    default_voice: String,
}

/// What a drop of files did, in the shape the window reports it.
#[derive(Debug, Clone, Serialize)]
struct IngestView {
    /// `001.jpg  <-  IMG_2931.jpg  +  Voice 014.m4a`, one row per scene.
    rows: Vec<IngestRow>,
    /// Files that were not media.
    skipped: Vec<String>,
    /// The one-line summary.
    summary: String,
    /// Recordings and scripts with no photo to pair with.
    orphans: usize,
}

/// One photo, and what came in with it.
#[derive(Debug, Clone, Serialize)]
struct IngestRow {
    name: String,
    from: String,
    with: String,
}

/// One voice the operator can choose.
#[derive(Debug, Clone, Serialize)]
struct VoiceView {
    id: String,
    locale: String,
    gender: String,
    note: String,
}

/// The finished film.
#[derive(Debug, Clone, Serialize)]
struct FilmView {
    path: String,
    duration: f64,
    scenes: usize,
    frames: u64,
    reused_audio: usize,
    reused_segments: usize,
}

/// Progress, in the shape the grid consumes.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum ProgressView {
    Planned {
        scenes: usize,
        jobs: usize,
        audio_jobs: usize,
        /// Estimated memory for one worker at this geometry, already worded
        /// for a person (D-144).
        per_worker: String,
        /// Whether memory rather than the core count chose `jobs`.
        limited_by_memory: bool,
    },
    /// This run plans to use more memory than the machine should give it.
    MemoryPressure {
        jobs: usize,
        needed: String,
        budget: String,
        fits: usize,
    },
    Audio {
        index: usize,
        id: String,
        source: String,
        duration: f64,
        reused: bool,
    },
    Segment {
        index: usize,
        id: String,
        frames: u32,
        duration: f64,
        reused: bool,
    },
    Failed {
        index: usize,
        id: String,
        detail: String,
    },
    Joining {
        segments: usize,
    },
}

/// The one piece of state the shell owns: whether a render is running, and how
/// to stop it. In Rust, not in the webview (plan.md §M4).
#[derive(Default)]
struct Active(Mutex<Option<Cancel>>);

/// Holds the window's one render slot for as long as the render runs (D-115).
///
/// The slot is what makes `cancel_render` able to reach a running render and
/// what stops a second one starting. Releasing it was a statement placed after
/// `handle.await?` — so an `Err` from the join, which is what a **panic** in
/// the render thread produces, returned past it and left the slot claimed
/// forever. The window then refused every later render with "a render is
/// already running in this window", and only restarting the app cleared it.
///
/// A guard cannot be skipped by an early return, and runs while a panic
/// unwinds.
struct ActiveRender<'a>(&'a Active);

impl<'a> ActiveRender<'a> {
    fn claim(active: &'a Active, cancel: Cancel) -> Result<Self, String> {
        let mut slot = active
            .0
            .lock()
            .map_err(|_| "the render state is poisoned")?;
        if slot.is_some() {
            return Err("a render is already running in this window".to_owned());
        }
        *slot = Some(cancel);
        drop(slot);
        Ok(ActiveRender(active))
    }
}

impl Drop for ActiveRender<'_> {
    fn drop(&mut self) {
        // A poisoned mutex is recovered from rather than propagated: leaving
        // the slot claimed is the failure this type exists to prevent, and
        // there is no invariant in an `Option<Cancel>` a panic could break.
        let mut slot = self
            .0
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *slot = None;
    }
}

/// The project this window is open on, and a refusal if the page named another
/// one (D-127).
///
/// `open_film` and `reveal_project` take no path at all, for a reason written
/// down beside `Session`: the frontend must never hand a path to a command that
/// acts on it. The commands that *write* — remove a scene, move one, edit a
/// narration, copy media in — were taking `root: String` straight from the
/// webview, so the rule was applied to the two that open a file and not to the
/// four that rearrange the operator's folder.
///
/// The supplied value is still compared rather than ignored, so a page that has
/// drifted onto a stale project is told so instead of silently rearranging the
/// one that happens to be open.
fn project_root(session: &Session, claimed: &str) -> Result<PathBuf, String> {
    let open = session
        .root
        .lock()
        .map_err(|_| "the window's project state is poisoned")?
        .clone()
        .ok_or("no project is open in this window")?;

    let same = std::fs::canonicalize(&open)
        .ok()
        .zip(std::fs::canonicalize(claimed).ok())
        .is_some_and(|(a, b)| a == b)
        || open.as_path() == std::path::Path::new(claimed);

    if same {
        Ok(open)
    } else {
        Err("that is not the project this window has open".to_owned())
    }
}

/// What the window is looking at, remembered on the Rust side.
///
/// Two things live here for one reason: **the frontend must never hand a path
/// to a command that opens something.** `open_film` and `reveal_project` take
/// no arguments at all, so there is no path for a compromised page to
/// substitute, and the shell needs no filesystem scope in its capability file.
#[derive(Default)]
struct Session {
    root: Mutex<Option<PathBuf>>,
    film: Mutex<Option<PathBuf>>,
}

/// Read a folder and report everything true about it — the same call
/// `still validate` makes.
#[tauri::command]
async fn validate_project(
    path: String,
    app: tauri::AppHandle,
    session: State<'_, Session>,
) -> Result<ProjectView, String> {
    let view = tauri::async_runtime::spawn_blocking(move || -> Result<ProjectView, String> {
        let root = PathBuf::from(&path);
        let project = spoonstill_app::import::load(&root, &spoonstill_app::ProbeCheck::from_env())
            .map_err(|e| e.to_string())?;

        let scenes: Vec<SceneView> = project
            .scenes
            .iter()
            .enumerate()
            .map(|(index, scene)| {
                let source = scene.spec.source.kind().to_owned();
                let (narration, voice, seconds) = match &scene.spec.source {
                    spoonstill_core::AudioSource::Silent { seconds } => {
                        (String::new(), String::new(), Some(*seconds))
                    }
                    spoonstill_core::AudioSource::File { .. } => {
                        (String::new(), String::new(), None)
                    }
                    spoonstill_core::AudioSource::Tts { text, voice, .. } => {
                        (text.clone(), voice.0.clone(), None)
                    }
                };
                SceneView {
                    index,
                    id: scene.spec.id.as_str().to_owned(),
                    // Every source renders now that slice 4 has landed; the
                    // field stays because a provider that is not installed is
                    // still a row the window must be able to mark.
                    renderable: true,
                    source,
                    image: relative(&scene.image, &project.root),
                    image_path: scene.image.display().to_string(),
                    audio: scene
                        .audio
                        .as_ref()
                        .and_then(|p| p.file_name())
                        .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
                    narration,
                    voice,
                    seconds,
                    caption: scene.spec.caption.clone().unwrap_or_default(),
                }
            })
            .collect();

        let problems = project
            .problems
            .iter()
            .map(|problem| {
                // A missing tool is the one problem class the window can end
                // rather than merely report, so it is the one that arrives in
                // three pieces instead of as one formatted line (D-105).
                let remedy = match &problem.kind {
                    spoonstill_core::ProblemKind::ToolingMissing { remedy } => Some(remedy),
                    _ => None,
                };
                ProblemView {
                    severity: problem.severity().as_str().to_owned(),
                    scene: problem.scene.as_ref().map(|id| id.as_str().to_owned()),
                    message: remedy
                        .map_or_else(|| problem.kind.to_string(), |remedy| remedy.need.clone()),
                    install: remedy.and_then(|remedy| remedy.install.clone()),
                    detail: remedy
                        .map(|remedy| remedy.detail.clone())
                        .unwrap_or_default(),
                }
            })
            .collect();

        let spec = project.settings.output_spec;
        let convention = matches!(project.mode, spoonstill_app::Mode::Convention);
        // The same call the renderer makes, so the path on screen is the path
        // the film lands at rather than a second guess at it. A project whose
        // `output` setting escapes its root still opens — the grid is how you
        // find that out — so the failure becomes an empty destination and the
        // problem list says why.
        let destination = spoonstill_app::film::destination(
            &project,
            &spoonstill_app::RenderProjectOptions::for_project(&project.root),
        )
        .unwrap_or_default();
        Ok(ProjectView {
            convention,
            voice: project.settings.voice.0.clone(),
            provider: project.settings.provider.0.clone(),
            name: project.root.file_name().map_or_else(
                || "project".to_owned(),
                |n| n.to_string_lossy().into_owned(),
            ),
            root: project.root.display().to_string(),
            mode: match &project.mode {
                spoonstill_app::Mode::Manifest(path) => format!(
                    "manifest {}",
                    path.file_name()
                        .unwrap_or(path.as_os_str())
                        .to_string_lossy()
                ),
                spoonstill_app::Mode::Convention => "stem-keyed pairing".to_owned(),
            },
            output: project.settings.output.display().to_string(),
            output_dir: destination
                .parent()
                .map_or_else(String::new, |p| p.display().to_string()),
            output_name: destination
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            output_path: destination.display().to_string(),
            geometry: format!("{}x{} @ {} fps", spec.width(), spec.height(), spec.fps()),
            aspect: spec.aspect().as_str().to_owned(),
            short_edge: spec.short_edge(),
            subtitles: project.settings.subtitles,
            subtitle_theme: project.settings.subtitle_theme.as_str().to_owned(),
            subtitle_placement: project.settings.subtitle_placement.as_str().to_owned(),
            has_errors: project.has_errors(),
            empty: project.scenes.is_empty()
                && project
                    .problems
                    .iter()
                    .all(|p| matches!(p.kind, spoonstill_core::ProblemKind::NoScenes)),
            scenes,
            problems,
        })
    })
    .await
    .map_err(|e| format!("the validation thread failed: {e}"))??;

    // Thumbnails are real files, so the webview has to be allowed to read
    // them — **and only them** (D-127). That sentence was already here; the
    // code under it granted the whole folder with `allow_directory`, which is
    // every recording, every script, `removed/` and `.spoonstill/` besides.
    // Granting the stills one at a time is what the comment always claimed.
    //
    // Additive on purpose, and there is no undoing it: Tauri's scope has an
    // allow list and a *deny* list, deny wins, and nothing removes from allow —
    // so `forbid_directory` on a project the operator navigated away from would
    // block it for the rest of the session, and reopening it would show a grid
    // of broken thumbnails. What is bounded instead is the *size* of the grant:
    // the stills actually displayed, rather than everything beside them.
    let scope = app.asset_protocol_scope();
    for scene in &view.scenes {
        if !scene.image_path.is_empty() {
            let _ = scope.allow_file(&scene.image_path);
        }
    }

    let root = PathBuf::from(&view.root);
    remember(&app, &root);
    if let Ok(mut slot) = session.root.lock() {
        *slot = Some(root);
    }
    Ok(view)
}

/// Write a scene's spoken line, or remove it.
///
/// This is the one command that writes into the operator's folder, and it
/// writes the one file that is unambiguously theirs: `NNN.txt`, the words they
/// typed. It is not the renderer writing state — that is what `.spoonstill/`
/// is for (D-013) — it is a text editor for a text file, in the window where
/// they are already looking at the scene.
///
/// **Manifest mode is refused.** There the CSV is the source of truth, and a
/// window that quietly wrote a `.txt` beside it would create exactly the
/// two-sources-disagree conflict D-056 rejects.
#[tauri::command]
async fn set_narration(
    session: State<'_, Session>,
    root: String,
    scene: String,
    text: String,
) -> Result<(), String> {
    let root = project_root(&session, &root)?;
    // The id came from a row we produced, but it reaches us as a string from a
    // webview, so it is checked like any other untrusted input (D-052, D-054).
    if scene.is_empty()
        || scene.contains(['/', '\\'])
        || scene.contains("..")
        || scene.starts_with('.')
    {
        return Err(format!("{scene:?} is not a scene name"));
    }

    tauri::async_runtime::spawn_blocking(move || {
        let root = PathBuf::from(&root);
        let project = spoonstill_app::import::load(&root, &spoonstill_app::ProbeCheck::from_env())
            .map_err(|e| e.to_string())?;
        if !matches!(project.mode, spoonstill_app::Mode::Convention) {
            return Err(
                "this project is driven by a manifest — edit the CSV rather than the folder"
                    .to_owned(),
            );
        }

        let path = root.join(format!("{scene}.txt"));
        let trimmed = text.trim();
        if trimmed.is_empty() {
            // Removing the words makes it a silent scene again, which is a
            // real state and not an error (D-050). An absent file and a blank
            // one are different things, so we remove rather than write "".
            match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("{}: {e}", path.display())),
            }
        } else {
            std::fs::write(&path, trimmed).map_err(|e| format!("{}: {e}", path.display()))
        }
    })
    .await
    .map_err(|e| format!("the write failed: {e}"))?
}

/// Whether a voice service can be reached right now.
///
/// D-002: the operator finds this out on the Voice tab in a second, not at
/// scene 340 of 500.
#[tauri::command]
async fn provider_status(provider: String) -> Result<ProviderStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut status = status_of(spoonstill_app::tooling::provider(&provider));
        // The one field a voice service has and FFmpeg does not: whose voice
        // the word `default` actually means (D-086). Asked even when the tool
        // is missing, because the Voice screen still has to name the project's
        // choice while the operator installs it.
        if let Ok(engine) = spoonstill_app::tts::provider(&provider) {
            status.default_voice = engine.default_voice().to_owned();
        }
        status
    })
    .await
    .map_err(|e| format!("the check failed: {e}"))
}

/// Make a new project folder.
///
/// The frontend passes a folder the operator chose in the system dialog and
/// gets back the one true path — it never joins a name onto a parent itself.
#[tauri::command]
async fn create_project(path: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        spoonstill_app::create_project(std::path::Path::new(&path))
            .map(|root| root.display().to_string())
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("the folder could not be made: {e}"))?
}

/// Copy photos, recordings and scripts into a project (D-080).
///
/// The copy can be gigabytes, so it runs on a blocking thread like the render
/// does. The report comes back as rows because the operator is entitled to see
/// exactly which of their files became which scene.
#[tauri::command]
async fn add_media(
    session: State<'_, Session>,
    root: String,
    files: Vec<String>,
) -> Result<IngestView, String> {
    let root = project_root(&session, &root)?;
    tauri::async_runtime::spawn_blocking(move || {
        let sources: Vec<PathBuf> = files.into_iter().map(PathBuf::from).collect();
        let report = spoonstill_app::add_media(std::path::Path::new(&root), &sources)
            .map_err(|e| e.to_string())?;

        let rows = report
            .images
            .iter()
            .map(|image| IngestRow {
                name: image.name.clone(),
                from: file_name(&image.source),
                with: match (report.narration_for(image), report.script_for(image)) {
                    (Some(audio), _) => file_name(&audio.source),
                    (None, Some(script)) => format!("{} (spoken)", file_name(&script.source)),
                    (None, None) => "silent".to_owned(),
                },
            })
            .collect();

        Ok(IngestView {
            rows,
            skipped: report
                .skipped
                .iter()
                .map(|s| format!("{} — {}", file_name(&s.source), s.reason))
                .collect(),
            summary: report.summary(),
            orphans: report.audio_without_image + report.script_without_image,
        })
    })
    .await
    .map_err(|e| format!("the copy failed: {e}"))?
}

/// Take a scene out of the project (D-099).
///
/// The files are moved, never deleted — `removed/` inside the project holds
/// them, and the window says so, because "delete" in a tool that owns the
/// operator's only copy of a photograph has to be a promise it can keep.
#[tauri::command]
async fn remove_scene(
    session: State<'_, Session>,
    root: String,
    scene: String,
) -> Result<String, String> {
    let root = project_root(&session, &root)?;
    tauri::async_runtime::spawn_blocking(move || {
        let removed = spoonstill_app::arrange::remove(std::path::Path::new(&root), &scene)
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "Scene {} removed — its {} file{} moved to {}/, not deleted.",
            removed.id,
            removed.files,
            if removed.files == 1 { "" } else { "s" },
            spoonstill_app::arrange::REMOVED_DIR
        ))
    })
    .await
    .map_err(|e| format!("the removal failed: {e}"))?
}

/// Move a scene to another position in the film (D-099).
///
/// `to` counts from 1 and is clamped, so the last row's "move down" is a no-op
/// rather than an error.
#[tauri::command]
async fn move_scene(
    session: State<'_, Session>,
    root: String,
    scene: String,
    to: usize,
) -> Result<String, String> {
    let root = project_root(&session, &root)?;
    tauri::async_runtime::spawn_blocking(move || {
        let after = spoonstill_app::arrange::move_to(std::path::Path::new(&root), &scene, to)
            .map_err(|e| e.to_string())?;
        let landed = to.max(1).min(after.len().max(1));
        Ok(format!("Scene moved to {landed} of {}.", after.len()))
    })
    .await
    .map_err(|e| format!("the move failed: {e}"))?
}

/// Every voice a provider offers, or the sentence that says why there are none.
#[tauri::command]
async fn voices(provider: String) -> Result<Vec<VoiceView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = spoonstill_app::tts::provider(&provider).map_err(|e| e.to_string())?;
        if let spoonstill_app::tts::Availability::Missing(remedy) = engine.availability() {
            return Err(remedy.to_string());
        }
        Ok(engine
            .voices()
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|v| VoiceView {
                id: v.id,
                locale: v.locale,
                gender: v.gender,
                note: v.note,
            })
            .collect())
    })
    .await
    .map_err(|e| format!("the voice list failed: {e}"))?
}

/// Where the film would go if the operator pressed Render right now.
///
/// The window holds a folder and a file name in two boxes, and this is what
/// turns them back into one path — **in Rust**, because joining a path from
/// parts in the webview is exactly the thing the rest of this file refuses to
/// do (D-052, D-054). It is also the validation: a name with a separator in
/// it, or a folder that is not there, is refused here with a sentence rather
/// than discovered forty minutes into a render.
#[tauri::command]
fn resolve_output(dir: String, name: String) -> Result<String, String> {
    // A file *name* is typed by a person into a box, so surrounding whitespace
    // is a slip and trimming it is a courtesy.
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("the film needs a file name".to_owned());
    }
    if trimmed.contains(['/', '\\']) {
        return Err(
            "a file name cannot contain a folder separator — choose the folder above".to_owned(),
        );
    }
    if trimmed.starts_with('.') || trimmed.contains("..") {
        return Err(format!("{trimmed:?} is not a file name"));
    }

    // A *folder* is not typed text, it is a path — from the picker, or from
    // the project's own root. It is never trimmed. `~/Downloads/RANDOM vidoe `
    // is a directory that genuinely exists on macOS, and trimming it named a
    // folder that does not, which greyed out Render with the only explanation
    // sitting on a screen the operator was not looking at. D-089.
    if dir.is_empty() {
        return Err("choose a folder to save the film into".to_owned());
    }
    let folder = PathBuf::from(&dir);
    if !folder.is_dir() {
        // Quoted, so a name that ends in a space is something the operator can
        // see rather than a sentence that reads as if it should have worked.
        return Err(format!("{dir:?} is not a folder"));
    }

    // A film is an MP4 (D-078 asserts the finished file against that profile),
    // so a name with no extension gets the one it was always going to have
    // rather than producing a file the operator's player will not open.
    let mut named = PathBuf::from(trimmed);
    if named.extension().is_none() {
        named.set_extension("mp4");
    }
    Ok(folder.join(named).display().to_string())
}

/// Speak a line in a voice, so the operator can hear it before committing 500
/// scenes to it (D-082's override, made audible).
///
/// Returns the artifact's path, which the window plays through the asset
/// protocol. This is the **one** place a path is granted to the webview
/// individually: it is a file this command just produced inside the project's
/// own state directory, not a path the frontend named.
#[tauri::command]
async fn preview_voice(
    session: State<'_, Session>,
    root: String,
    provider: String,
    voice: String,
    text: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    // An audition writes into the named project's audio cache, so it is bound
    // to the open project like the commands that rearrange it (D-127).
    let root = project_root(&session, &root)?;
    let spoken = tauri::async_runtime::spawn_blocking(move || {
        let line = if text.trim().is_empty() {
            spoonstill_app::audio::PREVIEW_LINE
        } else {
            text.trim()
        };
        spoonstill_app::audio::preview(std::path::Path::new(&root), &provider, &voice, line)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("the audition failed: {e}"))??;

    let _ = app.asset_protocol_scope().allow_file(&spoken.path);
    Ok(spoken.path.display().to_string())
}

/// A file's own name, for a report the operator can read at a glance.
fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// What the window asks for when it presses Render.
///
/// One struct rather than five arguments: the shape of a render request is a
/// thing, and it will grow — the aspect override and the draft toggle in the
/// design brief both land here.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderRequest {
    /// The project folder.
    path: String,
    /// Segments at once. `None` takes the measured default (D-076).
    jobs: Option<usize>,
    /// Narrations at once.
    audio_jobs: Option<usize>,
    /// Take the lock a crashed run left behind.
    #[serde(default)]
    force: bool,
    /// Speak every spoken scene in this voice, for this run only (D-082).
    voice: Option<String>,
    /// The folder to write the film into. `None` leaves the project's own
    /// `output` setting alone.
    out_dir: Option<String>,
    /// The film's file name, alongside `out_dir`. Both or neither.
    out_name: Option<String>,
    /// Burn subtitles for this run, whatever `project.yaml` says (D-106).
    /// `None` leaves the project's own answer alone.
    subtitles: Option<bool>,
    /// Which look, for this run.
    subtitle_theme: Option<String>,
    /// Which edge they sit against, for this run.
    subtitle_position: Option<String>,
    /// Render this run in a different shape — `16:9`, `9:16`, `1:1` (D-143).
    /// `None` leaves the project's own answer alone.
    aspect: Option<String>,
    /// The same, for the size: a name (`4k`) or a short edge in pixels.
    resolution: Option<String>,
}

/// The subtitle themes, for the window's chooser (D-106).
#[tauri::command]
fn subtitle_themes() -> Vec<ThemeView> {
    spoonstill_app::subtitles::themes()
        .into_iter()
        .map(|choice| ThemeView {
            id: choice.id.to_owned(),
            description: choice.description.to_owned(),
            default: choice.default,
        })
        .collect()
}

/// Every shape and size the Output screen can offer (D-143).
///
/// The dimensions are computed per aspect in Rust rather than multiplied in
/// the webview: 4K portrait is 2160x3840 and 4K landscape is 3840x2160, and a
/// page that worked that out itself would be a second implementation of
/// `OutputSpec` with nothing keeping the two honest (D-010).
#[tauri::command]
fn output_formats() -> FormatsView {
    let aspects: Vec<spoonstill_app::AspectChoice> = spoonstill_app::formats::aspects();
    let parsed: Vec<spoonstill_core::Aspect> = aspects
        .iter()
        .filter_map(|a| spoonstill_core::Aspect::parse(a.id))
        .collect();

    let sizes = spoonstill_app::formats::sizes(spoonstill_core::Aspect::Landscape16x9)
        .into_iter()
        .map(|size| SizeView {
            dimensions: parsed
                .iter()
                .map(|&aspect| {
                    let drawn = spoonstill_app::formats::sizes(aspect)
                        .into_iter()
                        .find(|s| s.id == size.id)
                        .map_or_else(|| "—".to_owned(), |s| s.dimensions());
                    (aspect.as_str().to_owned(), drawn)
                })
                .collect(),
            id: size.id.to_owned(),
            description: size.description.to_owned(),
            short_edge: size.short_edge,
            default: size.default,
        })
        .collect();

    FormatsView {
        aspects: aspects
            .into_iter()
            .map(|aspect| AspectView {
                id: aspect.id.to_owned(),
                description: aspect.description.to_owned(),
                default: aspect.default,
            })
            .collect(),
        sizes,
    }
}

/// The two lists the Output screen's choosers are built from.
#[derive(Debug, Clone, Serialize)]
struct FormatsView {
    aspects: Vec<AspectView>,
    sizes: Vec<SizeView>,
}

/// One shape.
#[derive(Debug, Clone, Serialize)]
struct AspectView {
    id: String,
    description: String,
    default: bool,
}

/// One size, with what it comes to in every shape.
#[derive(Debug, Clone, Serialize)]
struct SizeView {
    id: String,
    description: String,
    short_edge: u32,
    default: bool,
    /// Keyed by aspect id — `{"16:9": "3840x2160", "9:16": "2160x3840"}`.
    dimensions: std::collections::BTreeMap<String, String>,
}

/// One theme, drawn over a stand-in photograph, as raw RGBA.
///
/// Raw rather than a PNG because nothing in this program encodes a PNG and
/// nothing should have to: the webview paints it straight into a `<canvas>`
/// with `putImageData`, which wants exactly these bytes. The first eight are
/// the width and height as little-endian `u32`s, so one response carries the
/// picture and its shape and the two cannot disagree.
///
/// It is the *renderer's* preview (`spoonstill_app::subtitles::preview`), not
/// a CSS imitation — see that module for why that matters.
#[tauri::command]
async fn subtitle_preview(
    text: String,
    theme: String,
    position: String,
    short_edge: u32,
    aspect: Option<String>,
) -> Result<tauri::ipc::Response, String> {
    let preview = tauri::async_runtime::spawn_blocking(move || {
        spoonstill_app::subtitles::preview(
            &text,
            &theme,
            &position,
            short_edge,
            aspect.as_deref().unwrap_or_default(),
        )
    })
    .await
    .map_err(|e| format!("the preview thread failed: {e}"))??;

    let mut body = Vec::with_capacity(preview.rgba.len() + 8);
    body.extend_from_slice(&preview.width.to_le_bytes());
    body.extend_from_slice(&preview.height.to_le_bytes());
    body.extend_from_slice(&preview.rgba);
    Ok(tauri::ipc::Response::new(body))
}

/// One theme in the window's list.
#[derive(Debug, Clone, Serialize)]
struct ThemeView {
    id: String,
    description: String,
    default: bool,
}

/// Render the whole project, reporting each scene as it lands.
#[tauri::command]
async fn render_project(
    request: RenderRequest,
    on_progress: Channel<ProgressView>,
    active: State<'_, Active>,
    session: State<'_, Session>,
) -> Result<FilmView, String> {
    let RenderRequest {
        path,
        jobs,
        audio_jobs,
        force,
        voice,
        out_dir,
        out_name,
        subtitles,
        subtitle_theme,
        subtitle_position,
        aspect,
        resolution,
    } = request;

    // Refused here rather than deep in the render, for the same reason the
    // output path is: a name the window should never have sent is a bug worth
    // seeing, not a silent fallback to the default look (D-055).
    let subtitle_placement = match subtitle_position.filter(|p| !p.is_empty()) {
        None => None,
        Some(edge) => Some(
            spoonstill_core::captions::Placement::parse(&edge)
                .ok_or_else(|| format!("{edge:?} is not bottom or top"))?,
        ),
    };
    let subtitle_theme = match subtitle_theme.filter(|t| !t.is_empty()) {
        None => None,
        Some(name) => Some(
            spoonstill_core::captions::SubtitleTheme::parse(&name).ok_or_else(|| {
                format!(
                    "{name:?} is not one of {}",
                    spoonstill_core::captions::SubtitleTheme::names()
                )
            })?,
        ),
    };

    // Refused here for the same reason the theme is: a shape the window should
    // never have sent is a bug worth seeing. The render would refuse it too —
    // `apply_geometry_override` calls the same `OutputSpec::new` — but it
    // would do it after taking the lock and loading the project (D-143).
    let aspect = match aspect.filter(|a| !a.is_empty()) {
        None => None,
        Some(text) => Some(
            spoonstill_core::Aspect::parse(&text)
                .ok_or_else(|| format!("{text:?} is not one of 16:9, 9:16, 1:1"))?,
        ),
    };
    let short_edge = match resolution.filter(|r| !r.is_empty()) {
        None => None,
        Some(text) => Some(spoonstill_app::formats::parse_size(&text)?),
    };

    // Resolved and refused here, before the lock is taken and before a single
    // frame is encoded — the same check the box on screen ran as it was typed.
    let out = match (out_dir, out_name) {
        (Some(dir), Some(name)) => Some(PathBuf::from(resolve_output(dir, name)?)),
        _ => None,
    };

    let cancel = Cancel::new();
    // Claimed through a guard, so the slot is released on **every** exit path
    // (D-115). It used to be cleared by a line after `handle.await?`, which the
    // `?` skips: one panicked render and the window believed a render was
    // running for the rest of the session, refusing every later one.
    let _claim = ActiveRender::claim(&active, cancel.clone())?;

    let handle = tauri::async_runtime::spawn_blocking({
        let cancel = cancel.clone();
        move || {
            let defaults = RenderProjectOptions::for_project(&path);
            let options = RenderProjectOptions {
                // `None` lets `render_project` size the pool against this
                // run's geometry, which the window can change (D-143, D-144).
                jobs: jobs.map(|jobs| jobs.max(1)),
                audio_jobs: audio_jobs.unwrap_or(defaults.audio_jobs).max(1),
                force,
                // An override for this run. Nothing here writes to
                // `project.yaml` (D-013).
                voice: voice.filter(|v| !v.is_empty()),
                out,
                subtitles,
                subtitle_theme,
                subtitle_placement,
                aspect,
                short_edge,
                ..defaults
            };

            spoonstill_app::render_project(&options, &cancel, &|event| {
                let _ = on_progress.send(view_of(event));
            })
            .map(|film| FilmView {
                path: film.path.display().to_string(),
                duration: film.duration,
                scenes: film.scenes,
                frames: film.frames,
                reused_audio: film.reused_audio,
                reused_segments: film.reused_segments,
            })
            .map_err(|e| e.to_string())
        }
    });

    let result = handle
        .await
        .map_err(|e| format!("the render thread failed: {e}"))?;

    if let (Ok(film), Ok(mut slot)) = (&result, session.film.lock()) {
        *slot = Some(PathBuf::from(&film.path));
    }
    result
}

/// Play the film this window just made, in whatever the system uses for MP4s.
///
/// **Takes no path.** The only file it can open is the one a render in this
/// window produced, which is why the capability file grants the shell no
/// filesystem scope at all: there is no argument here to point somewhere else.
#[tauri::command]
fn open_film(app: tauri::AppHandle, session: State<'_, Session>) -> Result<(), String> {
    let film = session
        .film
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or("no film has been made in this window yet")?;
    app.opener()
        .open_path(film.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Where this machine's activity CSV is, and whether anything is in it yet.
#[derive(Debug, Clone, Serialize)]
struct ActivityView {
    path: String,
    exists: bool,
    /// Bytes, so the window can say "empty" rather than opening nothing.
    size: u64,
}

/// The one CSV holding every event from every project (D-093).
#[tauri::command]
fn activity_log() -> Result<ActivityView, String> {
    let path = spoonstill_app::runs_index_path()
        .ok_or("this machine will not say where its config directory is")?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(ActivityView {
        exists: path.exists(),
        size,
        path: path.display().to_string(),
    })
}

/// Open that CSV in whatever opens a CSV here, or show it in the file manager.
///
/// The path is **this program's**, resolved in Rust — the window names neither
/// a directory nor a file, so there is nothing here for a webview to point
/// somewhere else (D-085's rule, applied again).
#[tauri::command]
fn open_activity_log(app: tauri::AppHandle, reveal: bool) -> Result<(), String> {
    let path = spoonstill_app::runs_index_path()
        .ok_or("this machine will not say where its config directory is")?;
    if !path.exists() {
        return Err("nothing has been recorded yet — render something first".to_owned());
    }
    let target = if reveal {
        path.parent().unwrap_or(&path).to_path_buf()
    } else {
        path
    };
    app.opener()
        .open_path(target.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

/// Show the open project in the system file manager. Also takes no path.
#[tauri::command]
fn reveal_project(app: tauri::AppHandle, session: State<'_, Session>) -> Result<(), String> {
    let root = session
        .root
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .ok_or("no project is open")?;
    app.opener().reveal_item_in_dir(&root).map_err(|e| {
        // `reveal` wants something inside the folder on some platforms; the
        // folder itself is the fallback, and saying which failed is better
        // than a silent no-op — the bug this replaced was exactly that.
        let _ = app.opener().open_path(root.to_string_lossy(), None::<&str>);
        e.to_string()
    })
}

/// Ask a running render to stop (D-045: ask, wait, force — the ladder itself
/// lives at the process boundary, not here).
#[tauri::command]
fn cancel_render(active: State<'_, Active>) -> Result<bool, String> {
    let slot = active
        .0
        .lock()
        .map_err(|_| "the render state is poisoned")?;
    match slot.as_ref() {
        Some(cancel) => {
            cancel.request();
            Ok(true)
        }
        None => Ok(false),
    }
}

/// The project named on the command line, if there was one.
///
/// `spoonstill-desktop /path/to/film` opens that folder at startup — which is
/// what a file manager's "Open With" passes, and what lets the window be
/// driven from a terminal like every other part of this program (D-010: if the
/// CLI cannot do it, it does not exist — the reverse is worth honouring too).
#[tauri::command]
fn initial_project() -> Option<String> {
    std::env::args()
        .nth(1)
        .filter(|argument| !argument.starts_with('-'))
        .filter(|argument| std::path::Path::new(argument).is_dir())
}

fn view_of(event: FilmEvent) -> ProgressView {
    match event {
        FilmEvent::Planned {
            scenes,
            jobs,
            audio_jobs,
            per_worker,
            limited_by_memory,
        } => ProgressView::Planned {
            scenes,
            jobs,
            audio_jobs,
            // Worded in Rust, not the webview: a page that divides bytes is a
            // second answer to a question this crate already answers (D-010).
            per_worker: spoonstill_app::capacity::gigabytes(per_worker),
            limited_by_memory,
        },
        FilmEvent::MemoryPressure(pressure) => ProgressView::MemoryPressure {
            jobs: pressure.jobs,
            needed: spoonstill_app::capacity::gigabytes(pressure.needed),
            budget: spoonstill_app::capacity::gigabytes(pressure.budget),
            fits: pressure.fits,
        },
        FilmEvent::Audio {
            index,
            id,
            kind,
            duration,
            reused,
        } => ProgressView::Audio {
            index,
            id,
            source: kind.to_owned(),
            duration,
            reused,
        },
        FilmEvent::Segment {
            index,
            id,
            frames,
            duration,
            reused,
        } => ProgressView::Segment {
            index,
            id,
            frames,
            duration,
            reused,
        },
        FilmEvent::Failed { index, id, detail } => ProgressView::Failed { index, id, detail },
        FilmEvent::Joining { segments } => ProgressView::Joining { segments },
    }
}

/// A path relative to the project root when it is inside it — the grid is
/// unreadable at 500 rows of absolute paths.
fn relative(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(Active::default())
        .manage(Session::default())
        // Closing the window during a render asks it to stop, the same way the
        // Cancel button does (D-115). Without this the process simply ended
        // mid-encode: the CLI has had a signal ladder since D-045 and the
        // window had nothing, so every scene in flight left a `.partial` file
        // behind and the operator was told nothing.
        //
        // `request()` is cooperative and returns immediately — the pool stops
        // admitting work and each running FFmpeg is asked, then forced. The
        // close is **not** prevented: a window that refuses to shut is a worse
        // failure than a scene that has to be re-encoded, and every artifact is
        // written beside-then-renamed (D-042), so there is nothing half-written
        // for a caller to find.
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                use tauri::Manager;
                if let Some(active) = window.try_state::<Active>()
                    && let Ok(slot) = active.0.lock()
                    && let Some(cancel) = slot.as_ref()
                {
                    cancel.request();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            initial_project,
            recent_projects,
            forget_project,
            app_settings,
            set_default_voice,
            activity_log,
            open_activity_log,
            install_tool,
            ffmpeg_status,
            create_project,
            add_media,
            voices,
            preview_voice,
            resolve_output,
            provider_status,
            validate_project,
            set_narration,
            remove_scene,
            move_scene,
            subtitle_themes,
            output_formats,
            subtitle_preview,
            render_project,
            cancel_render,
            open_film,
            reveal_project
        ])
        .run(tauri::generate_context!())
        .expect("the spoonstill window could not start");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The window holds a folder and a file name in two boxes and this is the
    /// only thing that puts them back together, so it is the only thing that
    /// can refuse a name that would escape the folder the operator chose.
    #[test]
    fn a_name_that_is_a_path_is_refused() {
        let dir = std::env::temp_dir().display().to_string();
        for name in ["a/b.mp4", "..\\b.mp4", "../out.mp4", ".hidden.mp4", "  "] {
            assert!(
                resolve_output(dir.clone(), name.to_owned()).is_err(),
                "{name:?} should not be accepted as a file name"
            );
        }
    }

    /// A path is never trimmed (D-089).
    ///
    /// `~/Downloads/RANDOM vidoe ` — trailing space and all — is a folder a
    /// file manager will make and macOS will keep. Trimming it named a folder
    /// that does not exist, so `resolve_output` failed, so Render greyed out,
    /// on a real project with five valid scenes and nothing wrong with it.
    #[test]
    fn a_folder_whose_name_ends_in_a_space_is_still_that_folder() {
        let base = std::env::temp_dir().join(format!("spoonstill-d088-{}", std::process::id()));
        for folder in ["RANDOM vidoe ", " leading", "  both  "] {
            let dir = base.join(folder);
            std::fs::create_dir_all(&dir).expect("create the awkwardly named folder");

            let resolved = resolve_output(dir.display().to_string(), "film".to_owned())
                .unwrap_or_else(|e| panic!("{folder:?} should resolve, got {e}"));

            assert_eq!(
                PathBuf::from(&resolved),
                dir.join("film.mp4"),
                "the folder the operator chose is the folder the film lands in"
            );
        }
        std::fs::remove_dir_all(&base).ok();
    }

    /// The folder is not there, and the message has to make that *visible* —
    /// an unquoted path ending in a space reads like a path that should work.
    #[test]
    fn a_missing_folder_is_quoted_so_invisible_characters_are_visible() {
        let missing = std::env::temp_dir().join("spoonstill-no-such-folder ");
        std::fs::remove_dir_all(&missing).ok();
        let error = resolve_output(missing.display().to_string(), "film.mp4".to_owned())
            .expect_err("a folder that is not there cannot resolve");
        assert!(error.contains('"'), "{error}");
        assert!(error.ends_with("is not a folder"), "{error}");
    }

    /// A film is an MP4 and the operator who typed `holiday` meant
    /// `holiday.mp4` — not a file their player refuses to open.
    #[test]
    fn a_bare_name_gains_the_extension_it_was_always_going_to_have() {
        let dir = std::env::temp_dir();
        let resolved = resolve_output(dir.display().to_string(), "holiday".to_owned())
            .expect("the temp directory exists");
        assert_eq!(PathBuf::from(&resolved), dir.join("holiday.mp4"));

        // An extension the operator gave is theirs and is left alone.
        let kept = resolve_output(dir.display().to_string(), "holiday.mov".to_owned())
            .expect("the temp directory exists");
        assert!(kept.ends_with("holiday.mov"), "{kept}");
    }

    /// A folder that is not there is a sentence now rather than a failure forty
    /// minutes into a render (D-002).
    #[test]
    fn a_folder_that_is_not_there_is_named() {
        let missing = std::env::temp_dir().join("spoonstill-no-such-folder-9d2f1");
        let error = resolve_output(missing.display().to_string(), "out.mp4".to_owned())
            .expect_err("the folder does not exist");
        assert!(error.contains("is not a folder"), "{error}");

        let error =
            resolve_output(String::new(), "out.mp4".to_owned()).expect_err("no folder was chosen");
        assert!(error.contains("choose a folder"), "{error}");
    }

    /// D-115. The window's render slot is released on every exit path.
    ///
    /// It used to be released by a statement after `handle.await?`, and a
    /// panic in the render thread makes that `?` return — so the slot stayed
    /// claimed and the window refused every later render with "a render is
    /// already running in this window" until the app was restarted.
    #[test]
    fn the_render_slot_is_released_even_when_the_render_panics() {
        let active = Active::default();

        // Held: a second claim is refused, which is the behaviour that must
        // survive the fix rather than be traded away for it.
        let first = ActiveRender::claim(&active, Cancel::new()).expect("claims");
        assert!(
            ActiveRender::claim(&active, Cancel::new()).is_err(),
            "two renders at once in one window"
        );
        drop(first);
        assert!(active.0.lock().unwrap().is_none(), "not released on drop");

        // Unwound past: the guard still runs.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _claim = ActiveRender::claim(&active, Cancel::new()).expect("claims");
            panic!("the render thread died");
        }));
        assert!(panicked.is_err(), "the panic should have propagated");
        assert!(
            active.0.lock().unwrap().is_none(),
            "a panicked render left the window believing one was still running"
        );

        // So the next render can start, which is the whole point.
        let after = ActiveRender::claim(&active, Cancel::new()).expect("claims after a panic");
        drop(after);
    }

    /// Closing the window during a render asks it to stop (D-115).
    ///
    /// The handler itself needs a running Tauri window, so what is asserted
    /// here is the part that is ours: the slot a `CloseRequested` reaches
    /// carries a live `Cancel`, and requesting it is observable. Without the
    /// slot holding a cancel there would be nothing for the handler to call.
    #[test]
    fn an_active_render_can_be_cancelled_through_the_slot() {
        let active = Active::default();
        let cancel = Cancel::new();
        let _claim = ActiveRender::claim(&active, cancel.clone()).expect("claims");

        assert!(!cancel.is_requested(), "nothing has asked it to stop yet");
        // Exactly what the close handler and `cancel_render` both do.
        if let Some(held) = active.0.lock().unwrap().as_ref() {
            held.request();
        }
        assert!(
            cancel.is_requested(),
            "the cancel in the slot is not the one the render is watching"
        );
    }

    /// D-127. A command that writes acts on the project this window has open,
    /// not on a path the page named.
    ///
    /// `open_film` and `reveal_project` have taken no path since D-086, for the
    /// reason written beside `Session`. The four commands that *rearrange the
    /// operator's folder* — remove, move, edit a narration, copy media in —
    /// took `root: String` straight from the webview, so the rule covered the
    /// two that open a file and not the ones that move photographs.
    #[test]
    fn a_command_that_writes_is_bound_to_the_project_that_is_open() {
        let dir = std::env::temp_dir().join(format!("spoonstill-bound-{}", std::process::id()));
        let other = dir.join("somebody-elses-project");
        let open = dir.join("the-open-project");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&other).expect("scratch");
        std::fs::create_dir_all(&open).expect("scratch");

        let session = Session::default();

        // Nothing open: every write is refused, rather than acting on whatever
        // the page happened to send.
        let error =
            project_root(&session, &open.display().to_string()).expect_err("no project is open");
        assert!(error.contains("no project is open"), "{error}");

        *session.root.lock().unwrap() = Some(open.clone());

        // The project that is open, named the way the page names it.
        assert_eq!(
            project_root(&session, &open.display().to_string()).expect("the open project"),
            open
        );

        // Any other folder is refused, which is the whole point.
        let error = project_root(&session, &other.display().to_string())
            .expect_err("another project must be refused");
        assert!(
            error.contains("not the project this window has open"),
            "{error}"
        );

        // And a path that resolves to the same folder by another spelling is
        // the same project — a page is not wrong for saying `./x` where Rust
        // remembered `/tmp/x`.
        let spelled = open.join("..").join("the-open-project");
        assert_eq!(
            project_root(&session, &spelled.display().to_string()).expect("same folder"),
            open
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}
