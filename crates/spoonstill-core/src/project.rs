//! What a scene is, and what makes one invalid (D-013, D-020, D-050).
//!
//! A project arrives as rows — a CSV manifest, or filenames paired by stem
//! (D-050). This module is the stage between "some rows" and "something the
//! renderer will accept": it takes [`SceneDraft`]s exactly as written and
//! produces [`SceneSpec`]s, or a list of everything wrong with them.
//!
//! ## Every problem, not the first one
//!
//! plan.md §M2 is explicit, and it is worth restating because it shapes every
//! signature here: `still validate` reports **every** problem at once, with
//! scene IDs. An operator with a 500-row manifest cannot fix one typo per run
//! (D-002). So nothing here short-circuits: validation returns a
//! [`Validation`] holding both the scenes that survived and every
//! [`Problem`] found, and it is the caller that decides a non-empty problem
//! list means "do not render".
//!
//! ## Two things are deliberately *not* checked here
//!
//! - **Does the file exist, and is it really a JPEG?** Extensions are a hint,
//!   not evidence (plan.md §M2), and answering needs a disk and a probe.
//!   `spoonstill-core` does no I/O (D-010). The application layer resolves
//!   paths through [`crate::path_safety`], probes the media, and merges what
//!   it learns into the same [`Problem`] list — which is why
//!   [`ProblemKind::Path`] and [`ProblemKind::NotUsableMedia`] exist here but
//!   are never produced here.
//! - **How long is the narration?** Only `ffprobe` on the normalized artifact
//!   may answer that (D-021). A `duration` cell is a *declaration of a silent
//!   scene*, never an estimate of a spoken one.

use core::fmt;
use std::path::PathBuf;

use crate::diagnostics::Severity;
use crate::motion::{Anchor, MotionKind};
use crate::path_safety::PathError;
use crate::remedy::Remedy;

/// Longest single scene we will accept as a declared silent duration.
///
/// One hour. This is a sanity bound, not a product limit — D-021 asks that
/// absurd durations be rejected with the scene ID attached, and something has
/// to draw the line. A cell that says `36000` is a typo for `3.6` far more
/// often than it is an hour-long title card.
pub const MAX_SCENE_SECONDS: f64 = 3_600.0;

/// The largest a script file can be and still be one scene's narration
/// (D-126).
///
/// **Derived, not chosen.** A scene holds at most [`MAX_SCENE_SECONDS`], the
/// fastest speech `spoonstill-tts` has measured is 17.3 characters a second
/// (`edge::SPEECH_CHARS_PER_SECOND`, which already refuses a line longer than
/// the product), and UTF-8 spends at most four bytes on a character:
///
/// ```text
/// 3600 s x 17.3 chars/s x 4 bytes/char = 249 120 bytes
/// ```
///
/// Rounded up to 256 KiB. A `.txt` bigger than this is not a narration that was
/// merely too long — it is a file that landed in the folder by mistake, and
/// reading it whole to find that out is the thing this exists to avoid. A test
/// in `spoonstill-tts` keeps the two numbers in step, because they live in
/// crates that cannot see each other.
pub const MAX_SCRIPT_BYTES: u64 = 256 * 1024;

/// Stable, operator-visible name for one scene.
///
/// It is the join key in convention mode (D-050), the label in every
/// diagnostic (D-052), and — because it names the segment file on disk — it is
/// held to what a filename can be. That last part is why this is a validated
/// type and not a `String`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SceneId(String);

impl SceneId {
    /// Accept `text` as a scene ID, or say why not.
    ///
    /// Rejects the shapes that would either vanish or escape when used as a
    /// path component. Note what is *not* rejected: spaces, Unicode, and
    /// emoji all pass, because D-052 says those are the normal case and every
    /// command we build is an argument vector (D-011).
    ///
    /// # Errors
    ///
    /// [`SceneIdError`] describes the specific shape that was refused.
    pub fn new(text: &str) -> Result<Self, SceneIdError> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(SceneIdError::Blank);
        }
        if trimmed.contains(['/', '\\']) {
            return Err(SceneIdError::PathSeparator);
        }
        if trimmed == "." || trimmed == ".." {
            return Err(SceneIdError::RelativeDirectory);
        }
        // Control characters survive a CSV round trip and then corrupt every
        // log line the ID appears in (D-016).
        if trimmed.chars().any(char::is_control) {
            return Err(SceneIdError::ControlCharacter);
        }
        Ok(SceneId(trimmed.to_owned()))
    }

    /// The ID as written, after trimming.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SceneId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `pad`, not `write_str`: a scene ID is printed in a column of them,
        // and `write_str` silently ignores the formatter's width. The CLI's
        // scene list is the case that caught this.
        f.pad(&self.0)
    }
}

/// Why a scene ID was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SceneIdError {
    /// Empty, or nothing but whitespace.
    Blank,
    /// Contains `/` or `\`, so it is a path, not a name.
    PathSeparator,
    /// `.` or `..`.
    RelativeDirectory,
    /// Contains a control character.
    ControlCharacter,
}

impl fmt::Display for SceneIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SceneIdError::Blank => f.write_str("scene id is blank"),
            SceneIdError::PathSeparator => {
                f.write_str("scene id contains a path separator; it names a scene, not a file")
            }
            SceneIdError::RelativeDirectory => f.write_str("scene id is `.` or `..`"),
            SceneIdError::ControlCharacter => f.write_str("scene id contains a control character"),
        }
    }
}

/// Which TTS implementation speaks a line (D-023).
///
/// A string rather than an enum: the set of providers is a deployment
/// question, not a domain one, and `spoonstill-core` must not learn the name
/// of an SDK (D-010).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProviderId(pub String);

impl fmt::Display for ProviderId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Which voice, in whatever spelling that provider uses.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VoiceId(pub String);

impl fmt::Display for VoiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Provider-specific synthesis knobs, kept opaque on purpose.
///
/// Every provider has its own set — stability, similarity, style, rate — and
/// naming them here would put one vendor's API surface in the domain model
/// (D-010, D-023). What the domain actually needs from them is narrower and is
/// satisfied by an ordered list of pairs:
///
/// - they are **output-affecting**, so they must reach the cache key (D-043),
///   and a key derived from an ordered list of strings is stable across
///   provider versions in a way a struct is not;
/// - they must round-trip through a manifest an operator can hand-edit.
///
/// Ordering is the caller's responsibility to keep canonical — [`Self::sorted`]
/// is how, and the cache-key derivation must use it or two identical settings
/// written in different orders will hash differently.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TtsSettings {
    /// `(name, value)` pairs, as the provider spells them.
    pub values: Vec<(String, String)>,
}

impl TtsSettings {
    /// The same settings in canonical order, for hashing and comparison.
    #[must_use]
    pub fn sorted(&self) -> Self {
        let mut values = self.values.clone();
        values.sort();
        TtsSettings { values }
    }
}

/// Where a scene's narration comes from (D-020).
///
/// Every variant resolves to `(normalized_audio_path, authoritative_duration)`
/// and **nothing downstream branches on which one it was**. Duration maths,
/// motion, the segment render and the concat consume only that pair. The test
/// of this design is that adding a fourth source must not touch the renderer.
///
/// [`AudioSource::Silent`] is a real audio track, not a flag threaded through
/// the render path: title cards and breathing room are real, and making them a
/// special case is how the renderer grows a second code path that only some
/// scenes take.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioSource {
    /// Synthesize `text`.
    Tts {
        /// The line to speak, as written.
        text: String,
        /// Which implementation speaks it. Defaulted per distribution (D-023).
        provider: ProviderId,
        /// Which voice.
        voice: VoiceId,
        /// Provider-specific knobs.
        settings: TtsSettings,
    },
    /// Use a file the operator supplied.
    File {
        /// **The operator's file, exactly as they wrote it.**
        ///
        /// This is never opened for writing and never normalized in place
        /// (D-021). Ingest writes a normalized copy into the cache and probes
        /// that; this path stays untouched, and stays here so a diagnostic can
        /// name what the operator actually pointed at.
        original_path: PathBuf,
    },
    /// No narration: hold the still for exactly this long.
    Silent {
        /// Declared length in seconds. Still ends up measured — the generated
        /// silent track is probed like any other audio (D-021).
        seconds: f64,
    },
}

impl AudioSource {
    /// Short stable word for this source, for logs and the review grid's
    /// source badge (D-051).
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            AudioSource::Tts { .. } => "tts",
            AudioSource::File { .. } => "file",
            AudioSource::Silent { .. } => "silent",
        }
    }
}

/// Motion as the operator asked for it, which is usually "not at all".
///
/// An unspecified field is not a missing value to fill in with a constant: it
/// means *choose deterministically from the seed* (D-035), so that 500 scenes
/// do not all zoom in from the centre and a re-run picks the same variety it
/// picked last time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct MotionRequest {
    /// `zoom_direction` from the manifest, if given.
    pub kind: Option<MotionKind>,
    /// `zoom_anchor` from the manifest, if given.
    pub anchor: Option<Anchor>,
}

/// One row exactly as written, before anything is checked.
///
/// Every field is optional because a manifest cell can be blank and a
/// convention-mode pairing can come up short — the whole point of this stage
/// is to turn "optional and possibly contradictory" into "exactly one of
/// three, or a list of problems".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneDraft {
    /// Scene ID as written: the stem in convention mode, the `image` column's
    /// stem or an explicit ID in manifest mode.
    pub id: String,
    /// `image` — the still. Required.
    pub image: Option<String>,
    /// `text` — narrate this with TTS.
    pub text: Option<String>,
    /// `audio_file` — use this recording.
    pub audio: Option<String>,
    /// `duration` — hold silently for this many seconds.
    pub duration: Option<f64>,
    /// `voice` — only meaningful alongside `text`.
    pub voice: Option<String>,
    /// `zoom_direction`, as written.
    pub zoom_direction: Option<String>,
    /// `zoom_anchor`, as written.
    pub zoom_anchor: Option<String>,
    /// `caption` — the words to burn on screen, when they are not the same
    /// thing as the narration (D-106).
    ///
    /// **Not an audio source**, and deliberately outside D-020's exactly-one
    /// rule: a scene narrated by a recording has no `text`, and without this
    /// column it could never carry a subtitle. A scene that *is* narrated by
    /// `text` needs nothing here — the script is the caption.
    pub caption: Option<String>,
}

/// A row that passed the pure rules: it has an image, exactly one audio
/// source, and motion that parses.
///
/// Paths here are still **as the operator wrote them**. They have not been
/// resolved, contained, or probed — that is the application layer's job, using
/// [`crate::path_safety::resolve_within`]. Holding the unresolved form is
/// deliberate: a diagnostic that echoes a canonical path back at an operator
/// who typed a relative one is harder to act on, not easier.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneSpec {
    /// Validated ID.
    pub id: SceneId,
    /// The still, as written.
    pub image: PathBuf,
    /// Exactly one source (D-020).
    pub source: AudioSource,
    /// Motion, where the operator expressed a preference.
    pub motion: MotionRequest,
    /// What to put on screen if the project burns subtitles (D-106).
    ///
    /// Resolved here rather than at render time so that one rule decides it in
    /// one place: an explicit `caption`, else the spoken script, else nothing.
    /// A scene with a supplied recording and no `caption` has no subtitle, and
    /// that is a fact the operator can be told at validation rather than
    /// discovering in the finished film.
    pub caption: Option<String>,
}

/// Everything wrong with one project, in one list.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    /// Which scene, when the problem belongs to one. `None` for problems
    /// about the project as a whole.
    pub scene: Option<SceneId>,
    /// What is wrong.
    pub kind: ProblemKind,
}

impl Problem {
    /// A problem attached to a scene.
    #[must_use]
    pub fn in_scene(id: SceneId, kind: ProblemKind) -> Self {
        Problem {
            scene: Some(id),
            kind,
        }
    }

    /// A problem about the project rather than a scene.
    #[must_use]
    pub const fn in_project(kind: ProblemKind) -> Self {
        Problem { scene: None, kind }
    }
}

impl Problem {
    /// How much attention this deserves.
    ///
    /// A property of the kind, not of the caller: whether an unpaired audio
    /// file stops a render is a domain question with one answer, and letting
    /// each call site decide is how the same condition ends up fatal in the
    /// CLI and ignored in the shell.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        self.kind.severity()
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.scene {
            Some(id) => write!(f, "scene {id}: {}", self.kind),
            None => write!(f, "project: {}", self.kind),
        }
    }
}

/// What is wrong with a row, or with the project.
///
/// Two variants — [`ProblemKind::Path`] and [`ProblemKind::NotUsableMedia`] —
/// are never produced by this module. They exist so the application layer's
/// filesystem and probe findings land in the *same* list as the pure ones, and
/// the operator gets one report rather than one per stage.
#[derive(Debug, Clone, PartialEq)]
pub enum ProblemKind {
    /// The scene ID itself is unusable.
    UnusableId(SceneIdError),
    /// Two scenes claim the same ID — a copy-pasted manifest row.
    ///
    /// Convention mode reports [`ProblemKind::AmbiguousScene`] instead, which
    /// can name the files and say which slot they are competing for.
    DuplicateId,
    /// Files left part-way through a rename that was interrupted (D-121).
    ///
    /// `arrange` moves a scene's files to `.arranging-…` names and back, and a
    /// run that dies in between leaves them there — invisible to the folder
    /// scan, which ignores dotfiles, so the scenes simply vanish. Reported
    /// rather than passed over in silence: "1566 scenes, no problems" over a
    /// project that had 2000 is the most misleading thing this program can say.
    InterruptedRename {
        /// How many files are waiting to be put back.
        files: usize,
    },
    /// One scene, two files claiming the same job (D-111).
    ///
    /// `001.jpg` beside `001.png`, or `001.wav` beside `001.mp3`. The folder
    /// scan groups by stem, so both are candidates for one scene and there is
    /// nothing in the folder that says which the operator meant. Guessing is
    /// how a project renders 500 scenes of the wrong thing — the same reason
    /// [`ProblemKind::ConflictingAudioSources`] exists — so this is reported
    /// with the candidates named, and the operator removes or renames one.
    AmbiguousScene {
        /// Which job they compete for: `"image"`, `"narration"` or `"text"`.
        slot: &'static str,
        /// Every candidate, in sorted order so the message is stable.
        candidates: Vec<String>,
    },
    /// No `image` cell.
    MissingImage,
    /// None of `text`, `audio_file`, `duration` (D-020).
    NoAudioSource,
    /// More than one of them (D-020). Guessing which one the operator meant is
    /// how a project renders 500 scenes of the wrong thing.
    ConflictingAudioSources {
        /// The cells that were filled in, in manifest column order.
        declared: Vec<&'static str>,
    },
    /// `text` was present but held only whitespace. That is not a narration,
    /// and it is not a silent scene either — say so rather than synthesizing
    /// an empty utterance.
    BlankText,
    /// `duration` is zero, negative, or not a number (D-021).
    DurationNotPositive {
        /// The value as parsed.
        seconds: f64,
    },
    /// `duration` is beyond [`MAX_SCENE_SECONDS`] (D-021).
    DurationAbsurd {
        /// The value as parsed.
        seconds: f64,
    },
    /// `zoom_direction` is not one of [`MotionKind::ALL`].
    UnknownZoomDirection {
        /// The cell, as written.
        value: String,
    },
    /// `zoom_anchor` is not one of [`Anchor::ALL`].
    UnknownZoomAnchor {
        /// The cell, as written.
        value: String,
    },
    /// A path could not be resolved inside the project (D-054). Produced by
    /// the application layer.
    Path {
        /// Which cell: `"image"` or `"audio_file"`.
        field: &'static str,
        /// The path as written — safe to echo, it is the operator's own text.
        value: String,
        /// Why it was refused.
        error: PathError,
    },
    /// A file exists but is not the media it claims to be. Produced by the
    /// application layer, from a probe — extensions are a hint, not evidence.
    NotUsableMedia {
        /// Which cell.
        field: &'static str,
        /// The path as written.
        value: String,
        /// What the probe said, already mapped to a human cause (D-052).
        detail: String,
    },
    /// The project resolved to no scenes at all.
    NoScenes,
    /// The tooling a check needs is not on this machine (D-103).
    ///
    /// **Project-level, and deliberately not per scene.** A probe that cannot
    /// start says nothing whatever about the operator's photographs, and
    /// reporting it once per file turns one installable fact into five hundred
    /// errors that each appear to be about a different image. Produced by the
    /// application layer, before any file is probed.
    ToolingMissing {
        /// What is missing, what to say about it, and whether the window can
        /// install it (D-105). Not a sentence any more: a sentence cannot be
        /// pressed, and pressing it is the fix.
        remedy: Remedy,
    },
    /// A project-level setting in `project.yaml` is unusable. Produced by the
    /// application layer.
    UnusableSetting {
        /// The key, as it appears in the file.
        field: &'static str,
        /// The value, as written.
        value: String,
        /// What would have been accepted.
        expected: &'static str,
    },
    /// A narration or text file in the folder pairs with no image (D-050).
    ///
    /// A **warning**, not an error: it is far more often a leftover take than
    /// a scene the operator lost. Reported rather than skipped silently,
    /// because "my scene 12 never rendered" is otherwise unanswerable.
    UnpairedFile {
        /// The file, relative to the project root.
        value: String,
    },
    /// Every still is smaller than the frame it renders into, so the film is
    /// an upscale of the photographs (F-13).
    ///
    /// A **warning**. Upscaling is a legitimate choice — an operator who wants
    /// a 1080p film from 768px artwork is not making a mistake, they are
    /// making a trade — and D-089's rule is that a refusal must be something
    /// the operator can act on. What was wrong was the silence: 699 scenes of
    /// the author's own films were enlarged 1.4x before the Ken Burns zoom
    /// took another 1.12x on top, and `still validate` said "no problems".
    ///
    /// **Project-level, and deliberately not per scene**, on the same reasoning
    /// as [`ProblemKind::ToolingMissing`]: 699 identical lines about 699
    /// photographs state one fact 699 times, and the fix is one setting rather
    /// than 699 new photographs. The smallest is named so it can be found.
    UndersizedSources {
        /// How many stills do not cover the frame.
        scenes: usize,
        /// How many stills were measured. `scenes` of `total`.
        total: usize,
        /// The id of the scene whose still is furthest from covering it.
        smallest: String,
        /// That still's width in display pixels.
        width: u32,
        /// That still's height in display pixels.
        height: u32,
        /// The output frame's width.
        out_width: u32,
        /// The output frame's height.
        out_height: u32,
        /// The largest short edge at which **every** still renders without
        /// being enlarged, or `None` when no legal size does.
        native_short_edge: Option<u32>,
    },
    /// A scene's caption uses characters no bundled font can draw (D-157).
    ///
    /// A **warning**, and for the same reason as
    /// [`ProblemKind::UndersizedSources`]: the film still renders, and what is
    /// wrong is that nobody was told. Those characters burn into the picture as
    /// empty boxes, which is irreversible, and the operator finds out by
    /// watching the finished film.
    ///
    /// **Project-level and counted, not one line per scene.** A caption in an
    /// unbundled script is unbundled for every scene that uses it, so the fix
    /// is one decision rather than N.
    UndrawableCaption {
        /// How many scenes have caption text with such characters.
        scenes: usize,
        /// The id of the first such scene, so it can be found.
        first: String,
        /// The characters themselves, deduplicated, first-seen order, **one
        /// space between each**.
        ///
        /// Spaced because these are by definition characters the operator's
        /// own font may not draw either, and several scripts here combine: run
        /// together, a lone Bengali matra stacks onto the letter before it and
        /// the list reads as three characters instead of five. And printed
        /// plainly rather than through `{:?}`, which escapes a combining mark
        /// to `\u{9be}` — a message about characters that cannot be shown,
        /// which does not show them (D-150).
        characters: String,
        /// The scripts that *are* drawn, named so the message ends with
        /// something true rather than only with something wrong.
        drawn: String,
    },
    /// An image in the folder appears in no manifest row (D-056).
    ///
    /// A **warning**. The manifest is the complete list of scenes when it
    /// exists, so this image will not be rendered — which is fine when it is a
    /// source asset and wrong when it is a row someone forgot to add. Only the
    /// operator can tell those apart, so they are told.
    UnlistedImage {
        /// The file, relative to the project root.
        value: String,
    },
}

impl ProblemKind {
    /// Whether this stops a render.
    ///
    /// Two of the three warnings are the unresolved-input cases of D-050:
    /// something in the folder was not used. The third says the photographs
    /// are smaller than the frame, which is a trade and not a mistake.
    /// Everything else is an error, because
    /// everything else means a scene the operator asked for cannot be built —
    /// and rendering 499 of 500 scenes without saying so is the failure this
    /// whole validation stage exists to prevent.
    #[must_use]
    pub const fn severity(&self) -> Severity {
        match self {
            ProblemKind::UnpairedFile { .. }
            | ProblemKind::UnlistedImage { .. }
            | ProblemKind::UndersizedSources { .. }
            | ProblemKind::UndrawableCaption { .. } => Severity::Warn,
            _ => Severity::Error,
        }
    }
}

impl fmt::Display for ProblemKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProblemKind::UnusableId(e) => write!(f, "{e}"),
            ProblemKind::DuplicateId => f.write_str("duplicate scene id"),
            ProblemKind::InterruptedRename { files } => write!(
                f,
                "{files} file{} are part-way through a rename that did not finish, \
                 so the scenes they belong to are missing — `still remove` or \
                 `still move` on this project puts them back",
                if *files == 1 { " is" } else { "s" }
            ),
            ProblemKind::AmbiguousScene { slot, candidates } => write!(
                f,
                "{} files claim to be this scene's {slot} ({}) — remove or \
                 rename all but one",
                candidates.len(),
                candidates.join(", ")
            ),
            ProblemKind::MissingImage => f.write_str("no image"),
            ProblemKind::NoAudioSource => f.write_str(
                "no audio source — give exactly one of `text`, `audio_file` or `duration`",
            ),
            ProblemKind::ConflictingAudioSources { declared } => write!(
                f,
                "{} audio sources ({}) — give exactly one",
                declared.len(),
                declared.join(", ")
            ),
            ProblemKind::BlankText => f.write_str("`text` is blank"),
            ProblemKind::DurationNotPositive { seconds } => {
                write!(f, "`duration` must be greater than zero, got {seconds}")
            }
            ProblemKind::DurationAbsurd { seconds } => write!(
                f,
                "`duration` of {seconds}s is beyond the {MAX_SCENE_SECONDS}s limit"
            ),
            ProblemKind::UnknownZoomDirection { value } => {
                write!(f, "unknown `zoom_direction` {value:?}")
            }
            ProblemKind::UnknownZoomAnchor { value } => {
                write!(f, "unknown `zoom_anchor` {value:?}")
            }
            ProblemKind::Path {
                field,
                value,
                error,
            } => write!(f, "`{field}` {value:?}: {error}"),
            ProblemKind::NotUsableMedia {
                field,
                value,
                detail,
            } => write!(f, "`{field}` {value:?}: {detail}"),
            ProblemKind::NoScenes => {
                f.write_str("no scenes — no manifest rows and no image/narration pairs found")
            }
            ProblemKind::ToolingMissing { remedy } => write!(f, "{remedy}"),
            ProblemKind::UnusableSetting {
                field,
                value,
                expected,
            } => write!(f, "`{field}`: {value:?} is not {expected}"),
            ProblemKind::UnpairedFile { value } => {
                write!(
                    f,
                    "{value:?} pairs with no image, so it is not part of any scene"
                )
            }
            ProblemKind::UndersizedSources {
                scenes,
                total,
                smallest,
                width,
                height,
                out_width,
                out_height,
                native_short_edge,
            } => {
                write!(
                    f,
                    "{scenes} of {total} still{} smaller than the \
                     {out_width}x{out_height} frame and will be enlarged to \
                     fill it — the smallest is scene {smallest} at \
                     {width}x{height}; ",
                    if *scenes == 1 { " is" } else { "s are" }
                )?;
                match native_short_edge {
                    Some(edge) => write!(
                        f,
                        "`--short-edge {edge}` renders every scene at its own detail"
                    ),
                    None => f.write_str("no output size renders them all at their own detail"),
                }
            }
            ProblemKind::UndrawableCaption {
                scenes,
                first,
                characters,
                drawn,
            } => write!(
                f,
                "{scenes} scene{} caption characters no bundled font can draw, \
                 so they burn into the picture as empty boxes — the first is \
                 scene {first}, which uses {characters}; spoonstill draws {drawn}",
                if *scenes == 1 { " has" } else { "s have" }
            ),
            ProblemKind::UnlistedImage { value } => write!(
                f,
                "{value:?} is in the folder but in no manifest row, so it will not be rendered"
            ),
        }
    }
}

/// The outcome of validating a whole project: what survived, and everything
/// wrong.
///
/// Both halves are populated. A caller that only wants to render checks
/// [`Validation::is_clean`]; `still validate` prints [`Validation::problems`]
/// whether or not any scene survived, because the operator wants the full list
/// on the first run (plan.md §M2).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Validation {
    /// Rows that passed, in input order.
    pub scenes: Vec<SceneSpec>,
    /// Every problem found, in input order, and in a fixed order within a
    /// scene. Stable ordering matters: an operator diffing two runs should see
    /// only what they changed.
    pub problems: Vec<Problem>,
}

impl Validation {
    /// No problems at all — not even a warning.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.problems.is_empty()
    }

    /// Whether anything found stops a render.
    ///
    /// **This, not [`Self::is_clean`], is the render gate.** A warning means
    /// something in the folder went unused (D-050); it does not mean the
    /// scenes that did resolve are wrong, and refusing to render 500 good
    /// scenes over one stray take would train operators to ignore the output.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.problems
            .iter()
            .any(|p| p.severity() >= Severity::Error)
    }

    /// Every problem that stops the render.
    pub fn errors(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity() >= Severity::Error)
    }

    /// Every problem that does not.
    pub fn warnings(&self) -> impl Iterator<Item = &Problem> {
        self.problems
            .iter()
            .filter(|p| p.severity() < Severity::Error)
    }
}

/// The manifest column names, in the order a conflict is reported (D-050).
const SOURCE_COLUMNS: [&str; 3] = ["text", "audio_file", "duration"];

/// Validate one row against the pure rules.
///
/// The default provider and voice are passed in rather than chosen here:
/// which one is right depends on the distribution (D-023), and that is not
/// something the domain model can know.
///
/// # Errors
///
/// Every problem with the row, never just the first.
pub fn validate_draft(
    draft: &SceneDraft,
    default_provider: &ProviderId,
    default_voice: &VoiceId,
) -> Result<SceneSpec, Vec<Problem>> {
    let mut problems = Vec::new();

    // The ID comes first: without it, every other problem in this row is
    // unattributable, and an operator cannot act on "some scene has no image".
    let id = match SceneId::new(&draft.id) {
        Ok(id) => id,
        Err(e) => {
            return Err(vec![Problem::in_project(ProblemKind::UnusableId(e))]);
        }
    };

    let image = match draft.image.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => Some(PathBuf::from(text)),
        _ => {
            problems.push(Problem::in_scene(id.clone(), ProblemKind::MissingImage));
            None
        }
    };

    let source = validate_source(&id, draft, default_provider, default_voice, &mut problems);
    let motion = validate_motion(&id, draft, &mut problems);

    match (image, source) {
        (Some(image), Some(source)) if problems.is_empty() => {
            let caption = resolve_caption(draft, &source);
            Ok(SceneSpec {
                id,
                image,
                source,
                motion,
                caption,
            })
        }
        _ => Err(problems),
    }
}

/// What this scene would say on screen: the explicit `caption`, else the
/// spoken script, else nothing.
///
/// The script is used without the operator asking because it is already the
/// words being said — requiring them to be typed twice is the kind of clerical
/// work D-080 exists to refuse. An explicit `caption` wins because a scene
/// whose subtitle should differ from its narration is a deliberate act.
fn resolve_caption(draft: &SceneDraft, source: &AudioSource) -> Option<String> {
    if let Some(caption) = draft.caption.as_deref().map(str::trim)
        && !caption.is_empty()
    {
        return Some(caption.to_owned());
    }
    match source {
        AudioSource::Tts { text, .. } => Some(text.clone()),
        AudioSource::File { .. } | AudioSource::Silent { .. } => None,
    }
}

/// D-020's rule: exactly one of `text` | `audio_file` | `duration`.
///
/// Zero is an error and two is an error, and neither is recoverable by
/// guessing — this returns `None` and records why, rather than picking a
/// winner. Rejected explicitly: `MoneyPrinterTurbo`'s `is_no_voice()` sentinel,
/// which infers a silent scene from a magic voice name and an estimated text
/// length (D-020).
fn validate_source(
    id: &SceneId,
    draft: &SceneDraft,
    default_provider: &ProviderId,
    default_voice: &VoiceId,
    problems: &mut Vec<Problem>,
) -> Option<AudioSource> {
    let text = draft.text.as_deref().filter(|t| !t.is_empty());
    let audio = draft
        .audio
        .as_deref()
        .map(str::trim)
        .filter(|a| !a.is_empty());
    let duration = draft.duration;

    let declared: Vec<&'static str> = SOURCE_COLUMNS
        .into_iter()
        .zip([text.is_some(), audio.is_some(), duration.is_some()])
        .filter_map(|(name, present)| present.then_some(name))
        .collect();

    match declared.len() {
        0 => {
            problems.push(Problem::in_scene(id.clone(), ProblemKind::NoAudioSource));
            return None;
        }
        1 => {}
        _ => {
            problems.push(Problem::in_scene(
                id.clone(),
                ProblemKind::ConflictingAudioSources { declared },
            ));
            return None;
        }
    }

    if let Some(text) = text {
        // Present but blank is its own mistake, and a different one from
        // absent: the operator meant to narrate and the cell lost its content.
        if text.trim().is_empty() {
            problems.push(Problem::in_scene(id.clone(), ProblemKind::BlankText));
            return None;
        }
        let voice = draft
            .voice
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map_or_else(|| default_voice.clone(), |v| VoiceId(v.to_owned()));
        return Some(AudioSource::Tts {
            text: text.to_owned(),
            provider: default_provider.clone(),
            voice,
            settings: TtsSettings::default(),
        });
    }

    if let Some(audio) = audio {
        return Some(AudioSource::File {
            original_path: PathBuf::from(audio),
        });
    }

    let seconds = duration.expect("exactly one source was declared, and it is `duration`");
    if !seconds.is_finite() || seconds <= 0.0 {
        problems.push(Problem::in_scene(
            id.clone(),
            ProblemKind::DurationNotPositive { seconds },
        ));
        return None;
    }
    if seconds > MAX_SCENE_SECONDS {
        problems.push(Problem::in_scene(
            id.clone(),
            ProblemKind::DurationAbsurd { seconds },
        ));
        return None;
    }
    Some(AudioSource::Silent { seconds })
}

/// Parse the two motion cells. Absent is fine and means "seed it" (D-035);
/// present-but-unparseable is a problem, because silently seeding a scene
/// whose direction the operator specified is exactly the kind of quiet
/// substitution that makes a 500-scene render wrong in a way nobody notices.
fn validate_motion(id: &SceneId, draft: &SceneDraft, problems: &mut Vec<Problem>) -> MotionRequest {
    let mut request = MotionRequest::default();

    if let Some(value) = draft.zoom_direction.as_deref().map(str::trim)
        && !value.is_empty()
    {
        match MotionKind::parse(value) {
            Some(kind) => request.kind = Some(kind),
            None => problems.push(Problem::in_scene(
                id.clone(),
                ProblemKind::UnknownZoomDirection {
                    value: value.to_owned(),
                },
            )),
        }
    }

    if let Some(value) = draft.zoom_anchor.as_deref().map(str::trim)
        && !value.is_empty()
    {
        match Anchor::parse(value) {
            Some(anchor) => request.anchor = Some(anchor),
            None => problems.push(Problem::in_scene(
                id.clone(),
                ProblemKind::UnknownZoomAnchor {
                    value: value.to_owned(),
                },
            )),
        }
    }

    request
}

/// Validate every row, plus the rules that only exist across rows.
///
/// Nothing here stops at the first failure. A row with three problems
/// contributes three, a project with 500 broken rows reports 500, and the
/// caller prints the lot.
#[must_use]
pub fn validate_drafts(
    drafts: &[SceneDraft],
    default_provider: &ProviderId,
    default_voice: &VoiceId,
) -> Validation {
    let mut validation = Validation::default();

    if drafts.is_empty() {
        validation
            .problems
            .push(Problem::in_project(ProblemKind::NoScenes));
        return validation;
    }

    for draft in drafts {
        match validate_draft(draft, default_provider, default_voice) {
            Ok(scene) => validation.scenes.push(scene),
            Err(problems) => validation.problems.extend(problems),
        }
    }

    // Duplicate IDs, reported against the *second* and later occurrences so
    // the operator is pointed at the row they probably added. Only accepted
    // scenes are compared: a row that already failed has been reported.
    let mut seen: Vec<SceneId> = Vec::new();
    let mut duplicates: Vec<Problem> = Vec::new();
    validation.scenes.retain(|scene| {
        if seen.contains(&scene.id) {
            duplicates.push(Problem::in_scene(
                scene.id.clone(),
                ProblemKind::DuplicateId,
            ));
            false
        } else {
            seen.push(scene.id.clone());
            true
        }
    });
    validation.problems.extend(duplicates);

    validation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::Severity;

    fn provider() -> ProviderId {
        ProviderId("elevenlabs".into())
    }

    fn voice() -> VoiceId {
        VoiceId("default".into())
    }

    fn draft(id: &str) -> SceneDraft {
        SceneDraft {
            id: id.to_owned(),
            image: Some("img/001.jpg".to_owned()),
            ..SceneDraft::default()
        }
    }

    fn validate(draft: &SceneDraft) -> Result<SceneSpec, Vec<Problem>> {
        validate_draft(draft, &provider(), &voice())
    }

    fn kinds(problems: &[Problem]) -> Vec<&ProblemKind> {
        problems.iter().map(|p| &p.kind).collect()
    }

    #[test]
    fn text_alone_is_a_tts_scene() {
        let mut d = draft("001");
        d.text = Some("hello".to_owned());

        let scene = validate(&d).expect("one source is valid");
        assert_eq!(
            scene.source,
            AudioSource::Tts {
                text: "hello".to_owned(),
                provider: provider(),
                voice: voice(),
                settings: TtsSettings::default(),
            }
        );
    }

    #[test]
    fn audio_alone_is_a_file_scene_and_keeps_the_operators_spelling() {
        let mut d = draft("002");
        d.audio = Some("audio/take 3 ünïcode.mp3".to_owned());

        let scene = validate(&d).expect("one source is valid");
        assert_eq!(
            scene.source,
            AudioSource::File {
                original_path: PathBuf::from("audio/take 3 ünïcode.mp3"),
            },
            "D-021: the operator's original path is carried, not rewritten"
        );
    }

    #[test]
    fn duration_alone_is_a_silent_scene() {
        let mut d = draft("003");
        d.duration = Some(4.5);

        let scene = validate(&d).expect("one source is valid");
        assert_eq!(scene.source, AudioSource::Silent { seconds: 4.5 });
    }

    /// D-020, half one. Zero sources is an error, not a default.
    #[test]
    fn validation_rejects_a_scene_with_no_source() {
        let problems = validate(&draft("001")).expect_err("no source is invalid");
        assert_eq!(kinds(&problems), vec![&ProblemKind::NoAudioSource]);
        assert_eq!(problems[0].scene.as_ref().map(SceneId::as_str), Some("001"));
    }

    /// D-020, half two. Two sources is an error, and the message names both —
    /// picking a winner is how a project renders the wrong thing 500 times.
    #[test]
    fn validation_rejects_a_scene_with_two_sources() {
        let mut d = draft("001");
        d.text = Some("hello".to_owned());
        d.audio = Some("a.mp3".to_owned());

        let problems = validate(&d).expect_err("two sources is invalid");
        assert_eq!(
            kinds(&problems),
            vec![&ProblemKind::ConflictingAudioSources {
                declared: vec!["text", "audio_file"],
            }]
        );
        assert!(problems[0].to_string().contains("give exactly one"));
    }

    #[test]
    fn validation_rejects_all_three_sources_and_names_all_three() {
        let mut d = draft("001");
        d.text = Some("hello".to_owned());
        d.audio = Some("a.mp3".to_owned());
        d.duration = Some(3.0);

        let problems = validate(&d).expect_err("three sources is invalid");
        assert_eq!(
            kinds(&problems),
            vec![&ProblemKind::ConflictingAudioSources {
                declared: vec!["text", "audio_file", "duration"],
            }]
        );
    }

    /// plan.md §M2: every problem at once. One row, three mistakes, three
    /// entries — the operator fixes them in one pass, not three runs.
    #[test]
    fn every_problem_in_a_row_is_reported_at_once() {
        let d = SceneDraft {
            id: "007".to_owned(),
            image: None,
            zoom_direction: Some("sideways".to_owned()),
            zoom_anchor: Some("northwest".to_owned()),
            ..SceneDraft::default()
        };

        let problems = validate(&d).expect_err("three problems");
        assert_eq!(
            kinds(&problems),
            vec![
                &ProblemKind::MissingImage,
                &ProblemKind::NoAudioSource,
                &ProblemKind::UnknownZoomDirection {
                    value: "sideways".to_owned()
                },
                &ProblemKind::UnknownZoomAnchor {
                    value: "northwest".to_owned()
                },
            ]
        );
        for problem in &problems {
            assert_eq!(problem.scene.as_ref().map(SceneId::as_str), Some("007"));
        }
    }

    /// D-021: reject zero, and reject absurd, with the scene ID attached.
    #[test]
    fn validation_rejects_unusable_durations() {
        for (seconds, expected) in [
            (0.0, ProblemKind::DurationNotPositive { seconds: 0.0 }),
            (-1.0, ProblemKind::DurationNotPositive { seconds: -1.0 }),
            (
                f64::NAN,
                ProblemKind::DurationNotPositive { seconds: f64::NAN },
            ),
            (
                f64::INFINITY,
                ProblemKind::DurationNotPositive {
                    seconds: f64::INFINITY,
                },
            ),
            (90_000.0, ProblemKind::DurationAbsurd { seconds: 90_000.0 }),
        ] {
            let mut d = draft("001");
            d.duration = Some(seconds);
            let problems = validate(&d).expect_err("unusable duration");

            assert_eq!(problems.len(), 1, "duration {seconds}");
            assert_eq!(
                core::mem::discriminant(&problems[0].kind),
                core::mem::discriminant(&expected),
                "duration {seconds}"
            );
            assert_eq!(problems[0].scene.as_ref().map(SceneId::as_str), Some("001"));
        }
    }

    #[test]
    fn a_blank_text_cell_is_not_silence() {
        let mut d = draft("001");
        d.text = Some("   ".to_owned());

        let problems = validate(&d).expect_err("blank text is invalid");
        assert_eq!(kinds(&problems), vec![&ProblemKind::BlankText]);
    }

    #[test]
    fn motion_cells_are_optional_and_absent_means_seeded() {
        let mut d = draft("001");
        d.duration = Some(3.0);

        let scene = validate(&d).expect("valid");
        assert_eq!(scene.motion, MotionRequest::default());
        assert!(scene.motion.kind.is_none(), "D-035 seeds it later");
    }

    #[test]
    fn motion_cells_are_parsed_when_given() {
        let mut d = draft("001");
        d.duration = Some(3.0);
        d.zoom_direction = Some("zoom_in".to_owned());
        d.zoom_anchor = Some("top_left".to_owned());

        let scene = validate(&d).expect("valid");
        assert_eq!(scene.motion.kind, Some(MotionKind::ZoomIn));
        assert_eq!(scene.motion.anchor, Some(Anchor::NorthWest));
    }

    /// Printed in a column, so it has to honour a width. `write_str` does not.
    #[test]
    fn a_scene_id_pads_to_a_column_width() {
        let id = SceneId::new("001").expect("valid");
        assert_eq!(format!("[{id:<7}]"), "[001    ]");
        assert_eq!(format!("[{id:>7}]"), "[    001]");
    }

    #[test]
    fn a_scene_id_may_contain_spaces_and_unicode_but_not_a_path() {
        assert_eq!(
            SceneId::new("ünïcode spaced 名前")
                .expect("D-052: this is the normal case")
                .as_str(),
            "ünïcode spaced 名前"
        );
        assert_eq!(SceneId::new("  001  ").expect("trimmed").as_str(), "001");
        assert_eq!(SceneId::new(""), Err(SceneIdError::Blank));
        assert_eq!(SceneId::new("   "), Err(SceneIdError::Blank));
        assert_eq!(SceneId::new("a/b"), Err(SceneIdError::PathSeparator));
        assert_eq!(SceneId::new(r"a\b"), Err(SceneIdError::PathSeparator));
        assert_eq!(SceneId::new(".."), Err(SceneIdError::RelativeDirectory));
        assert_eq!(SceneId::new("a\nb"), Err(SceneIdError::ControlCharacter));
    }

    #[test]
    fn an_empty_project_is_a_problem_rather_than_an_empty_render() {
        let validation = validate_drafts(&[], &provider(), &voice());
        assert!(!validation.is_clean());
        assert_eq!(kinds(&validation.problems), vec![&ProblemKind::NoScenes]);
    }

    #[test]
    fn duplicate_ids_are_reported_and_the_second_row_is_dropped() {
        let mut first = draft("001");
        first.duration = Some(1.0);
        let mut second = draft("001");
        second.duration = Some(2.0);
        let mut third = draft("002");
        third.duration = Some(3.0);

        let validation = validate_drafts(&[first, second, third], &provider(), &voice());

        assert_eq!(kinds(&validation.problems), vec![&ProblemKind::DuplicateId]);
        assert_eq!(
            validation
                .scenes
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            vec!["001", "002"],
            "the first occurrence survives; the duplicate is reported, not merged"
        );
    }

    /// A whole project's worth of problems, in input order. This is the shape
    /// `still validate` prints.
    #[test]
    fn problems_from_many_rows_keep_input_order() {
        let mut ok = draft("001");
        ok.duration = Some(1.0);
        let bad_a = draft("002");
        let mut bad_b = draft("003");
        bad_b.text = Some("hi".to_owned());
        bad_b.duration = Some(1.0);

        let validation = validate_drafts(&[ok, bad_a, bad_b], &provider(), &voice());

        assert_eq!(validation.scenes.len(), 1);
        assert_eq!(
            validation
                .problems
                .iter()
                .map(|p| p.scene.as_ref().map(SceneId::as_str))
                .collect::<Vec<_>>(),
            vec![Some("002"), Some("003")]
        );
    }

    #[test]
    fn a_clean_project_says_so() {
        let mut a = draft("001");
        a.text = Some("hello".to_owned());
        let mut b = draft("002");
        b.duration = Some(2.0);

        let validation = validate_drafts(&[a, b], &provider(), &voice());
        assert!(validation.is_clean());
        assert_eq!(validation.scenes.len(), 2);
    }

    /// The renderer must never branch on where audio came from (D-020). The
    /// cheapest standing proof is that the badge is the only thing that
    /// differs — everything else about a scene is source-independent.
    #[test]
    fn every_source_kind_has_a_stable_badge() {
        let mut tts = draft("001");
        tts.text = Some("hi".to_owned());
        let mut file = draft("002");
        file.audio = Some("a.mp3".to_owned());
        let mut silent = draft("003");
        silent.duration = Some(1.0);

        let badges: Vec<&str> = [tts, file, silent]
            .iter()
            .map(|d| validate(d).expect("valid").source.kind())
            .collect();
        assert_eq!(badges, vec!["tts", "file", "silent"]);
    }

    /// The render gate is `has_errors`, not `is_clean`. A stray take in the
    /// folder must not stop 500 good scenes (D-050) — and an operator who is
    /// blocked by a warning learns to ignore warnings.
    #[test]
    fn a_warning_does_not_stop_a_render_but_an_error_does() {
        let warning = Problem::in_project(ProblemKind::UnpairedFile {
            value: "take-2.mp3".to_owned(),
        });
        let error = Problem::in_project(ProblemKind::NoScenes);

        assert_eq!(warning.severity(), Severity::Warn);
        assert_eq!(error.severity(), Severity::Error);

        let validation = Validation {
            scenes: Vec::new(),
            problems: vec![warning],
        };
        assert!(!validation.is_clean(), "the warning is still reported");
        assert!(!validation.has_errors(), "but it does not stop the render");
        assert_eq!(validation.warnings().count(), 1);
        assert_eq!(validation.errors().count(), 0);
    }

    #[test]
    fn tts_settings_sort_into_a_canonical_order_for_the_cache_key() {
        let a = TtsSettings {
            values: vec![
                ("style".into(), "0.2".into()),
                ("rate".into(), "1.0".into()),
            ],
        };
        let b = TtsSettings {
            values: vec![
                ("rate".into(), "1.0".into()),
                ("style".into(), "0.2".into()),
            ],
        };
        assert_ne!(a, b, "written order differs");
        assert_eq!(a.sorted(), b.sorted(), "D-043: the key must not");
    }
}
