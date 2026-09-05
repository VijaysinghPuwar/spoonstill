//! The Edge provider against the real `edge-tts` and the real service.
//!
//! **Every test here is `#[ignore]`d**, and that is the point. `make test` has
//! to be the same answer on a plane as it is in CI, and a test that speaks to
//! Microsoft over a websocket is neither. Run them deliberately:
//!
//! ```text
//! make tts-live
//! ```
//!
//! What they are for is the half of `edge.rs` that unit tests cannot reach.
//! The classifier in that module is built on recorded stderr, and recorded
//! stderr goes stale the moment `edge-tts` is upgraded — so these tests provoke
//! each failure against the installed tool and check that it is still
//! classified as the module claims. When one of them fails, the fixtures in
//! `edge.rs` are the thing that is out of date, not the service.
//!
//! They skip rather than fail when `edge-tts` is absent, in the same spirit as
//! M2's gate 7: a machine without the tool is a supported machine.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use spoonstill_tts::edge::{DEFAULT_VOICE, Edge};
use spoonstill_tts::{Availability, Provider, Request, TtsError};

/// A directory of this test's own.
fn scratch(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("spoonstill-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a scratch directory");
    path
}

/// Everything left in a directory — the check that no attempt leaked a script
/// or a half-written media file.
fn leftovers(directory: &Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .expect("readable")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}

/// The provider, or `None` on a machine that does not have the tool.
fn installed() -> Option<Edge> {
    let edge = Edge::from_env();
    match edge.availability() {
        Availability::Ready => Some(edge),
        Availability::Missing(detail) => {
            eprintln!("skipped: {detail}");
            None
        }
    }
}

#[test]
#[ignore = "talks to Microsoft; run with `make tts-live`"]
fn the_whole_catalogue_comes_back_and_every_voice_can_be_filed_under_a_language() {
    let Some(edge) = installed() else { return };
    let voices = edge.voices().expect("the catalogue");

    assert!(
        voices.len() > 100,
        "the service offers hundreds of voices, not {}",
        voices.len()
    );
    assert!(
        voices.iter().any(|v| v.id == DEFAULT_VOICE),
        "the voice we default to must be one the service still has"
    );
    let unfiled: Vec<&str> = voices
        .iter()
        .filter(|v| v.locale.is_empty() || !v.id.starts_with(&v.locale))
        .map(|v| v.id.as_str())
        .collect();
    assert!(
        unfiled.is_empty(),
        "these would show up in the window with no language: {unfiled:?}"
    );
    let genderless: Vec<&str> = voices
        .iter()
        .filter(|v| v.gender.is_empty())
        .map(|v| v.id.as_str())
        .collect();
    assert!(
        genderless.is_empty(),
        "the gender filter needs these: {genderless:?}"
    );
}

/// D-158. Every voice the script detector can choose is really offered.
///
/// `DEFAULT_VOICES` is a table of names written by hand, and the compiler
/// cannot check one. A name that has been retired turns "a Hindi project now
/// renders" into `Invalid voice`, which is a worse failure than the one the
/// table exists to fix, and nothing local would notice. This is the check, and
/// it is here rather than in a unit test because only the service knows.
#[test]
#[ignore = "talks to Microsoft; run with `make tts-live`"]
fn every_voice_the_script_can_choose_is_one_the_service_still_offers() {
    let Some(edge) = installed() else { return };
    let catalogue = edge.voices().expect("the catalogue");

    // Every script the detector recognises, in a line of that script.
    for line in [
        "नमस्ते दुनिया",
        "চলো বাংলা",
        "ગુજરાતી લખાણ",
        "தமிழ் எழுத்து",
        "తెలుగు వచనం",
        "ಕನ್ನಡ ಪಠ್ಯ",
        "മലയാളം എഴുത്ത്",
        "සිංහල පෙළ",
        "Καλημέρα κόσμε",
        "Здравствуй мир",
        "שלום עולם",
        "مرحبا بالعالم",
        "สวัสดีชาวโลก",
        "ສະບາຍດີ",
        "こんにちは",
        "မင်္ဂလာပါ",
        "გამარჯობა",
        "ሰላም ዓለም",
        "សួស្តី",
        "안녕하세요",
        "你好世界",
        "Hello world",
    ] {
        let chosen = spoonstill_tts::edge::default_voice_for(line);
        assert!(
            catalogue.iter().any(|v| v.id == chosen),
            "{line:?} would be spoken by {chosen}, which the service no longer \
             offers — a project in that script fails with `Invalid voice`"
        );
    }
}

#[test]
#[ignore = "talks to Microsoft; run with `make tts-live`"]
fn a_line_becomes_audio_that_ffprobe_can_measure() {
    let Some(edge) = installed() else { return };
    let directory = scratch("speak");
    let destination = directory.join("line.mp3");

    let spoken = edge
        .speak(
            &Request {
                text: "The harbour was empty by the time we arrived.",
                voice: DEFAULT_VOICE,
                settings: &[],
            },
            &destination,
        )
        .expect("a line the service will say");

    assert!(
        spoken.bytes > 1_000,
        "{} bytes is not a sentence",
        spoken.bytes
    );
    assert!(
        !spoken.how.contains("harbour"),
        "the script must never reach the logged command (D-016): {}",
        spoken.how
    );

    // D-021: the duration is measured, never taken from a header we wrote or a
    // word count we guessed.
    let probed = spoonstill_media::probe::probe(
        &spoonstill_media::Tools::from_env(),
        &destination,
        Duration::from_secs(30),
    )
    .expect("ffprobe reads it");
    let seconds = probed.audio_duration().expect("an audio stream");
    assert!(
        (1.0..12.0).contains(&seconds),
        "that sentence is a couple of seconds long, not {seconds}"
    );

    assert_eq!(
        leftovers(&directory),
        vec!["line.mp3".to_owned()],
        "the script and every temporary are gone"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// The classifier's main claim, checked against the tool rather than against a
/// string recorded in 2026: a voice that does not exist is **permanent**, so it
/// is reported at once, in a sentence, with the way out named.
#[test]
#[ignore = "talks to Microsoft; run with `make tts-live`"]
fn a_voice_that_does_not_exist_is_refused_immediately_and_in_english() {
    let Some(edge) = installed() else { return };
    let directory = scratch("bad-voice");
    let started = Instant::now();

    let error = edge
        .speak(
            &Request {
                text: "A line worth saying.",
                voice: "en-GB-NotARealVoice",
                settings: &[],
            },
            &directory.join("line.mp3"),
        )
        .expect_err("no such voice");

    match &error {
        TtsError::BadRequest { detail, .. } => {
            assert!(detail.contains("en-GB-NotARealVoice"), "{detail}");
            assert!(detail.contains("still voices"), "{detail}");
            assert!(!detail.contains("Traceback"), "{detail}");
        }
        other => panic!("a bad voice is a bad request, not: {other}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "a permanent failure must not sit through two backoffs first"
    );
    assert!(
        leftovers(&directory).is_empty(),
        "{:?}",
        leftovers(&directory)
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// The other permanent failure, and the one that reaches real operators: a
/// caption that is only punctuation. The service accepts it, sends no audio,
/// and no amount of retrying changes that.
#[test]
#[ignore = "talks to Microsoft; run with `make tts-live`"]
fn a_line_with_nothing_speakable_in_it_is_not_retried() {
    let Some(edge) = installed() else { return };
    let directory = scratch("no-audio");
    let started = Instant::now();

    let error = edge
        .speak(
            &Request {
                text: "...",
                voice: DEFAULT_VOICE,
                settings: &[],
            },
            &directory.join("line.mp3"),
        )
        .expect_err("nothing to say");

    match &error {
        TtsError::NoAudio { text, detail, .. } => {
            assert_eq!(text, "...", "the row is identified");
            assert!(
                !detail.contains("times in a row"),
                "this one is permanent, so it is attempted once: {detail}"
            );
        }
        other => panic!("expected no-audio, got: {other}"),
    }
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "three attempts at a line that will never work is a minute wasted per scene"
    );
    assert!(
        leftovers(&directory).is_empty(),
        "{:?}",
        leftovers(&directory)
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Long form against the real service (D-095): a narration too big for one
/// request comes back as one file of the right length.
///
/// The measurement this defends: 20 000 characters is a single 128-second
/// request producing 19 minutes of audio, and any failure in it throws away
/// all 128 seconds. Split, the same narration is a few requests that each
/// stand or fall on their own — and the joined result has to be
/// indistinguishable from the unsplit one to everything downstream, which
/// means ffprobe has to read it as one continuous track of the expected
/// length.
#[test]
#[ignore = "talks to Microsoft for a couple of minutes; run with `make tts-live`"]
fn a_narration_longer_than_one_request_comes_back_as_one_file() {
    let Some(edge) = installed() else { return };
    let directory = scratch("long-form");
    let destination = directory.join("narration.mp3");

    // Three requests' worth: past the limit, and short enough to wait for.
    const SENTENCE: &str = "The harbour was empty by the time we arrived. ";
    let target = spoonstill_tts::edge::CHUNK_CHARS * 3 - SENTENCE.len() * 4;
    let narration = SENTENCE.repeat(target / SENTENCE.len() + 1);
    let characters = narration.chars().count();

    let spoken = edge
        .speak(
            &Request {
                text: &narration,
                voice: DEFAULT_VOICE,
                settings: &[],
            },
            &destination,
        )
        .expect("a long narration");

    assert!(spoken.how.contains("more parts"), "{}", spoken.how);
    assert!(
        !spoken.how.contains("harbour"),
        "the script still never reaches the log (D-016): {}",
        spoken.how
    );

    let probed = spoonstill_media::probe::probe(
        &spoonstill_media::Tools::from_env(),
        &destination,
        Duration::from_secs(60),
    )
    .expect("ffprobe reads the joined file as one track");
    let seconds = probed.audio_duration().expect("an audio stream");

    // 17.3 characters per second of speech, measured 2026-08-26. A join that
    // dropped or duplicated a part would miss this by a whole part.
    let expected = characters as f64 / 17.3;
    assert!(
        (seconds - expected).abs() < expected * 0.15,
        "{characters} characters should be about {expected:.0}s of speech, not {seconds:.0}s \
         — a part is missing or repeated"
    );

    assert_eq!(
        leftovers(&directory),
        vec!["narration.mp3".to_owned()],
        "every part temporary is gone"
    );
    let _ = std::fs::remove_dir_all(&directory);
}

/// Eight lines at once, which is what the audio pool of D-044 does. The point
/// is not speed: it is that nothing collides — no shared temporary, no
/// half-file, and eight distinct results.
#[test]
#[ignore = "talks to Microsoft; run with `make tts-live`"]
fn eight_lines_spoken_at_once_do_not_collide() {
    let Some(edge) = installed() else { return };
    let directory = scratch("parallel");

    let lines: Vec<String> = (0..8)
        .map(|n| format!("This is line number {n}."))
        .collect();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = lines
            .iter()
            .enumerate()
            .map(|(n, text)| {
                let edge = &edge;
                let destination = directory.join(format!("line-{n}.mp3"));
                scope.spawn(move || {
                    edge.speak(
                        &Request {
                            text,
                            voice: DEFAULT_VOICE,
                            settings: &[],
                        },
                        &destination,
                    )
                    .map(|spoken| spoken.bytes)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("no panic"))
            .collect()
    });

    for (n, result) in results.iter().enumerate() {
        assert!(
            result.is_ok(),
            "line {n}: {:?}",
            result.as_ref().err().map(ToString::to_string)
        );
    }
    let mut left = leftovers(&directory);
    left.sort();
    assert_eq!(left.len(), 8, "eight files, no temporaries: {left:?}");
    let _ = std::fs::remove_dir_all(&directory);
}
