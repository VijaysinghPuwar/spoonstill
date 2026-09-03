//! Rendering a whole project: the film (D-040, D-042, D-044).
//!
//! `still render DIR` is this function. It is M1's single scene, multiplied
//! and ordered:
//!
//! ```text
//! import        every problem at once; nothing renders if any of them is an error
//! lock          one render per project at a time
//! audio pool    every scene resolved to (normalized path, measured duration)
//! render pool   every scene rendered to a validated segment, N at a time
//! concat        stream copy, then assert the film's own profile and length
//! ```
//!
//! ## Two pools, not one (D-044)
//!
//! The audio phase is I/O — and at slice 4 it becomes network I/O against a
//! rate-limited API. The render phase is CPU and memory: a 1080p segment peaks
//! near 780 MB (`ffmpeg-findings.md` §10b). Sizing those from one number would
//! mean either starving the network queue or exhausting memory, so they have
//! independent caps and run as separate phases.
//!
//! ## Concurrency changes the timing and nothing else
//!
//! Every scene's move is seeded from stable identity *before* the pool starts
//! (D-035), each worker writes to its own content-addressed segment path, and
//! results are collected in scene order by [`crate::pool`]. So `--jobs 8` and
//! `--jobs 1` produce the same film, byte for byte. That is asserted by the M2
//! gates rather than assumed.
//!
//! ## A failed scene does not discard the finished ones
//!
//! D-042: segments live in `.spoonstill/segments/`, named by content, and each
//! one only gets that name after passing the profile assertion. A run that
//! fails at scene 147 leaves 146 valid segments, and the next run reuses them
//! rather than re-encoding. The state *database* that makes this a real resume
//! story is M3; the content-addressed directory is what M2 can offer without
//! it, and it is enough to make a re-run cheap.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use spoonstill_core::captions::{self, Placement, SubtitleSpec, SubtitleTheme};
use spoonstill_core::diagnostics::{Diagnostics, Event};
use spoonstill_core::project::MotionRequest;
use spoonstill_core::{Aspect, MotionSpec, OutputSpec, STATE_DIR, hash, timing};
use spoonstill_media::scene::{Cancel, EncodeSettings, SceneRequest};
use spoonstill_media::{MediaError, Tools, concat, profile};
use spoonstill_state::FileLog;

use crate::audio::{AudioCache, AudioError, ResolvedAudio};
use crate::capacity;
use crate::import::{ImportError, ProbeCheck, Project, ResolvedScene};
use crate::pool::{self, Outcome};

/// Where validated segments live, under [`STATE_DIR`].
pub const SEGMENTS_DIR: &str = "segments";

/// The lock file that keeps two renders of one project apart.
pub const LOCK_FILE: &str = "render.lock";

/// What to render, and how hard.
#[derive(Debug, Clone)]
pub struct RenderProjectOptions {
    /// The project folder.
    pub root: PathBuf,
    /// Where the film goes. `None` means the project's own `output` setting.
    pub out: Option<PathBuf>,
    /// Segments rendered at once (D-044). `None` derives it from the machine
    /// and this run's geometry (D-144).
    ///
    /// An override, like [`Self::aspect`] and for a sharper reason: the cost of
    /// a worker is decided by the output size, and the output size is not known
    /// until [`apply_geometry_override`] has run. A number fixed at
    /// construction is a number chosen before the question was asked — which is
    /// how four 4K workers came to be the default on a machine that could
    /// afford two.
    pub jobs: Option<usize>,
    /// Narrations resolved at once (D-044).
    ///
    /// Not geometry-derived: ingest normalization is short and I/O-bound, and
    /// holds no canvas. D-144 is about the segment pool only.
    pub audio_jobs: usize,
    /// Take the lock even if another run appears to hold it.
    pub force: bool,
    /// Speak every spoken scene in this voice instead of the project's own.
    ///
    /// `project.yaml` is an input and the renderer never writes to it (D-013),
    /// so a voice chosen in the window — or on the command line — is an
    /// override for **this run**, not a change to the project. An operator who
    /// wants it to stick writes it into `project.yaml` themselves, which is
    /// also the only way it survives into someone else's checkout.
    pub voice: Option<String>,
    /// The same, for the provider.
    pub provider: Option<String>,
    /// Burn subtitles for this run, whatever `project.yaml` says (D-106).
    ///
    /// An override for one run, exactly like [`Self::voice`] and for the same
    /// reason: `project.yaml` is an input and the renderer never writes to it
    /// (D-013). `None` means "whatever the project decided".
    pub subtitles: Option<bool>,
    /// The same, for which look.
    pub subtitle_theme: Option<SubtitleTheme>,
    /// The same, for which edge they sit against.
    ///
    /// Worth overriding per run more than the others: whether a caption
    /// collides with words already in the artwork is a fact about *these*
    /// photographs, and the answer can change between one batch and the next.
    pub subtitle_placement: Option<Placement>,
    /// Keep every segment this render did not use, instead of sweeping the
    /// oldest generations (D-109).
    ///
    /// Off by default, because a project that is iterated on accumulates one
    /// dead generation per render and nothing ever removed them. On for an
    /// operator who would rather spend the disk than ever re-encode.
    pub keep_cache: bool,
    /// Render this run in a different aspect than `project.yaml` asks for
    /// (D-143).
    ///
    /// An override for one run, like [`Self::voice`] and for the same reason:
    /// `project.yaml` is an input and the renderer never writes to it (D-013).
    /// The same folder of photographs is a landscape film and a YouTube Short
    /// on two consecutive runs, and neither run edits the project.
    pub aspect: Option<Aspect>,
    /// The same, for the output's short edge — 1440 for 2K, 2160 for 4K.
    ///
    /// A number rather than a [`spoonstill_core::Resolution`], because the
    /// name is a surface convenience and this is the layer below it: both
    /// control surfaces resolve a name to its short edge before they get here,
    /// and a project that sets `short_edge: 900` is still overridable.
    pub short_edge: Option<u32>,
    /// The same, for the frame rate.
    pub fps: Option<u32>,
}

impl RenderProjectOptions {
    /// Defaults for a project folder: the project's own output setting, and a
    /// worker count derived from the machine.
    #[must_use]
    pub fn for_project(root: impl Into<PathBuf>) -> Self {
        RenderProjectOptions {
            root: root.into(),
            out: None,
            // Left undecided here on purpose: `render_project` resolves it
            // once the geometry override has been applied (D-144).
            jobs: None,
            // Ingest normalization is a short, I/O-bound FFmpeg run, so it can
            // afford more workers than the encoder can. At slice 4 the TTS
            // provider's own rate limit becomes the binding constraint on this
            // number (D-023) — which is exactly why it is a separate one.
            audio_jobs: pool::default_jobs().saturating_mul(2).max(1),
            force: false,
            voice: None,
            provider: None,
            subtitles: None,
            subtitle_theme: None,
            subtitle_placement: None,
            keep_cache: false,
            aspect: None,
            short_edge: None,
            fps: None,
        }
    }
}

/// What happened, as it happens. Called from worker threads, hence `Sync`.
#[derive(Debug, Clone)]
pub enum FilmEvent {
    /// The plan, before any work starts.
    Planned {
        /// How many scenes will render.
        scenes: usize,
        /// Segment workers.
        jobs: usize,
        /// Audio workers.
        audio_jobs: usize,
        /// Estimated memory for one segment worker at this geometry (D-144).
        per_worker: u64,
        /// Whether memory, rather than the core count, chose `jobs`.
        ///
        /// Only ever true when the operator did not name a number themselves:
        /// an explicit `--jobs` is obeyed and warned about, never quietly
        /// lowered (D-076 — the flag is not capped in either direction).
        limited_by_memory: bool,
    },
    /// This run is planning to use more memory than the machine should give it
    /// (D-144).
    ///
    /// Emitted before any worker starts, and only when the operator asked for
    /// the count themselves — the automatic count already respects the budget.
    MemoryPressure(crate::capacity::Pressure),
    /// One scene's narration is resolved.
    Audio {
        /// Position in render order.
        index: usize,
        /// The operator's scene id.
        id: String,
        /// What the source was: `tts`, `file` or `silent`.
        kind: &'static str,
        /// Measured duration (D-021).
        duration: f64,
        /// Whether the cache already held it (D-043).
        reused: bool,
    },
    /// One scene's segment is finished and validated.
    Segment {
        /// Position in render order.
        index: usize,
        /// The operator's scene id.
        id: String,
        /// Frame count, asserted against the file.
        frames: u32,
        /// Segment duration.
        duration: f64,
        /// Whether a validated segment was already on disk.
        reused: bool,
    },
    /// A scene failed. The run continues so that every failure is reported.
    Failed {
        /// Position in render order.
        index: usize,
        /// The operator's scene id.
        id: String,
        /// What went wrong, already phrased for an operator.
        detail: String,
    },
    /// Every segment is valid; the join is starting.
    Joining {
        /// How many segments.
        segments: usize,
    },
}

/// One scene that did not render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneFailure {
    /// Position in render order.
    pub index: usize,
    /// The operator's scene id.
    pub id: String,
    /// What went wrong.
    pub detail: String,
}

impl std::fmt::Display for SceneFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "scene {}: {}", self.id, self.detail)
    }
}

/// The finished film.
#[derive(Debug, Clone)]
pub struct RenderedFilm {
    /// Where it is.
    pub path: PathBuf,
    /// Its measured duration.
    pub duration: f64,
    /// The duration computed from the scenes, which it was checked against.
    pub expected_duration: f64,
    /// How many scenes it holds.
    pub scenes: usize,
    /// Total frames across the scenes.
    pub frames: u64,
    /// Narrations that came from the cache (D-043).
    pub reused_audio: usize,
    /// Segments that were already rendered and still valid (D-042).
    pub reused_segments: usize,
    /// Where the segments are, for a re-run and for diagnostics.
    pub segments_dir: PathBuf,
    /// Bytes of superseded segments swept after the join (D-109). Zero when
    /// there was nothing to sweep, and when `keep_cache` asked us not to.
    pub freed_bytes: u64,
}

/// Why a project did not render.
#[derive(Debug)]
pub enum FilmError {
    /// The folder could not be read as a project at all.
    Import(ImportError),
    /// Validation found errors. They have already been reported; this is the
    /// refusal to render on top of them.
    HasProblems {
        /// How many stop the render.
        errors: usize,
    },
    /// A project with nothing to render.
    NoScenes,
    /// Another render holds this project.
    Locked {
        /// The lock file.
        path: PathBuf,
        /// What it says about its owner.
        held_by: String,
        /// Whether the operator passed `--force`, so the message can say why
        /// it did not help (D-113).
        forced: bool,
    },
    /// The project's `output` setting points outside the project (D-054).
    OutputOutsideProject {
        /// The setting, as written.
        value: String,
    },
    /// A geometry override for this run is not one this renders (D-143).
    ///
    /// Its own variant rather than a [`Problem`](spoonstill_core::Problem),
    /// because it is not a fact about the project: `project.yaml` may be
    /// perfectly valid and the flag wrong, and blaming the folder for a
    /// command-line typo sends the operator to the wrong file.
    UnusableGeometry {
        /// The refusal, already phrased as a sentence by `GeometryError`.
        detail: String,
    },
    /// The voice service is not usable, and scenes need it (D-002, D-094).
    ///
    /// Found before the pool starts, not at scene 340 of 500.
    VoiceService {
        /// Which provider the project asked for.
        provider: String,
        /// The sentence naming the fix.
        detail: String,
        /// How many scenes were going to need it.
        scenes: usize,
    },
    /// One or more narrations could not be resolved.
    Audio {
        /// Every failure, in render order.
        failures: Vec<SceneFailure>,
    },
    /// One or more segments could not be rendered.
    Render {
        /// Every failure, in render order.
        failures: Vec<SceneFailure>,
    },
    /// The run was cancelled (D-045).
    Cancelled,
    /// The join, or a probe of the film, failed.
    Media(Box<MediaError>),
    /// Filesystem trouble, with the path attached.
    Io {
        /// What we were doing.
        doing: &'static str,
        /// The path.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for FilmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilmError::Import(e) => write!(f, "{e}"),
            FilmError::HasProblems { errors } => write!(
                f,
                "{errors} problem{} stop{} this render — fix them, or run `still validate` \
                 to see them again",
                if *errors == 1 { "" } else { "s" },
                if *errors == 1 { "s" } else { "" }
            ),
            FilmError::NoScenes => f.write_str("there are no scenes to render"),
            FilmError::Locked {
                path,
                held_by,
                forced,
            } => {
                write!(f, "another render is working on this project ({held_by})")?;
                if *forced {
                    // The lock is the operating system's now (D-113), so this
                    // is not a leftover file: some process really is holding
                    // it. Overriding that is what produced three concurrent
                    // renders of one project.
                    f.write_str(
                        "\n--force cannot take a lock a running render holds. \
                         Wait for it, or stop it.",
                    )
                } else {
                    write!(
                        f,
                        "\nWait for it to finish. A run this machine lost releases \
                         its lock by itself, so {} is never left holding one.",
                        path.display()
                    )
                }
            }
            FilmError::OutputOutsideProject { value } => write!(
                f,
                "the project's output setting {value:?} points outside the project folder \
                 (D-054). Pass --out to write somewhere else deliberately."
            ),
            FilmError::UnusableGeometry { detail } => write!(
                f,
                "this run asked for geometry that will not render: {detail}\n\
                 `still resolutions` lists the sizes and aspects that will."
            ),
            FilmError::VoiceService {
                provider,
                detail,
                scenes,
            } => write!(
                f,
                "{scenes} scene{} need{} the {provider} voice service, and it is not \
                 usable: {detail}\n\
                 Nothing was rendered. This is asked before the first scene rather than \
                 discovered at the last one (D-002).",
                plural(*scenes),
                if *scenes == 1 { "s" } else { "" }
            ),
            FilmError::Audio { failures } => {
                write!(
                    f,
                    "{} narration{} could not be resolved:",
                    failures.len(),
                    plural(failures.len())
                )?;
                for failure in failures {
                    write!(f, "\n  {failure}")?;
                }
                Ok(())
            }
            FilmError::Render { failures } => {
                write!(
                    f,
                    "{} scene{} failed to render:",
                    failures.len(),
                    plural(failures.len())
                )?;
                for failure in failures {
                    write!(f, "\n  {failure}")?;
                }
                Ok(())
            }
            FilmError::Cancelled => f.write_str(
                "cancelled — finished segments are kept, so the next run resumes from them",
            ),
            FilmError::Media(e) => write!(f, "{e}"),
            FilmError::Io {
                doing,
                path,
                source,
            } => {
                write!(f, "{doing} {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for FilmError {}

impl From<MediaError> for FilmError {
    fn from(e: MediaError) -> Self {
        FilmError::Media(Box::new(e))
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// Render a project folder into one film.
///
/// `on_event` is called from worker threads and must be safe to call from
/// several at once — which is why it is `Sync` rather than `FnMut`. The CLI
/// wraps a line printer in a mutex; the Tauri shell at M4 will push into a
/// channel.
///
/// # Errors
///
/// [`FilmError`]. Every scene-level failure is collected and reported
/// together, because an operator with 500 scenes cannot fix one per run
/// (D-002, D-055).
pub fn render_project(
    options: &RenderProjectOptions,
    cancel: &Cancel,
    on_event: &(dyn Fn(FilmEvent) + Sync),
) -> Result<RenderedFilm, FilmError> {
    let mut project =
        crate::import::load(&options.root, &ProbeCheck::from_env()).map_err(FilmError::Import)?;
    apply_voice_override(&mut project, options);
    apply_geometry_override(&mut project, options)?;

    let errors = project.errors().count();
    if errors > 0 {
        return Err(FilmError::HasProblems { errors });
    }
    if project.scenes.is_empty() {
        return Err(FilmError::NoScenes);
    }

    // The geometry is final, so the cost of a worker is knowable and the pool
    // can be sized against it (D-144). This is the earliest point it could be:
    // `apply_geometry_override` decided the output spec four lines up.
    let capacity = capacity::plan(project.settings.output_spec);
    let jobs = options.jobs.map_or(capacity.jobs, |asked| asked.max(1));
    let audio_jobs = options.audio_jobs.max(1);
    let pressure = capacity::pressure(project.settings.output_spec, jobs);

    let out = destination(&project, options)?;

    // The log is opened before the work, not after a failure (D-016). Two
    // sinks, not one: the project's own JSON Lines, which is the authority and
    // travels with the folder, and the machine's CSV, which is the only place
    // that can answer "what went wrong just now" without already knowing which
    // folder to look in (D-093). Either may be absent; neither may fail a
    // render.
    let log = FileLog::open(&project.root).ok();
    let index = spoonstill_state::ActivityLog::for_project(&project.root);
    let both = log
        .as_ref()
        .zip(index.as_ref())
        .map(|(l, i)| spoonstill_state::Tee(l as &dyn Diagnostics, i as &dyn Diagnostics));
    let sink: &dyn Diagnostics = match (&both, &log, &index) {
        (Some(tee), _, _) => tee,
        (None, Some(l), _) => l,
        (None, None, Some(i)) => i,
        _ => &spoonstill_core::diagnostics::Noop,
    };

    let _lock = Lock::take(&project.root, options.force)?;

    sink.record(
        &Event::info("render", "project invoked")
            .with("root", project.root.display().to_string())
            .with("out", out.display().to_string())
            .with("scenes", project.scenes.len().to_string())
            .with("jobs", jobs.to_string())
            .with("audio_jobs", audio_jobs.to_string())
            .with(
                "geometry",
                format!(
                    "{}x{}@{}",
                    project.settings.output_spec.width(),
                    project.settings.output_spec.height(),
                    project.settings.output_spec.fps()
                ),
            )
            .with("worker_memory", capacity::gigabytes(capacity.per_worker)),
    );

    on_event(FilmEvent::Planned {
        scenes: project.scenes.len(),
        jobs,
        audio_jobs,
        per_worker: capacity.per_worker,
        limited_by_memory: options.jobs.is_none() && capacity.limited_by_memory(),
    });

    // Said before the pool starts, because the thing it warns about is a
    // machine that stops responding — an operator cannot read a warning that
    // arrives after their machine has frozen (D-144).
    if let Some(pressure) = pressure {
        sink.record(
            &Event::warn("render", "this render may exhaust memory")
                .with("jobs", pressure.jobs.to_string())
                .with("needed", capacity::gigabytes(pressure.needed))
                .with("budget", capacity::gigabytes(pressure.budget))
                .with("fits", pressure.fits.to_string()),
        );
        on_event(FilmEvent::MemoryPressure(pressure));
    }

    let tools = Tools::from_env();
    let output = project.settings.output_spec;
    let encode = EncodeSettings {
        preset: project.settings.preset.clone(),
        crf: project.settings.crf,
    };

    // Phase one: every narration, N at a time.
    let audio = resolve_audio(&project, audio_jobs, &tools, cancel, sink, on_event)?;

    // Phase two: every segment, N at a time. A different N.
    let segments_dir = project.root.join(STATE_DIR).join(SEGMENTS_DIR);
    std::fs::create_dir_all(&segments_dir).map_err(|source| FilmError::Io {
        doing: "creating the segment directory",
        path: segments_dir.clone(),
        source,
    })?;

    let rendered = render_segments(
        &project,
        &audio,
        options,
        jobs,
        &segments_dir,
        output,
        &encode,
        &tools,
        cancel,
        sink,
        on_event,
    )?;

    // Phase three: the join. Only reached when every segment above passed its
    // own profile assertion (D-040).
    on_event(FilmEvent::Joining {
        segments: rendered.len(),
    });

    let paths: Vec<PathBuf> = rendered.iter().map(|s| s.path.clone()).collect();
    let frames: u64 = rendered.iter().map(|s| u64::from(s.frames)).sum();
    // Computed from the asserted frame counts, never measured back off the
    // segments: the film is checked against what the scenes *are*, and a
    // segment that drifted would already have failed its own assertion.
    let expected_total: f64 = rendered
        .iter()
        .map(|s| timing::duration_for_frames(s.frames, output.fps()))
        .sum();

    let film = concat::concat(
        &tools,
        &paths,
        &out,
        &concat::Expectation {
            profile: &profile::SegmentProfile::for_output(output),
            frames,
            duration: expected_total,
            fps: output.fps(),
        },
        sink,
    )?;

    // The film is made and asserted. Only now is a superseded segment safe to
    // sweep — a failed or cancelled render leaves the whole cache alone, so
    // the next attempt is as fast as this one would have been (D-109).
    let freed = if options.keep_cache {
        0
    } else {
        prune_segments(&segments_dir, &paths, sink)
    };

    sink.record(
        &Event::info("render", "film complete")
            .with("out", film.path.display().to_string())
            .with("duration_s", format!("{:.6}", film.duration))
            .with("scenes", rendered.len().to_string())
            .with("frames", frames.to_string()),
    );

    Ok(RenderedFilm {
        path: film.path,
        duration: film.duration,
        expected_duration: expected_total,
        scenes: rendered.len(),
        frames,
        reused_audio: audio.iter().filter(|a| a.reused).count(),
        reused_segments: rendered.iter().filter(|s| s.reused).count(),
        segments_dir,
        freed_bytes: freed,
    })
}

/// How many superseded generations of segments to keep beside the live one.
///
/// Not zero, and the reason is the operator's actual working loop: choosing
/// between subtitle themes, or between two voices, means rendering A, then B,
/// then A again. Keeping only what the last film used would make every one of
/// those flips a full re-encode — at 200 scenes, seven and a half minutes to
/// see a theme you already rendered. Two spares makes flipping between three
/// answers free and still bounds the directory at three times the film.
const SPARE_GENERATIONS: usize = 2;

/// A temporary abandoned by a render that did not finish (D-115).
///
/// `atomic::partial_path` writes `.<name>.partial-<pid>-<n>.<ext>` beside the
/// artifact and renames it into place on success, so a run that is killed —
/// the window closed mid-encode, a crash, a power cut — leaves these behind and
/// **nothing ever removed them**. They are not segments, so D-109's sweep did
/// not see them either: it matches only names that earned their place.
///
/// Safe to delete at the point the sweep runs because the render lock is held
/// for the whole run and is exclusive per project (D-113): no other render of
/// this project can exist, and this run's own temporaries have all been renamed
/// away by the time the film is joined. So anything still called `.partial-` is
/// litter from a run that is gone.
fn is_abandoned_partial(name: &str) -> bool {
    name.starts_with('.') && name.contains(".partial-")
}

/// Is this a file we made, and are therefore entitled to delete?
///
/// The pattern `render_segments` writes, and nothing else: `seg-`, four digits,
/// `-`, sixteen hex, `.mp4`. A sweep that deleted by extension would eventually
/// meet an operator who put something in this folder, and a cache is not a
/// licence to delete a stranger's file.
fn is_our_segment(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("seg-") else {
        return false;
    };
    let Some(rest) = rest.strip_suffix(".mp4") else {
        return false;
    };
    let Some((index, key)) = rest.split_once('-') else {
        return false;
    };
    index.len() == 4
        && index.bytes().all(|b| b.is_ascii_digit())
        && key.len() == 16
        && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Sweep superseded segments, keeping the live set and `SPARE_GENERATIONS`
/// more of the rest, newest first (D-109).
///
/// Returns the bytes reclaimed. **Never fails a render**: a cache that cannot
/// be tidied is not a reason to withhold a film that is already made and
/// already asserted, so every error here is logged and stepped over.
fn prune_segments(dir: &Path, live: &[PathBuf], log: &dyn Diagnostics) -> u64 {
    let keep: std::collections::HashSet<&std::ffi::OsStr> =
        live.iter().filter_map(|p| p.file_name()).collect();

    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };

    let mut freed = 0;
    let mut swept = 0;

    // (modified, size, path) for everything we made and did not just use.
    let mut dead: Vec<(std::time::SystemTime, u64, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if keep.contains(name.as_os_str()) {
            continue;
        }
        let Some(name) = name.to_str() else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }

        // Litter from a run that never finished. Removed outright rather than
        // kept as a spare: a partial is not a segment and can never be reused.
        if is_abandoned_partial(name) {
            if std::fs::remove_file(&path).is_ok() {
                freed += meta.len();
                swept += 1;
            }
            continue;
        }

        if !is_our_segment(name) {
            continue;
        }
        let when = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        dead.push((when, meta.len(), path));
    }

    // Newest first, so the spares kept are the generations most likely to be
    // flipped back to.
    dead.sort_unstable_by(|a, b| b.0.cmp(&a.0));

    let spare = live.len().saturating_mul(SPARE_GENERATIONS);
    for (_, size, path) in dead.into_iter().skip(spare) {
        if std::fs::remove_file(&path).is_ok() {
            freed += size;
            swept += 1;
        }
    }

    if swept > 0 {
        log.record(
            &Event::info("render", "swept superseded segments")
                .with("files", swept.to_string())
                .with("bytes", freed.to_string())
                .with("kept_spare", spare.to_string()),
        );
    }
    freed
}

/// Where the film goes, and whether we are allowed to write it there.
///
/// An explicit `--out` is an argument the operator typed and is honoured
/// wherever it points. The `output:` setting is *manifest data* and is held to
/// D-054 like every other path in the file — a project that renders itself
/// into `../../etc` is the thing containment exists to prevent.
/// Point every spoken scene at the voice this run asked for.
///
/// Applied to the loaded model rather than to the file, and applied before the
/// cache key is computed — so switching voices is a cache miss, as it must be,
/// and switching back is a hit (D-043).
fn apply_voice_override(project: &mut crate::import::Project, options: &RenderProjectOptions) {
    if options.voice.is_none() && options.provider.is_none() {
        return;
    }
    for scene in &mut project.scenes {
        if let spoonstill_core::AudioSource::Tts {
            provider, voice, ..
        } = &mut scene.spec.source
        {
            if let Some(chosen) = &options.voice {
                *voice = spoonstill_core::project::VoiceId(chosen.clone());
            }
            if let Some(chosen) = &options.provider {
                *provider = spoonstill_core::project::ProviderId(chosen.clone());
            }
        }
    }
}

/// Replace the project's geometry with this run's, if it named one (D-143).
///
/// Mutating `project.settings.output_spec` rather than threading a second spec
/// through the render is deliberate: the spec is read in five places — the
/// segment key, the filter graph, the caption rasterizer, the profile
/// assertion, and the film's own assertion — and two of them disagreeing is a
/// segment that is cached under one geometry and asserted against another.
/// There is one spec, and this is where it is decided.
///
/// # Errors
///
/// [`FilmError::UnusableGeometry`] when the requested combination is not one
/// [`OutputSpec::new`] accepts. Every rule is that constructor's; nothing is
/// re-derived here (D-114).
fn apply_geometry_override(
    project: &mut crate::import::Project,
    options: &RenderProjectOptions,
) -> Result<(), FilmError> {
    if options.aspect.is_none() && options.short_edge.is_none() && options.fps.is_none() {
        return Ok(());
    }
    let current = project.settings.output_spec;
    let aspect = options.aspect.unwrap_or_else(|| current.aspect());
    let short_edge = options.short_edge.unwrap_or_else(|| current.short_edge());
    let fps = options.fps.unwrap_or_else(|| current.fps());
    project.settings.output_spec =
        OutputSpec::new(aspect, short_edge, fps).map_err(|error| FilmError::UnusableGeometry {
            detail: format!("{short_edge} at {aspect}, {fps} fps — {error}"),
        })?;
    Ok(())
}

/// Where this run's film will be written.
///
/// Public because the window has to be able to *show* the operator the file
/// before they press Render — "no option to see where it saves" was a real
/// complaint, and the honest answer is one the renderer already computes.
/// Resolving it in two places would be two answers; this is the one.
///
/// # Errors
///
/// [`FilmError::OutputOutsideProject`] when `project.yaml`'s own `output`
/// setting escapes the project root and no explicit `out` overrides it.
pub fn destination(
    project: &Project,
    options: &RenderProjectOptions,
) -> Result<PathBuf, FilmError> {
    if let Some(out) = &options.out {
        return Ok(out.clone());
    }

    let relative = &project.settings.output;

    // Held to D-054 like every other path in the file, and by the same
    // function (D-112). The lexical test this used to be — reject an absolute
    // path, reject `..` — cannot see a symlink, so a project carrying
    // `escape -> /tmp` and `output: escape/film.mp4` wrote its film outside the
    // folder and reported success.
    spoonstill_core::path_safety::resolve_destination_within(
        &project.root,
        relative,
        &crate::import::StdFs,
    )
    .map_err(|_| FilmError::OutputOutsideProject {
        value: relative.display().to_string(),
    })
}

/// Ask every voice service this run will actually call whether it works, once.
///
/// "Actually call" is doing the work: a line already in the speech cache needs
/// no service, so a re-render of a finished project on a machine that has since
/// lost `edge-tts` still renders. That is the same rule the cache follows
/// everywhere else — the question is never "could this need work", it is
/// "does this need work".
fn check_voice_service(
    project: &Project,
    cache: &AudioCache,
    log: &dyn Diagnostics,
) -> Result<(), FilmError> {
    // Ordered by first appearance rather than sorted: with one provider it
    // makes no difference, and with two the operator reads about theirs in the
    // order their project names them.
    let mut wanted: Vec<(String, usize)> = Vec::new();
    for scene in &project.scenes {
        let Some(id) = crate::audio::provider_needed(cache, &scene.spec.source) else {
            continue;
        };
        match wanted.iter_mut().find(|(name, _)| name == id) {
            Some((_, count)) => *count += 1,
            None => wanted.push((id.to_owned(), 1)),
        }
    }

    for (id, scenes) in wanted {
        let engine = match crate::tts::provider(&id) {
            Ok(engine) => engine,
            Err(e) => {
                return Err(FilmError::VoiceService {
                    provider: id,
                    detail: e.to_string(),
                    scenes,
                });
            }
        };
        if let crate::tts::Availability::Missing(remedy) = engine.availability() {
            return Err(FilmError::VoiceService {
                provider: id,
                detail: remedy.to_string(),
                scenes,
            });
        }
        log.record(
            &spoonstill_core::diagnostics::Event::info("tts", "voice service is ready")
                .with("provider", &id)
                .with("scenes", scenes.to_string()),
        );
    }
    Ok(())
}

/// Phase one: resolve every scene's narration, `audio_jobs` at a time.
fn resolve_audio(
    project: &Project,
    audio_jobs: usize,
    tools: &Tools,
    cancel: &Cancel,
    log: &dyn Diagnostics,
    on_event: &(dyn Fn(FilmEvent) + Sync),
) -> Result<Vec<ResolvedAudio>, FilmError> {
    let cache = AudioCache::in_project(&project.root);
    let policy = &crate::audio::AudioPolicy {
        trim: spoonstill_media::audio::Trim {
            head_seconds: project.settings.trim_head,
            tail_seconds: project.settings.trim_tail,
        },
    };

    // Before the pool: is the voice service even there? (D-002, D-094.)
    //
    // Every scene that needs speech would otherwise discover this for itself,
    // which at n=500 means five hundred processes failing one at a time, and
    // — if the scenes needing speech come late — forty minutes of rendering
    // thrown away to learn something one `--version` call knew at the start.
    check_voice_service(project, &cache, log)?;

    let outcomes = pool::run(
        &project.scenes,
        audio_jobs,
        cancel,
        |index, scene: &ResolvedScene| -> Result<ResolvedAudio, AudioError> {
            let resolved = crate::audio::resolve(
                &cache,
                tools,
                &scene.spec.source,
                scene.audio.as_deref(),
                policy,
                log,
            );
            match &resolved {
                Ok(audio) => on_event(FilmEvent::Audio {
                    index,
                    id: scene.spec.id.as_str().to_owned(),
                    kind: audio.kind,
                    duration: audio.duration,
                    reused: audio.reused,
                }),
                Err(error) => on_event(FilmEvent::Failed {
                    index,
                    id: scene.spec.id.as_str().to_owned(),
                    detail: error.to_string(),
                }),
            }
            resolved
        },
    );

    collect(project, outcomes, cancel).map_err(|failures| FilmError::Audio { failures })
}

/// A segment, and whether this run had to make it.
struct Segment {
    path: PathBuf,
    frames: u32,
    duration: f64,
    reused: bool,
}

/// Phase two: render every scene, `jobs` at a time.
#[allow(clippy::too_many_arguments)]
fn render_segments(
    project: &Project,
    audio: &[ResolvedAudio],
    options: &RenderProjectOptions,
    jobs: usize,
    segments_dir: &Path,
    output: OutputSpec,
    encode: &EncodeSettings,
    tools: &Tools,
    cancel: &Cancel,
    log: &dyn Diagnostics,
    on_event: &(dyn Fn(FilmEvent) + Sync),
) -> Result<Vec<Segment>, FilmError> {
    let project_id = project_id(&project.root);

    // Scene identity — and therefore the move, and therefore the segment's
    // name — is decided *before* the pool starts. Nothing a worker does can
    // depend on which worker it is, or on what order the workers ran in.
    let mut plans = Vec::with_capacity(project.scenes.len());
    for (index, scene) in project.scenes.iter().enumerate() {
        let narration = audio[index].duration;
        let frames = timing::frames_for_duration(narration, output.fps());
        // One implementation, in `spoonstill-media` (D-126). This used to be a
        // second streaming loop here, and the pair had to agree on the cache
        // key for `still render` and `still render-scene` to share a segment.
        let content = spoonstill_media::scene::hash_file(&scene.image)
            .map_err(|source| FilmError::Media(Box::new(source)))?;

        let index32 = u32::try_from(index).unwrap_or(u32::MAX);
        let motion = motion_for(&scene.spec.motion, &project_id, index32, &content, project);
        let subtitles = subtitles_for(project, options, scene, narration);
        let key = segment_key(
            &content,
            audio[index].key,
            frames,
            motion,
            output,
            encode,
            subtitles.as_ref(),
        );

        plans.push(Plan {
            request: SceneRequest {
                image: scene.image.clone(),
                audio: audio[index].path.clone(),
                out: segments_dir.join(format!("seg-{index:04}-{key:016x}.mp4")),
                output,
                motion: Some(motion),
                project_id: project_id.clone(),
                scene_index: index32,
                encode: encode.clone(),
                subtitles,
            },
            id: scene.spec.id.as_str().to_owned(),
            frames,
        });
    }

    let outcomes = pool::run(&plans, jobs, cancel, |index, plan: &Plan| {
        let result = render_one(tools, plan, cancel, log);
        match &result {
            Ok(segment) => on_event(FilmEvent::Segment {
                index,
                id: plan.id.clone(),
                frames: segment.frames,
                duration: segment.duration,
                reused: segment.reused,
            }),
            Err(error) => on_event(FilmEvent::Failed {
                index,
                id: plan.id.clone(),
                detail: error.to_string(),
            }),
        }
        result
    });

    collect(project, outcomes, cancel).map_err(|failures| FilmError::Render { failures })
}

/// One scene's plan, fixed before any worker starts.
struct Plan {
    request: SceneRequest,
    id: String,
    frames: u32,
}

/// Render one segment, or reuse the one that is already there.
///
/// A segment file only ever *gets* its content-addressed name by passing the
/// profile assertion and being renamed into place (D-042), so its presence is
/// meaningful. It is still re-probed here rather than trusted: a file can be
/// truncated by a full disk, edited by hand, or copied in from elsewhere, and
/// D-041 is explicit that the assertion is ours to make.
fn render_one(
    tools: &Tools,
    plan: &Plan,
    cancel: &Cancel,
    log: &dyn Diagnostics,
) -> Result<Segment, MediaError> {
    let expected = profile::SegmentProfile::for_output(plan.request.output);

    if plan.request.out.exists() {
        let probed = spoonstill_media::probe(
            tools,
            &plan.request.out,
            spoonstill_media::probe::DEFAULT_PROBE_TIMEOUT,
        );
        if let Ok(probed) = probed {
            let duration = timing::duration_for_frames(plan.frames, plan.request.output.fps());
            // Both, and the length one is not redundant: `SegmentProfile` pins
            // codec, geometry and colour and says nothing about how long a
            // segment is (D-110).
            if profile::assert_matches_profile(&expected, &probed).is_ok()
                && is_the_planned_length(&probed, plan)
            {
                return Ok(Segment {
                    path: plan.request.out.clone(),
                    frames: plan.frames,
                    duration,
                    reused: true,
                });
            }
        }
        // Present but not valid: it is not a segment, whatever its name says.
        let _ = std::fs::remove_file(&plan.request.out);
    }

    let rendered = spoonstill_media::render_scene(tools, &plan.request, cancel, log, &mut |_| {})?;
    Ok(Segment {
        path: rendered.path,
        frames: rendered.frames,
        duration: rendered.duration,
        reused: false,
    })
}

/// Is the segment already on disk as long as this plan needs it to be?
///
/// The reuse check probes the file and asserts its profile, and the profile
/// pins codec, geometry and colour — **not length**. So a file with a segment's
/// name and a segment's shape was reused whatever its duration, and the frame
/// count reported for it was `plan.frames`: a number nothing had checked
/// (D-110).
///
/// Read from the container header, never counted. Counting means decoding, and
/// D-096 is explicit that the reuse check must stay as fast on six hours as on
/// four seconds — at 200 scenes a decoding probe per scene would cost more than
/// the re-encode it saves. The header is the right evidence here for the same
/// reason the film's own assertion trusts it: this file was written by our
/// muxer, and it only ever *got* this name by passing the counted assertion in
/// `scene.rs` (D-042).
fn is_the_planned_length(probed: &spoonstill_media::probe::ProbeResult, plan: &Plan) -> bool {
    let Some(video) = probed
        .streams
        .iter()
        .find(|s| s.kind == spoonstill_media::probe::StreamKind::Video)
    else {
        return false;
    };

    if let Some(frames) = video.nb_frames {
        return frames == u64::from(plan.frames);
    }

    // No declared count. Our own MP4s always carry one, so this is a file we
    // did not write; fall back to the duration, within a frame.
    if let Some(seconds) = video.duration {
        let fps = f64::from(plan.request.output.fps());
        let expected = timing::duration_for_frames(plan.frames, plan.request.output.fps());
        return (seconds - expected).abs() <= 1.0 / fps;
    }

    // Neither. Nothing here establishes the length, so it is not reusable —
    // re-rendering costs one scene, and trusting it costs a wrong film.
    false
}

/// Turn a pool's outcomes into either every value or every failure.
fn collect<T>(
    project: &Project,
    outcomes: Vec<Outcome<Result<T, impl std::fmt::Display>>>,
    cancel: &Cancel,
) -> Result<Vec<T>, Vec<SceneFailure>> {
    let mut values = Vec::with_capacity(outcomes.len());
    let mut failures = Vec::new();

    for (index, outcome) in outcomes.into_iter().enumerate() {
        let id = project
            .scenes
            .get(index)
            .map_or_else(|| index.to_string(), |s| s.spec.id.as_str().to_owned());
        match outcome {
            Outcome::Done(Ok(value)) => values.push(value),
            Outcome::Done(Err(error)) => failures.push(SceneFailure {
                index,
                id,
                detail: error.to_string(),
            }),
            Outcome::NotAdmitted => failures.push(SceneFailure {
                index,
                id,
                detail: "not started — the run was cancelled".to_owned(),
            }),
        }
    }

    if failures.is_empty() {
        Ok(values)
    } else if cancel.is_requested() {
        // A cancelled run's failures are mostly "we stopped", which is not a
        // list an operator needs to read.
        Err(vec![SceneFailure {
            index: 0,
            id: "*".to_owned(),
            detail: "cancelled".to_owned(),
        }])
    } else {
        Err(failures)
    }
}

/// The move for one scene: the operator's, where they expressed one, and the
/// seeded choice where they did not (D-035).
fn motion_for(
    request: &MotionRequest,
    project_id: &str,
    index: u32,
    content: &str,
    project: &Project,
) -> MotionSpec {
    let seeded = MotionSpec::seeded(project_id, index, content);
    MotionSpec {
        kind: request.kind.unwrap_or(seeded.kind),
        anchor: request.anchor.unwrap_or(seeded.anchor),
        // A project-level `amount` is a deliberate setting; the seeded value
        // is the variety D-035 asks for when nobody chose. The seed is kept
        // either way, because it is what makes the choice reproducible.
        amount: if request.kind.is_some() || request.anchor.is_some() {
            project.settings.amount
        } else {
            seeded.amount
        },
        seed: seeded.seed,
    }
}

/// What this scene puts on screen, if anything (D-106).
///
/// Three things have to be true for a scene to carry a subtitle: the project
/// (or this run) asked for one, the scene has words, and the scene has time.
/// Any of them missing yields `None`, and `None` renders exactly the segment a
/// project without subtitles renders — same filter graph, same cache key, same
/// bytes.
///
/// The cues are cut against the **narration** duration rather than the padded
/// segment duration, so the caption leaves the screen when the speaking stops
/// (D-022's padding stays silent in both senses).
fn subtitles_for(
    project: &Project,
    options: &RenderProjectOptions,
    scene: &ResolvedScene,
    narration: f64,
) -> Option<SubtitleSpec> {
    let enabled = options.subtitles.unwrap_or(project.settings.subtitles);
    if !enabled {
        return None;
    }
    let theme = options
        .subtitle_theme
        .unwrap_or(project.settings.subtitle_theme);
    let text = scene.spec.caption.as_deref()?;

    let cues = captions::cues(text, narration, theme.style().max_chars);
    if cues.is_empty() {
        return None;
    }
    Some(SubtitleSpec {
        theme,
        placement: options
            .subtitle_placement
            .unwrap_or(project.settings.subtitle_placement),
        cues,
    })
}

/// The segment cache key (D-043, D-107).
///
/// Everything that changes the bytes: the image content, **the narration's own
/// content key**, the resolved length, the move, the output geometry, the
/// encoder settings, and the subtitles burned into the picture. Nothing that
/// does not — not the path, not the scene id, not the machine.
///
/// `audio` is the key the audio cache stored the normalized artifact under, so
/// it is the narration's content and normalization profile and nothing else.
/// It is here because a segment *contains* the narration: without it, replacing
/// a recording with a different one of the same length reused the old segment
/// and the operator got a film of their previous voice-over (D-107).
fn segment_key(
    content: &str,
    audio: u64,
    frames: u32,
    motion: MotionSpec,
    output: OutputSpec,
    encode: &EncodeSettings,
    subtitles: Option<&SubtitleSpec>,
) -> u64 {
    let geometry = format!("{}x{}@{}", output.width(), output.height(), output.fps());
    let encoder = format!("{}:{}", encode.preset, encode.crf);
    let motion_text = format!("{}:{:016x}", motion.descriptor(), motion.seed);

    // Each subtitle field stays its own field (D-118). They used to be joined
    // on `\u{1f}` and hashed as one, which is the byte `fnv1a_fields` uses as
    // its own separator — so a cue whose text carried one was indistinguishable
    // from a field boundary, and two genuinely different sets of cues keyed the
    // same. `0x1f` is not whitespace, so it survives `normalize` out of an
    // operator's `.txt` untouched.
    //
    // A scene with no subtitles contributes no fields at all, so turning the
    // feature on for a project whose scenes have no words is a cache hit —
    // which is the honest answer, because the bytes really are the same.
    let subtitle_fields = subtitles.map(SubtitleSpec::key_fields).unwrap_or_default();

    let audio_bytes = audio.to_be_bytes();
    let frame_bytes = frames.to_be_bytes();
    let timescale_bytes = profile::VIDEO_TIMESCALE.to_be_bytes();

    let mut fields: Vec<&[u8]> = vec![
        content.as_bytes(),
        &audio_bytes,
        &frame_bytes,
        motion_text.as_bytes(),
        geometry.as_bytes(),
        encoder.as_bytes(),
        // The segment profile is part of what a segment *is* (D-040): a
        // change to the pinned colour or timescale must miss the cache.
        profile::PIX_FMT.as_bytes(),
        profile::COLOR_SPACE.as_bytes(),
        &timescale_bytes,
    ];
    fields.extend(subtitle_fields.iter().map(String::as_bytes));

    // Length-prefixed, not separated: this list ends with an operator's own
    // words, and a separator is only unambiguous while no field can contain it.
    hash::fnv1a_prefixed(&fields)
}

/// Stable project identity for the motion seed (D-035).
///
/// The folder's own name, not its full path: copying a project to another
/// machine, or to another directory on this one, must not re-roll every move
/// and miss every cache entry. Renaming the folder does change it — that is
/// the trade, and it is the more visible of the two.
fn project_id(root: &Path) -> String {
    root.file_name().map_or_else(
        || "project".to_owned(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// One render per project at a time.
///
/// Two runs against one project would race on the same segment paths, the same
/// cache entries and the same output file. Each of those is individually safe
/// — every write is a temporary plus a rename — but the *film* is not: two
/// runs with different settings would interleave segments from both.
///
/// **The lock is the kernel's, not the file's** (D-113). `render.lock` is
/// opened and locked with `std::fs::File::try_lock`; the file's *existence*
/// carries no authority at all, and it is never deleted.
///
/// That one change answers both of the questions the previous design got
/// wrong. A lock cannot go stale, because the operating system releases it
/// when the holding process dies — crash, kill, or power loss — so the
/// "a crashed run left a lock behind" case that `--force` existed for no longer
/// happens. And releasing is closing a handle rather than unlinking a shared
/// path, so one run can no longer unlock another.
#[derive(Debug)]
struct Lock {
    /// Holding this open **is** holding the lock. Dropping it releases.
    file: std::fs::File,
}

impl Lock {
    fn take(root: &Path, force: bool) -> Result<Self, FilmError> {
        let directory = root.join(STATE_DIR);
        std::fs::create_dir_all(&directory).map_err(|source| FilmError::Io {
            doing: "creating the state directory",
            path: directory.clone(),
            source,
        })?;
        let path = directory.join(LOCK_FILE);

        // Not `create_new`: a file left by an older build, or by a run this
        // machine lost, is an ordinary file to be locked rather than evidence
        // of anything.
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|source| FilmError::Io {
                doing: "taking the render lock at",
                path: path.clone(),
                source,
            })?;

        if file.try_lock().is_err() {
            // Held by a *live* process — that is what the kernel refusing
            // means — so there is nothing here for `--force` to rescue, and
            // overriding it is the corruption this lock exists to prevent.
            let held_by = std::fs::read_to_string(&path)
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| "unknown".to_owned());
            return Err(FilmError::Locked {
                path,
                held_by,
                forced: force,
            });
        }

        // Who we are, for the other run's error message. Written under the
        // lock and best-effort: failing to describe ourselves is not a reason
        // to refuse a render we are entitled to.
        use std::io::{Seek, Write};
        let _ = file.set_len(0);
        let _ = (&file).seek(std::io::SeekFrom::Start(0));
        let _ = (&file).write_all(format!("pid {}\n", std::process::id()).as_bytes());

        Ok(Lock { file })
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        // Closing the handle releases the kernel lock. The file stays: it is a
        // marker, and deleting a path another run may already have opened is
        // exactly the race this design removed.
        let _ = self.file.unlock();
    }
}

/// A `Fn` event sink that serializes calls from the pool's workers.
///
/// Provided here rather than left to each caller because getting it wrong is
/// silent: interleaved `print!` from eight threads produces a garbled line
/// nobody can read, and only sometimes.
pub struct SerialEvents<F>(Mutex<F>);

impl<F: FnMut(FilmEvent) + Send> SerialEvents<F> {
    /// Wrap a closure so the pool can call it from any worker.
    pub const fn new(sink: F) -> Self {
        SerialEvents(Mutex::new(sink))
    }

    /// Call it, from wherever.
    pub fn emit(&self, event: FilmEvent) {
        if let Ok(mut sink) = self.0.lock() {
            sink(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoonstill_core::{Anchor, Aspect, MotionKind};

    /// A stand-in for the narration's content key. Any value works; what the
    /// tests care about is that changing it changes the segment key.
    const AUDIO: u64 = 0x5150_5150_5150_5150;

    fn output() -> OutputSpec {
        OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap()
    }

    /// D-035, D-043: the key covers everything that changes the bytes. Each of
    /// these edits must produce a different segment file rather than silently
    /// reusing the last one.
    use spoonstill_core::project::AudioSource;

    /// A directory of this test's own, removed when it is done.
    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "spoonstill-film-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    /// A scene that needs a line spoken by `provider`.
    fn spoken_scene(id: &str, text: &str, provider: &str) -> ResolvedScene {
        use spoonstill_core::project::{
            MotionRequest, ProviderId, SceneId, SceneSpec, TtsSettings, VoiceId,
        };
        ResolvedScene {
            spec: SceneSpec {
                id: SceneId::new(id).expect("a legal id"),
                image: PathBuf::from("001.jpg"),
                source: AudioSource::Tts {
                    text: text.to_owned(),
                    provider: ProviderId(provider.to_owned()),
                    voice: VoiceId("en-US-AvaNeural".to_owned()),
                    settings: TtsSettings::default(),
                },
                motion: MotionRequest::default(),
                caption: Some(text.to_owned()),
            },
            image: PathBuf::from("001.jpg"),
            audio: None,
        }
    }

    fn project_of(root: &Path, scenes: Vec<ResolvedScene>) -> Project {
        Project {
            root: root.to_path_buf(),
            settings: crate::import::settings::Settings::default(),
            mode: crate::import::Mode::Convention,
            scenes,
            problems: Vec::new(),
        }
    }

    /// D-002 in one test: the run stops before the pool, naming the provider,
    /// the fix, and how many scenes were about to fail one at a time.
    #[test]
    fn a_missing_voice_service_stops_the_run_before_any_scene_is_touched() {
        let root = scratch("no-service");
        // A provider name this build does not have stands in for a machine
        // with no `edge-tts`: both are "the service is not usable", and only
        // this one is the same on every machine the tests run on.
        let project = project_of(
            &root,
            vec![
                spoken_scene("001", "The harbour was empty.", "nonesuch"),
                spoken_scene("002", "The light had gone.", "nonesuch"),
            ],
        );

        let error = check_voice_service(
            &project,
            &AudioCache::in_project(&root),
            &spoonstill_core::diagnostics::Noop,
        )
        .expect_err("no such provider");

        match error {
            FilmError::VoiceService {
                provider,
                detail,
                scenes,
            } => {
                assert_eq!(provider, "nonesuch");
                assert_eq!(scenes, 2, "it counts what was about to fail");
                assert!(
                    detail.contains("edge"),
                    "it names what does exist: {detail}"
                );
            }
            other => panic!("wrong error: {other}"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The regression this check could have caused: a project whose lines are
    /// all already spoken re-renders on a machine with no voice service at
    /// all. The cache is the answer to "does this need work", here as
    /// everywhere else (D-075, D-084).
    #[test]
    fn a_project_whose_lines_are_already_spoken_needs_no_voice_service() {
        let root = scratch("warm-cache");
        let scene = spoken_scene("001", "A line already spoken.", "nonesuch");
        let cache = AudioCache::in_project(&root);

        assert_eq!(
            crate::audio::provider_needed(&cache, &scene.spec.source),
            Some("nonesuch"),
            "cold: the service is needed"
        );

        // Warm it exactly the way a real run would.
        std::fs::create_dir_all(cache.directory()).expect("the cache directory");
        let spoken = match &scene.spec.source {
            AudioSource::Tts { .. } => {
                let key = crate::audio::speech_key(&scene.spec.source).expect("a spoken source");
                cache.spoken_path(key)
            }
            _ => unreachable!(),
        };
        std::fs::write(&spoken, b"not really an mp3").expect("write");

        assert_eq!(
            crate::audio::provider_needed(&cache, &scene.spec.source),
            None,
            "warm: nothing to ask a service for"
        );
        check_voice_service(
            &project_of(&root, vec![scene]),
            &cache,
            &spoonstill_core::diagnostics::Noop,
        )
        .expect("a warm cache renders with no voice service");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A recording and a silent still never ask a service anything.
    #[test]
    fn a_recording_and_a_silence_need_no_voice_service() {
        let root = scratch("no-speech");
        let cache = AudioCache::in_project(&root);
        assert_eq!(
            crate::audio::provider_needed(
                &cache,
                &AudioSource::File {
                    original_path: PathBuf::from("001.wav")
                }
            ),
            None
        );
        assert_eq!(
            crate::audio::provider_needed(&cache, &AudioSource::Silent { seconds: 4.0 }),
            None
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn the_segment_key_changes_with_everything_that_changes_the_segment() {
        let motion = MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center);
        let encode = EncodeSettings::default();
        let base = segment_key(
            "aaaaaaaaaaaaaaaa",
            AUDIO,
            112,
            motion,
            output(),
            &encode,
            None,
        );

        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO + 1,
                112,
                motion,
                output(),
                &encode,
                None
            ),
            "a different narration — the one input whose absence let a re-recorded \
             line reuse the old segment (D-107)"
        );

        assert_eq!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO,
                112,
                motion,
                output(),
                &encode,
                None
            ),
            "the same scene must key the same, or nothing is ever reused"
        );

        assert_ne!(
            base,
            segment_key(
                "bbbbbbbbbbbbbbbb",
                AUDIO,
                112,
                motion,
                output(),
                &encode,
                None
            ),
            "different image"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO,
                113,
                motion,
                output(),
                &encode,
                None
            ),
            "one more frame"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO,
                112,
                MotionSpec::new(MotionKind::ZoomOut, 0.1, Anchor::Center),
                output(),
                &encode,
                None
            ),
            "a different move"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO,
                112,
                motion,
                OutputSpec::new(Aspect::Portrait9x16, 1080, 30).unwrap(),
                &encode,
                None
            ),
            "a different aspect"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO,
                112,
                motion,
                output(),
                &EncodeSettings {
                    preset: "slow".to_owned(),
                    crf: 18
                },
                None
            ),
            "a different preset"
        );
    }

    /// D-118. The segment key holds an operator's own words, so no field
    /// content may be mistakable for a field boundary.
    ///
    /// `fnv1a_fields` separates with `0x1f` and says in its own documentation
    /// that no field may contain that byte — true of every original caller
    /// (paths, ids, hex digests) and quietly untrue once D-106 began feeding it
    /// subtitle text. `0x1f` is **not whitespace**, so it survives `normalize`
    /// out of a `.txt` intact.
    #[test]
    fn two_different_sets_of_cues_never_key_the_same() {
        use spoonstill_core::captions::{Cue, Placement, SubtitleTheme};

        let spec = |cues: Vec<Cue>| SubtitleSpec {
            theme: SubtitleTheme::Classic,
            placement: Placement::Bottom,
            cues,
        };
        let cue = |text: &str| Cue {
            text: text.to_owned(),
            start: 0.0,
            end: 2.0,
        };

        // Two cues, against one cue whose text carries the separator and then
        // spells out the second cue's whole field. Joined on `0x1f` these are
        // the *same string*, which is what made them the same key.
        let two = spec(vec![cue("alpha beta"), cue("gamma")]);
        let one = spec(vec![cue("alpha beta\u{1f}0.000>2.000:gamma")]);

        assert_eq!(
            two.key_fields().join("\u{1f}"),
            one.key_fields().join("\u{1f}"),
            "the collision this test exists for should still be constructible \
             at the string level — only the key must no longer share it"
        );

        let motion = MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center);
        let encode = EncodeSettings::default();
        let key_of = |spec: &SubtitleSpec| {
            segment_key(
                "aaaaaaaaaaaaaaaa",
                AUDIO,
                112,
                motion,
                output(),
                &encode,
                Some(spec),
            )
        };
        assert_ne!(
            key_of(&two),
            key_of(&one),
            "two different films share one segment, so one of them renders the \
             other's subtitles"
        );
    }

    /// The name that reaches the concat list. D-052 says an operator's own
    /// spelling is hostile input; a content-addressed name never carries it.
    #[test]
    fn segment_names_are_safe_for_the_concat_list() {
        let key = segment_key(
            "aaaaaaaaaaaaaaaa",
            AUDIO,
            112,
            MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
            output(),
            &EncodeSettings::default(),
            None,
        );
        let name = format!("seg-{:04}-{key:016x}.mp4", 7);
        assert!(
            concat::is_safe_list_name(&name),
            "{name} would have to be escaped into the concat list"
        );
    }

    /// D-035: copying a project must not re-roll every move. The identity is
    /// the folder name, so the same folder under a different parent keeps its
    /// moves and its cache.
    #[test]
    fn project_identity_survives_being_moved() {
        assert_eq!(project_id(Path::new("/a/b/demo")), "demo");
        assert_eq!(project_id(Path::new("/elsewhere/demo")), "demo");
        assert_eq!(project_id(Path::new("/")), "project");
    }

    /// D-054 reaches the output setting too: a manifest that writes the film
    /// outside the project is refused, and the message says how to do it
    /// deliberately.
    #[test]
    fn an_output_setting_that_escapes_the_project_is_refused() {
        let settings = crate::import::Settings {
            output: PathBuf::from("../../out.mp4"),
            ..Default::default()
        };
        let project = Project {
            root: PathBuf::from("/projects/demo"),
            settings,
            mode: crate::import::Mode::Convention,
            scenes: Vec::new(),
            problems: Vec::new(),
        };

        let options = RenderProjectOptions::for_project("/projects/demo");
        let error = destination(&project, &options).expect_err("escapes");
        assert!(matches!(error, FilmError::OutputOutsideProject { .. }));
        assert!(error.to_string().contains("--out"), "{error}");

        // An explicit --out is the operator's own instruction and is honoured.
        let deliberate = RenderProjectOptions {
            out: Some(PathBuf::from("/tmp/anywhere.mp4")),
            ..options
        };
        assert_eq!(
            destination(&project, &deliberate).unwrap(),
            PathBuf::from("/tmp/anywhere.mp4")
        );
    }

    /// The `output:` setting is manifest data and is held to D-054 containment
    /// by the same function every input path uses (D-112).
    ///
    /// A real folder, not an invented path: containment is decided on canonical
    /// paths, and a root that does not exist cannot contain anything — which is
    /// the honest answer, and what this test used to paper over.
    #[test]
    fn the_output_setting_resolves_inside_the_project() {
        let root = std::env::temp_dir().join(format!("spoonstill-out-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let of = |output: &str| Project {
            root: root.clone(),
            settings: crate::import::Settings {
                output: PathBuf::from(output),
                ..crate::import::Settings::default()
            },
            mode: crate::import::Mode::Convention,
            scenes: Vec::new(),
            problems: Vec::new(),
        };
        let options = RenderProjectOptions::for_project(&root);
        // De-prefixed, because that is what `real_path` now hands back on
        // Windows and what the operator sees written (D-142); the identity
        // function elsewhere.
        let real_root = spoonstill_core::path_safety::without_verbatim_prefix(
            std::fs::canonicalize(&root).unwrap(),
        );

        // The ordinary case, and a nested one that does not exist yet: a
        // destination is normally absent, which is exactly why it cannot use
        // the input resolver.
        assert_eq!(
            destination(&of("out.mp4"), &options).unwrap(),
            real_root.join("out.mp4")
        );
        assert_eq!(
            destination(&of("renders/2026/out.mp4"), &options).unwrap(),
            real_root.join("renders/2026/out.mp4")
        );

        // The lexical escapes, which the old check did catch.
        for escape in ["../out.mp4", "/tmp/out.mp4"] {
            assert!(
                matches!(
                    destination(&of(escape), &options),
                    Err(FilmError::OutputOutsideProject { .. })
                ),
                "{escape} was allowed"
            );
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The escape a lexical check cannot see (D-112). `..` and an absolute
    /// path were both refused; a symlink is neither, so a project carrying
    /// `escape -> /tmp` and `output: escape/film.mp4` wrote its film outside
    /// the folder and reported success.
    #[cfg(unix)]
    #[test]
    fn an_output_that_leaves_through_a_symlink_is_refused() {
        let base = std::env::temp_dir().join(format!("spoonstill-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("project");
        let elsewhere = base.join("elsewhere");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, root.join("escape")).unwrap();
        // And one that stays inside, which must keep working.
        std::fs::create_dir_all(root.join("real")).unwrap();
        std::os::unix::fs::symlink(root.join("real"), root.join("here")).unwrap();

        let of = |output: &str| Project {
            root: root.clone(),
            settings: crate::import::Settings {
                output: PathBuf::from(output),
                ..crate::import::Settings::default()
            },
            mode: crate::import::Mode::Convention,
            scenes: Vec::new(),
            problems: Vec::new(),
        };
        let options = RenderProjectOptions::for_project(&root);

        assert!(
            matches!(
                destination(&of("escape/film.mp4"), &options),
                Err(FilmError::OutputOutsideProject { .. })
            ),
            "a symlinked output escaped the project"
        );

        // A link that stays inside resolves to where it really points — the
        // operator's spelling is a request, not an address.
        assert_eq!(
            destination(&of("here/film.mp4"), &options).unwrap(),
            std::fs::canonicalize(root.join("real"))
                .unwrap()
                .join("film.mp4"),
        );

        // An explicit --out is the operator's own instruction (D-054's note on
        // deliberate destinations) and is honoured wherever it points.
        let deliberate = RenderProjectOptions {
            out: Some(elsewhere.join("deliberate.mp4")),
            ..RenderProjectOptions::for_project(&root)
        };
        assert_eq!(
            destination(&of("escape/film.mp4"), &deliberate).unwrap(),
            elsewhere.join("deliberate.mp4")
        );

        let _ = std::fs::remove_dir_all(&base);
    }

    /// Two runs against one project would interleave segments into one film.
    ///
    /// D-113 changed what this test asserts, and the change is the fix: it
    /// used to end `Lock::take(&root, true).expect("forced")` — **`--force`
    /// taking a lock a live run was holding, asserted as correct.** Doing that
    /// for real produced two concurrent renders, and then three, because the
    /// second run's `Drop` deleted the shared file while the first still ran.
    #[test]
    fn a_second_render_of_one_project_is_refused_until_the_first_finishes() {
        let root = std::env::temp_dir().join(format!("spoonstill-lock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let first = Lock::take(&root, false).expect("the first run takes it");

        let error = Lock::take(&root, false).expect_err("the second is refused");
        assert!(matches!(error, FilmError::Locked { .. }), "{error}");

        // The fix: --force does not override a lock a running render holds,
        // and says so rather than appearing to do nothing.
        let forced = Lock::take(&root, true).expect_err("--force is refused too");
        assert!(
            matches!(forced, FilmError::Locked { forced: true, .. }),
            "{forced}"
        );
        assert!(forced.to_string().contains("--force cannot"), "{forced}");

        drop(first);

        // Released, so the next run is clean — even though the file is still
        // there. The file's existence was never the lock (D-113).
        let path = root.join(STATE_DIR).join(LOCK_FILE);
        assert!(path.exists(), "the marker file is deliberately not deleted");
        let second = Lock::take(&root, false).expect("the next run takes it");
        drop(second);

        let _ = std::fs::remove_dir_all(&root);
    }

    /// A lock file with nothing holding it is not a lock (D-113).
    ///
    /// This is the case `--force` existed for — a run the machine lost leaves
    /// its file behind — and it now needs no flag, because the operating
    /// system released the lock when that process died. Gate 5 used to write
    /// `pid 999999` into the file and require a refusal; requiring that was
    /// requiring the tool to be stuck.
    #[test]
    fn a_leftover_lock_file_does_not_stop_a_render() {
        let root = std::env::temp_dir().join(format!("spoonstill-stale-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(STATE_DIR)).unwrap();
        std::fs::write(root.join(STATE_DIR).join(LOCK_FILE), "pid 999999\n").unwrap();

        let taken = Lock::take(&root, false).expect("a file nobody holds is not a lock");
        drop(taken);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// D-044's two pools have two defaults, and neither is zero.
    ///
    /// Since D-144 only one of them is a number at construction: the segment
    /// pool cannot be sized until the geometry is known, so it is `None` here
    /// and resolved inside `render_project`.
    #[test]
    fn the_two_pools_are_sized_independently() {
        let options = RenderProjectOptions::for_project("/projects/demo");
        assert_eq!(
            options.jobs, None,
            "the segment pool is sized against the output geometry, which this \
             constructor has not seen (D-144)"
        );
        let hd = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).expect("1080p");
        let resolved = capacity::plan(hd).jobs;
        assert!(resolved >= 1);
        assert!(
            options.audio_jobs >= resolved,
            "ingest is I/O-bound and should not be capped by the encoder's pool"
        );
        assert!(options.out.is_none() && !options.force);
    }

    /// D-109. The sweep deletes files we made, by the name we gave them, and
    /// nothing else. A cache is not a licence to delete a stranger's file.
    #[test]
    fn the_sweep_recognises_only_the_names_we_write() {
        assert!(is_our_segment("seg-0000-0123456789abcdef.mp4"));
        assert!(is_our_segment("seg-9999-ffffffffffffffff.mp4"));

        for foreign in [
            "holiday.mp4",                    // an operator's own file
            "notes.txt",                      // not even a film
            "seg-99-abc.mp4",                 // our prefix, not our shape
            "seg-0000-0123456789abcdef.mov",  // right name, wrong extension
            "seg-000a-0123456789abcdef.mp4",  // non-digit index
            "seg-0000-0123456789abcdeg.mp4",  // 'g' is not hex
            "seg-0000-0123456789abcde.mp4",   // fifteen hex digits
            "seg-0000-0123456789abcdef0.mp4", // seventeen
            "xseg-0000-0123456789abcdef.mp4", // prefixed
        ] {
            assert!(!is_our_segment(foreign), "would have deleted {foreign}");
        }
    }

    /// The bound, and what is inside it. Live segments always survive; the
    /// spares kept are the *newest* dead ones, because those are the
    /// generation an operator flipping between two themes goes back to.
    #[test]
    fn the_sweep_keeps_the_live_set_and_the_newest_spares() {
        let dir = std::env::temp_dir().join(format!("spoonstill-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Two live, and six dead written oldest to newest.
        let name = |i: usize, k: usize| format!("seg-{i:04}-{k:016x}.mp4");
        let mut live = Vec::new();
        for i in 0..2 {
            let path = dir.join(name(i, 0xffff));
            std::fs::write(&path, b"live").unwrap();
            live.push(path);
        }
        let mut dead = Vec::new();
        for k in 0..6 {
            let path = dir.join(name(0, k));
            std::fs::write(&path, b"dead-and-eight-bytes").unwrap();
            // mtime resolution is coarse enough that written-in-a-loop files
            // can share a timestamp; stamp them explicitly so "newest" means
            // something. Ordering is the whole behaviour under test.
            let when = std::time::SystemTime::UNIX_EPOCH
                + std::time::Duration::from_secs(1_700_000_000 + k as u64 * 60);
            let _ = filetime_set(&path, when);
            dead.push(path);
        }

        let freed = prune_segments(&dir, &live, &spoonstill_core::diagnostics::Noop);

        for path in &live {
            assert!(
                path.exists(),
                "a live segment was swept: {}",
                path.display()
            );
        }
        // 2 live * 2 spare generations = 4 kept, so the 2 oldest go.
        assert!(!dead[0].exists(), "the oldest spare survived");
        assert!(!dead[1].exists(), "the second oldest spare survived");
        for path in &dead[2..] {
            assert!(
                path.exists(),
                "a recent spare was swept: {}",
                path.display()
            );
        }
        assert_eq!(freed, 2 * 20, "freed bytes should be the two files removed");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Set a file's modification time without pulling in a dependency for it.
    fn filetime_set(path: &Path, when: std::time::SystemTime) -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new().write(true).open(path)?;
        file.set_modified(when)
    }

    /// D-110. The reuse check's length gate, over the four shapes a probed
    /// file can take. The profile assertion cannot do this job: it pins codec,
    /// geometry and colour and knows nothing about how long a segment is.
    #[test]
    fn a_cached_segment_is_reused_only_at_the_planned_length() {
        use spoonstill_media::probe::{ProbeResult, Stream, StreamKind};

        fn plan_of(frames: u32) -> Plan {
            Plan {
                request: SceneRequest {
                    image: PathBuf::from("001.jpg"),
                    audio: PathBuf::from("001.wav"),
                    out: PathBuf::from("seg.mp4"),
                    output: output(),
                    motion: None,
                    project_id: "p".to_owned(),
                    scene_index: 0,
                    encode: EncodeSettings::default(),
                    subtitles: None,
                },
                id: "001".to_owned(),
                frames,
            }
        }

        fn probed(nb_frames: Option<u64>, duration: Option<f64>) -> ProbeResult {
            ProbeResult {
                path: PathBuf::from("seg.mp4"),
                format_name: "mov,mp4,m4a,3gp,3g2,mj2".to_owned(),
                format_duration: duration,
                streams: vec![Stream {
                    index: 0,
                    kind: StreamKind::Video,
                    codec_name: "h264".to_owned(),
                    profile: None,
                    level: None,
                    pix_fmt: None,
                    color_range: None,
                    color_space: None,
                    color_primaries: None,
                    color_transfer: None,
                    width: None,
                    height: None,
                    sample_aspect_ratio: None,
                    r_frame_rate: None,
                    time_base: None,
                    sample_rate: None,
                    sample_fmt: None,
                    channels: None,
                    channel_layout: None,
                    duration,
                    nb_read_frames: None,
                    nb_frames,
                }],
            }
        }

        let plan = plan_of(240);

        assert!(
            is_the_planned_length(&probed(Some(240), Some(8.0)), &plan),
            "the segment this plan asked for must still be reused"
        );
        assert!(
            !is_the_planned_length(&probed(Some(60), Some(2.0)), &plan),
            "a segment of another scene's length was reused, which is the \
             defect: the profile matches in every field it checks"
        );
        assert!(
            !is_the_planned_length(&probed(Some(241), Some(8.0)), &plan),
            "one frame out is still out — the join asserts an exact total"
        );

        // No declared count: fall back to the duration, within one frame.
        assert!(is_the_planned_length(&probed(None, Some(8.0)), &plan));
        assert!(is_the_planned_length(&probed(None, Some(8.02)), &plan));
        assert!(!is_the_planned_length(&probed(None, Some(2.0)), &plan));

        // Neither: nothing here establishes a length, so it is not reusable.
        // Re-rendering costs one scene; trusting it costs a wrong film.
        assert!(!is_the_planned_length(&probed(None, None), &plan));
    }

    /// D-115. A render that is killed leaves `.partial-<pid>-<n>.<ext>` files
    /// beside the artifacts they were becoming, and nothing removed them — not
    /// even D-109's sweep, which matches only names a segment earned.
    ///
    /// Measured: SIGKILL on a four-worker render left four of them, and they
    /// survive every later render. Safe to remove at the sweep because the
    /// render lock is exclusive per project (D-113), so nothing else can own a
    /// temporary here and ours are renamed away by the time the film is joined.
    #[test]
    fn a_killed_render_leaves_temporaries_and_the_next_one_sweeps_them() {
        assert!(is_abandoned_partial(
            ".seg-0011-2575b4e3c0fe8fc8.mp4.partial-66085-12.mp4"
        ));
        assert!(is_abandoned_partial(".film.mp4.partial-1-0.mp4"));

        for keeper in [
            "seg-0000-0123456789abcdef.mp4", // a segment
            ".hidden-notes.txt",             // an operator's own dotfile
            ".DS_Store",
            "partial-1-0.mp4", // no leading dot: not ours
            "holiday.mp4",
        ] {
            assert!(!is_abandoned_partial(keeper), "would have deleted {keeper}");
        }

        // And through the sweep itself, beside a live segment and a stranger.
        let dir = std::env::temp_dir().join(format!("spoonstill-partial-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let live = dir.join("seg-0000-000000000000ffff.mp4");
        std::fs::write(&live, b"live").unwrap();
        let litter = dir.join(".seg-0001-000000000000aaaa.mp4.partial-999-3.mp4");
        std::fs::write(&litter, b"abandoned").unwrap();
        let stranger = dir.join(".notes.txt");
        std::fs::write(&stranger, b"mine").unwrap();

        let freed = prune_segments(
            &dir,
            std::slice::from_ref(&live),
            &spoonstill_core::diagnostics::Noop,
        );

        assert!(!litter.exists(), "the abandoned temporary was left behind");
        assert!(live.exists(), "the live segment was swept");
        assert!(stranger.exists(), "a stranger's dotfile was swept");
        assert_eq!(freed, 9, "freed bytes should be the abandoned temporary");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
