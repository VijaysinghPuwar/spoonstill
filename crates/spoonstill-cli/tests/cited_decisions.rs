//! Every `D-nnn` this tree cites is a decision that exists (D-150).
//!
//! Six places cited **D-099** for the arrange work, which is **D-100** —
//! D-099 is Gatekeeper quarantine, a different subject entirely. Two of the
//! six were doc comments on `still remove` and `still move`, so the wrong
//! number printed in `still --help` and every operator read it.
//!
//! A source scan cannot know that D-099 is the *wrong* decision when D-099
//! exists, and this test says so rather than pretending otherwise. What it
//! does catch is the other half of the same mistake — a number that is not a
//! decision at all, which is what a typo or a renumbering produces — and it is
//! the only mechanical check available. The six were found by reading; the
//! seventh will be found by reading too, and this narrows what is left to read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root")
        .to_path_buf()
}

/// Every source file under the crates and the app, skipping build output.
fn sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') || name == "target" || name == "node_modules" {
            continue;
        }
        if path.is_dir() {
            sources(&path, found);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("rs" | "js" | "css" | "html" | "sh" | "toml")
        ) {
            found.push(path);
        }
    }
}

#[test]
fn every_decision_cited_in_the_tree_exists() {
    let workspace = root();
    let decisions = std::fs::read_to_string(workspace.join("decisions.md"))
        .expect("decisions.md is the single source of truth");

    // Every heading, in the one form this file uses.
    let existing: BTreeSet<String> = decisions
        .lines()
        .filter_map(|line| line.trim_start_matches('#').trim().strip_prefix("D-"))
        .filter_map(|rest| rest.get(..3).map(|n| format!("D-{n}")))
        .filter(|id| id.chars().skip(2).all(|c| c.is_ascii_digit()))
        .collect();
    assert!(
        existing.len() > 100,
        "only {} decisions found — the heading format changed",
        existing.len()
    );

    let mut files = Vec::new();
    for dir in ["crates", "apps", "scripts"] {
        sources(&workspace.join(dir), &mut files);
    }
    assert!(
        files.len() > 30,
        "only {} source files scanned",
        files.len()
    );

    let mut dangling: Vec<String> = Vec::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            continue;
        };
        let bytes = text.as_bytes();
        for (at, _) in text.match_indices("D-") {
            let digits: String = bytes[at + 2..]
                .iter()
                .take(3)
                .take_while(|b| b.is_ascii_digit())
                .map(|b| *b as char)
                .collect();
            if digits.len() != 3 {
                continue;
            }
            let id = format!("D-{digits}");
            if !existing.contains(&id) {
                dangling.push(format!(
                    "{} cites {id}, which is not in decisions.md",
                    file.strip_prefix(&workspace).unwrap_or(file).display()
                ));
            }
        }
    }
    dangling.sort();
    dangling.dedup();
    assert!(dangling.is_empty(), "{}", dangling.join("\n"));
}
