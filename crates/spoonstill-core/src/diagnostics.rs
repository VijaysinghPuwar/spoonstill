//! Structured diagnostics: what gets recorded, and what must never be.
//!
//! # Why this exists
//!
//! A render fails on an operator's machine, three time zones away, on media the
//! developer has never seen. The only way to find out why is for that machine
//! to have written down enough at the time — after the fact, the FFmpeg stderr
//! that explained everything is gone.
//!
//! So spoonstill records as it goes, to a file, always; and a single command
//! packages those records into one file an operator can send. The pieces that
//! actually answer questions are the ones this module is shaped around: the
//! exact command that was run, the exact `ffprobe` output that failed the
//! profile assertion, the exact stderr, and the exact build of FFmpeg that
//! produced them.
//!
//! # What must never be recorded
//!
//! A diagnostics bundle is a file the operator sends to someone else. With BYOK
//! (D-014, D-023) their machine holds an API key, and a log line that quotes an
//! environment variable or a request header would put it in that file. So
//! [`redact`] runs over every recorded value, and it is a pure function with
//! tests rather than a discipline anyone has to remember.
//!
//! This crate stays pure (D-010): it defines the vocabulary and the redaction,
//! and it does not open a file or read a clock. The sink that does both lives in
//! `spoonstill-state`, which already owns [`crate::STATE_DIR`].

use core::fmt;

/// How much attention a record deserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Routine progress: what was attempted, and what it produced.
    Info,
    /// Something surprising that did not stop the render.
    Warn,
    /// Something that stopped it.
    Error,
}

impl Severity {
    /// Stable lowercase name, as written into the log.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One thing worth writing down.
///
/// Fields are key/value rather than an interpolated sentence, so that a bundle
/// can be scanned mechanically — "show me every command that exited non-zero"
/// is a grep, not a reading exercise.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// Severity.
    pub severity: Severity,
    /// Which part of the system produced this — `render`, `probe`, `ffmpeg`.
    pub scope: &'static str,
    /// A short human sentence.
    pub message: String,
    /// Structured detail, already redacted.
    pub fields: Vec<(String, String)>,
}

impl Event {
    /// A new event. Every value passed through [`redact`] on the way in, so a
    /// caller cannot forget.
    #[must_use]
    pub fn new(severity: Severity, scope: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity,
            scope,
            message: redact(&message.into()),
            fields: Vec::new(),
        }
    }

    /// An informational event.
    #[must_use]
    pub fn info(scope: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Info, scope, message)
    }

    /// A warning.
    #[must_use]
    pub fn warn(scope: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Warn, scope, message)
    }

    /// An error.
    #[must_use]
    pub fn error(scope: &'static str, message: impl Into<String>) -> Self {
        Self::new(Severity::Error, scope, message)
    }

    /// Attach one structured field.
    ///
    /// The **field name is part of the decision**: a value under a key called
    /// `api_key` is dropped whole, without waiting for the value itself to look
    /// suspicious. A 24-character key is too short to trip the shape rule in
    /// [`redact`] and contains no marker of its own, so keying off the name is
    /// the only thing that catches it.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        let key = key.into();
        let value = if is_secret_key(&key) {
            REDACTED.to_string()
        } else {
            redact(&value.into())
        };
        self.fields.push((key, value));
        self
    }
}

/// Somewhere for events to go.
///
/// Object-safe on purpose: `spoonstill-media` holds a `&dyn Diagnostics` and
/// has no idea whether it is writing to a file, to a test buffer, or nowhere.
pub trait Diagnostics: Send + Sync {
    /// Record one event. Must not fail the caller — a renderer that dies
    /// because its log file is read-only is worse than one that renders
    /// silently.
    fn record(&self, event: &Event);
}

/// A sink that discards everything. The default in tests and in library use.
#[derive(Debug, Clone, Copy, Default)]
pub struct Noop;

impl Diagnostics for Noop {
    fn record(&self, _event: &Event) {}
}

impl Diagnostics for &dyn Diagnostics {
    fn record(&self, event: &Event) {
        (**self).record(event);
    }
}

/// Markers that indicate a value is a credential rather than a diagnostic.
///
/// Substring matching, deliberately: it fires on `ELEVENLABS_API_KEY`,
/// `api_key=`, `Authorization: Bearer`, and on anything else whose name admits
/// what it is. It cannot catch a secret that is named nothing in particular,
/// which is why [`redact`] also has the length-and-shape rule below.
const SECRET_MARKERS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "secret",
    "password",
    "passwd",
    "token",
    "authorization",
    "bearer ",
    "xi-api-key",
    "access_key",
    "private_key",
    "credential",
];

/// What replaces a redacted value.
pub const REDACTED: &str = "[redacted]";

/// Whether a field name announces that its value is a credential.
///
/// Checked separately from [`redact`] because a field's *name* is evidence the
/// value itself may not carry: `api_key = "sk_live_abcdefghijklmnop"` is 24
/// characters, short of the shape rule, and contains no marker.
#[must_use]
pub fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    SECRET_MARKERS.iter().any(|m| lowered.contains(m.trim()))
}

/// Remove anything that looks like a credential.
///
/// Two rules, both conservative:
///
/// 1. If a `key=value` or `key: value` pair has a key naming a secret, the
///    value goes.
/// 2. A bare token that is long, high-entropy-looking and has no spaces —
///    32 or more characters of base64/hex alphabet — goes regardless of what it
///    is called, because that is the shape of every API key we might ever see
///    and none of the diagnostics we actually want.
///
/// Rule 2 has a false-positive cost: a long hash in a log line is replaced.
/// That is the right trade. A content hash that reads `[redacted]` costs one
/// round trip with the operator; a leaked key costs them money.
///
/// Line endings are normalized to `\n` as a side effect of working line by
/// line. That is stated rather than left to be discovered: captured stderr from
/// a Windows FFmpeg arrives CRLF-delimited and comes out LF-delimited, which
/// changes nothing a reader needs and keeps the log free of stray carriage
/// returns.
#[must_use]
pub fn redact(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&redact_line(line));
    }
    // `lines()` drops a trailing newline; put it back so a captured stderr
    // block keeps its shape.
    if text.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn redact_line(line: &str) -> String {
    let lowered = line.to_ascii_lowercase();

    // Rule 1: a named secret takes the rest of the line with it, because the
    // value may itself contain spaces or separators.
    for marker in SECRET_MARKERS {
        if let Some(at) = lowered.find(marker) {
            let after = &line[at + marker.len()..];
            if let Some(separator) = after.find(['=', ':']) {
                let keep_to = at + marker.len() + separator + 1;
                return format!("{}{REDACTED}", &line[..keep_to]);
            }
            // `Bearer ` and friends carry the value directly after the marker.
            if marker.ends_with(' ') {
                return format!("{}{REDACTED}", &line[..at + marker.len()]);
            }
        }
    }

    // Rule 2: a long opaque token, wherever it appears.
    line.split_inclusive(char::is_whitespace)
        .map(|word| {
            let trimmed = word.trim_end();
            let suffix = &word[trimmed.len()..];
            if looks_like_a_secret(trimmed) {
                format!("{REDACTED}{suffix}")
            } else {
                word.to_string()
            }
        })
        .collect()
}

/// Whether a bare word has the shape of a credential.
fn looks_like_a_secret(word: &str) -> bool {
    // Trim punctuation a log line would put around a value.
    let word = word.trim_matches(|c: char| "\"',;()[]{}".contains(c));
    if word.len() < 32 {
        return false;
    }
    // A path is long and opaque too, and is exactly what we need to keep.
    if word.contains('/') || word.contains('\\') || word.contains('.') {
        return false;
    }
    word.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '+')
}

/// Format epoch milliseconds as an ISO-8601 UTC timestamp.
///
/// Written out rather than pulled in, for the same reason as [`crate::hash`]:
/// this crate depends on nothing concrete (D-010), and a date formatter is not
/// a good enough reason to breach that. UTC only — a bundle read on another
/// continent must not need the writer's time zone to make sense.
#[must_use]
pub fn format_utc(epoch_millis: u64) -> String {
    let total_seconds = epoch_millis / 1000;
    let millis = epoch_millis % 1000;
    let (days, seconds_today) = (total_seconds / 86_400, total_seconds % 86_400);
    let (hour, minute, second) = (
        seconds_today / 3600,
        (seconds_today % 3600) / 60,
        seconds_today % 60,
    );
    let (year, month, day) = civil_from_days(days as i64);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z")
}

/// Days since the Unix epoch to a civil date.
///
/// Howard Hinnant's `civil_from_days`, which is the standard closed-form answer
/// and correct for every date this program can encounter.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_secret_loses_its_value() {
        for line in [
            "ELEVENLABS_API_KEY=sk_abc123",
            "api_key: sk_abc123",
            "X-Api-Key=sk_abc123",
            "authorization: Bearer sk_abc123",
            "password=hunter2",
            "some_token = abc",
        ] {
            let out = redact(line);
            assert!(out.contains(REDACTED), "{line:?} was not redacted: {out}");
            assert!(!out.contains("sk_abc123"), "{line:?} leaked: {out}");
            assert!(!out.contains("hunter2"), "{line:?} leaked: {out}");
        }
    }

    /// The rule that catches a secret nobody labelled.
    #[test]
    fn a_long_opaque_token_is_redacted_even_unnamed() {
        let key = "a1B2c3D4e5F6g7H8i9J0k1L2m3N4o5P6q7R8";
        assert_eq!(key.len(), 36);
        let out = redact(&format!("request failed with {key} attached"));
        assert!(!out.contains(key), "{out}");
        assert!(out.contains(REDACTED), "{out}");
    }

    /// The things a bundle exists to carry must survive redaction, or the
    /// feature is useless.
    #[test]
    fn diagnostics_that_matter_survive() {
        let keep = [
            "/Users/someone/Renders/project/scene-147.jpg",
            "ffmpeg -i input.jpg -frames:v 112 out.mp4",
            "sample_aspect_ratio: expected \"1:1\", found \"30007:30000\"",
            "Error opening input file /tmp/a b.mp4",
            "C:\\Users\\someone\\Videos\\scene.jpg",
        ];
        for line in keep {
            assert_eq!(redact(line), line, "redaction ate a needed diagnostic");
        }
    }

    /// A path is long and opaque and must not trip rule 2.
    #[test]
    fn long_paths_are_not_mistaken_for_secrets() {
        let path =
            "/Users/someone/Library/Application Support/spoonstill/renders/segment-000147.mp4";
        assert_eq!(redact(path), path);
    }

    /// Multi-line captured stderr keeps its shape, so it stays readable.
    #[test]
    fn multi_line_text_keeps_its_shape() {
        let stderr = "line one\nline two\nline three\n";
        assert_eq!(redact(stderr), stderr);
    }

    /// CRLF from a Windows FFmpeg normalizes to LF. Documented behaviour, not
    /// an accident — the line count and the content are what a reader needs.
    #[test]
    fn windows_line_endings_normalize() {
        assert_eq!(redact("one\r\ntwo\r\n"), "one\ntwo\n");
    }

    /// Redaction happens on the way in, so a caller cannot forget it.
    #[test]
    fn events_redact_their_own_fields() {
        let event = Event::error("tts", "request failed")
            .with("endpoint", "https://api.example.com/v1/speak")
            .with("api_key", "sk_live_abcdefghijklmnop");
        let joined: String = event
            .fields
            .iter()
            .map(|(k, v)| format!("{k}={v} "))
            .collect();
        assert!(!joined.contains("sk_live_abcdefghijklmnop"), "{joined}");
        assert!(
            joined.contains("https://api.example.com/v1/speak"),
            "{joined}"
        );
    }

    /// A short key under a telling name is the case the value-only rules miss.
    #[test]
    fn a_field_name_alone_is_enough_to_redact() {
        for key in ["api_key", "ELEVENLABS_API_KEY", "authorization", "secret"] {
            let event = Event::info("tts", "call").with(key, "short-value-123");
            assert_eq!(event.fields[0].1, REDACTED, "field {key} leaked");
        }
        assert!(is_secret_key("xi-api-key"));
        assert!(!is_secret_key("scene_index"));
        assert!(!is_secret_key("command"));
    }

    #[test]
    fn timestamps_are_iso_utc() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00.000Z");
        // 2026-08-26T12:34:56.789Z
        assert_eq!(format_utc(1787747696789), "2026-08-26T12:34:56.789Z");
        // A leap day, because the closed form is where that goes wrong.
        assert_eq!(format_utc(1_709_164_800_000), "2024-02-29T00:00:00.000Z");
    }

    #[test]
    fn the_noop_sink_accepts_anything() {
        let sink = Noop;
        sink.record(&Event::info("test", "nothing happens"));
    }
}
