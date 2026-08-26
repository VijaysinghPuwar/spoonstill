//! Machine-owned render state, in SQLite under
//! [`spoonstill_core::STATE_DIR`] (D-013).
//!
//! One transaction per state transition, and the legal transitions are a type
//! rather than a set of booleans:
//! `Pending -> Resolving -> Resolved -> Rendering -> Rendered -> Validated`,
//! plus `Failed { reason }` and `Cancelled`.
//!
//! The guarantee that shapes the schema (D-042): failing at scene 147 of 200
//! must never discard the first 146, and a crash must leave either a complete
//! valid segment or nothing that looks valid.

#![warn(missing_docs)]

pub mod logs;

pub use logs::{BundleReport, EnvironmentLine, FileLog, write_bundle};

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
