//! Application services — the layer that orchestrates, and the only layer
//! above [`spoonstill_core`] that both the CLI and the Tauri shell share.
//!
//! It owns the two bounded queues of D-044 (TTS is network- and rate-limit
//! bound; rendering is CPU- and RAM-bound, so they never share a pool), the
//! cancellation ladder of D-045, and the concat gate of D-040/D-041.
//!
//! It does not own a terminal and it does not own a webview. Both are adapters
//! over this crate, which is what makes n=500 testable in CI (D-002, D-010).

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
