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
//! Nothing here touches a network, and the retry tests set the backoff to a
//! millisecond, so these run in `make test` like any other test. D-186's two
//! cancellation tests are the exception and say why in place: one needs a
//! five-second backoff because the run count alone cannot tell an
//! interruptible pause from an uninterruptible one, and one needs a stand-in
//! that never answers. The suite still finishes in under three seconds,
//! because both are *cancelled* rather than waited out — which is the thing
//! they are checking.
//!
//! ## What Windows does not get from this file, and what covers it there
//!
//! All of it, because the whole file is gated — so D-186's cancellation
//! wiring into `edge.rs` is checked on unix only. What runs everywhere is the
//! machinery underneath it: `command.rs`'s
//! `a_wait_that_watches_the_flag_stops_when_it_is_set` and
//! `a_pause_between_attempts_can_be_interrupted` drive
//! `FfmpegChild::wait_until_cancellable` and `Cancel::sleep` through a real
//! child, using the test binary itself as the stand-in (D-155) precisely so
//! they are not unix-only. What is unproven on Windows is that `say_one`
//! passes the flag to them — a one-line wiring, and it is named here rather
//! than left to be discovered (D-179).

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_tts::edge::{Edge, Retry};
use spoonstill_tts::{Cancel, Provider, Request, TtsError};

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
        &Cancel::new(),
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
            &Cancel::new(),
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
            &Cancel::new(),
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
            &Cancel::new(),
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
            &Cancel::new(),
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

/// D-186. A line is not spoken for a run the operator has stopped.
///
/// **The count is the test.** A cancelled `speak` returning the right error
/// while still having called the service would be the defect, not the fix:
/// under D-014's bring-your-own-key that call is the operator's money, spent
/// on work they cancelled. `runs` reads the stand-in's own tally, so the
/// claim is about how many times the tool ran and not about what came back.
#[test]
fn a_stopped_run_does_not_speak_the_line() {
    let directory = scratch("cancelled-before");
    let script = fake_edge_tts(&directory, "edge-tts", 0, "");
    let destination = directory.join("line.mp3");

    let cancel = Cancel::new();
    cancel.request();
    let error = Edge::at(&script)
        .with_retry(brisk())
        .speak(
            &Request {
                text: "The harbour was empty by the time we arrived.",
                voice: "en-US-AvaNeural",
                settings: &[],
            },
            &destination,
            &cancel,
        )
        .expect_err("a stopped run speaks nothing");

    assert!(
        matches!(error, TtsError::Cancelled { .. }),
        "expected a cancellation, got: {error}"
    );
    assert_eq!(
        runs(&directory, "edge-tts"),
        0,
        "the service was called for a run the operator had already stopped"
    );
    assert!(!destination.exists(), "and nothing was left on disk");

    let _ = std::fs::remove_dir_all(&directory);
}

/// And the retry loop stops too, rather than waiting out D-094's backoff.
///
/// The stand-in fails the way a dropped connection does, so the first attempt
/// is transient and a second is due. The flag is set **between** them, by a
/// thread the stand-in itself releases — so "cancelled mid-retry" is a fact
/// rather than a sleep, and the assertion is that the tool ran **once**.
///
/// Against a loop that ignored the flag this is 3, which is `brisk()`'s whole
/// budget: the operator pays for two more calls after pressing Stop.
#[test]
fn a_stopped_run_does_not_try_again() {
    let directory = scratch("cancelled-mid-retry");
    // Every attempt fails transiently, so the loop always wants another.
    let script = fake_edge_tts(
        &directory,
        "edge-tts",
        99,
        "aiohttp.client_exceptions.ClientConnectorError: cannot connect",
    );
    let destination = directory.join("line.mp3");

    let cancel = Cancel::new();
    let watcher = cancel.clone();
    let counted = directory.clone();
    // Stops the run the moment the tool has been called once. Bounded, so a
    // stand-in that never runs fails as a deadline rather than hanging
    // (D-149).
    let ticker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while runs(&counted, "edge-tts") == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the stand-in was never run, so nothing was interrupted"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        watcher.request();
    });

    let started = std::time::Instant::now();
    let error = Edge::at(&script)
        .with_retry(Retry {
            attempts: 3,
            // **Five seconds on purpose.** The run count alone does not
            // distinguish an interruptible pause from an uninterruptible one
            // — the attempt closure catches the flag either way, and the
            // only difference is how long the operator waits. Measured: with
            // `std::thread::sleep` in place of `cancel.sleep` this test
            // passed, which is how that gap was found.
            backoff: Duration::from_secs(5),
        })
        .speak(
            &Request {
                text: "The harbour was empty by the time we arrived.",
                voice: "en-US-AvaNeural",
                settings: &[],
            },
            &destination,
            &cancel,
        )
        .expect_err("every attempt fails");
    ticker.join().expect("the watcher finished");

    assert!(
        matches!(error, TtsError::Cancelled { .. }),
        "expected a cancellation rather than an exhausted retry: {error}"
    );
    assert_eq!(
        runs(&directory, "edge-tts"),
        1,
        "the loop tried again after the run was stopped"
    );
    let waited = started.elapsed();
    assert!(
        waited < Duration::from_secs(2),
        "the run took {waited:?} to stop against a five-second backoff — the \
         pause between attempts cannot be interrupted, so Stop means waiting \
         out D-094's retry schedule"
    );

    let _ = std::fs::remove_dir_all(&directory);
}

/// A stand-in that starts and then does not finish, so there is a child in
/// flight to interrupt.
///
/// `fake_edge_tts` answers at once, which is right for the retry tests and
/// useless here: a wait that ignores the flag looks identical against a child
/// that has already exited. Measured — with `wait_until` in place of
/// `wait_until_cancellable` the cancellation tests above still passed, and
/// only this one fails.
fn slow_edge_tts(directory: &Path, name: &str) -> PathBuf {
    let script = directory.join(name);
    let counter = directory.join(format!("{name}.runs"));
    // **`exec`, not a plain `sleep`.** Without it the shell forks a
    // grandchild that inherits the pipes, so killing the shell leaves them
    // open and collecting the dead child's output blocks for the full thirty
    // seconds — which made this test fail against working code. `edge-tts` is
    // one process and does not have that shape; the stand-in has to match it
    // or it is testing the stand-in (this file's own opening note).
    let body = format!(
        r#"#!/bin/sh
printf 'x' >> "{counter}"
exec sleep 30
"#,
        counter = counter.display(),
    );
    std::fs::write(&script, body).expect("write the script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    script
}

/// D-186. A request already in flight is stopped, not waited out.
///
/// `speak_timeout` is sixty seconds plus thirty milliseconds a character, so
/// before this an operator who pressed Stop during a call waited for the
/// service to answer or for that ceiling — whichever came first. The stand-in
/// here never answers, so the only way this returns quickly is by the wait
/// looking at the flag.
#[test]
fn a_request_already_in_flight_is_stopped() {
    let directory = scratch("cancelled-in-flight");
    let script = slow_edge_tts(&directory, "edge-tts");
    let destination = directory.join("line.mp3");

    let cancel = Cancel::new();
    let watcher = cancel.clone();
    let counted = directory.clone();
    // Set once the child is running — a fact, not a sleep — and bounded, so a
    // stand-in that never starts fails as a deadline (D-149).
    let ticker = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while runs(&counted, "edge-tts") == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "the stand-in never started, so no request was in flight"
            );
            std::thread::sleep(Duration::from_millis(2));
        }
        watcher.request();
    });

    let started = std::time::Instant::now();
    let error = Edge::at(&script)
        .with_retry(Retry {
            attempts: 1,
            backoff: Duration::ZERO,
        })
        .speak(
            &Request {
                text: "The harbour was empty by the time we arrived.",
                voice: "en-US-AvaNeural",
                settings: &[],
            },
            &destination,
            &cancel,
        )
        .expect_err("a stopped request produces nothing");
    let waited = started.elapsed();
    ticker.join().expect("the watcher finished");

    // `CANCEL_GRACE` is two seconds of asking politely before the child is
    // forced, and this stand-in ignores the asking — so the floor is that,
    // and the ceiling is well under the thirty the child would have slept.
    assert!(
        waited < Duration::from_secs(15),
        "a request in flight took {waited:?} to stop — the wait is not looking \
         at the flag"
    );
    assert!(
        error.to_string().to_lowercase().contains("cancel")
            || matches!(error, TtsError::Cancelled { .. }),
        "expected a cancellation, got: {error}"
    );
    assert!(!destination.exists(), "and nothing was left on disk");

    let _ = std::fs::remove_dir_all(&directory);
}
