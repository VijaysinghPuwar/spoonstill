//! Application services — the layer that orchestrates, and the only layer
//! above [`spoonstill_core`] that both the CLI and the Tauri shell share.
//!
//! It owns the two bounded queues of D-044 (TTS is network- and rate-limit
//! bound; rendering is CPU- and RAM-bound, so they never share a pool), the
//! cancellation ladder of D-045, and the concat gate of D-040/D-041.
//!
//! It does not own a terminal and it does not own a webview. Both are adapters
//! over this crate, which is what makes n=500 testable in CI (D-002, D-010).

#![warn(missing_docs)]

pub mod arrange;
pub mod audio;
pub mod capacity;
pub mod diagnostics;
pub mod film;
pub mod formats;
pub mod import;
pub mod ingest;
pub mod pool;
pub mod render;
pub mod subtitles;
pub mod tooling;

pub use audio::{AudioCache, AudioError, ResolvedAudio};
pub use film::{FilmError, FilmEvent, RenderProjectOptions, RenderedFilm, render_project};
pub use formats::{AspectChoice, SizeChoice};
pub use import::{ImportError, MediaCheck, Mode, ProbeCheck, Project, ResolvedScene, Role};
pub use ingest::{IngestError, Ingested, add_media, create_project};
pub use render::{RenderError, RenderSceneOptions, render_scene};
/// Where this machine's activity CSV lives (D-093). Re-exported because the
/// control surfaces reach this crate and stop (D-010).
pub use spoonstill_state::runs::index_path as runs_index_path;
pub use subtitles::{Preview, ThemeChoice};

/// The types the control surface needs, re-exported through the layer it is
/// allowed to depend on.
///
/// D-010 forbids `spoonstill-cli` from reaching past this crate into
/// `spoonstill-media` or `spoonstill-state`, and
/// `spoonstill-cli/tests/architecture.rs` enforces it. That rule is the reason
/// the Tauri shell at M4 will not have to re-derive any of this: whatever the
/// CLI can reach, the shell can reach, through the same door.
/// The voice services, re-exported through the layer the control surface is
/// allowed to depend on (D-010).
///
/// The CLI and the shell both need to *list* voices and to report a provider
/// that is not installed. Neither may reach `spoonstill-tts` directly — that
/// is the same rule that keeps the FFmpeg boundary out of both of them, and
/// `spoonstill-cli/tests/architecture.rs` enforces it.
pub mod tts {
    pub use spoonstill_tts::{
        Availability, Provider, TtsError, Voice, opening, provider, providers,
    };
}

/// Types the control surface needs from the infrastructure layer.
pub mod surface {
    pub use spoonstill_media::Progress;
    pub use spoonstill_media::scene::{Cancel, EncodeSettings, RenderedScene};
    pub use spoonstill_state::logs::LOGS_DIR;
}

/// This crate's package name, resolved at compile time.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

#[cfg(test)]
mod tests {
    use super::*;

    /// D-073 renamed every crate in this workspace. A crate whose `[package]`
    /// name drifts out of the family is a rename that was applied by hand and
    /// missed a spot — which is the exact failure that lost the name once
    /// already. Cheap to assert, so assert it.
    #[test]
    fn crate_is_part_of_the_spoonstill_family() {
        assert!(
            CRATE_NAME.starts_with("spoonstill-"),
            "expected a spoonstill-* crate, found {CRATE_NAME:?}"
        );
    }
}
