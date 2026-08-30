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

/// Every asset a finished release carries, without `SHA256SUMS.txt`.
///
/// Five, not six: the `.msi` was the same Windows app as the `.exe` and asked
/// the person downloading to choose between two things that install the same
/// program (D-133).
const ASSETS: [&str; 5] = [
    "still-macOS-AppleSilicon.tar.gz",
    "still-macOS-Intel.tar.gz",
    "still-Windows.zip",
    "spoonstill-macOS.dmg",
    "spoonstill-Windows-Installer.exe",
];

/// The one file that carries every checksum (D-133).
const SUMS: &str = "SHA256SUMS.txt";

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
///
/// The gate's list is what the **build jobs upload**, which is still one
/// `.sha256` per asset: they are computed by the job that built each binary and
/// verified here, and only then folded into one file (D-133).
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

/// D-133. The finished release page is five downloads and one list.
///
/// The build jobs still write a `.sha256` per asset and the publish gate still
/// verifies every one of them — that check is what catches a corrupted upload,
/// and it happens before anybody can download anything. What changed is the
/// *page*: the five twins are folded into `SHA256SUMS.txt` and then deleted, so
/// someone choosing a file to click reads five names instead of eleven.
///
/// Three ways this breaks silently, one assertion each: the gate stops making
/// the file (installers 404), it stops deleting the twins (the page is noisy
/// again and nobody notices, because everything still works), or an installer
/// keeps asking for the per-asset name that no longer exists.
#[test]
fn the_release_publishes_one_checksum_file_and_deletes_the_twins() {
    let workflow = read(".github/workflows/release.yml");

    assert!(
        workflow.contains(&format!("> {SUMS}"))
            && workflow.contains(&format!("upload \"$TAG\" {SUMS}")),
        "{SUMS} is never built or never uploaded, so both installers download a \
         404 and refuse to install anything"
    );
    assert!(
        workflow.contains("gh release delete-asset"),
        "the per-asset .sha256 files are uploaded and never removed, so the \
         release page is back to a checksum twin after every download"
    );

    // Both installers read the one file, and neither constructs the old name.
    for installer in ["scripts/install.sh", "scripts/install.ps1"] {
        let text = read(installer);
        assert!(
            text.contains(SUMS),
            "{installer} does not download {SUMS}, so it has nothing to verify \
             against"
        );
        assert!(
            !text.contains("$Asset.sha256") && !text.contains("$ASSET.sha256"),
            "{installer} still asks for a per-asset checksum, which the release \
             no longer publishes — it would 404 and refuse to install"
        );
    }
}

/// The `.msi` is gone, and nothing may quietly ask for it again (D-133).
///
/// It bundled the same Windows application as the `.exe`. Two installers for
/// one program is a question put to the person downloading that they have no
/// way to answer, and `tauri.conf.json` producing one that the workflow never
/// collects would be a silent build cost.
#[test]
fn nothing_still_builds_or_expects_the_windows_msi() {
    for file in [
        ".github/workflows/release.yml",
        "apps/desktop/tauri.conf.json",
        "scripts/install.sh",
        "scripts/install.ps1",
        "README.md",
    ] {
        let text = read(file);
        assert!(
            !text.contains(".msi"),
            "{file} still refers to a .msi, which is no longer built — either \
             the reference is dead or the bundle target came back"
        );
    }
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
