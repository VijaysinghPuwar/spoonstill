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

use spoonstill_core::diagnostics::{Diagnostics, Event};
use spoonstill_core::project::MotionRequest;
use spoonstill_core::{MotionSpec, OutputSpec, STATE_DIR, hash, timing};
use spoonstill_media::scene::{Cancel, EncodeSettings, SceneRequest};
use spoonstill_media::{MediaError, Tools, concat, profile};
use spoonstill_state::FileLog;

use crate::audio::{AudioCache, AudioError, ResolvedAudio};
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
    /// Segments rendered at once (D-044).
    pub jobs: usize,
    /// Narrations resolved at once (D-044).
    pub audio_jobs: usize,
    /// Take the lock even if another run appears to hold it.
    pub force: bool,
}

impl RenderProjectOptions {
    /// Defaults for a project folder: the project's own output setting, and a
    /// worker count derived from the machine.
    #[must_use]
    pub fn for_project(root: impl Into<PathBuf>) -> Self {
        RenderProjectOptions {
            root: root.into(),
            out: None,
            jobs: pool::default_jobs(),
            // Ingest normalization is a short, I/O-bound FFmpeg run, so it can
            // afford more workers than the encoder can. At slice 4 the TTS
            // provider's own rate limit becomes the binding constraint on this
            // number (D-023) — which is exactly why it is a separate one.
            audio_jobs: pool::default_jobs().saturating_mul(2).max(1),
            force: false,
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
    },
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
    },
    /// The project's `output` setting points outside the project (D-054).
    OutputOutsideProject {
        /// The setting, as written.
        value: String,
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
            FilmError::Locked { path, held_by } => write!(
                f,
                "another render is working on this project ({held_by})\n\
                 If that is not true, delete {} or pass --force.",
                path.display()
            ),
            FilmError::OutputOutsideProject { value } => write!(
                f,
                "the project's output setting {value:?} points outside the project folder \
                 (D-054). Pass --out to write somewhere else deliberately."
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
    let project =
        crate::import::load(&options.root, &ProbeCheck::from_env()).map_err(FilmError::Import)?;

    let errors = project.errors().count();
    if errors > 0 {
        return Err(FilmError::HasProblems { errors });
    }
    if project.scenes.is_empty() {
        return Err(FilmError::NoScenes);
    }

    let out = destination(&project, options)?;

    // The log is opened before the work, not after a failure (D-016).
    let log = FileLog::open(&project.root).ok();
    let sink: &dyn Diagnostics = log
        .as_ref()
        .map_or(&spoonstill_core::diagnostics::Noop, |l| {
            l as &dyn Diagnostics
        });

    let _lock = Lock::take(&project.root, options.force)?;

    sink.record(
        &Event::info("render", "project invoked")
            .with("root", project.root.display().to_string())
            .with("out", out.display().to_string())
            .with("scenes", project.scenes.len().to_string())
            .with("jobs", options.jobs.to_string())
            .with("audio_jobs", options.audio_jobs.to_string()),
    );

    on_event(FilmEvent::Planned {
        scenes: project.scenes.len(),
        jobs: options.jobs,
        audio_jobs: options.audio_jobs,
    });

    let tools = Tools::from_env();
    let output = project.settings.output_spec;
    let encode = EncodeSettings {
        preset: project.settings.preset.clone(),
        crf: project.settings.crf,
    };

    // Phase one: every narration, N at a time.
    let audio = resolve_audio(&project, options, &tools, cancel, sink, on_event)?;

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
    })
}

/// Where the film goes, and whether we are allowed to write it there.
///
/// An explicit `--out` is an argument the operator typed and is honoured
/// wherever it points. The `output:` setting is *manifest data* and is held to
/// D-054 like every other path in the file — a project that renders itself
/// into `../../etc` is the thing containment exists to prevent.
fn destination(project: &Project, options: &RenderProjectOptions) -> Result<PathBuf, FilmError> {
    if let Some(out) = &options.out {
        return Ok(out.clone());
    }

    let relative = &project.settings.output;
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(FilmError::OutputOutsideProject {
            value: relative.display().to_string(),
        });
    }
    Ok(project.root.join(relative))
}

/// Phase one: resolve every scene's narration, `audio_jobs` at a time.
fn resolve_audio(
    project: &Project,
    options: &RenderProjectOptions,
    tools: &Tools,
    cancel: &Cancel,
    log: &dyn Diagnostics,
    on_event: &(dyn Fn(FilmEvent) + Sync),
) -> Result<Vec<ResolvedAudio>, FilmError> {
    let cache = AudioCache::in_project(&project.root);

    let outcomes = pool::run(
        &project.scenes,
        options.audio_jobs,
        cancel,
        |index, scene: &ResolvedScene| -> Result<ResolvedAudio, AudioError> {
            let resolved = crate::audio::resolve(
                &cache,
                tools,
                &scene.spec.source,
                scene.audio.as_deref(),
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
        let content = hash_file(&scene.image).map_err(|source| FilmError::Io {
            doing: "reading",
            path: scene.image.clone(),
            source,
        })?;

        let index32 = u32::try_from(index).unwrap_or(u32::MAX);
        let motion = motion_for(&scene.spec.motion, &project_id, index32, &content, project);
        let key = segment_key(&content, frames, motion, output, encode);

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
            },
            id: scene.spec.id.as_str().to_owned(),
            frames,
        });
    }

    let outcomes = pool::run(&plans, options.jobs, cancel, |index, plan: &Plan| {
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
            if profile::assert_matches_profile(&expected, &probed).is_ok() {
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

/// The segment cache key (D-043).
///
/// Everything that changes the bytes: the image content, the resolved length,
/// the move, the output geometry, and the encoder settings. Nothing that does
/// not — not the path, not the scene id, not the machine.
fn segment_key(
    content: &str,
    frames: u32,
    motion: MotionSpec,
    output: OutputSpec,
    encode: &EncodeSettings,
) -> u64 {
    let geometry = format!("{}x{}@{}", output.width(), output.height(), output.fps());
    let encoder = format!("{}:{}", encode.preset, encode.crf);
    let motion_text = format!("{}:{:016x}", motion.descriptor(), motion.seed);

    hash::fnv1a_fields(&[
        content.as_bytes(),
        &frames.to_be_bytes(),
        motion_text.as_bytes(),
        geometry.as_bytes(),
        encoder.as_bytes(),
        // The segment profile is part of what a segment *is* (D-040): a
        // change to the pinned colour or timescale must miss the cache.
        profile::PIX_FMT.as_bytes(),
        profile::COLOR_SPACE.as_bytes(),
        &profile::VIDEO_TIMESCALE.to_be_bytes(),
    ])
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

/// Stable content hash of a file, streamed (D-043).
fn hash_file(path: &Path) -> Result<String, std::io::Error> {
    use std::io::Read;

    let mut file = std::fs::File::open(path)?;
    let mut hash = hash::Fnv1a::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.write(&buffer[..read]);
    }
    Ok(format!("{:016x}", hash.finish()))
}

/// One render per project at a time.
///
/// Two runs against one project would race on the same segment paths, the same
/// cache entries and the same output file. Each of those is individually safe
/// — every write is a temporary plus a rename — but the *film* is not: two
/// runs with different settings would interleave segments from both.
///
/// The lock is advisory and self-describing: it holds the process id that took
/// it, so the error can say who, and `--force` exists because a machine that
/// lost power leaves a lock behind and an operator should not have to know
/// where it lives.
#[derive(Debug)]
struct Lock {
    path: PathBuf,
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

        let body = format!("pid {}\n", std::process::id());
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(mut file) => {
                use std::io::Write;
                let _ = file.write_all(body.as_bytes());
                Ok(Lock { path })
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if force {
                    std::fs::write(&path, &body).map_err(|source| FilmError::Io {
                        doing: "taking the render lock at",
                        path: path.clone(),
                        source,
                    })?;
                    return Ok(Lock { path });
                }
                let held_by = std::fs::read_to_string(&path)
                    .map(|s| s.trim().to_owned())
                    .unwrap_or_else(|_| "unknown".to_owned());
                Err(FilmError::Locked { path, held_by })
            }
            Err(source) => Err(FilmError::Io {
                doing: "taking the render lock at",
                path,
                source,
            }),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
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

    fn output() -> OutputSpec {
        OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap()
    }

    /// D-035, D-043: the key covers everything that changes the bytes. Each of
    /// these edits must produce a different segment file rather than silently
    /// reusing the last one.
    #[test]
    fn the_segment_key_changes_with_everything_that_changes_the_segment() {
        let motion = MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center);
        let encode = EncodeSettings::default();
        let base = segment_key("aaaaaaaaaaaaaaaa", 112, motion, output(), &encode);

        assert_eq!(
            base,
            segment_key("aaaaaaaaaaaaaaaa", 112, motion, output(), &encode),
            "the same scene must key the same, or nothing is ever reused"
        );

        assert_ne!(
            base,
            segment_key("bbbbbbbbbbbbbbbb", 112, motion, output(), &encode),
            "different image"
        );
        assert_ne!(
            base,
            segment_key("aaaaaaaaaaaaaaaa", 113, motion, output(), &encode),
            "one more frame"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                112,
                MotionSpec::new(MotionKind::ZoomOut, 0.1, Anchor::Center),
                output(),
                &encode
            ),
            "a different move"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                112,
                motion,
                OutputSpec::new(Aspect::Portrait9x16, 1080, 30).unwrap(),
                &encode
            ),
            "a different aspect"
        );
        assert_ne!(
            base,
            segment_key(
                "aaaaaaaaaaaaaaaa",
                112,
                motion,
                output(),
                &EncodeSettings {
                    preset: "slow".to_owned(),
                    crf: 18
                }
            ),
            "a different preset"
        );
    }

    /// The name that reaches the concat list. D-052 says an operator's own
    /// spelling is hostile input; a content-addressed name never carries it.
    #[test]
    fn segment_names_are_safe_for_the_concat_list() {
        let key = segment_key(
            "aaaaaaaaaaaaaaaa",
            112,
            MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
            output(),
            &EncodeSettings::default(),
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

    #[test]
    fn the_output_setting_resolves_inside_the_project() {
        let project = Project {
            root: PathBuf::from("/projects/demo"),
            settings: crate::import::Settings::default(),
            mode: crate::import::Mode::Convention,
            scenes: Vec::new(),
            problems: Vec::new(),
        };
        let out = destination(
            &project,
            &RenderProjectOptions::for_project("/projects/demo"),
        )
        .unwrap();
        assert_eq!(out, PathBuf::from("/projects/demo/out.mp4"));
    }

    /// Two runs against one project would interleave segments into one film.
    #[test]
    fn a_second_render_of_one_project_is_refused_until_the_first_finishes() {
        let root = std::env::temp_dir().join(format!("spoonstill-lock-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();

        let first = Lock::take(&root, false).expect("the first run takes it");
        let error = Lock::take(&root, false).expect_err("the second is refused");
        assert!(matches!(error, FilmError::Locked { .. }), "{error}");
        assert!(error.to_string().contains("--force"), "{error}");

        // --force is the answer to a lock left behind by a crash.
        let forced = Lock::take(&root, true).expect("forced");
        drop(forced);
        drop(first);

        // And the lock is gone once the run ends, so the next one is clean.
        assert!(Lock::take(&root, false).is_ok());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// D-044's two pools have two defaults, and neither is zero.
    #[test]
    fn the_two_pools_are_sized_independently() {
        let options = RenderProjectOptions::for_project("/projects/demo");
        assert!(options.jobs >= 1);
        assert!(
            options.audio_jobs >= options.jobs,
            "ingest is I/O-bound and should not be capped by the encoder's pool"
        );
        assert!(options.out.is_none() && !options.force);
    }
}
