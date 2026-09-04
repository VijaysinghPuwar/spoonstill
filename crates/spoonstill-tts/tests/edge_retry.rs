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

/// A narration long enough to become roughly `parts` requests, built out of
/// whole sentences so the splitter cuts where it is supposed to.
fn long_line(parts: usize) -> String {
    const SENTENCE: &str = "The harbour was empty by the time we arrived. ";
    // A little under `parts` whole chunks, so the last part is a real piece of
    // narration rather than a stray fragment.
    let target = spoonstill_tts::edge::CHUNK_CHARS * parts - SENTENCE.len() * 4;
    SENTENCE.repeat(target / SENTENCE.len() + 1)
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

/// D-151. `default` is not a voice (D-086), and a provider resolves it before
/// it speaks — so a caller that logs what it *asked for* records `default` on a
/// row whose own argv says `--voice en-US-AvaNeural`. The resolution belongs to
/// the provider, so the provider reports it.
#[test]
fn the_voice_reported_is_the_one_that_spoke_not_the_one_asked_for() {
    let directory = scratch("resolved-voice");
    let script = fake_edge_tts(&directory, "plain", 0, "");
    let edge = Edge::at(&script);

    let asked = edge
        .speak(
            &Request {
                text: "A line, in whichever voice this provider defaults to.",
                voice: "default",
                settings: &[],
            },
            &directory.join("default.mp3"),
        )
        .expect("it speaks");
    assert_ne!(
        asked.voice, "default",
        "`default` reached the log as a voice"
    );
    assert_eq!(asked.voice, edge.default_voice());

    // And a voice that *is* a voice comes back untouched.
    let named = edge
        .speak(
            &Request {
                text: "The same line, named.",
                voice: "en-GB-RyanNeural",
                settings: &[],
            },
            &directory.join("named.mp3"),
        )
        .expect("it speaks");
    assert_eq!(named.voice, "en-GB-RyanNeural");

    let _ = std::fs::remove_dir_all(&directory);
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

/// Long form, offline: a line too big for one request becomes several, and
/// what lands on disk is all of them, in order.
///
/// The fake writes its own run number into the file, so the joined result
/// spells out the order the parts were spoken in — a join that silently
/// reversed or dropped a part would still be an MP3 of about the right size.
#[test]
fn a_long_line_becomes_several_requests_joined_in_order() {
    let directory = scratch("chunked");
    let script = directory.join("numbering");
    let counter = directory.join("numbering.runs");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
printf 'x' >> "{counter}"
runs=$(wc -c < "{counter}" | tr -d ' ')
media=""
while [ $# -gt 0 ]; do
  case "$1" in --write-media) media="$2"; shift 2 ;; *) shift ;; esac
done
# An mp3 frame sync, then this run's number, so order is visible.
printf '\377\363' > "$media"
printf '%s' "$runs" >> "$media"
exit 0
"#,
            counter = counter.display()
        ),
    )
    .expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    // Four sentences, and a limit that fits about one of them.
    // Long enough to cross the provider's real chunk limit a few times. The
    // limit is not a test knob: a test that lowered it would be testing a
    // number that never runs.
    let line = long_line(3);
    let line = line.as_str();
    let destination = directory.join("line.mp3");
    let spoken = Edge::at(&script)
        .with_retry(brisk())
        .speak(
            &Request {
                text: line,
                voice: "en-US-AvaNeural",
                settings: &[],
            },
            &destination,
        )
        .expect("a long line still produces one file");

    let joined = std::fs::read(&destination).expect("the artifact");
    let numbers: String = String::from_utf8_lossy(&joined)
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    assert!(
        numbers.len() >= 2,
        "a line this long is more than one request: {numbers:?}"
    );
    assert_eq!(
        numbers,
        (1..=numbers.len())
            .map(|n| n.to_string())
            .collect::<String>(),
        "the parts are joined in the order they were spoken"
    );
    assert_eq!(spoken.bytes, joined.len() as u64, "the count is the file");
    assert!(spoken.how.contains("more parts"), "{}", spoken.how);

    let left: Vec<_> = std::fs::read_dir(&directory)
        .expect("readable")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("part") || n.contains("partial"))
        .collect();
    assert!(left.is_empty(), "every part temporary is gone: {left:?}");
    let _ = std::fs::remove_dir_all(&directory);
}

/// One part failing must not leave the other parts on disk, and must say which
/// part it was — "no audio for 'Alpha bravo…'" is unhelpful when the line was
/// eleven requests.
#[test]
fn a_part_that_fails_takes_the_whole_line_with_it_and_says_which_part() {
    let directory = scratch("chunk-fails");
    let script = directory.join("second-fails");
    let counter = directory.join("second-fails.runs");
    std::fs::write(
        &script,
        format!(
            r#"#!/bin/sh
printf 'x' >> "{counter}"
runs=$(wc -c < "{counter}" | tr -d ' ')
media=""
while [ $# -gt 0 ]; do
  case "$1" in --write-media) media="$2"; shift 2 ;; *) shift ;; esac
done
if [ "$runs" -ge 2 ]; then
  printf 'ValueError: Invalid voice %s
' "nope" >&2
  exit 1
fi
printf '\377\363\144\304' > "$media"
exit 0
"#,
            counter = counter.display()
        ),
    )
    .expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let line = long_line(3);
    let line = line.as_str();
    let destination = directory.join("line.mp3");
    let error = Edge::at(&script)
        .with_retry(brisk())
        .speak(
            &Request {
                text: line,
                voice: "en-US-AvaNeural",
                settings: &[],
            },
            &destination,
        )
        .expect_err("the second part refuses");

    assert!(matches!(error, TtsError::BadRequest { .. }), "{error}");
    assert!(!destination.exists(), "no partial narration is left behind");
    let left: Vec<_> = std::fs::read_dir(&directory)
        .expect("readable")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("part") || n.contains("partial"))
        .collect();
    assert!(
        left.is_empty(),
        "including the part that did work: {left:?}"
    );
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
