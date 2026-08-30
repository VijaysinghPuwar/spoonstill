//! Domain model for spoonstill.
//!
//! This crate depends on nothing concrete (D-010). It defines the vocabulary
//! every other crate speaks: projects, scenes, audio sources, motion specs and
//! cache keys — plus the traits that infrastructure crates implement.
//!
//! The rule that keeps it honest: if a type here needs to *do* I/O, it is the
//! wrong type. Model the result of the I/O, and put the doing behind a trait.

/// Machine-owned state directory, relative to the project root (D-013).
///
/// Everything spoonstill writes about a render lives here. Deleting it must
/// lose progress and nothing else — the manifest plus the cache can always
/// rebuild it.
///
/// This is the single constant D-073 refers to: the whole coupling between the
/// project's name and its on-disk footprint is this one string.
pub const STATE_DIR: &str = ".spoonstill";

pub mod captions;
pub mod diagnostics;
pub mod geometry;
pub mod hash;
pub mod motion;
pub mod path_safety;
pub mod project;
pub mod remedy;
pub mod timing;

pub use captions::{Cue, Placement, Rgba, SubtitleSpec, SubtitleTheme, ThemeStyle, Weight};
pub use diagnostics::{Diagnostics, Event, Severity};
pub use geometry::{Aspect, GeometryError, OutputSpec, PRESCALE_FACTOR, SourceGeometry};
pub use motion::{Anchor, MotionKind, MotionSpec, build_filter};
pub use path_safety::{PathError, RealPath, resolve_within};
pub use project::{
    AudioSource, MotionRequest, Problem, ProblemKind, ProviderId, SceneDraft, SceneId, SceneSpec,
    TtsSettings, Validation, VoiceId, validate_draft, validate_drafts,
};
pub use remedy::Remedy;
pub use timing::{SAMPLE_RATE, duration_for_frames, frames_for_duration, samples_for_frames};

/// Human-owned manifest, relative to the project root (D-013).
///
/// **The renderer never writes to this file.** It is an input. Hand-editable,
/// diffable, and copyable to another machine, which is exactly why machine
/// state lives in [`STATE_DIR`] instead.
pub const MANIFEST_FILE: &str = "project.yaml";

#[cfg(test)]
mod tests {
    use super::*;

    /// D-013 + D-073. The state directory is a dotfile named for the project,
    /// and it is the only place the name reaches the filesystem.
    #[test]
    fn state_dir_is_a_dotdir_named_for_the_project() {
        assert_eq!(STATE_DIR, ".spoonstill");
        assert!(STATE_DIR.starts_with('.'), "state dir must be hidden");
        assert!(
            !STATE_DIR.contains('/') && !STATE_DIR.contains('\\'),
            "state dir is a single path component, joined by the caller"
        );
    }

    /// D-013. The manifest is an input, and its name is not negotiable —
    /// operators hand-write it and copy it between machines.
    #[test]
    fn manifest_is_yaml_and_not_hidden() {
        assert_eq!(MANIFEST_FILE, "project.yaml");
        assert!(
            !MANIFEST_FILE.starts_with('.'),
            "the manifest is human-owned; hiding it works against D-013"
        );
    }
}
