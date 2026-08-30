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
