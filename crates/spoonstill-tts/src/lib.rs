//! Text-to-speech providers behind one trait (D-023).
//!
//! One module per provider. A giant `match` on a provider name string is the
//! mistake this crate is shaped to avoid — adding a provider must not require
//! editing a dispatch table that every other provider also lives in.
//!
//! Defaults differ by distribution: Edge TTS internally (free, and it emits the
//! word-boundary events karaoke captions will need), ElevenLabs BYOK in a sold
//! build — because no reverse-engineered endpoint may be load-bearing in a
//! shipped product. Keys come from `keyring-rs` (D-014) and never leave the
//! machine.

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
