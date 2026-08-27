//! The retry loop through the real process boundary (D-094).
//!
//! `edge.rs` unit-tests the classifier against recorded stderr and the loop
//! against a closure. Both can be right while the two are wired together
//! wrongly — the thing that actually has to work is *spawn a tool, read what it
//! said, decide, spawn it again*, and that needs a tool.
//!
//! So these tests write one: a shell script that fails the way the service
//! fails and then stops. Unix only, because a `.sh` is not a program on Windows
//! and faking one there would be testing the fake. D-071 keeps Windows in
//! scope for the code, not for every test of it; the classifier and the loop
//! themselves are covered by unit tests that run everywhere.
//!
//! Nothing here touches a network, and the backoff is set to a millisecond, so
//! these run in `make test` like any other test.

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_tts::edge::{Edge, Retry};
use spoonstill_tts::{Provider, Request, TtsError};

/// The bytes a real `edge-tts` writes first: an MPEG frame sync, from a file
/// this provider actually produced on 2026-08-26.
const MP3_HEAD: &[u8] = &[0xFF, 0xF3, 0x64, 0xC4, 0x00, 0x00, 0x00, 0x03];

/// A stand-in `edge-tts` that fails `failures` times and then works.
///
/// It counts its own runs in a file beside itself, because the point of the
/// test is how many times it was run.
fn fake_edge_tts(directory: &Path, name: &str, failures: u32, stderr: &str) -> PathBuf {
    let script = directory.join(name);
    let counter = directory.join(format!("{name}.runs"));
    let body = format!(
        r#"#!/bin/sh
# Count this run.
printf 'x' >> "{counter}"
runs=$(wc -c < "{counter}" | tr -d ' ')

# Find --write-media's value without assuming argument order.
media=""
while [ $# -gt 0 ]; do
  case "$1" in
    --write-media) media="$2"; shift 2 ;;
    *) shift ;;
  esac
done

if [ "$runs" -le {failures} ]; then
  printf '%s\n' "{stderr}" >&2
  exit 1
fi

# A file that begins like an mp3, which is all the provider inspects.
printf '\377\363\144\304\000\000\000\003' > "$media"
exit 0
"#,
        counter = counter.display(),
        failures = failures,
        stderr = stderr,
    );
    std::fs::write(&script, body).expect("write the script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "spoonstill-retry-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path
}

fn runs(directory: &Path, name: &str) -> u64 {
    std::fs::metadata(directory.join(format!("{name}.runs")))
        .map(|m| m.len())
        .unwrap_or(0)
}

/// A pace no test waits for. The delays themselves are unit-tested in
/// `edge.rs`; what this file is for is the number of attempts.
fn brisk() -> Retry {
    Retry {
        attempts: 3,
        backoff: Duration::from_millis(1),
    }
}

fn say(edge: &Edge, destination: &Path) -> Result<u64, TtsError> {
    edge.speak(
        &Request {
            text: "The harbour was empty by the time we arrived.",
            voice: "en-US-AvaNeural",
            settings: &[],
        },
        destination,
    )
    .map(|spoken| spoken.bytes)
}

/// The whole point: a dropped connection costs a pause, not a render.
#[test]
fn two_dropped_connections_and_the_third_attempt_delivers_the_line() {
    let directory = scratch("recovers");
    let script = fake_edge_tts(
        &directory,
        "flaky",
        2,
        "aiohttp.client_exceptions.ClientConnectorError: Cannot connect to host",
    );
    let destination = directory.join("line.mp3");

    let bytes =
        say(&Edge::at(&script).with_retry(brisk()), &destination).expect("the third attempt works");

    assert_eq!(bytes, MP3_HEAD.len() as u64);
    assert_eq!(runs(&directory, "flaky"), 3, "it kept trying");
    assert_eq!(
        std::fs::read(&destination).expect("the artifact"),
        MP3_HEAD,
        "the file that arrived is the one the last attempt wrote"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// And when the network never comes back, the error says so — once, naming the
/// row and quoting what the last attempt said, instead of pasting a traceback.
#[test]
fn a_service_that_never_comes_back_is_reported_after_the_last_attempt() {
    let directory = scratch("never");
    let script = fake_edge_tts(
        &directory,
        "dead",
        99,
        "aiohttp.client_exceptions.ClientConnectorError: Cannot connect to host",
    );

    let error = say(
        &Edge::at(&script).with_retry(brisk()),
        &directory.join("line.mp3"),
    )
    .expect_err("the network never returns");

    match &error {
        TtsError::NoAudio { text, detail, .. } => {
            assert!(text.starts_with("The harbour"), "the row is named: {text}");
            assert!(detail.contains("3 times"), "{detail}");
            assert!(detail.contains("Cannot connect to host"), "{detail}");
        }
        other => panic!("wrong error: {other}"),
    }
    assert_eq!(runs(&directory, "dead"), 3, "three attempts, not more");

    let left: Vec<_> = std::fs::read_dir(&directory)
        .expect("readable")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains("line"))
        .collect();
    assert!(left.is_empty(), "no half-file survives: {left:?}");
    let _ = std::fs::remove_dir_all(&directory);
}

/// The saving that matters at n=500: a failure the service will repeat is
/// attempted exactly once. Three attempts each with a backoff, five hundred
/// times, is the difference between a run that fails in a minute and one that
/// fails in half an hour.
#[test]
fn a_permanent_failure_costs_one_attempt_and_not_three() {
    let directory = scratch("permanent");
    let script = fake_edge_tts(
        &directory,
        "refuses",
        99,
        "edge_tts.exceptions.NoAudioReceived: No audio was received.",
    );

    let error = say(
        &Edge::at(&script).with_retry(brisk()),
        &directory.join("line.mp3"),
    )
    .expect_err("nothing speakable");

    assert!(matches!(error, TtsError::NoAudio { .. }), "{error}");
    assert!(
        error.to_string().contains("punctuation"),
        "it explains itself: {error}"
    );
    assert_eq!(runs(&directory, "refuses"), 1, "asked once, told once");
    let _ = std::fs::remove_dir_all(&directory);
}

/// A tool that exits zero having written a Python traceback into the media
/// file. The provider must not hand that to FFmpeg and call it narration.
#[test]
fn something_that_is_not_audio_is_not_accepted_as_audio() {
    let directory = scratch("not-audio");
    let script = directory.join("liar");
    std::fs::write(
        &script,
        "#!/bin/sh\nwhile [ $# -gt 0 ]; do case \"$1\" in --write-media) \
         printf 'Traceback (most recent call last):' > \"$2\"; shift 2 ;; *) shift ;; \
         esac; done\nexit 0\n",
    )
    .expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let destination = directory.join("line.mp3");
    let error =
        say(&Edge::at(&script).with_retry(brisk()), &destination).expect_err("that is not an mp3");

    assert!(error.to_string().contains("not an audio file"), "{error}");
    assert!(!destination.exists(), "and it is not left on disk");
    let _ = std::fs::remove_dir_all(&directory);
}
