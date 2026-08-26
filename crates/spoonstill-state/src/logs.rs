//! The on-disk diagnostics log, and the bundle an operator can send.
//!
//! # The problem this solves
//!
//! A render fails on someone else's machine. By the time they say so, the
//! FFmpeg stderr that explained it is gone, and the developer has neither the
//! media nor the environment. The only fix is for the failing machine to have
//! written enough down *while it was failing*, and for there to be one command
//! that packages it.
//!
//! So: every run appends to a JSON Lines file under [`spoonstill_core::STATE_DIR`],
//! whether or not anything goes wrong, and [`write_bundle`] collects those files
//! plus the environment into a single text file to attach to a report.
//!
//! # Why JSON Lines
//!
//! Append-only, one self-contained record per line. A crash mid-write loses at
//! most the last line rather than corrupting the file, and neither reading nor
//! appending requires parsing what came before — which matters when the file is
//! the evidence for the crash that truncated it.
//!
//! # What is not in it
//!
//! Credentials. Every value is passed through [`spoonstill_core::diagnostics::redact`]
//! before it is written, on the way into the event, and the bundle header tells
//! the operator in plain words what the file does contain — because a person
//! about to email a diagnostics file deserves to know it holds their paths.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use spoonstill_core::STATE_DIR;
use spoonstill_core::diagnostics::{Diagnostics, Event, format_utc};

/// Subdirectory of [`STATE_DIR`] holding the log files.
pub const LOGS_DIR: &str = "logs";

/// Size at which a log file is rotated.
///
/// A 500-scene render writes a few hundred kilobytes of routine records; a
/// failing one can write far more, because retained FFmpeg stderr is verbose by
/// design. Rotating at 8 MiB keeps a runaway log from filling a disk while
/// leaving room for the run that actually needs explaining.
pub const ROTATE_AT_BYTES: u64 = 8 * 1024 * 1024;

/// How many rotated files to keep, newest first.
pub const KEEP_FILES: usize = 5;

/// A diagnostics sink that appends to a file.
///
/// Failure to write is swallowed on purpose: a renderer that dies because its
/// log directory is read-only is worse than one that renders without a log.
/// The failure is not silent to the operator, though — [`FileLog::last_error`]
/// carries it, and the CLI reports it once at the end of a run.
#[derive(Debug)]
pub struct FileLog {
    path: PathBuf,
    file: Mutex<Option<File>>,
    last_error: Mutex<Option<String>>,
}

impl FileLog {
    /// Open (or create) the log for a project rooted at `project_root`.
    ///
    /// # Errors
    ///
    /// Returns the underlying error only for the initial directory creation.
    /// Per-record write failures are retained rather than returned, so that
    /// logging can never be the thing that fails a render.
    pub fn open(project_root: &Path) -> std::io::Result<Self> {
        let dir = project_root.join(STATE_DIR).join(LOGS_DIR);
        std::fs::create_dir_all(&dir)?;

        let path = dir.join(format!("spoonstill-{}.jsonl", today_stamp()));
        rotate_if_needed(&path)?;
        prune(&dir);

        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        Ok(Self {
            path,
            file: Mutex::new(Some(file)),
            last_error: Mutex::new(None),
        })
    }

    /// Where this log is being written.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The most recent write failure, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|e| e.clone())
    }

    fn note_error(&self, error: &std::io::Error) {
        if let Ok(mut slot) = self.last_error.lock() {
            *slot = Some(error.to_string());
        }
    }
}

impl Diagnostics for FileLog {
    fn record(&self, event: &Event) {
        let line = to_json_line(event, now_millis());
        let Ok(mut guard) = self.file.lock() else {
            return; // A poisoned lock means another thread panicked mid-write.
        };
        let Some(file) = guard.as_mut() else { return };
        if let Err(error) = file.write_all(line.as_bytes()) {
            self.note_error(&error);
            // Stop trying: a full disk will not empty itself, and retrying per
            // record would turn one failure into thousands.
            *guard = None;
        }
    }
}

/// Serialize one event as a JSON Lines record.
///
/// Hand-written rather than via serde: this crate has no serialization
/// dependency, the shape is four fields and a flat map, and the escaping it
/// needs is the small documented set below. `escape` is tested against the
/// values that actually occur — Windows paths and captured stderr.
fn to_json_line(event: &Event, at_millis: u64) -> String {
    let mut out = String::with_capacity(256);
    out.push_str(r#"{"at":""#);
    out.push_str(&format_utc(at_millis));
    out.push_str(r#"","level":""#);
    out.push_str(event.severity.as_str());
    out.push_str(r#"","scope":""#);
    out.push_str(&escape(event.scope));
    out.push_str(r#"","message":""#);
    out.push_str(&escape(&event.message));
    out.push('"');
    for (key, value) in &event.fields {
        out.push(',');
        out.push('"');
        out.push_str(&escape(key));
        out.push_str(r#"":""#);
        out.push_str(&escape(value));
        out.push('"');
    }
    out.push_str("}\n");
    out
}

/// JSON string escaping.
///
/// A Windows path is full of backslashes and captured FFmpeg stderr is full of
/// newlines and carriage returns, so these are not hypothetical cases — they
/// are the two most common values this function will ever see.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `YYYYMMDD` for today, UTC.
fn today_stamp() -> String {
    format_utc(now_millis())
        .chars()
        .take(10)
        .filter(|c| *c != '-')
        .collect()
}

/// Move the current log aside if it has grown past the rotation size.
fn rotate_if_needed(path: &Path) -> std::io::Result<()> {
    let Ok(metadata) = std::fs::metadata(path) else {
        return Ok(()); // Nothing there yet.
    };
    if metadata.len() < ROTATE_AT_BYTES {
        return Ok(());
    }
    let rotated = path.with_extension(format!("{}.jsonl", now_millis()));
    std::fs::rename(path, rotated)
}

/// Delete all but the newest [`KEEP_FILES`] logs.
fn prune(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let modified = e.metadata().ok()?.modified().ok()?;
            Some((modified, e.path()))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, path) in files.into_iter().skip(KEEP_FILES) {
        let _ = std::fs::remove_file(path);
    }
}

/// One line of environment detail for the bundle header.
#[derive(Debug, Clone)]
pub struct EnvironmentLine {
    /// What this describes.
    pub key: String,
    /// Its value.
    pub value: String,
}

/// Write a single self-contained diagnostics file.
///
/// One file, not an archive: an operator has to attach it to an email or drop
/// it into a chat window, and "one text file" is the format with the fewest
/// ways to go wrong. It opens with a plain-language statement of what it
/// contains, because the person sending it is entitled to know.
///
/// # Errors
///
/// Any filesystem error writing `destination`.
pub fn write_bundle(
    project_root: &Path,
    destination: &Path,
    environment: &[EnvironmentLine],
) -> std::io::Result<BundleReport> {
    let dir = project_root.join(STATE_DIR).join(LOGS_DIR);

    let mut out = String::with_capacity(64 * 1024);
    out.push_str("spoonstill diagnostics bundle\n");
    out.push_str("=============================\n\n");
    out.push_str(&format!("generated: {}\n", format_utc(now_millis())));
    out.push_str(&format!("project:   {}\n\n", project_root.display()));

    out.push_str(
        "WHAT IS IN THIS FILE\n\
         --------------------\n\
         The environment below, then every diagnostic record spoonstill wrote for\n\
         this project: the exact FFmpeg commands it ran, what ffprobe reported, and\n\
         the full error output of anything that failed.\n\n\
         It DOES contain file paths and file names from this machine, because a\n\
         failure usually cannot be diagnosed without them.\n\n\
         It does NOT contain API keys, passwords or tokens. Every value is passed\n\
         through a redaction pass before it is written, and anything that looked\n\
         like a credential reads as [redacted].\n\n\
         It does NOT contain your images, audio, or rendered video.\n\n",
    );

    out.push_str("ENVIRONMENT\n-----------\n");
    for line in environment {
        out.push_str(&format!(
            "{:<22} {}\n",
            format!("{}:", line.key),
            line.value
        ));
    }
    out.push('\n');

    let mut logs = collect_logs(&dir);
    logs.sort();
    let mut records = 0_usize;

    if logs.is_empty() {
        out.push_str(
            "LOGS\n----\n(no log files found — either nothing has been rendered for this\n\
             project yet, or the state directory was deleted)\n",
        );
    }
    for log in &logs {
        out.push_str(&format!(
            "\nLOG {}\n{}\n",
            log.display(),
            "-".repeat(4 + log.display().to_string().len())
        ));
        match std::fs::read_to_string(log) {
            Ok(text) => {
                records += text.lines().filter(|l| !l.trim().is_empty()).count();
                out.push_str(&text);
                if !text.ends_with('\n') {
                    out.push('\n');
                }
            }
            Err(error) => out.push_str(&format!("(could not be read: {error})\n")),
        }
    }

    if let Some(parent) = destination.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(destination, out.as_bytes())?;

    Ok(BundleReport {
        path: destination.to_path_buf(),
        log_files: logs.len(),
        records,
        bytes: std::fs::metadata(destination).map(|m| m.len()).unwrap_or(0),
    })
}

fn collect_logs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

/// What a bundle turned out to contain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleReport {
    /// Where it was written.
    pub path: PathBuf,
    /// How many log files went into it.
    pub log_files: usize,
    /// How many records those files held.
    pub records: usize,
    /// Size on disk.
    pub bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoonstill_core::diagnostics::{REDACTED, Severity};

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spoonstill-logs-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn records_are_one_json_object_per_line() {
        let event = Event::info("render", "started")
            .with("scene", "147")
            .with("frames", "112");
        let line = to_json_line(&event, 0);

        assert!(line.ends_with('\n'));
        assert_eq!(
            line.matches('\n').count(),
            1,
            "one record, one line: {line}"
        );
        assert!(
            line.starts_with(r#"{"at":"1970-01-01T00:00:00.000Z","level":"info""#),
            "{line}"
        );
        assert!(line.contains(r#""scene":"147""#), "{line}");
    }

    /// Captured FFmpeg stderr is multi-line, and a raw newline would split one
    /// record across two lines and break the format.
    ///
    /// CRLF from a Windows FFmpeg is normalized to LF on the way in by the
    /// redaction pass, which is documented behaviour; what matters here is that
    /// whatever arrives ends up as one record on one line.
    #[test]
    fn multi_line_stderr_stays_on_one_line() {
        let event =
            Event::error("ffmpeg", "failed").with("stderr", "line one\nline two\r\nline three");
        let line = to_json_line(&event, 0);
        assert_eq!(line.matches('\n').count(), 1, "{line}");
        assert!(line.contains(r"line one\nline two\nline three"), "{line}");
    }

    /// A Windows path is backslashes all the way down (D-071).
    #[test]
    fn windows_paths_are_escaped() {
        let event = Event::info("render", "output")
            .with("path", r#"C:\Users\someone\Videos\scene "quoted".mp4"#);
        let line = to_json_line(&event, 0);
        assert!(line.contains(r"C:\\Users\\someone"), "{line}");
        assert!(line.contains(r#"\"quoted\""#), "{line}");
    }

    /// The whole point of the redaction pass, checked at the file boundary
    /// rather than only in core.
    #[test]
    fn credentials_never_reach_the_file() {
        let dir = scratch("redaction");
        let log = FileLog::open(&dir).unwrap();
        log.record(&Event::error("tts", "auth failed").with("api_key", "sk_live_secret_value"));
        drop(log);

        let text = std::fs::read_to_string(
            std::fs::read_dir(dir.join(STATE_DIR).join(LOGS_DIR))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
        )
        .unwrap();
        assert!(!text.contains("sk_live_secret_value"), "{text}");
        assert!(text.contains(REDACTED), "{text}");
    }

    #[test]
    fn a_log_is_created_under_the_state_directory() {
        let dir = scratch("creation");
        let log = FileLog::open(&dir).unwrap();
        assert!(log.path().starts_with(dir.join(STATE_DIR).join(LOGS_DIR)));
        log.record(&Event::new(Severity::Warn, "probe", "slow"));
        drop(log);
        assert!(
            std::fs::read_to_string(
                dir.join(STATE_DIR).join(LOGS_DIR).join(
                    std::fs::read_dir(dir.join(STATE_DIR).join(LOGS_DIR))
                        .unwrap()
                        .next()
                        .unwrap()
                        .unwrap()
                        .file_name()
                )
            )
            .unwrap()
            .contains("slow")
        );
    }

    /// The bundle must carry the records, and must say what it is.
    #[test]
    fn a_bundle_carries_the_records_and_explains_itself() {
        let dir = scratch("bundle");
        let log = FileLog::open(&dir).unwrap();
        log.record(&Event::error("ffmpeg", "exited 1").with("command", "ffmpeg -i a.jpg out.mp4"));
        log.record(&Event::info("render", "scene 3 complete"));
        drop(log);

        let destination = dir.join("bundle.txt");
        let report = write_bundle(
            &dir,
            &destination,
            &[EnvironmentLine {
                key: "os".into(),
                value: "macos 15.0 arm64".into(),
            }],
        )
        .unwrap();

        assert_eq!(report.log_files, 1);
        assert_eq!(report.records, 2);
        assert!(report.bytes > 0);

        let text = std::fs::read_to_string(&destination).unwrap();
        assert!(
            text.contains("ffmpeg -i a.jpg out.mp4"),
            "the command is missing"
        );
        assert!(text.contains("scene 3 complete"));
        assert!(text.contains("macos 15.0 arm64"));
        // The operator is told what they are about to send.
        assert!(text.contains("does NOT contain API keys"));
        assert!(text.contains("DOES contain file paths"));
    }

    /// Asking for a bundle before anything has run must produce a usable file
    /// that says so, not an error and not an empty one.
    #[test]
    fn a_bundle_with_no_logs_still_explains_itself() {
        let dir = scratch("empty-bundle");
        let destination = dir.join("bundle.txt");
        let report = write_bundle(&dir, &destination, &[]).unwrap();
        assert_eq!(report.log_files, 0);
        assert_eq!(report.records, 0);
        let text = std::fs::read_to_string(&destination).unwrap();
        assert!(text.contains("no log files found"), "{text}");
    }

    /// A log that cannot be written must not take the render down with it.
    #[test]
    fn a_failing_sink_never_panics() {
        let dir = scratch("failing-sink");
        let log = FileLog::open(&dir).unwrap();
        // Simulate the disk going away mid-run.
        *log.file.lock().unwrap() = None;
        log.record(&Event::error("render", "this must not panic"));
    }
}
