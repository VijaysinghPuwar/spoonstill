//! "Argument vectors, never shell strings." Enforced against this crate's own
//! source, not left to convention.
//!
//! plan.md M1: *"a test asserts no code path ever builds a command from a
//! formatted string."* Two things make that assertion real:
//!
//! - the behavioural half lives in `segment_integrity.rs`, where a filename
//!   containing `$(pwd)` and backticks is rendered and nothing executes;
//! - the structural half is here, and reads the source.
//!
//! A source scan is a blunt instrument, and it is the right one: the failure it
//! prevents is somebody *adding* a second way to spawn a process six months
//! from now, which no behavioural test of today's code paths can catch.

use std::path::{Path, PathBuf};

fn crate_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

fn rust_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("readable source directory") {
            let path = entry.expect("readable entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("readable source file");
                out.push((path, text));
            }
        }
    }
    assert!(!out.is_empty(), "found no sources under {}", dir.display());
    out
}

/// Strip `//` line comments so the denylist below does not fire on prose that
/// merely *names* the thing it forbids — including this file's own doc header.
fn code_only(text: &str) -> String {
    text.lines()
        .map(|line| line.split_once("//").map_or(line, |(code, _)| code))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exactly one module may construct a process.
///
/// Concentrating it is what makes the rest of the guarantee checkable: there is
/// one place to audit, one place where `OsStr` discipline has to hold, and one
/// place that has to hide the console window on Windows.
#[test]
fn only_the_command_module_spawns_a_process() {
    let mut offenders = Vec::new();
    for (path, text) in rust_sources(&crate_src()) {
        if path.file_name().is_some_and(|n| n == "command.rs") {
            continue;
        }
        // `FfmpegCommand::new` is this crate's own builder and contains the
        // same substring, so it is renamed away before the check rather than
        // matched by accident.
        let code = code_only(&text).replace("FfmpegCommand", "<builder>");
        if code.contains("Command::new") || code.contains("process::Command") {
            offenders.push(path);
        }
    }
    assert!(
        offenders.is_empty(),
        "only src/command.rs may construct a process, but these also do: {offenders:?}\n\
         D-011: the FFmpeg boundary is one module, so that the argument-vector rule \
         has one place to hold."
    );
}

/// No shell, anywhere, in any spelling.
#[test]
fn no_source_reaches_for_a_shell() {
    // Each entry is a fragment that only appears when someone is building a
    // command line rather than an argument vector.
    const FORBIDDEN: &[(&str, &str)] = &[
        ("sh -c", "a POSIX shell invocation"),
        ("/bin/sh", "a POSIX shell path"),
        ("bash -c", "a bash invocation"),
        ("cmd /c", "a Windows shell invocation"),
        ("cmd.exe", "a Windows shell path"),
        ("powershell", "a PowerShell invocation"),
        (".shell(", "a shell helper"),
        (
            "shell_words",
            "a command-line splitter, which implies a command line",
        ),
    ];

    let mut offenders = Vec::new();
    for (path, text) in rust_sources(&crate_src()) {
        let code = code_only(&text).to_ascii_lowercase();
        for (fragment, why) in FORBIDDEN {
            if code.contains(fragment) {
                offenders.push(format!("{}: {fragment:?} — {why}", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "D-011/D-052: FFmpeg is reached through an argument vector, never a shell \
         string. A filename is data; through a shell it becomes syntax.\n  {}",
        offenders.join("\n  ")
    );
}

/// The one function that formats a command line is display-only, and says so.
///
/// `FfmpegCommand::display` exists because operators need to paste a failing
/// command into a terminal. That is also exactly the shape of function that
/// someone later feeds back into a spawn, so the invariant is pinned here.
#[test]
fn the_display_form_is_never_executed() {
    let text = std::fs::read_to_string(crate_src().join("command.rs")).unwrap();
    let code = code_only(&text);

    // The only construction of a real process takes `&self.program` — a path —
    // and never the assembled display string.
    assert!(
        code.contains("Command::new(&self.program)"),
        "the process is built from the program path, not from a formatted string"
    );
    for spawn_from_display in [
        "Command::new(display",
        "Command::new(&display",
        "Command::new(self.display",
    ] {
        assert!(
            !code.contains(spawn_from_display),
            "the display form reached a spawn: {spawn_from_display}"
        );
    }
}

/// Arguments are `OsStr`, so a path that is not valid UTF-8 survives.
///
/// Forcing arguments through `String` would either lose such a path or refuse
/// the file — and D-052 says hostile input is the normal case.
#[test]
fn arguments_are_os_strings_not_strings() {
    let text = std::fs::read_to_string(crate_src().join("command.rs")).unwrap();
    assert!(
        text.contains("pub fn arg(&mut self, arg: impl AsRef<OsStr>)"),
        "arg() must accept AsRef<OsStr>, not AsRef<str>"
    );
    assert!(
        text.contains("args: Vec<OsString>"),
        "arguments are retained as OsString"
    );
}
