//! `still` — the spoonstill command line.
//!
//! D-010: this is the permanent, complete control surface, not a stepping stone
//! to the GUI. **If the CLI cannot do it, it does not exist.** The Tauri shell
//! arriving at M4 is an adapter over `spoonstill-app` and owns no business
//! logic, so every capability lands here first — and this file stays a thin
//! translation between arguments and that crate.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use spoonstill_app::diagnostics;
use spoonstill_app::film::{FilmEvent, SerialEvents};
use spoonstill_app::render::RenderSceneOptions;
use spoonstill_app::surface::{Cancel, EncodeSettings};
use spoonstill_core::{Anchor, Aspect, MotionKind, MotionSpec};

#[derive(Debug, Parser)]
#[command(
    name = "still",
    version,
    about = "Batch (still image + narration) -> one MP4 with Ken Burns motion.",
    long_about = "spoonstill turns pairs of (still image, narration) into one MP4, \
                  with Ken Burns motion on each still and cuts on narration \
                  boundaries.\n\nIt is a batch renderer, not a video editor: there is \
                  no timeline and no scrubber (D-001)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Make a new project folder, and optionally fill it in one go.
    New(NewArgs),
    /// Copy photos and recordings into a project, numbered and paired.
    Add(AddArgs),
    /// Check a project folder and report every problem at once.
    Validate(ValidateArgs),
    /// Render a whole project folder to one film.
    Render(RenderArgs),
    /// Render a single scene to one segment.
    RenderScene(RenderSceneArgs),
    /// List the voices a text-to-speech provider offers.
    Voices(VoicesArgs),
    /// Diagnostics: logs and the bundle to send when something goes wrong.
    #[command(subcommand)]
    Diagnostics(DiagnosticsCommand),
}

#[derive(Debug, Args)]
struct NewArgs {
    /// Where the project folder goes. Created if it is not there.
    #[arg(value_name = "DIR")]
    project: PathBuf,

    /// Photos and recordings to fill it with, in any order (D-080).
    ///
    /// Folders contribute the media directly inside them, so this accepts a
    /// camera's export folder as readily as a list of files.
    #[arg(value_name = "FILE")]
    media: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct AddArgs {
    /// The project folder to add to.
    #[arg(value_name = "DIR")]
    project: PathBuf,

    /// Photos and recordings, or folders of them.
    #[arg(value_name = "FILE", required = true)]
    media: Vec<PathBuf>,
}

#[derive(Debug, Args)]
struct ValidateArgs {
    /// The project folder. Defaults to the current directory.
    #[arg(value_name = "DIR", default_value = ".")]
    project: PathBuf,

    /// List every scene, however many there are. Without this, the list is
    /// printed only for a project small enough to read.
    #[arg(long)]
    list: bool,

    /// Skip the ffprobe check on every referenced file.
    ///
    /// Faster, and weaker: extensions are a hint, not evidence (D-052), so a
    /// truncated JPEG or a zero-byte MP3 will pass and then fail the render.
    #[arg(long)]
    no_probe: bool,
}

#[derive(Debug, Args)]
struct RenderArgs {
    /// The project folder. Defaults to the current directory.
    #[arg(value_name = "DIR", default_value = ".")]
    project: PathBuf,

    /// Where to write the film. Defaults to the project's `output` setting.
    #[arg(long, value_name = "PATH")]
    out: Option<PathBuf>,

    /// How many scenes to render at once (D-044).
    ///
    /// Defaults to one per two cores, capped at 4 — measured, not guessed:
    /// the curve flattens at three because x264 already threads internally,
    /// while memory keeps climbing at about 780 MB per concurrent segment.
    /// Higher values are allowed; they are a decision about memory.
    #[arg(long, short = 'j', value_name = "N")]
    jobs: Option<usize>,

    /// How many narrations to resolve at once (D-044).
    ///
    /// A separate pool because ingest is I/O-bound and rendering is not — and
    /// because at slice 4 this becomes the TTS provider's rate limit.
    #[arg(long, value_name = "N")]
    audio_jobs: Option<usize>,

    /// Render even though another run appears to hold this project.
    ///
    /// For the lock a crashed run left behind. Two renders of one project at
    /// the same time would interleave segments into one film.
    #[arg(long)]
    force: bool,

    /// Speak every spoken scene in this voice, for this run only.
    ///
    /// An override, not an edit: `project.yaml` is an input and nothing here
    /// writes to it (D-013). `still voices` lists what a provider offers.
    #[arg(long, value_name = "VOICE")]
    voice: Option<String>,

    /// Use this TTS provider for this run only.
    #[arg(long, value_name = "NAME")]
    provider: Option<String>,
}

#[derive(Debug, Args)]
struct VoicesArgs {
    /// Which provider to ask. Defaults to the one a project with no settings
    /// would use.
    #[arg(long, value_name = "NAME", default_value = spoonstill_app::import::settings::DEFAULT_PROVIDER)]
    provider: String,

    /// Show only voices whose name or locale contains this, case-insensitively.
    #[arg(value_name = "FILTER")]
    filter: Option<String>,

    /// Install the provider's tooling, through this machine's own package
    /// manager, and then list what it offers.
    ///
    /// The window has a button for this; the rule is that the CLI can do
    /// everything the window can (D-010), so it is here first.
    #[arg(long)]
    install: bool,
}

#[derive(Debug, Args)]
struct RenderSceneArgs {
    /// The still image.
    #[arg(long, value_name = "PATH")]
    image: PathBuf,

    /// The narration. Its measured duration decides the segment length (D-021).
    #[arg(long, value_name = "PATH")]
    audio: PathBuf,

    /// Where to write the segment.
    #[arg(long, value_name = "PATH")]
    out: PathBuf,

    /// Output aspect ratio.
    #[arg(long, default_value = "16:9", value_parser = parse_aspect)]
    aspect: Aspect,

    /// Output short edge in pixels: 1080 gives 1920x1080, 1080x1920 or
    /// 1080x1080 depending on the aspect.
    #[arg(long, default_value_t = 1080, value_name = "PIXELS")]
    short_edge: u32,

    /// Frame rate.
    #[arg(long, default_value_t = 30, value_name = "FPS")]
    fps: u32,

    /// Force a specific move. Omit to derive one from scene identity (D-035),
    /// which is what keeps a re-render byte-identical.
    #[arg(long, value_parser = parse_motion, value_name = "KIND")]
    motion: Option<MotionKind>,

    /// Focal point for a zoom, or cross-axis position for a pan.
    #[arg(long, default_value = "center", value_parser = parse_anchor)]
    anchor: Anchor,

    /// Zoom span as a fraction, e.g. 0.10 for a 10% push.
    #[arg(long, default_value_t = spoonstill_core::motion::DEFAULT_AMOUNT)]
    amount: f64,

    /// x264 preset (D-036).
    #[arg(long, default_value = "medium")]
    preset: String,

    /// x264 CRF (D-036).
    #[arg(long, default_value_t = 18)]
    crf: u32,

    /// Project identity, used to seed the move and key the cache.
    #[arg(long, default_value = "single-scene", value_name = "ID")]
    project_id: String,

    /// Scene index within the project, also part of the seed.
    #[arg(long, default_value_t = 0, value_name = "N")]
    scene_index: u32,
}

#[derive(Debug, Subcommand)]
enum DiagnosticsCommand {
    /// Write one file describing what happened, to send when reporting a
    /// problem.
    Export {
        /// Where to write it. Defaults to a timestamped file here.
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,

        /// The project whose logs to collect. Defaults to the current
        /// directory.
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
    /// Show where diagnostics are being written.
    Where {
        /// The project to report on. Defaults to the current directory.
        #[arg(long, value_name = "DIR")]
        project: Option<PathBuf>,
    },
}

fn parse_aspect(text: &str) -> Result<Aspect, String> {
    Aspect::parse(text).ok_or_else(|| {
        format!("{text:?} is not one of 16:9, 9:16, 1:1 (or landscape, portrait, square)")
    })
}

fn parse_motion(text: &str) -> Result<MotionKind, String> {
    MotionKind::parse(text).ok_or_else(|| {
        let names: Vec<&str> = MotionKind::ALL.iter().map(|k| k.as_str()).collect();
        format!("{text:?} is not one of {}", names.join(", "))
    })
}

fn parse_anchor(text: &str) -> Result<Anchor, String> {
    let normalized = text.trim().to_ascii_lowercase().replace(['_', ' '], "-");
    Anchor::ALL
        .into_iter()
        .find(|a| a.as_str() == normalized)
        .ok_or_else(|| {
            let names: Vec<&str> = Anchor::ALL.iter().map(|a| a.as_str()).collect();
            format!("{text:?} is not one of {}", names.join(", "))
        })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("still: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), String> {
    match cli.command {
        Command::New(args) => new_project(args),
        Command::Add(args) => add_media(&args.project, &args.media),
        Command::Validate(args) => validate(args),
        Command::Voices(args) => list_voices(&args),
        Command::Render(args) => render_project(args),
        Command::RenderScene(args) => render_scene(args),
        Command::Diagnostics(DiagnosticsCommand::Export { out, project }) => {
            export_diagnostics(out, project)
        }
        Command::Diagnostics(DiagnosticsCommand::Where { project }) => show_log_location(project),
    }
}

/// `still new DIR [FILE...]` — the first command anyone runs.
///
/// D-080: a project is a folder, so making one is making a folder. The media
/// arguments are optional because the operator may prefer to drag files in
/// themselves, and mandatory in spirit because a folder with nothing in it is
/// not yet a film.
fn new_project(args: NewArgs) -> Result<(), String> {
    let root = spoonstill_app::create_project(&args.project).map_err(|e| e.to_string())?;
    println!("{}", root.display());
    if args.media.is_empty() {
        println!(
            "  empty — add photos with `still add {} FILE...`",
            args.project.display()
        );
        return Ok(());
    }
    add_media(&root, &args.media)
}

/// `still add DIR FILE...` — copy media in under names the renderer pairs.
///
/// Prints what each file became, because renaming someone's files for them is
/// only acceptable if they can see exactly what happened (D-080).
fn add_media(root: &std::path::Path, media: &[PathBuf]) -> Result<(), String> {
    let report = spoonstill_app::add_media(root, media).map_err(|e| e.to_string())?;

    for image in &report.images {
        let narration = match (report.narration_for(image), report.script_for(image)) {
            (Some(audio), _) => name_of(&audio.source),
            (None, Some(script)) => format!("{} (spoken)", name_of(&script.source)),
            (None, None) => "silent".to_owned(),
        };
        println!(
            "  {:<12} {}  +  {}",
            image.name,
            name_of(&image.source),
            narration
        );
    }
    for skipped in &report.skipped {
        println!(
            "  {:<12} {} — {}",
            "skipped",
            name_of(&skipped.source),
            skipped.reason
        );
    }

    if report.is_empty() {
        return Err(format!(
            "nothing usable in those {} argument{}",
            media.len(),
            if media.len() == 1 { "" } else { "s" }
        ));
    }
    println!("  {}", report.summary());

    if report.audio_without_image > 0 {
        println!(
            "  note: {} recording{} had no photo left to pair with and {} not copied",
            report.audio_without_image,
            if report.audio_without_image == 1 {
                ""
            } else {
                "s"
            },
            if report.audio_without_image == 1 {
                "was"
            } else {
                "were"
            }
        );
    }
    Ok(())
}

/// A file's own name, for a report that lines up.
fn name_of(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

/// `still voices` — what can this build actually say a line in.
///
/// D-002: the operator finds out that a provider is unreachable here, in one
/// second, rather than at scene 340 of 500.
fn list_voices(args: &VoicesArgs) -> Result<(), String> {
    let provider = spoonstill_app::tts::provider(&args.provider).map_err(|e| e.to_string())?;

    if let spoonstill_app::tts::Availability::Missing(detail) = provider.availability() {
        if !args.install {
            return Err(format!("{detail}\n  try `still voices --install`"));
        }
        println!("  installing {}…", provider.id());
        let ran = provider.install().map_err(|e| e.to_string())?;
        println!("  {ran}");
    } else if args.install {
        println!("  {} is already installed", provider.id());
    }

    let wanted = args.filter.as_deref().map(str::to_lowercase);
    let voices = provider.voices().map_err(|e| e.to_string())?;
    let mut shown = 0;
    for voice in &voices {
        if let Some(filter) = &wanted
            && !voice.id.to_lowercase().contains(filter)
            && !voice.locale.to_lowercase().contains(filter)
        {
            continue;
        }
        shown += 1;
        println!("  {:<34} {:<8} {}", voice.id, voice.gender, voice.note);
    }
    println!(
        "  {shown} of {} voices from {}",
        voices.len(),
        provider.id()
    );
    Ok(())
}

/// How many scenes are worth listing without being asked.
///
/// A three-scene project wants the list — it is the review grid of D-051 in
/// text. A 500-scene project does not, and printing it anyway is how the
/// problem summary scrolls off the screen (D-002).
const SCENES_WORTH_LISTING: usize = 20;

/// Names every file, resolves every path, probes every file, and reports the
/// lot. The one command an operator runs before committing to a batch.
fn validate(args: ValidateArgs) -> Result<(), String> {
    // D-052: extensions are a hint, not evidence. `--no-probe` is offered
    // because a 500-file probe is not free, and it is named for what it gives
    // up rather than for what it saves.
    let probe = spoonstill_app::ProbeCheck::from_env();
    let skip = SkipProbe;
    let media: &dyn spoonstill_app::MediaCheck = if args.no_probe { &skip } else { &probe };

    let project = spoonstill_app::import::load(&args.project, media).map_err(|e| e.to_string())?;

    let source = match &project.mode {
        spoonstill_app::Mode::Manifest(path) => format!(
            "manifest {}",
            path.file_name()
                .unwrap_or(path.as_os_str())
                .to_string_lossy()
        ),
        spoonstill_app::Mode::Convention => "stem-keyed pairing".to_owned(),
    };
    println!("{} — {}", project.root.display(), source);

    let (mut tts, mut file, mut silent) = (0_usize, 0_usize, 0_usize);
    for scene in &project.scenes {
        match scene.spec.source.kind() {
            "tts" => tts += 1,
            "file" => file += 1,
            _ => silent += 1,
        }
    }
    println!(
        "  {} scene{} — {tts} narrated, {file} supplied, {silent} silent",
        project.scenes.len(),
        if project.scenes.len() == 1 { "" } else { "s" }
    );

    if args.list || project.scenes.len() <= SCENES_WORTH_LISTING {
        // Width from the longest id, so a project of `001`s and one of
        // `opening`/`middle`/`closing` both line up.
        let width = project
            .scenes
            .iter()
            .map(|s| s.spec.id.as_str().chars().count())
            .max()
            .unwrap_or(0);
        for (index, scene) in project.scenes.iter().enumerate() {
            println!(
                "  {index:>4}  {:<width$}  {:<6}  {}",
                scene.spec.id,
                scene.spec.source.kind(),
                short(&scene.image, &project.root)
            );
        }
    }

    let errors = project.errors().count();
    let warnings = project.warnings().count();
    if project.problems.is_empty() {
        println!("  no problems");
        return Ok(());
    }

    println!();
    for problem in &project.problems {
        println!("  {:<5} {problem}", problem.severity().as_str());
    }
    println!();

    if errors == 0 {
        println!(
            "  {warnings} warning{} — nothing here stops a render",
            if warnings == 1 { "" } else { "s" }
        );
        return Ok(());
    }

    Err(format!(
        "{errors} problem{} in {} ({warnings} warning{})",
        if errors == 1 { "" } else { "s" },
        args.project.display(),
        if warnings == 1 { "" } else { "s" }
    ))
}

/// `--no-probe`: believe every extension.
struct SkipProbe;

impl spoonstill_app::MediaCheck for SkipProbe {
    fn check(&self, _path: &std::path::Path, _role: spoonstill_app::Role) -> Result<(), String> {
        Ok(())
    }
}

/// A path relative to the project root when it is inside it, for a report that
/// is readable at 500 rows.
fn short(path: &std::path::Path, root: &std::path::Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Render a whole project, several scenes at a time.
///
/// The one thing this function owns that the library does not: what a parallel
/// render *looks like* in a terminal. Eight workers finishing in whatever order
/// they finish would produce interleaved half-lines, so every event goes
/// through one mutex and every line is written whole, tagged with the scene it
/// belongs to and a completed-so-far counter rather than a percentage that
/// jumps around.
fn render_project(args: RenderArgs) -> Result<(), String> {
    let defaults = spoonstill_app::RenderProjectOptions::for_project(&args.project);
    let options = spoonstill_app::RenderProjectOptions {
        out: args.out,
        jobs: args.jobs.unwrap_or(defaults.jobs).max(1),
        audio_jobs: args.audio_jobs.unwrap_or(defaults.audio_jobs).max(1),
        force: args.force,
        voice: args.voice.clone(),
        provider: args.provider.clone(),
        ..defaults
    };

    // D-045, same ladder as the single scene: the flag stops the pool
    // admitting work, and each running FFmpeg gets asked, then forced.
    let cancel = Cancel::new();
    let handler = cancel.clone();
    if let Err(error) = ctrlc::set_handler(move || handler.request()) {
        eprintln!("still: warning: Ctrl-C will not be graceful ({error})");
    }

    let total = std::cell::Cell::new(0_usize);
    let done = std::cell::Cell::new(0_usize);
    let events = SerialEvents::new(move |event| match event {
        FilmEvent::Planned {
            scenes,
            jobs,
            audio_jobs,
        } => {
            total.set(scenes);
            done.set(0);
            println!(
                "  {scenes} scene{}, {jobs} at a time ({audio_jobs} for audio)",
                if scenes == 1 { "" } else { "s" }
            );
        }
        FilmEvent::Audio {
            id,
            kind,
            duration,
            reused,
            ..
        } => {
            println!(
                "  audio  {id:<12} {kind:<6} {duration:>8.3}s{}",
                if reused { "  (cached)" } else { "" }
            );
        }
        FilmEvent::Segment {
            id,
            frames,
            duration,
            reused,
            ..
        } => {
            done.set(done.get() + 1);
            println!(
                "  [{:>3}/{}] {id:<12} {frames:>6} frames {duration:>8.3}s{}",
                done.get(),
                total.get(),
                if reused { "  (reused)" } else { "" }
            );
        }
        FilmEvent::Failed { id, detail, .. } => {
            eprintln!("  FAILED {id}: {detail}");
        }
        FilmEvent::Joining { segments } => {
            println!("  joining {segments} segments (stream copy, D-040)");
        }
    });

    let film = spoonstill_app::render_project(&options, &cancel, &|event| events.emit(event))
        .map_err(|error| error.to_string())?;

    println!("{}", film.path.display());
    println!(
        "  {} scene{}, {} frames, {:.3}s (expected {:.3}s)",
        film.scenes,
        if film.scenes == 1 { "" } else { "s" },
        film.frames,
        film.duration,
        film.expected_duration
    );
    println!(
        "  {} narration{} from cache, {} segment{} reused — segments in {}",
        film.reused_audio,
        if film.reused_audio == 1 { "" } else { "s" },
        film.reused_segments,
        if film.reused_segments == 1 { "" } else { "s" },
        film.segments_dir.display()
    );
    Ok(())
}

fn render_scene(args: RenderSceneArgs) -> Result<(), String> {
    let options = RenderSceneOptions {
        image: args.image,
        audio: args.audio,
        out: args.out,
        aspect: args.aspect,
        short_edge: args.short_edge,
        fps: args.fps,
        motion: args
            .motion
            .map(|kind| MotionSpec::new(kind, args.amount, args.anchor)),
        encode: EncodeSettings {
            preset: args.preset,
            crf: args.crf,
        },
        project_id: args.project_id,
        scene_index: args.scene_index,
    };

    // D-045: Ctrl-C sets the flag; the render then asks FFmpeg to quit, waits,
    // and forces. Nothing is deleted here because nothing was ever written to
    // the destination — the segment only exists once it has passed the profile
    // assertion (D-042).
    let cancel = Cancel::new();
    let handler = cancel.clone();
    if let Err(error) = ctrlc::set_handler(move || handler.request()) {
        eprintln!("still: warning: Ctrl-C will not be graceful ({error})");
    }

    // Progress on one rewriting line, so a 500-scene batch does not become
    // 500 pages of scrollback later.
    let mut last_frame = 0_u64;
    let rendered = spoonstill_app::render_scene(&options, &cancel, &mut |progress| {
        if let Some(frame) = progress.frame {
            last_frame = frame;
            eprint!("\r  rendering: frame {frame}");
        }
    })
    .map_err(|error| {
        if last_frame > 0 {
            eprintln!();
        }
        format!("{error}")
    })?;

    if last_frame > 0 {
        eprintln!();
    }

    println!("{}", rendered.path.display());
    println!(
        "  {} frames at {:.6}s — narration {:.6}s plus {:.1}ms of pad (D-022)",
        rendered.frames,
        rendered.duration,
        rendered.narration,
        rendered.pad * 1000.0
    );
    println!(
        "  motion {} (seed {:016x})",
        rendered.motion.descriptor(),
        rendered.motion.seed
    );
    Ok(())
}

fn export_diagnostics(out: Option<PathBuf>, project: Option<PathBuf>) -> Result<(), String> {
    let root = project.unwrap_or_else(|| PathBuf::from("."));
    let destination = out.unwrap_or_else(|| diagnostics::default_bundle_name(&stamp()));

    let report = diagnostics::export(&root, &destination)
        .map_err(|e| format!("could not write {}: {e}", destination.display()))?;

    println!("{}", report.path.display());
    println!(
        "  {} record{} from {} log file{}, {} KiB",
        report.records,
        if report.records == 1 { "" } else { "s" },
        report.log_files,
        if report.log_files == 1 { "" } else { "s" },
        report.bytes.div_ceil(1024)
    );
    if report.records == 0 {
        println!(
            "  (nothing has been rendered for {} yet — run this from the directory \
             containing the failing render, or pass --project)",
            root.display()
        );
    }
    println!();
    println!("Send that file when reporting a problem. It contains your file paths");
    println!("and the exact FFmpeg commands and errors — but no keys, and no media.");
    Ok(())
}

fn show_log_location(project: Option<PathBuf>) -> Result<(), String> {
    let root = project.unwrap_or_else(|| PathBuf::from("."));
    let dir = root
        .join(spoonstill_core::STATE_DIR)
        .join(spoonstill_app::surface::LOGS_DIR);

    println!("{}", dir.display());

    // And the machine-wide one, which is the file to open when the question is
    // "what went wrong" rather than "what went wrong in this project" (D-093).
    if let Some(index) = spoonstill_app::runs_index_path() {
        let size = std::fs::metadata(&index).map(|m| m.len()).unwrap_or(0);
        println!(
            "{}   every project, {}",
            index.display(),
            if size == 0 {
                "nothing recorded yet".to_owned()
            } else {
                format!("{} KB", size / 1024)
            }
        );
    }

    if dir.exists() {
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| format!("could not read {}: {e}", dir.display()))?
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        files.sort();
        for file in &files {
            println!("  {file}");
        }
        if files.is_empty() {
            println!("  (empty)");
        }
    } else {
        println!("  (not created yet — it appears on the first render)");
    }
    println!();
    println!("`still diagnostics export` packages these into one file to send.");
    Ok(())
}

/// A filename-safe UTC stamp: `YYYYMMDD-HHMMSS`.
fn stamp() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let iso = spoonstill_core::diagnostics::format_utc(millis);
    // 2026-08-26T12:34:56.789Z -> 20260826-123456
    let date: String = iso.chars().take(10).filter(|c| *c != '-').collect();
    let time: String = iso.chars().skip(11).take(8).filter(|c| *c != ':').collect();
    format!("{date}-{time}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap's own consistency check: conflicting flags, bad defaults, and
    /// duplicate names are all build-time mistakes that only surface here.
    #[test]
    fn the_command_tree_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn render_scene_parses_its_required_arguments() {
        let cli = Cli::try_parse_from([
            "still",
            "render-scene",
            "--image",
            "a b.jpg",
            "--audio",
            "n.wav",
            "--out",
            "seg.mp4",
        ])
        .expect("the documented invocation parses");
        let Command::RenderScene(args) = cli.command else {
            panic!("wrong subcommand")
        };
        // A path with a space is one argument, not two (D-052).
        assert_eq!(args.image, PathBuf::from("a b.jpg"));
        assert_eq!(args.aspect, Aspect::Landscape16x9);
        assert_eq!((args.short_edge, args.fps), (1080, 30));
        assert_eq!((args.preset.as_str(), args.crf), ("medium", 18));
        // No `--motion` means seeded, which is what keeps re-renders stable.
        assert!(args.motion.is_none());
    }

    /// D-070's default in practice: all three aspects are reachable from the
    /// command line, in the spellings an operator will actually type.
    #[test]
    fn every_v1_aspect_is_reachable() {
        for (text, expected) in [
            ("16:9", Aspect::Landscape16x9),
            ("9:16", Aspect::Portrait9x16),
            ("1:1", Aspect::Square1x1),
            ("portrait", Aspect::Portrait9x16),
            ("square", Aspect::Square1x1),
        ] {
            assert_eq!(parse_aspect(text).unwrap(), expected, "aspect {text}");
        }
    }

    /// A rejected value must list what was acceptable — an operator should not
    /// have to open the docs to find out there are three.
    #[test]
    fn a_bad_value_lists_the_good_ones() {
        let error = parse_aspect("4:3").unwrap_err();
        assert!(error.contains("16:9") && error.contains("9:16") && error.contains("1:1"));

        let error = parse_motion("dolly").unwrap_err();
        assert!(
            error.contains("zoom-in") && error.contains("pan-right"),
            "{error}"
        );

        let error = parse_anchor("middle").unwrap_err();
        assert!(error.contains("center"), "{error}");
    }

    #[test]
    fn motion_and_anchor_parse_the_forms_an_operator_types() {
        assert_eq!(parse_motion("zoom_in").unwrap(), MotionKind::ZoomIn);
        assert_eq!(parse_motion("Pan Right").unwrap(), MotionKind::PanRight);
        assert_eq!(parse_anchor("north-west").unwrap(), Anchor::NorthWest);
        assert_eq!(parse_anchor("North West").unwrap(), Anchor::NorthWest);
    }

    #[test]
    fn diagnostics_export_defaults_to_the_current_project() {
        let cli = Cli::try_parse_from(["still", "diagnostics", "export"]).unwrap();
        let Command::Diagnostics(DiagnosticsCommand::Export { out, project }) = cli.command else {
            panic!("wrong subcommand")
        };
        assert!(out.is_none() && project.is_none());
    }

    /// The stamp goes into a filename, so it must contain nothing a filesystem
    /// objects to — a colon is illegal on Windows (D-071).
    #[test]
    fn the_bundle_stamp_is_filename_safe() {
        let stamp = stamp();
        assert_eq!(stamp.len(), 15, "{stamp}");
        assert!(
            stamp.chars().all(|c| c.is_ascii_digit() || c == '-'),
            "{stamp} is not filename-safe"
        );
    }

    /// D-013: machine state sits beside the work, so `diagnostics where`
    /// points at the same place a render would have written to.
    #[test]
    fn the_state_root_follows_the_output() {
        assert_eq!(
            spoonstill_app::render::state_root_for(&PathBuf::from("/renders/seg.mp4")),
            PathBuf::from("/renders")
        );
    }
}
