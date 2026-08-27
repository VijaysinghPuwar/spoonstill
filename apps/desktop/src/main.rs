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
    /// Whether this scene can render. Kept as a field rather than assumed,
    /// because a provider that is not installed is still a row the grid must
    /// be able to mark.
    renderable: bool,
}

/// One problem, exactly as `still validate` would print it.
#[derive(Debug, Clone, Serialize)]
struct ProblemView {
    severity: String,
    scene: Option<String>,
    message: String,
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
    scenes: Vec<SceneView>,
    problems: Vec<ProblemView>,
    has_errors: bool,
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

/// Install a provider's tooling, and say what was run.
///
/// The window used to print `pip install edge-tts` and leave the operator to
/// find a terminal, which is the program declining to do the thing it just
/// asked for (D-092).
#[tauri::command]
async fn install_provider(provider: String) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        spoonstill_app::tts::provider(&provider)
            .map_err(|e| e.to_string())?
            .install()
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("the install thread failed: {e}"))?
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

/// Whether a voice service can be reached, and what to do if not.
#[derive(Debug, Clone, Serialize)]
struct ProviderStatus {
    id: String,
    ready: bool,
    /// The sentence naming the fix, when it is not ready.
    detail: String,
    /// What this provider speaks when a project names no voice. `project.yaml`
    /// says the word `default`; the window has to be able to say whose voice
    /// that actually is.
    default_voice: String,
}

/// One line from a project's log, for the Runs tab.
#[derive(Debug, Clone, Serialize)]
struct RunLine {
    at: String,
    severity: String,
    scope: String,
    message: String,
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
                }
            })
            .collect();

        let problems = project
            .problems
            .iter()
            .map(|problem| ProblemView {
                severity: problem.severity().as_str().to_owned(),
                scene: problem.scene.as_ref().map(|id| id.as_str().to_owned()),
                message: problem.kind.to_string(),
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
            has_errors: project.has_errors(),
            scenes,
            problems,
        })
    })
    .await
    .map_err(|e| format!("the validation thread failed: {e}"))??;

    // Thumbnails are real files, so the webview has to be allowed to read
    // them — and only them. The grant is per project and happens here, when
    // the operator has just chosen the folder, rather than as a wildcard in
    // the capability file.
    let root = PathBuf::from(&view.root);
    let _ = app.asset_protocol_scope().allow_directory(&root, false);
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
async fn set_narration(root: String, scene: String, text: String) -> Result<(), String> {
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
    tauri::async_runtime::spawn_blocking(move || match spoonstill_app::tts::provider(&provider) {
        Err(e) => Ok(ProviderStatus {
            id: provider,
            ready: false,
            detail: e.to_string(),
            default_voice: String::new(),
        }),
        Ok(engine) => Ok(match engine.availability() {
            spoonstill_app::tts::Availability::Ready => ProviderStatus {
                id: engine.id().to_owned(),
                ready: true,
                detail: String::new(),
                default_voice: engine.default_voice().to_owned(),
            },
            spoonstill_app::tts::Availability::Missing(detail) => ProviderStatus {
                id: engine.id().to_owned(),
                ready: false,
                detail,
                default_voice: engine.default_voice().to_owned(),
            },
        }),
    })
    .await
    .map_err(|e| format!("the check failed: {e}"))?
}

/// The tail of this project's log — every run, successful or not (D-016).
#[tauri::command]
async fn runs(root: String, limit: usize) -> Result<Vec<RunLine>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let dir = PathBuf::from(root)
            .join(spoonstill_core::STATE_DIR)
            .join(spoonstill_app::surface::LOGS_DIR);

        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
            .collect();
        files.sort();

        let mut lines: Vec<RunLine> = Vec::new();
        for file in files.iter().rev().take(2) {
            let text = std::fs::read_to_string(file).unwrap_or_default();
            for line in text.lines() {
                if let Some(parsed) = parse_line(line) {
                    lines.push(parsed);
                }
            }
        }
        // Newest first, because the thing that just went wrong is the thing
        // being looked for.
        lines.reverse();
        lines.truncate(limit.clamp(1, 2000));
        Ok(lines)
    })
    .await
    .map_err(|e| format!("the log could not be read: {e}"))?
}

/// One JSON Lines record, read without a JSON dependency.
///
/// The shell has `serde_json` and could parse this properly; it does not,
/// because the log's shape is `spoonstill-state`'s to define and a second
/// parser here would be a second definition. This reads the four fields the
/// tab shows and ignores everything else, so a new field in the writer cannot
/// break the reader.
fn parse_line(line: &str) -> Option<RunLine> {
    let value: serde_json::Value = serde_json::from_str(line).ok()?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    Some(RunLine {
        at: field("at"),
        severity: field("severity"),
        scope: field("scope"),
        message: field("message"),
    })
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
async fn add_media(root: String, files: Vec<String>) -> Result<IngestView, String> {
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

/// Every voice a provider offers, or the sentence that says why there are none.
#[tauri::command]
async fn voices(provider: String) -> Result<Vec<VoiceView>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = spoonstill_app::tts::provider(&provider).map_err(|e| e.to_string())?;
        if let spoonstill_app::tts::Availability::Missing(detail) = engine.availability() {
            return Err(detail);
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
    root: String,
    provider: String,
    voice: String,
    text: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
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
    } = request;

    // Resolved and refused here, before the lock is taken and before a single
    // frame is encoded — the same check the box on screen ran as it was typed.
    let out = match (out_dir, out_name) {
        (Some(dir), Some(name)) => Some(PathBuf::from(resolve_output(dir, name)?)),
        _ => None,
    };

    let cancel = Cancel::new();
    {
        let mut slot = active
            .0
            .lock()
            .map_err(|_| "the render state is poisoned")?;
        if slot.is_some() {
            return Err("a render is already running in this window".to_owned());
        }
        *slot = Some(cancel.clone());
    }

    let handle = tauri::async_runtime::spawn_blocking({
        let cancel = cancel.clone();
        move || {
            let defaults = RenderProjectOptions::for_project(&path);
            let options = RenderProjectOptions {
                jobs: jobs.unwrap_or(defaults.jobs).max(1),
                audio_jobs: audio_jobs.unwrap_or(defaults.audio_jobs).max(1),
                force,
                // An override for this run. Nothing here writes to
                // `project.yaml` (D-013).
                voice: voice.filter(|v| !v.is_empty()),
                out,
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

    if let Ok(mut slot) = active.0.lock() {
        *slot = None;
    }
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

/// Where a project's diagnostics go, so the window can point at them (D-016).
#[tauri::command]
fn log_directory(path: String) -> String {
    PathBuf::from(path)
        .join(spoonstill_core::STATE_DIR)
        .join(spoonstill_app::surface::LOGS_DIR)
        .display()
        .to_string()
}

fn view_of(event: FilmEvent) -> ProgressView {
    match event {
        FilmEvent::Planned {
            scenes,
            jobs,
            audio_jobs,
        } => ProgressView::Planned {
            scenes,
            jobs,
            audio_jobs,
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
        .invoke_handler(tauri::generate_handler![
            initial_project,
            recent_projects,
            forget_project,
            app_settings,
            set_default_voice,
            install_provider,
            create_project,
            add_media,
            voices,
            preview_voice,
            resolve_output,
            provider_status,
            validate_project,
            set_narration,
            render_project,
            cancel_render,
            open_film,
            reveal_project,
            runs,
            log_directory
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
}
