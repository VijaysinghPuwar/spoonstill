//! One CSV of everything this machine has done, across every project (D-093).
//!
//! `logs` writes the same events into the project they belong to, and that
//! stays true: the diagnostics bundle needs it, and a project that moves takes
//! its own history with it (D-016).
//!
//! What that arrangement cannot answer is *"what went wrong just now"*, because
//! the answer is spread across every folder the operator has ever used and the
//! folder is often exactly what they are trying to work out. So every event is
//! also appended here, in the machine's own config directory, in a format a
//! spreadsheet opens and sorts.
//!
//! Every failure in this module is **silent by design**: this is a convenience
//! view over authority that lives elsewhere, and a render must never fail
//! because a spreadsheet could not be written.

use spoonstill_core::diagnostics::{Diagnostics, Event, format_utc};

use crate::logs::FileLog;

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// The file, inside [`config_dir`].
pub const RUNS_FILE: &str = "runs.csv";

/// The header, written once when the file is created.
///
/// Column order is part of the file's contract: a spreadsheet someone has
/// already opened, filtered and saved must not have its columns move under it.
/// Append new columns at the end, never in the middle.
const HEADER: &str = "when,level,scope,project,what happened,details,folder";

/// Roll over past this many bytes, so an unattended machine cannot fill a disk
/// with a diagnostic convenience. One previous file is kept — long enough to
/// cover the run before the one that went wrong.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// What the rolled-over file is called.
pub const PREVIOUS_FILE: &str = "runs-previous.csv";

/// Where this machine keeps what belongs to the operator rather than to any one
/// project — the same reasoning as the recent-projects list (D-086).
///
/// Returns `None` when the platform will not say where that is, which is a
/// machine we simply do not write an index on.
#[must_use]
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            Path::new(&home)
                .join("Library")
                .join("Application Support")
                .join("spoonstill"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")?;
        Some(Path::new(&base).join("spoonstill"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(Path::new(&xdg).join("spoonstill"));
        }
        let home = std::env::var_os("HOME")?;
        Some(Path::new(&home).join(".config").join("spoonstill"))
    }
}

/// The index's full path, whether or not it exists yet.
#[must_use]
pub fn index_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join(RUNS_FILE))
}

/// Open the activity log for an append that can actually be locked.
///
/// `read` is not here to read. On Windows a handle opened only for append
/// carries neither `GENERIC_READ` nor `FILE_WRITE_DATA`, and `LockFileEx`
/// needs one of them — so `File::lock` returned `Err` on every Windows write
/// this program has ever made, and D-122's lock existed on macOS only.
/// `std::fs::File::lock` says so in as many words: *"locking a file will fail
/// if the file is opened only for append. To lock a file, open it with one of
/// `.read(true)`, `.read(true).append(true)`, or `.write(true)`."*
///
/// It is a function rather than four lines inside `append_line` so that
/// `the_log_handle_can_actually_be_locked` tests the handle this program
/// really opens, instead of a copy of it that can drift.
fn open_for_locked_append(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)
}

/// A [`Diagnostics`] sink that appends every event to the machine-wide CSV.
///
/// Bound to one project at construction, because the column that makes this
/// file worth having is the one saying *which* project a line came from.
#[derive(Debug, Clone)]
pub struct ActivityLog {
    path: PathBuf,
    project: String,
    folder: String,
}

impl ActivityLog {
    /// Open the index for events from `root`.
    ///
    /// `None` only when the platform will not say where its config directory
    /// is, or the directory cannot be made — both of which mean "this machine
    /// does not get an index", never "this render fails".
    #[must_use]
    pub fn for_project(root: &Path) -> Option<Self> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            path: dir.join(RUNS_FILE),
            project: root
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            folder: root.display().to_string(),
        })
    }

    /// The index for events that belong to the machine rather than to any
    /// project — `still doctor`, `still voices`, installing a tool.
    ///
    /// The two project columns are left **empty** rather than filled with a
    /// placeholder: the honest answer to "which project was this" is that
    /// there wasn't one, and an operator sorting the file by project should
    /// see these together at one end rather than under a made-up name.
    #[must_use]
    pub fn for_machine() -> Option<Self> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir).ok()?;
        Some(Self {
            path: dir.join(RUNS_FILE),
            project: String::new(),
            folder: String::new(),
        })
    }

    /// Where this sink writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one row, under the file's own lock (D-122).
    ///
    /// This file is **machine-wide**, so its writers are different processes
    /// rendering different projects — the one case a per-project lock cannot
    /// cover. Without a lock the three steps below interleave, and they did:
    /// four concurrent renders into a fresh log produced **three header lines**,
    /// two of them welded onto other rows —
    /// `…,folder"2026-08-30T05:45:02.056Z","info",…` and
    /// `…,folderwhen,level,scope,…` — a file no spreadsheet can read.
    ///
    /// Three changes, and each removes one of the ways that happened:
    ///
    /// - **The lock**, held across deciding and writing.
    /// - **"Fresh" is the locked file's length**, not `path.exists()`. That was
    ///   a check against one fact and an act on another, and it is also more
    ///   correct: a file that exists but is empty needs a header too.
    /// - **One `write_all`.** The header used to be three separate writes, so
    ///   another process could land between the header and its newline.
    ///
    /// Still silent on failure, by design: a render must never fail because a
    /// spreadsheet could not be written.
    fn append_line(&self, line: &str) {
        // Twice at most: once to discover the file is due to roll, once to
        // write into the fresh one it becomes.
        for _ in 0..2 {
            let Ok(file) = open_for_locked_append(&self.path) else {
                return;
            };
            // Blocking: a log line is worth waiting a moment for, and the
            // alternative is dropping it. A poisoned or unsupported lock is not
            // a reason to lose the row, so failure here falls through to the
            // write rather than returning.
            //
            // That tolerance is why the Windows defect above stayed invisible:
            // a lock whose failure is survivable is a lock whose failure is
            // silent. `the_log_handle_can_actually_be_locked` is the test that
            // makes this particular failure loud, because it is not a race —
            // it was every write on that platform.
            let locked = file.lock().is_ok();

            let len = file.metadata().map_or(0, |m| m.len());
            if len >= MAX_BYTES {
                if let Some(dir) = self.path.parent() {
                    let _ = std::fs::rename(&self.path, dir.join(PREVIOUS_FILE));
                }
                if locked {
                    let _ = file.unlock();
                }
                drop(file);
                continue;
            }

            let mut body = String::with_capacity(line.len() + HEADER.len() + 1);
            if len == 0 {
                body.push_str(HEADER);
                body.push('\n');
            }
            body.push_str(line);
            let _ = (&file).write_all(body.as_bytes());

            if locked {
                let _ = file.unlock();
            }
            return;
        }
    }
}

impl Diagnostics for ActivityLog {
    fn record(&self, event: &Event) {
        let details = event
            .fields
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");

        let mut line = String::with_capacity(256);
        let _ = writeln!(
            line,
            "{},{},{},{},{},{},{}",
            field(&stamp()),
            field(event.severity.as_str()),
            field(event.scope),
            field(&self.project),
            field(&event.message),
            field(&details),
            field(&self.folder),
        );
        self.append_line(&line);
    }
}

/// Both of this program's log sinks, owned, for one surface's whole run
/// (D-148).
///
/// **This replaces D-093's `Tee`**, which composed two sinks that both exist.
/// The pair is normally two `Option`s — a project with an unwritable
/// `.spoonstill/`, a machine whose config directory cannot be found, an event
/// that belongs to no project at all — and expressing that with `Tee` took a
/// `zip`, a `map` and a four-arm match at every call site. There was exactly
/// one call site, inside `render_project`, and that is the whole story of why
/// for five months only renders reached `runs.csv`: `still validate`, `new`,
/// `add`, `voices`, `doctor`, `remove`, `move` and the **entire desktop
/// window** wrote nothing to the one file D-093 calls the one to open when
/// something has gone wrong. A sink that is awkward to build is a sink that
/// gets built once.
///
/// A `Journal` with neither sink is legal and does nothing, which is the same
/// promise the two sinks make individually: **a command must never fail
/// because a log could not be written.**
pub struct Journal {
    project: Option<FileLog>,
    machine: Option<ActivityLog>,
}

impl Journal {
    /// Both sinks for one project.
    ///
    /// Neither is required. A read-only folder still gets the machine index; a
    /// machine with no config directory still gets the project's own log.
    #[must_use]
    pub fn for_project(root: &Path) -> Self {
        Journal {
            project: FileLog::open(root).ok(),
            machine: ActivityLog::for_project(root),
        }
    }

    /// The machine index alone, for an event that belongs to no project.
    #[must_use]
    pub fn for_machine() -> Self {
        Journal {
            project: None,
            machine: ActivityLog::for_machine(),
        }
    }

    /// The journal a **control surface** uses to write down that a command ran.
    ///
    /// The machine index always, bound to `root` when the command named one.
    /// The project's own log **only where this program already keeps state for
    /// that project** — that is, where `.spoonstill/` exists.
    ///
    /// That condition is the whole point of this constructor.
    /// [`crate::FileLog::open`] *creates* `.spoonstill/logs/`, and a surface
    /// wrapper runs for every command including the ones that only ask a
    /// question. `still validate ~/Pictures` must not leave a state directory
    /// in a folder because somebody pointed at it — asking about a folder is
    /// not adopting it. A project that has been rendered already has the
    /// directory, and gets these rows in its diagnostics bundle (D-016) as
    /// well as in the index.
    #[must_use]
    pub fn for_surface(root: Option<&Path>) -> Self {
        let adopted = root.filter(|r| r.join(spoonstill_core::STATE_DIR).is_dir());
        Journal {
            project: adopted.and_then(|r| FileLog::open(r).ok()),
            machine: match root {
                Some(root) => ActivityLog::for_project(root),
                None => ActivityLog::for_machine(),
            },
        }
    }

    /// A journal that writes nowhere, for a caller that has been asked not to.
    #[must_use]
    pub const fn silent() -> Self {
        Journal {
            project: None,
            machine: None,
        }
    }

    /// The project's own JSON Lines file, when there is one.
    ///
    /// Exposed because the diagnostics bundle names it and `still diagnostics
    /// where` prints it (D-016).
    #[must_use]
    pub fn project_log(&self) -> Option<&FileLog> {
        self.project.as_ref()
    }
}

impl Diagnostics for Journal {
    fn record(&self, event: &Event) {
        if let Some(log) = &self.project {
            log.record(event);
        }
        if let Some(index) = &self.machine {
            index.record(event);
        }
    }
}

/// Now, in the same shape the per-project log stamps its lines with.
#[must_use]
pub fn stamp() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    format_utc(millis)
}

/// One CSV field, quoted per RFC 4180.
///
/// A project folder is the operator's to name and routinely holds a comma, a
/// quote or a newline (D-052 — hostile input is the normal case). A field that
/// is not quoted here is a row that silently gains a column in a spreadsheet,
/// and captured FFmpeg stderr is full of all three.
fn field(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    let cleaned = defuse_formula(&cleaned);
    format!("\"{}\"", cleaned.replace('"', "\"\""))
}

/// Stop a spreadsheet treating a logged value as something to *run* (D-122).
///
/// RFC 4180 quoting is about parsing, not evaluation: Excel, LibreOffice and
/// Numbers all strip the quotes and then read a leading `=`, `+`, `-`, `@`,
/// tab or carriage return as the start of a formula. This file is written for
/// a human to open in a spreadsheet — Settings > Activity log, and
/// `still diagnostics where` — so that is not a theoretical audience.
///
/// It is reachable through the operator's own folder name, which is the
/// `project` column. Verified: a project in a folder called `=1+1`, `+1`,
/// `-2+3` or `@SUM(A1:A9)` put exactly that in the column, quoted and
/// therefore live. D-052's rule is that a name someone chose is hostile input,
/// and a name someone *else* chose — a shared project folder — more so.
///
/// A leading `'` is the conventional defusing and survives being read back.
/// **Numbers are left alone**: `-16.0` is a loudness reading and belongs in the
/// column as a number, and it cannot be a formula on its own.
fn defuse_formula(value: &str) -> String {
    let dangerous = value
        .chars()
        .next()
        .is_some_and(|c| matches!(c, '=' | '+' | '-' | '@' | '\t' | '\r'));
    if dangerous && value.parse::<f64>().is_err() {
        format!("'{value}")
    } else {
        value.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "spoonstill-journal-{name}-{}-{}",
            std::process::id(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// D-148. Asking about a folder is not adopting it: a surface journal must
    /// not create `.spoonstill/` in a folder somebody merely pointed at.
    /// `still validate ~/Pictures` used to be a question and would have become
    /// a question that leaves state behind.
    #[test]
    fn a_folder_nobody_adopted_gains_no_state_directory() {
        let dir = scratch("unadopted");
        let journal = Journal::for_surface(Some(&dir));

        assert!(journal.project_log().is_none());
        assert!(
            !dir.join(spoonstill_core::STATE_DIR).exists(),
            "a question about a folder created state inside it"
        );
    }

    /// And the other half: a project this program already keeps state for gets
    /// these rows in its own log, so they reach the diagnostics bundle (D-016).
    #[test]
    fn a_project_the_program_already_owns_gets_its_own_log_too() {
        let dir = scratch("adopted");
        std::fs::create_dir_all(dir.join(spoonstill_core::STATE_DIR)).unwrap();

        let journal = Journal::for_surface(Some(&dir));
        let log = journal.project_log().expect("the project's own log");
        assert!(log.path().starts_with(&dir), "{}", log.path().display());
    }

    /// One record reaches both sinks, and the pair is what D-093 asked for:
    /// the project's own authority, and the machine's one answer to "what went
    /// wrong just now".
    #[test]
    fn one_event_reaches_both_sinks() {
        let project = scratch("both-project");
        let index = scratch("both-index");
        let journal = Journal {
            project: Some(FileLog::open(&project).unwrap()),
            machine: Some(ActivityLog {
                path: index.join(RUNS_FILE),
                project: "demo".to_owned(),
                folder: project.display().to_string(),
            }),
        };

        journal
            .record(&Event::error("validate", "command failed").with("detail", "no such folder"));

        let jsonl = journal.project_log().unwrap().path().to_path_buf();
        let one = std::fs::read_to_string(&jsonl).unwrap();
        assert!(one.contains("command failed"), "{one}");
        let two = std::fs::read_to_string(index.join(RUNS_FILE)).unwrap();
        assert!(two.contains("command failed"), "{two}");
        assert!(two.contains("no such folder"), "{two}");
    }

    /// A journal with neither sink is legal and does nothing. This is the
    /// promise the whole module rests on: a command must never fail because a
    /// log could not be written.
    #[test]
    fn a_journal_with_no_sinks_records_nothing_and_does_not_complain() {
        let journal = Journal::silent();
        journal.record(&Event::info("validate", "command invoked"));
        assert!(journal.project_log().is_none());
    }

    /// A folder called `holiday, "best" one` is a folder someone will make.
    #[test]
    fn a_field_that_would_break_a_spreadsheet_is_quoted() {
        assert_eq!(
            field(r#"holiday, "best" one"#),
            r#""holiday, ""best"" one""#
        );
        assert_eq!(field("two\nlines"), r#""two lines""#);
        assert_eq!(field("plain"), r#""plain""#);
    }

    /// The header is the file's contract with a spreadsheet the operator may
    /// already have open. Pinned so a column cannot move by accident.
    #[test]
    fn the_column_order_is_fixed() {
        assert_eq!(
            HEADER,
            "when,level,scope,project,what happened,details,folder"
        );
        assert_eq!(HEADER.split(',').count(), 7);
    }

    /// Every row is one event, and every field is quoted — including a project
    /// folder with a comma in it, which is a folder someone will make.
    #[test]
    fn an_event_becomes_one_quoted_row() {
        let dir = std::env::temp_dir().join(format!("spoonstill-runs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let log = ActivityLog {
            path: dir.join("runs.csv"),
            project: r#"holiday, "best""#.to_owned(),
            folder: "/tmp/holiday".to_owned(),
        };

        log.record(&Event::info("render", "film complete").with("scenes", "11"));
        let text = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(lines[0], HEADER, "the header is written once, first");
        assert_eq!(lines.len(), 2, "one event is one row");
        assert!(lines[1].contains(r#""holiday, ""best""""#), "{}", lines[1]);
        assert!(lines[1].contains(r#""film complete""#), "{}", lines[1]);
        assert!(lines[1].contains(r#""scenes=11""#), "{}", lines[1]);

        log.record(&Event::info("render", "again"));
        let after = std::fs::read_to_string(log.path()).unwrap();
        assert_eq!(after.lines().count(), 3, "the header is not written twice");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// The index goes beside the operator's other machine-level state, never
    /// inside a project — the whole point is that it outlives any one of them.
    #[test]
    fn the_index_is_not_inside_a_project() {
        if let Some(path) = index_path() {
            assert!(path.ends_with(RUNS_FILE), "{}", path.display());
            assert!(
                !path.to_string_lossy().contains(".spoonstill"),
                "{}",
                path.display()
            );
        }
    }

    /// D-122. Quoting is about parsing; a spreadsheet strips the quotes and
    /// then decides whether to *run* what is inside.
    ///
    /// Reachable through the operator's own folder name, which is the `project`
    /// column: verified end to end that a project in a folder called `=1+1`
    /// put exactly `=1+1` in that column, quoted and live.
    #[test]
    fn a_value_a_spreadsheet_would_execute_is_defused() {
        for payload in [
            "=1+1",
            "=cmd|' /C calc'!A1",
            "@SUM(A1:A9)",
            "-2+3",
            "+1+cmd|' /C calc'!A1",
            "\tstarts with a tab",
        ] {
            let out = field(payload);
            assert!(
                out.starts_with("\"'"),
                "{payload:?} reached the file able to run: {out}"
            );
        }

        // Numbers are left alone. `-16.0` is a loudness reading and belongs in
        // the column as a number; it cannot be a formula on its own, which is
        // exactly what parsing as one proves.
        for number in ["-16.0", "+1", "-2", "0.5", "-0.75"] {
            assert_eq!(
                field(number),
                format!("\"{number}\""),
                "a number should not have been defused"
            );
        }

        // And ordinary text is untouched.
        assert_eq!(field("render"), "\"render\"");
    }

    /// The lock D-122 relies on must actually be acquired, not merely attempted.
    ///
    /// `append_line` tolerates a failed lock — dropping an operator's log row is
    /// worse than a rare interleave — and that tolerance hid a permanent
    /// failure: on Windows the handle was opened for append only, `LockFileEx`
    /// refused it, and every write on that platform went unlocked. A race that
    /// is lost occasionally looks like flakiness; this asserts the thing that
    /// was never true there at all.
    ///
    /// It exercises `open_for_locked_append`, which is the function
    /// `append_line` itself calls, so the two cannot drift apart.
    #[test]
    fn the_log_handle_can_actually_be_locked() {
        let dir =
            std::env::temp_dir().join(format!("spoonstill-runs-lockable-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("runs.csv");

        // Both states the real code meets: a log that does not exist yet, and
        // one that already has rows in it.
        for round in ["a fresh log", "an existing log"] {
            let file = open_for_locked_append(&path).expect("open");
            file.lock().unwrap_or_else(|e| {
                panic!(
                    "{round}: the activity log cannot be locked, so D-122's \
                     cross-process guarantee does not hold here: {e}"
                )
            });
            file.unlock().expect("unlock");
            drop(file);
            std::fs::write(&path, "a row\n").expect("seed");
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The file is machine-wide, so its writers are *different processes* —
    /// the one case a per-project lock cannot cover (D-122).
    ///
    /// Measured before the fix: four concurrent `still render` **processes**
    /// into a fresh log gave three header lines, two of them welded onto other
    /// rows; afterwards, eight rounds clean.
    ///
    /// **What this test does and does not cover.** It fails against the
    /// original `append_line`, which is what makes it a regression test.
    ///
    /// It was documented here as *not* isolating the lock — "within one
    /// process the length check and the single `write_all` are enough". That
    /// was wrong, and Windows disproved it: with no effective lock two threads
    /// both observe `len == 0` between the open and the write, and both write a
    /// header. The reasoning mistook a check and a write in sequence for an
    /// atomic one. This test found the Windows defect precisely because the
    /// claim was false — but only when the race lost, so it passed two CI runs
    /// and failed the third. `the_log_handle_can_actually_be_locked` is the
    /// deterministic half.
    #[test]
    fn concurrent_writers_produce_one_header_and_whole_rows() {
        // Its own folder: the test above uses `spoonstill-runs-<pid>/runs.csv`
        // and they run in one process, so sharing it makes both flaky.
        let dir =
            std::env::temp_dir().join(format!("spoonstill-runs-concurrent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let path = dir.join("runs.csv");

        std::thread::scope(|scope| {
            for worker in 0..8 {
                let path = path.clone();
                scope.spawn(move || {
                    let log = ActivityLog {
                        path,
                        project: format!("project-{worker}"),
                        folder: "/somewhere".to_owned(),
                    };
                    for n in 0..40 {
                        log.record(
                            &spoonstill_core::diagnostics::Event::info("render", "a row")
                                .with("worker", worker.to_string())
                                .with("n", n.to_string()),
                        );
                    }
                });
            }
        });

        let text = std::fs::read_to_string(&path).expect("read");
        let lines: Vec<&str> = text.lines().collect();

        assert_eq!(
            lines.iter().filter(|l| l.starts_with("when,level")).count(),
            1,
            "more than one header — two writers both thought the file was new"
        );
        assert_eq!(lines[0], HEADER, "the header must be the first line, whole");
        assert_eq!(
            lines.len(),
            8 * 40 + 1,
            "a row was lost or split: {} lines",
            lines.len()
        );
        for line in &lines[1..] {
            assert!(
                line.starts_with('"') && line.ends_with('"'),
                "a torn row: {line}"
            );
        }

        let _ = std::fs::remove_dir_all(&dir);
    }
}
