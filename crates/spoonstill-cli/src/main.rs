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
    /// Render a single scene to one segment.
    RenderScene(RenderSceneArgs),
    /// Diagnostics: logs and the bundle to send when something goes wrong.
    #[command(subcommand)]
    Diagnostics(DiagnosticsCommand),
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
        Command::RenderScene(args) => render_scene(args),
        Command::Diagnostics(DiagnosticsCommand::Export { out, project }) => {
            export_diagnostics(out, project)
        }
        Command::Diagnostics(DiagnosticsCommand::Where { project }) => show_log_location(project),
    }
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
