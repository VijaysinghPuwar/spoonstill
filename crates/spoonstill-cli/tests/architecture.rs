//! D-010's dependency direction, enforced by the compiler's own manifests.
//!
//! > "Required dependency direction — a violation is a build break, not a style
//! > note." — decisions.md D-010
//!
//! plan.md M0 is explicit that a comment is not enforcement. So this test reads
//! every `crates/*/Cargo.toml` in the workspace and checks the real edges. It
//! lives in `spoonstill-cli` because the CLI sits at the top of the graph and a
//! dev-dependency here cannot reach any shipped artifact.
//!
//! Only dependencies that actually ship are checked — `[dependencies]` and
//! `[build-dependencies]`. `[dev-dependencies]` are deliberately exempt: a test
//! harness is not part of the architecture, and `cargo tree -p spoonstill-core`
//! in the M0 exit gate does not report them for a non-test build.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Layer index. A crate may depend only on **strictly lower** layers (D-010).
///
/// ```text
/// React -> Tauri adapter -> spoonstill-app -> spoonstill-core
/// CLI ------------------------^             -> infrastructure traits
/// ```
fn layer(crate_name: &str) -> Option<u8> {
    Some(match crate_name {
        "spoonstill-core" => 0,
        "spoonstill-media" | "spoonstill-tts" | "spoonstill-state" => 1,
        "spoonstill-app" => 2,
        "spoonstill-cli" => 3,
        _ => return None,
    })
}

/// Concrete things `spoonstill-core` must never reach, by substring of the
/// dependency name. This is a denylist on top of the "core has zero
/// dependencies" rule below, so that a future relaxation of that rule — a
/// `serde` here, a `thiserror` there — cannot quietly let a runtime in.
const FORBIDDEN_IN_CORE: &[&str] = &[
    "tauri",      // D-010: core never knows a shell exists
    "wry",        // Tauri's webview, same reason
    "reqwest",    // D-010/D-011: no HTTP client in the domain
    "hyper",      //
    "ureq",       //
    "elevenlabs", // D-023: no TTS SDK
    "azure",      //
    "rusqlite",   // D-013: persistence is spoonstill-state's problem
    "sqlx",       //
    "keyring",    // D-014: credential storage is behind a trait
    "ffmpeg",     // D-011: the process boundary is spoonstill-media
    "duct",       // process spawning of any flavour
    "subprocess", //
];

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is crates/spoonstill-cli
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two levels above the cli manifest")
        .to_path_buf()
}

/// name -> shipped dependency names, for every `crates/*` member.
fn manifests() -> BTreeMap<String, Vec<String>> {
    let crates_dir = workspace_root().join("crates");
    let mut out = BTreeMap::new();

    for entry in std::fs::read_dir(&crates_dir).expect("crates/ is readable") {
        let manifest = entry.expect("readable dir entry").path().join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        let doc: toml::Table = text
            .parse()
            .unwrap_or_else(|e| panic!("parse {}: {e}", manifest.display()));

        let name = doc["package"]["name"]
            .as_str()
            .expect("package.name is a string")
            .to_owned();

        // Shipped edges only. dev-dependencies are not architecture.
        let mut deps = Vec::new();
        for table in ["dependencies", "build-dependencies"] {
            if let Some(t) = doc.get(table).and_then(|v| v.as_table()) {
                deps.extend(t.keys().cloned());
            }
        }
        deps.sort();
        out.insert(name, deps);
    }
    out
}

/// The headline rule. `spoonstill-core` depends on nothing concrete (D-010),
/// and the cheapest way to guarantee that is for it to depend on nothing at
/// all. If this ever needs to relax, relax it deliberately — and
/// [`core_reaches_nothing_concrete`] still holds the line.
#[test]
fn core_has_no_dependencies() {
    let deps = &manifests()["spoonstill-core"];
    assert!(
        deps.is_empty(),
        "D-010: spoonstill-core must depend on nothing concrete, found {deps:?}.\n\
         If one of these is genuinely a pure-domain dependency, say so in \
         decisions.md first — this test is the enforcement that D-010 asks for."
    );
}

#[test]
fn core_reaches_nothing_concrete() {
    for dep in &manifests()["spoonstill-core"] {
        let lower = dep.to_lowercase();
        for forbidden in FORBIDDEN_IN_CORE {
            assert!(
                !lower.contains(forbidden),
                "D-010: spoonstill-core must not depend on {dep:?} \
                 (matched forbidden marker {forbidden:?})"
            );
        }
    }
}

/// No upward or sideways edges. This is what actually stops the UI from owning
/// the render queue, and `spoonstill-app` from being imported by an
/// infrastructure crate that it is supposed to sit above.
#[test]
fn dependencies_only_point_downward() {
    let manifests = manifests();
    let mut violations = Vec::new();

    for (name, deps) in &manifests {
        let Some(from) = layer(name) else {
            panic!(
                "{name} is a workspace member with no assigned layer. \
                 Add it to `layer()` in this test — a new crate must state \
                 where it sits in D-010's graph before it can build."
            );
        };
        for dep in deps {
            let Some(to) = layer(dep) else { continue }; // third-party
            if to >= from {
                violations.push(format!("{name} (layer {from}) -> {dep} (layer {to})"));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "D-010: dependencies must point strictly downward.\n  {}\n\n\
         Direction is:\n    cli -> app -> {{media, tts, state}} -> core",
        violations.join("\n  ")
    );
}

/// The CLI is an adapter over `spoonstill-app`, not a second application layer.
/// Reaching past `app` into `media`/`tts`/`state` is how business logic starts
/// living in the control surface — and then has to be written twice when the
/// Tauri shell arrives at M4.
#[test]
fn cli_does_not_reach_past_the_application_layer() {
    let deps = &manifests()["spoonstill-cli"];
    for infra in ["spoonstill-media", "spoonstill-tts", "spoonstill-state"] {
        assert!(
            !deps.contains(&infra.to_string()),
            "D-010: spoonstill-cli must go through spoonstill-app, not \
             directly to {infra}. The Tauri shell will need the same path."
        );
    }
}

/// Every crate carries the project name (D-073). The rename that produced these
/// names was applied across six manifests by hand; this is the test that would
/// have caught a missed one.
#[test]
fn every_member_is_a_spoonstill_crate() {
    for name in manifests().keys() {
        assert!(
            name.starts_with("spoonstill-"),
            "workspace member {name:?} is outside the spoonstill-* family"
        );
    }
}
