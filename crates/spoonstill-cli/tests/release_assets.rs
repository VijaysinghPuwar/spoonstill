//! The release's asset names are a contract between three files (D-125).
//!
//! `release.yml` builds and names them, the publish gate asserts the exact set
//! before undrafting, and `install.sh` / `install.ps1` construct the same names
//! to download. D-098 already recorded what happens when one of the three
//! changes alone: every installer 404s on a release that looks complete.
//!
//! Nothing in a YAML file or a shell script is type-checked, so this is the
//! only thing that can hold them together.

use std::path::{Path, PathBuf};

/// Every asset a finished release carries, without the `.sha256` companions.
const ASSETS: [&str; 6] = [
    "still-macOS-AppleSilicon.tar.gz",
    "still-macOS-Intel.tar.gz",
    "still-Windows.zip",
    "spoonstill-macOS.dmg",
    "spoonstill-Windows-Installer.exe",
    "spoonstill-Windows-Installer.msi",
];

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root")
        .to_path_buf()
}

fn read(relative: &str) -> String {
    std::fs::read_to_string(root().join(relative)).unwrap_or_else(|e| panic!("{relative}: {e}"))
}

/// The publish gate lists the exact set, so every asset — and every checksum
/// beside it — has to appear in it.
#[test]
fn the_publish_gate_lists_every_asset_and_its_checksum() {
    let workflow = read(".github/workflows/release.yml");
    let gate = workflow
        .split("NAMES")
        .nth(1)
        .expect("the publish gate's heredoc");

    for asset in ASSETS {
        assert!(
            gate.contains(asset),
            "{asset} is built but the publish gate does not require it, so a \
             release missing it would still be undrafted"
        );
        assert!(
            gate.contains(&format!("{asset}.sha256")),
            "{asset} has no checksum in the publish gate — the installers \
             verify before they install, so a missing one fails there instead"
        );
    }
}

/// And every asset the gate demands is one something actually produces.
#[test]
fn the_publish_gate_demands_nothing_that_is_never_built() {
    let workflow = read(".github/workflows/release.yml");
    let gate = workflow
        .split("NAMES")
        .nth(1)
        .expect("the publish gate's heredoc");

    for line in gate.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Some(name) = line.strip_suffix(".sha256").or(Some(line)) else {
            continue;
        };
        if !name.contains('.') || name.starts_with(')') {
            continue;
        }
        assert!(
            ASSETS.contains(&name),
            "the gate requires {name}, which this test does not know how to \
             build — one of the two lists is out of date"
        );
    }
}

/// The installers download by name. A rename that reaches only the workflow
/// produces a release that publishes and cannot be installed (D-098).
#[test]
fn the_installers_ask_for_the_names_the_release_publishes() {
    let unix = read("scripts/install.sh");
    for asset in [
        "still-macOS-AppleSilicon.tar.gz",
        "still-macOS-Intel.tar.gz",
        "spoonstill-macOS.dmg",
    ] {
        assert!(
            unix.contains(asset),
            "install.sh does not mention {asset}, so it is asking for something \
             the release does not have"
        );
    }

    let windows = read("scripts/install.ps1");
    assert!(
        windows.contains("still-Windows.zip"),
        "install.ps1 does not mention still-Windows.zip"
    );
}

/// D-128. The Windows installer cannot be run from here, so what can be
/// checked is checked.
///
/// There is no PowerShell on the machine this project is developed on, and
/// D-071 still puts Windows in scope. These are the two shapes that were wrong
/// and would be invisible until an operator hit them.
#[test]
fn the_windows_installer_compares_path_entries_rather_than_substrings() {
    let ps1 = read("scripts/install.ps1");

    assert!(
        !ps1.contains("-notlike \"*$InstallDir*\""),
        "the substring PATH test is back: an unrelated `…\\spoonstill\\bin-old` \
         entry counts as this one, so the real folder is never added and \
         `still` is not found after installing"
    );
    assert!(
        ps1.contains("$userPath.Split(';')"),
        "PATH is a list and has to be compared as one"
    );

    // Beside, then over — the same rule as D-119 and D-120.
    assert!(
        ps1.contains("still.exe.new") && ps1.contains("Move-Item"),
        "the installer copies straight onto the live still.exe, so a copy that \
         fails part-way leaves neither the old build nor the new one"
    );
}

/// The same replacement rule on the Unix side, which *can* be run — and was.
#[test]
fn the_unix_installer_stages_before_it_replaces() {
    let sh = read("scripts/install.sh");
    assert!(
        sh.contains("still.new") && sh.contains("mv -f"),
        "install.sh writes over the live binary in place"
    );
}

/// D-129. The advisory ignore list is a record of somebody having looked, and
/// it goes stale.
///
/// Every entry names the advisory it silences, and the file carries a review
/// date. This asserts the shape rather than the contents — the contents are
/// checked by `cargo audit` itself in CI, which fails on anything not listed.
#[test]
fn the_advisory_ignore_list_says_why_and_when_it_was_last_read() {
    let audit = read(".cargo/audit.toml");

    assert!(
        audit.contains("Reviewed 2026-08-30"),
        "the ignore list has no review date, so nobody can tell whether it \
         describes today's dependencies or last year's"
    );
    assert!(
        audit.contains("Review again by"),
        "the ignore list is not time-bounded"
    );

    // Every ignored advisory is annotated. An unexplained RUSTSEC id in this
    // file is exactly the thing the file exists to prevent.
    for line in audit.lines() {
        let line = line.trim();
        if line.starts_with("\"RUSTSEC-") {
            assert!(
                line.contains('#'),
                "an advisory is silenced with no reason beside it: {line}"
            );
        }
    }

    // And CI actually runs it, or the list is decoration.
    let ci = read(".github/workflows/ci.yml");
    assert!(
        ci.contains("cargo audit --deny warnings"),
        "nothing runs the audit, so a new advisory would never be noticed"
    );
}
