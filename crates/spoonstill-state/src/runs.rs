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

    /// Where this sink writes.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    fn append_line(&self, line: &str) {
        self.roll_if_large();
        let fresh = !self.path.exists();
        let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        else {
            return;
        };
        if fresh {
            let _ = file.write_all(HEADER.as_bytes());
            let _ = file.write_all(b"\n");
        }
        let _ = file.write_all(line.as_bytes());
    }

    fn roll_if_large(&self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return;
        };
        if meta.len() < MAX_BYTES {
            return;
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::rename(&self.path, dir.join(PREVIOUS_FILE));
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

/// Write to two sinks at once — the project's own log, and this machine's.
///
/// Composed rather than folded into `FileLog`, so that the per-project log
/// keeps working unchanged on a machine that has no config directory, and so
/// that neither sink can be made to depend on the other.
pub struct Tee<A, B>(pub A, pub B);

impl<A: Diagnostics, B: Diagnostics> Diagnostics for Tee<A, B> {
    fn record(&self, event: &Event) {
        self.0.record(event);
        self.1.record(event);
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
    format!("\"{}\"", cleaned.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
