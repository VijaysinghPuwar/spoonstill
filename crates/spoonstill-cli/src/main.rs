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
use spoonstill_core::captions::{Placement, SubtitleTheme};
use spoonstill_core::diagnostics::{Diagnostics, Event};
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
    /// Take a scene out of a project, keeping its files (D-100).
    Remove(RemoveArgs),
    /// Move a scene to another position in the film (D-100).
    Move(MoveArgs),
    /// Check a project folder and report every problem at once.
    Validate(ValidateArgs),
    /// Render a whole project folder to one film.
    Render(RenderArgs),
    /// Render a single scene to one segment.
    RenderScene(RenderSceneArgs),
    /// List the voices a text-to-speech provider offers.
    Voices(VoicesArgs),
    /// List the subtitle themes, and what each one is for (D-106).
    Subtitles,
    /// List the sizes and aspects a film can be rendered at (D-143).
    #[command(visible_alias = "formats")]
    Resolutions,
    /// Check every program spoonstill needs, and offer to install what is
    /// missing (D-105).
    Doctor(DoctorArgs),
    /// Show the licences of everything built into this binary (D-124).
    #[command(visible_alias = "licenses")]
    Licences,
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
struct RemoveArgs {
    /// The project folder.
    #[arg(value_name = "DIR")]
    project: PathBuf,

    /// Which scene, by its number: `7`, `07` and `007` are the same scene.
    #[arg(value_name = "SCENE", required = true)]
    scenes: Vec<String>,
}

#[derive(Debug, Args)]
struct MoveArgs {
    /// The project folder.
    #[arg(value_name = "DIR")]
    project: PathBuf,

    /// Which scene to move.
    #[arg(value_name = "SCENE")]
    scene: String,

    /// Where it should end up, counting from 1. Past the end means the end.
    #[arg(value_name = "POSITION")]
    to: usize,
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

    /// Kept for scripts. It cannot override a lock a running render holds.
    ///
    /// The lock is the operating system's now (D-113), so a run this machine
    /// lost releases it by itself and the crashed-lock case this flag existed
    /// for needs no flag. What is left is a *live* render, and taking that
    /// project out from under it is the interleaving the lock prevents.
    #[arg(long)]
    force: bool,

    /// Keep every superseded segment instead of sweeping the oldest.
    ///
    /// A render normally keeps the segments it just used plus two spare
    /// generations, so flipping between two themes or two voices stays free
    /// while the folder stays bounded (D-109). This keeps all of them, for an
    /// operator who would rather spend the disk than ever re-encode.
    #[arg(long)]
    keep_cache: bool,

    /// Speak every spoken scene in this voice, for this run only.
    ///
    /// An override, not an edit: `project.yaml` is an input and nothing here
    /// writes to it (D-013). `still voices` lists what a provider offers.
    #[arg(long, value_name = "VOICE")]
    voice: Option<String>,

    /// Use this TTS provider for this run only.
    #[arg(long, value_name = "NAME")]
    provider: Option<String>,

    /// Burn subtitles into the picture for this run (D-106).
    ///
    /// Optionally naming the look: `--subtitles` uses the project's theme,
    /// `--subtitles boxed` overrides it. `still subtitles` lists them all.
    /// An override, not an edit — nothing here writes to `project.yaml`
    /// (D-013).
    #[arg(long, value_name = "THEME", num_args = 0..=1, default_missing_value = "")]
    subtitles: Option<String>,

    /// Render without subtitles, whatever `project.yaml` says.
    #[arg(long, conflicts_with = "subtitles")]
    no_subtitles: bool,

    /// Which edge the subtitles sit against: `bottom` or `top`.
    ///
    /// Worth reaching for when the photographs already carry words along the
    /// bottom, which a burned-in caption would otherwise land on.
    #[arg(long, value_name = "EDGE", conflicts_with = "no_subtitles")]
    subtitle_position: Option<String>,

    /// Render this run in a different shape: `16:9`, `9:16`, `1:1` (D-143).
    ///
    /// `shorts`, `reel`, `tiktok` and `story` all mean `9:16` — an operator
    /// naming the destination gets the frame. An override, not an edit:
    /// nothing here writes to `project.yaml` (D-013).
    #[arg(long, value_name = "RATIO", value_parser = parse_aspect)]
    aspect: Option<Aspect>,

    /// Render this run at a named size: `720p`, `1080p`, `1440p` (2K),
    /// `2160p` (4K).
    ///
    /// `still resolutions` lists them with the pixel dimensions each one
    /// produces in each aspect. The same override rule applies.
    #[arg(long, value_name = "SIZE", conflicts_with = "short_edge")]
    resolution: Option<String>,

    /// The same, as a short edge in pixels, for a size with no name.
    #[arg(long, value_name = "PIXELS")]
    short_edge: Option<u32>,

    /// Render this run at a different frame rate.
    #[arg(long, value_name = "FPS")]
    fps: Option<u32>,
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

    /// The same, by name: `720p`, `1080p`, `1440p` (2K), `2160p` (4K).
    ///
    /// Two spellings of one setting, so clap refuses both at once rather than
    /// letting one silently win (D-143).
    #[arg(long, value_name = "SIZE", conflicts_with = "short_edge")]
    resolution: Option<String>,

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

/// Run one command, and write down that it ran (D-148).
///
/// **One place, above every command**, rather than a line in each: this is
/// exactly the shape D-093 asked for and did not get. The machine-wide
/// `runs.csv` was built *inside* `render_project`, so for five months it held
/// renders and nothing else — `validate`, `new`, `add`, `voices`, `doctor`,
/// `remove` and `move` all wrote to it not at all. The failure the author
/// actually reported (D-141, a Voice screen full of Python) left no trace in
/// the file whose whole purpose is answering "what went wrong just now".
///
/// Two rows per command, not one. The `invoked` row is what says a command
/// **started and never came back** — a hang, a crash, a machine that froze —
/// which is precisely how D-144 was diagnosed: `runs.csv` ended mid-render
/// with four children started and no completion.
///
/// Silent on failure, by design, and the journal itself enforces that: a
/// command must never fail because a log could not be written (D-093).
fn run(cli: Cli) -> Result<(), String> {
    let scope = cli.command.scope();
    let journal = spoonstill_app::Journal::for_surface(cli.command.project());
    journal.record(&Event::info(scope, "command invoked"));

    let outcome = dispatch(cli.command);

    match &outcome {
        Ok(()) => journal.record(&Event::info(scope, "command finished")),
        Err(detail) => {
            journal.record(&Event::error(scope, "command failed").with("detail", detail.clone()))
        }
    }
    outcome
}

impl Command {
    /// The word this command is logged under — its own name, and stable.
    ///
    /// `&'static str` rather than something derived from `clap`, because a
    /// scope that changed when a help string was reworded would make a
    /// six-month-old `runs.csv` unfilterable.
    const fn scope(&self) -> &'static str {
        match self {
            Command::New(_) => "new",
            Command::Add(_) => "add",
            Command::Remove(_) => "remove",
            Command::Move(_) => "move",
            Command::Validate(_) => "validate",
            Command::Voices(_) => "voices",
            Command::Subtitles => "subtitles",
            Command::Resolutions => "resolutions",
            Command::Licences => "licences",
            Command::Doctor(_) => "doctor",
            Command::Render(_) => "render",
            Command::RenderScene(_) => "render-scene",
            Command::Diagnostics(_) => "diagnostics",
        }
    }

    /// The project this command is about, when it is about one.
    ///
    /// `None` for the machine-level commands — `doctor`, `voices` and the
    /// listings — whose two project columns are then left empty rather than
    /// filled with a guess.
    fn project(&self) -> Option<&std::path::Path> {
        match self {
            Command::New(a) => Some(&a.project),
            Command::Add(a) => Some(&a.project),
            Command::Remove(a) => Some(&a.project),
            Command::Move(a) => Some(&a.project),
            Command::Validate(a) => Some(&a.project),
            Command::Render(a) => Some(&a.project),
            Command::Diagnostics(
                DiagnosticsCommand::Export { project, .. } | DiagnosticsCommand::Where { project },
            ) => project.as_deref(),
            // `render-scene` renders one segment and names no folder; its own
            // log goes beside the file it writes (`state_root_for`).
            Command::RenderScene(_)
            | Command::Voices(_)
            | Command::Subtitles
            | Command::Resolutions
            | Command::Licences
            | Command::Doctor(_) => None,
        }
    }
}

fn dispatch(command: Command) -> Result<(), String> {
    match command {
        Command::New(args) => new_project(args),
        Command::Add(args) => add_media(&args.project, &args.media),
        Command::Remove(args) => remove_scenes(&args.project, &args.scenes),
        Command::Move(args) => move_scene(&args.project, &args.scene, args.to),
        Command::Validate(args) => validate(args),
        Command::Voices(args) => list_voices(&args),
        Command::Subtitles => list_subtitle_themes(),
        Command::Resolutions => list_resolutions(),
        Command::Licences => show_licences(),
        Command::Doctor(args) => doctor(&args),
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
/// `still remove DIR SCENE...` — take scenes out, keeping their files (D-100).
///
/// Several at once, and **highest first**: removing 002 renumbers everything
/// after it, so `still remove p 002 005` would otherwise take 002 and then
/// whatever happened to land on 005 afterwards — which is scene 6. Sorting
/// descending means each removal only renumbers scenes the operator has
/// already named.
fn remove_scenes(root: &std::path::Path, ids: &[String]) -> Result<(), String> {
    let mut wanted: Vec<&String> = ids.iter().collect();
    wanted.sort_by_key(|id| std::cmp::Reverse(id.trim().parse::<usize>().unwrap_or(0)));

    let mut last = None;
    for id in wanted {
        let removed = spoonstill_app::arrange::remove(root, id).map_err(|e| e.to_string())?;
        println!(
            "  removed {:<8} {} file{} moved to {}/",
            removed.id,
            removed.files,
            if removed.files == 1 { "" } else { "s" },
            spoonstill_app::arrange::REMOVED_DIR
        );
        last = Some(removed);
    }

    if let Some(removed) = last {
        println!(
            "  {} scene{} left. Nothing was deleted — {} holds what came out.",
            removed.remaining,
            if removed.remaining == 1 { "" } else { "s" },
            removed.moved_to.display()
        );
    }
    Ok(())
}

/// `still move DIR SCENE POSITION` — put a scene somewhere else in the film.
fn move_scene(root: &std::path::Path, id: &str, to: usize) -> Result<(), String> {
    let moved = spoonstill_app::arrange::move_to(root, id, to).map_err(|e| e.to_string())?;

    // What moved, and where — not the scene list, every row of which reads
    // `00N  00N.jpg` after a renumber and so confirms nothing (D-150).
    println!(
        "  {} → {}   moved from {} to {} of {}",
        moved.was,
        moved.now,
        moved.from,
        moved.to,
        moved.scenes.len()
    );
    for file in &moved.files {
        println!("           {}", name_of(file));
    }
    let renumbered = moved.from.abs_diff(moved.to);
    if renumbered > 0 {
        println!(
            "  {renumbered} scene{} between them were renumbered.",
            if renumbered == 1 { " was" } else { "s" }
        );
    }
    Ok(())
}

fn add_media(root: &std::path::Path, media: &[PathBuf]) -> Result<(), String> {
    let report = spoonstill_app::add_media(root, media).map_err(|e| e.to_string())?;

    for image in &report.images {
        // A `.txt` beside a recording is the scene's caption (D-106), and this
        // arm used to drop it — so the operator could not see that it had
        // arrived at all (D-150).
        let narration = match (report.narration_for(image), report.script_for(image)) {
            (Some(audio), Some(caption)) => format!(
                "{}  +  {} (caption)",
                name_of(&audio.source),
                name_of(&caption.source)
            ),
            (Some(audio), None) => name_of(&audio.source),
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

#[derive(Debug, Args)]
struct DoctorArgs {
    /// Install whatever is missing, rather than only reporting it.
    ///
    /// Every install is this machine's own package manager — Homebrew, winget,
    /// pipx — run because it was asked for. Nothing is downloaded from us
    /// (D-012).
    #[arg(long)]
    install: bool,
}

/// `still doctor` — every external program, checked in one place (D-105).
///
/// The window grew Install buttons for a missing FFmpeg and a missing voice
/// service, and "if the CLI cannot do it, it does not exist" means this had to
/// exist the same day. It is also the first thing to run when a report says
/// the application opened a good folder as zero scenes: that was D-103, and
/// the answer was always one of these two lines.
fn doctor(args: &DoctorArgs) -> Result<(), String> {
    let mut unresolved = 0;
    // Whether there is an FFmpeg to ask about hardware at all. Without it every
    // candidate answers "not in this ffmpeg", which is four lines of noise
    // underneath the one line that matters — D-103's defect exactly, where one
    // missing binary is reported once as itself rather than many times as
    // something else.
    let mut ffmpeg_ready = false;

    for tool in spoonstill_app::tooling::check_all() {
        if tool.ready {
            if tool.id == spoonstill_app::tooling::FFMPEG {
                ffmpeg_ready = true;
            }
            println!("  ok       {} — {}", tool.id, tool.purpose);
            // Which build, not just whether there is one (D-151). The version
            // is the first thing a report from a stranger's machine is checked
            // against, and until now no surface printed it.
            if let Some(version) = tool.version.as_deref() {
                println!("           {version}");
            }
            continue;
        }

        let remedy = tool
            .remedy
            .clone()
            .unwrap_or_else(|| spoonstill_core::Remedy::manual("not available", ""));
        println!("  missing  {} — {}", tool.id, tool.purpose);
        println!("           {}", remedy.need);
        if !remedy.detail.is_empty() {
            println!("           {}", remedy.detail);
        }

        if !args.install {
            unresolved += 1;
            if tool.is_installable() {
                println!("           try `still doctor --install`");
            }
            continue;
        }
        if !tool.is_installable() {
            unresolved += 1;
            continue;
        }

        println!("           installing…");
        match spoonstill_app::tooling::install(&tool.id) {
            Ok(ran) => println!("           {ran}"),
            Err(failed) => {
                unresolved += 1;
                println!("           {failed}");
            }
        }
    }

    if ffmpeg_ready {
        report_graphics();
    }

    if unresolved == 0 {
        return Ok(());
    }
    Err(format!(
        "{unresolved} thing{} spoonstill needs {} still missing",
        if unresolved == 1 { "" } else { "s" },
        if unresolved == 1 { "is" } else { "are" }
    ))
}

/// What hardware encoding this machine could do (D-159).
///
/// Printed by `still doctor` because "is it using my graphics card" had no
/// answer on any surface, and an operator who has one asks it first. It is
/// deliberately phrased as *what this machine has*, not as a list of things
/// that are wrong: a missing `h264_qsv` on a machine with no Intel graphics is
/// the correct pairing, so nothing here counts towards `unresolved` and nothing
/// here can fail `still doctor`.
///
/// Only called when there is an FFmpeg to ask. Without one every candidate
/// answers "not in this ffmpeg", and printing four of those under a heading
/// about the operator's graphics card buries the single line that actually
/// needs acting on.
fn report_graphics() {
    let found = spoonstill_app::tooling::hardware();
    if found.is_empty() {
        return;
    }

    println!();
    println!("  graphics — hardware encoders this machine can run");
    for accel in &found {
        let status = if accel.usable {
            "usable"
        } else if accel.present {
            "no"
        } else {
            "not in this ffmpeg"
        };
        println!("    {status:<20} {} ({})", accel.vendor, accel.encoder);
        if !accel.detail.is_empty() {
            println!("      {}", accel.detail);
        }
    }

    // Said every time, because the list above invites exactly one wrong
    // conclusion. D-036 chose libx264 on quality grounds for this content, and
    // the encoder is a fifth of a 4K render at most — 14% measured on macOS
    // (D-144) and 22.7% here, with NVENC itself worth 1.23x (D-159). So a
    // usable line above is a fact about the machine, not a setting somebody has
    // failed to turn on.
    println!("    Films render on the CPU with libx264 (D-036). The encoder is at most a");
    println!("    fifth of a 4K render, so hardware would not make one much faster (D-159).");
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

    if let spoonstill_app::tts::Availability::Missing(remedy) = provider.availability() {
        if !args.install {
            return Err(format!("{remedy}\n  try `still voices --install`"));
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
    fn check(
        &self,
        _path: &std::path::Path,
        _role: spoonstill_app::Role,
    ) -> Result<Option<spoonstill_core::SourceGeometry>, String> {
        // Nothing is measured, so nothing is known — including how big the
        // stills are, which is why `--no-probe` produces no F-13 warning.
        Ok(None)
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
/// `still subtitles` — the themes, and what each is for.
///
/// The CLI half of the window's theme chooser (D-010): if the window can show
/// it, the command line can print it. The window adds a rendered preview,
/// which a terminal cannot; the words are the same words.
fn list_subtitle_themes() -> Result<(), String> {
    let default = spoonstill_app::import::settings::DEFAULT_THEME;
    println!("Subtitle themes (D-106). Burned into the picture, not a sidecar file.\n");

    let width = SubtitleTheme::ALL
        .iter()
        .map(|t| t.as_str().len())
        .max()
        .unwrap_or(8);

    for theme in SubtitleTheme::ALL {
        let mark = if theme == default { " (default)" } else { "" };
        println!(
            "  {name:<width$}  {description}{mark}",
            name = theme.as_str(),
            description = theme.description(),
        );
    }

    println!(
        "\nTurn them on for one run:   still render DIR --subtitles boxed\n\
         Or for the project, in project.yaml:\n\
         \n  subtitles:\n    enabled: true\n    theme: boxed\n    position: bottom\n\
         \nA scene is captioned when it has words: the script it speaks, or a\n\
         `caption` column, or a .txt beside a recording."
    );
    Ok(())
}

/// `still resolutions` — every size and shape a film can come out at (D-143).
///
/// The pixel dimensions are printed per aspect rather than described, because
/// "4K portrait" is 2160x3840 and an operator who expects 3840 wide has to be
/// able to see that they do not get it.
fn list_resolutions() -> Result<(), String> {
    println!("Output sizes and shapes (D-143). An override for one run:\n");

    let aspects = spoonstill_app::formats::aspects();
    let width = aspects.iter().map(|a| a.id.len()).max().unwrap_or(4);
    for aspect in &aspects {
        let mark = if aspect.default { " (default)" } else { "" };
        println!(
            "  {id:<width$}  {description}{mark}",
            id = aspect.id,
            description = aspect.description,
        );
    }

    println!("\nSizes, and what each one is in each shape:\n");
    let parsed: Vec<Aspect> = aspects
        .iter()
        .map(|a| Aspect::parse(a.id).expect("a listed aspect parses"))
        .collect();

    let header: Vec<String> = parsed.iter().map(|a| a.as_str().to_owned()).collect();
    println!(
        "  {:<8}  {:<12}  {:<12}  {:<12}  what it is for",
        "size", header[0], header[1], header[2]
    );
    for size in spoonstill_app::formats::sizes(parsed[0]) {
        let cells: Vec<String> = parsed
            .iter()
            .map(|&aspect| {
                spoonstill_app::formats::sizes(aspect)
                    .into_iter()
                    .find(|s| s.id == size.id)
                    .map_or_else(|| "—".to_owned(), |s| s.dimensions())
            })
            .collect();
        println!(
            "  {:<8}  {:<12}  {:<12}  {:<12}  {}",
            size.id, cells[0], cells[1], cells[2], size.description
        );
    }

    println!(
        "\nAliases: {}\n",
        spoonstill_app::formats::sizes(parsed[0])
            .iter()
            .filter(|s| !s.aliases.is_empty())
            .map(|s| format!("{} = {}", s.aliases.join(", "), s.id))
            .collect::<Vec<_>>()
            .join("; ")
    );

    println!(
        "A YouTube Short, a Reel, a TikTok and a Story are all 9:16, so all four\n\
         words mean that shape:\n\
         \n  still render DIR --aspect shorts --resolution 1080p\n  \
         still render DIR --resolution 4k\n\
         \nOr for the project, in project.yaml:\n\
         \n  aspect: 9:16\n  resolution: 4k\n\
         \n`resolution` and `short_edge` are two spellings of one setting — set one.\n\
         4K is the largest this renders: past it the segment profile would have to\n\
         declare an H.264 level no decoder honours (D-114)."
    );
    Ok(())
}

fn render_project(args: RenderArgs) -> Result<(), String> {
    // `--subtitles` with no value means "on, with the project's own theme";
    // with a value it also picks the theme. `--no-subtitles` is the other
    // direction, and clap already refuses both at once.
    let (subtitles, subtitle_theme) = match (&args.subtitles, args.no_subtitles) {
        (_, true) => (Some(false), None),
        (Some(theme), _) if theme.is_empty() => (Some(true), None),
        (Some(theme), _) => {
            let parsed = SubtitleTheme::parse(theme).ok_or_else(|| {
                format!(
                    "{theme:?} is not a subtitle theme. It is one of {} —                      run `still subtitles` to see what each one looks like.",
                    SubtitleTheme::names()
                )
            })?;
            (Some(true), Some(parsed))
        }
        (None, false) => (None, None),
    };

    let subtitle_placement =
        match &args.subtitle_position {
            None => None,
            Some(edge) => Some(Placement::parse(edge).ok_or_else(|| {
                format!("{edge:?} is not a subtitle position. It is bottom or top.")
            })?),
        };

    let defaults = spoonstill_app::RenderProjectOptions::for_project(&args.project);
    let options = spoonstill_app::RenderProjectOptions {
        out: args.out,
        // Passed through rather than resolved here: only `render_project`
        // knows this run's geometry, and the geometry decides what a worker
        // costs (D-144). `None` means "you decide".
        jobs: args.jobs.map(|jobs| jobs.max(1)),
        audio_jobs: args.audio_jobs.unwrap_or(defaults.audio_jobs).max(1),
        force: args.force,
        keep_cache: args.keep_cache,
        voice: args.voice.clone(),
        provider: args.provider.clone(),
        subtitles,
        subtitle_theme,
        subtitle_placement,
        aspect: args.aspect,
        // Two spellings, one setting: clap has already refused both at once,
        // so there is no precedence to get wrong here (D-143).
        short_edge: match &args.resolution {
            Some(name) => Some(spoonstill_app::formats::parse_size(name)?),
            None => args.short_edge,
        },
        fps: args.fps,
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
            per_worker,
            limited_by_memory,
        } => {
            total.set(scenes);
            done.set(0);
            println!(
                "  {scenes} scene{}, {jobs} at a time ({audio_jobs} for audio)",
                if scenes == 1 { "" } else { "s" }
            );
            // Only when memory, not the core count, chose the number — an
            // operator who gets fewer workers than their machine has cores
            // deserves to know it was deliberate (D-144).
            if limited_by_memory {
                println!(
                    "  {} per worker at this size, so the pool is smaller than this machine's cores",
                    spoonstill_app::capacity::gigabytes(per_worker)
                );
            }
        }
        FilmEvent::MemoryPressure(pressure) => {
            // Before any worker starts. The failure this warns about is a
            // machine that stops responding, and nothing printed afterwards
            // would be read (D-144).
            eprintln!(
                "  warning: {} workers at this size need about {}, and this machine \
                 should spare about {}.",
                pressure.jobs,
                spoonstill_app::capacity::gigabytes(pressure.needed),
                spoonstill_app::capacity::gigabytes(pressure.budget)
            );
            if pressure.fits >= 1 {
                eprintln!("  try --jobs {} instead.", pressure.fits);
            } else {
                eprintln!(
                    "  even one worker exceeds it — render at a smaller \
                     --resolution, or close other applications."
                );
            }
        }
        FilmEvent::Warned { detail } => {
            // Before the pool, and on stderr beside the memory warning: this
            // is everything `still validate` prints that does not stop a run,
            // and `still render` used to print none of it (F-13).
            eprintln!("  warning: {detail}");
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
            // "done", because the pool finishes scenes in whatever order
            // workers free up and this counter is a *count*, not a position.
            // Without the word, `[  1/4] 004` reads as "004 is the first scene
            // in the film" — which is what D-091 fixed in the window, where
            // logging completion order "read as a scrambled film". The CLI is
            // the permanent complete control surface (D-091 says so itself) and
            // kept the same misreading until somebody rendered a real project
            // and read the output as a stranger would.
            println!(
                "  [{:>3}/{} done] {id:<12} {frames:>6} frames {duration:>8.3}s{}",
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
    if film.freed_bytes > 0 {
        println!(
            "  swept {} of superseded segments and normalized audio \
             (--keep-cache keeps them)",
            human_bytes(film.freed_bytes)
        );
    }
    Ok(())
}

/// Bytes as an operator reads them. Three significant figures is enough to
/// answer "was that worth doing" and no more precision means anything here.
fn human_bytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let mut n = bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if n < 1024.0 || unit == "GB" {
            return if unit == "B" {
                format!("{bytes} B")
            } else {
                format!("{n:.1} {unit}")
            };
        }
        n /= 1024.0;
    }
    unreachable!()
}

/// Everything third-party that is built into this binary (D-124).
///
/// The notices are `include_str!`'d rather than read from beside the
/// executable, so **every copy of the binary contains them** however it was
/// obtained — extracted from a release archive, copied off another machine, or
/// built here. That is the form the SIL Open Font License itself allows for
/// embedded material: *"machine-readable metadata fields within text or binary
/// files as long as those fields can be easily viewed by the user"*. A command
/// that prints them is as easily viewed as it gets.
fn show_licences() -> Result<(), String> {
    print!("{}", include_str!("../../../THIRD-PARTY-NOTICES.md"));
    Ok(())
}

fn render_scene(args: RenderSceneArgs) -> Result<(), String> {
    let options = RenderSceneOptions {
        image: args.image,
        audio: args.audio,
        out: args.out,
        aspect: args.aspect,
        short_edge: match &args.resolution {
            Some(name) => spoonstill_app::formats::parse_size(name)?,
            None => args.short_edge,
        },
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

    // The project's own log and its status, together (D-150). This block used
    // to sit *after* the machine-wide line below, so `(not created yet)` read
    // as belonging to `runs.csv` — a file the very same line had just reported
    // the size of.
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
        println!("  (not created yet — it appears the first time this folder is used)");
    }

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
                human_bytes(size)
            }
        );
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

    /// D-143. Both control surfaces have to be able to ask for 2K, 4K and a
    /// Short, and `still render` had none of the three — geometry was
    /// reachable only from `project.yaml` and from `render-scene`, which
    /// renders one segment.
    #[test]
    fn a_whole_project_can_be_rendered_at_another_size_and_shape() {
        let args = Cli::try_parse_from([
            "still",
            "render",
            "proj",
            "--aspect",
            "shorts",
            "--resolution",
            "4k",
        ])
        .expect("parses");
        let Command::Render(args) = args.command else {
            panic!("not a render")
        };
        assert_eq!(args.aspect, Some(Aspect::Portrait9x16));
        assert_eq!(args.resolution.as_deref(), Some("4k"));
        assert_eq!(
            spoonstill_app::formats::parse_size("4k"),
            Ok(2160),
            "a 4K Short is 2160x3840"
        );
    }

    /// Two spellings of one setting, so clap refuses both rather than letting
    /// one silently win — on both commands that take geometry.
    #[test]
    fn a_size_cannot_be_named_twice() {
        for command in [
            vec![
                "still",
                "render",
                "proj",
                "--resolution",
                "4k",
                "--short-edge",
                "1080",
            ],
            vec![
                "still",
                "render-scene",
                "--image",
                "a.jpg",
                "--audio",
                "a.wav",
                "--out",
                "o.mp4",
                "--resolution",
                "4k",
                "--short-edge",
                "1080",
            ],
        ] {
            let error = Cli::try_parse_from(command.clone()).expect_err("refused");
            assert_eq!(
                error.kind(),
                clap::error::ErrorKind::ArgumentConflict,
                "{command:?}"
            );
        }
    }

    /// A render with no geometry flags leaves the project's own answer alone —
    /// this is an override, not a default that quietly overwrites (D-013).
    #[test]
    fn a_render_with_no_geometry_flags_overrides_nothing() {
        let args = Cli::try_parse_from(["still", "render", "proj"]).expect("parses");
        let Command::Render(args) = args.command else {
            panic!("not a render")
        };
        assert_eq!(
            (args.aspect, args.resolution, args.short_edge, args.fps),
            (None, None, None, None)
        );
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
