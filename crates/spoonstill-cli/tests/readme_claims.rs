//! The README's countable claims, held against the things they count.
//!
//! `README.md` told a reader that `make gates` runs 25 checks. It runs 29 —
//! M2's four cache gates (D-107 through D-110) landed with the audit and the
//! front door was never updated. Nobody noticed because a number in prose is
//! not checked by anything, which is the same shape as D-125: three files
//! agreeing about asset names with nothing type-checking the agreement.
//!
//! So the number is derived here rather than restated. A gate added to any of
//! the three scripts fails this test until the README says so.

use std::path::{Path, PathBuf};

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

/// How many gates a gate script declares.
///
/// Both spellings count: M0 calls its runner `gate`, M1 and M2 call theirs
/// `check`. A declaration is the line that opens with the runner's name and a
/// quoted description — which is exactly what prints one PASS/FAIL row.
fn gates_in(script: &str) -> usize {
    read(script)
        .lines()
        .filter(|line| {
            let l = line.trim_start();
            (l.starts_with("gate \"") || l.starts_with("check \"")) && !l.starts_with('#')
        })
        .count()
}

#[test]
fn the_readme_counts_the_gates_that_exist() {
    let counted = gates_in("scripts/m0-gates.sh")
        + gates_in("scripts/m1-gates.sh")
        + gates_in("scripts/m2-gates.sh");

    let readme = read("README.md");
    let claimed = readme
        .split_once("It runs ")
        .map(|(_, rest)| rest)
        .and_then(|rest| rest.split_once(" checks"))
        .map(|(n, _)| n.trim().to_owned())
        .expect("README.md should say `It runs N checks` about `make gates`");

    let claimed: usize = claimed
        .parse()
        .unwrap_or_else(|_| panic!("the README's gate count is not a number: {claimed:?}"));

    assert_eq!(
        claimed,
        counted,
        "README.md says `make gates` runs {claimed} checks; the scripts declare {counted} \
         (M0 {}, M1 {}, M2 {})",
        gates_in("scripts/m0-gates.sh"),
        gates_in("scripts/m1-gates.sh"),
        gates_in("scripts/m2-gates.sh"),
    );
}

/// The same number appears a second time, in the Windows caveat near the top —
/// and it was **stale** (30 against 31) the moment a gate landed, because only
/// the "It runs N checks" phrasing was ever checked. A claim counted once and
/// written twice is a claim that drifts.
#[test]
fn every_gate_count_in_the_readme_is_the_same_number() {
    let counted = gates_in("scripts/m0-gates.sh")
        + gates_in("scripts/m1-gates.sh")
        + gates_in("scripts/m2-gates.sh");
    let readme = read("README.md");
    let phrase = format!("{counted} `make gates` checks");
    assert!(
        readme.contains(&phrase),
        "README.md should say {phrase:?}; it says: {:?}",
        readme
            .lines()
            .find(|l| l.contains("`make gates` checks"))
            .unwrap_or("(no such line)"),
    );
}

/// Every `#[test]` in the tree, counted the way a reader would.
///
/// Walks `crates/` and `apps/` rather than asking `cargo`, so it needs no build
/// and no test binary: the number the README states is *how many test functions
/// are written down*, which is a fact about the source.
fn test_functions() -> usize {
    fn walk(dir: &Path, found: &mut usize) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // `target/` is build output, and a vendored copy of anything
                // would be somebody else's tests.
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                walk(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                *found += text
                    .lines()
                    .filter(|line| line.trim_start().starts_with("#[test]"))
                    .count();
            }
        }
    }
    let mut found = 0;
    walk(&root().join("crates"), &mut found);
    walk(&root().join("apps"), &mut found);
    found
}

/// D-175. Two more numbers this README states and nothing counted.
///
/// Measured at the time of writing: **601 claimed against 626 real**, and 131
/// decisions claimed — in two separate places — against 136. Same shape as the
/// gate count above, and the same reason it drifted: a number in prose is not
/// checked by anything. Derived here so it cannot.
#[test]
fn the_readme_counts_the_tests_that_exist() {
    let counted = test_functions();
    let readme = read("README.md");
    let phrase = format!("**{counted} `#[test]` functions**");
    assert!(
        readme.contains(&phrase),
        "README.md should say {phrase:?}; it says: {:?}",
        readme
            .lines()
            .find(|l| l.contains("`#[test]` functions"))
            .unwrap_or("(no such line)"),
    );
}

/// And the decision count, which is written **twice** — the trap
/// `every_gate_count_in_the_readme_is_the_same_number` was added for.
#[test]
fn every_decision_count_in_the_readme_is_the_same_number() {
    let counted = read("decisions.md")
        .lines()
        .filter(|line| {
            line.starts_with("### D-")
                && line
                    .trim_start_matches("### D-")
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
        })
        .count();

    let readme = read("README.md");
    let mentions: Vec<&str> = readme
        .lines()
        .filter(|l| l.contains("numbered decisions"))
        .collect();
    assert!(
        mentions.len() >= 2,
        "the README used to state this number in two places; if that changed, \
         this test should change with it: {mentions:?}"
    );
    let phrase = format!("{counted} numbered decisions");
    for line in mentions {
        assert!(
            line.contains(&phrase),
            "decisions.md holds {counted} numbered decisions, but the README says:\n  {line}",
        );
    }
}

/// The per-milestone table on the same page is the same claim, split three ways.
#[test]
fn the_readme_milestone_table_counts_the_same_gates() {
    let readme = read("README.md");
    for (script, milestone) in [
        ("scripts/m0-gates.sh", "**M0**"),
        ("scripts/m1-gates.sh", "**M1**"),
        ("scripts/m2-gates.sh", "**M2**"),
    ] {
        let n = gates_in(script);
        let row = readme
            .lines()
            .find(|l| l.contains(milestone))
            .unwrap_or_else(|| panic!("README.md has no {milestone} row"));
        let want = format!("{n}/{n} gates");
        assert!(
            row.contains(&want),
            "{milestone} declares {n} gates in {script}, but its README row reads:\n  {row}",
        );
    }
}
