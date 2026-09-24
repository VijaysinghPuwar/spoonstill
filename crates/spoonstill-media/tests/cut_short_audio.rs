//! D-190, against the artifacts FFmpeg really writes.
//!
//! `whole_wav` rests on one claim about somebody else's program: that every
//! normalized WAV FFmpeg writes to a seekable file carries a RIFF size equal
//! to its length less eight. The unit tests beside it can only state that
//! claim over bytes they built themselves, so here it is checked against all
//! three kinds of artifact the audio cache holds — silence, a supplied
//! recording, and a trimmed "spoken" line — and then each is cut short the way
//! a crash or a network volume cuts one (D-178) and must be refused.
//!
//! Without the check the 200-byte case is the defect in full: `ffprobe` reads
//! it as 48 kHz stereo s16 of 0.0006 s, `measure` accepts it, and the scene
//! renders as one frame in a film reported as a success.

mod common;

use std::path::{Path, PathBuf};

use spoonstill_core::diagnostics::Noop;
use spoonstill_media::audio::{Shape, Trim};
use spoonstill_media::{MediaError, measure, normalize, silence};

fn artifacts(dir: &Path) -> Vec<(&'static str, PathBuf, f64)> {
    let tools = common::tools();
    let log = Noop;
    let source = common::narration(2.0);

    let silent = silence(&tools, 96_000, &dir.join("silent.wav"), &log).expect("silence");
    let supplied = normalize(
        &tools,
        &source,
        &dir.join("supplied.wav"),
        &Shape::as_supplied(),
        &log,
    )
    .expect("normalize a supplied recording");
    let spoken = normalize(
        &tools,
        &source,
        &dir.join("spoken.wav"),
        &Shape::spoken(Trim {
            head_seconds: 0.10,
            tail_seconds: 0.25,
        }),
        &log,
    )
    .expect("normalize a spoken line");

    vec![
        ("silence", silent.path, silent.duration),
        ("a supplied recording", supplied.path, supplied.duration),
        ("a spoken line", spoken.path, spoken.duration),
    ]
}

/// What FFmpeg writes passes, and measures the length it was made at.
///
/// The half that stops the check becoming a refusal of every cache entry —
/// which would re-normalize a whole project on every render and never say so.
#[test]
fn every_artifact_ffmpeg_writes_is_whole() {
    let dir = common::out_dir("cut_short_whole");
    for (what, path, made) in artifacts(&dir) {
        let measured = measure(&common::tools(), &path)
            .unwrap_or_else(|e| panic!("{what} as FFmpeg wrote it was refused: {e}"));
        assert!(
            (measured.duration - made).abs() < 1e-6,
            "{what}: measured {} against {made} when it was made",
            measured.duration
        );
    }
}

/// Cut short anywhere, every artifact is refused rather than believed.
#[test]
fn an_artifact_cut_short_is_refused_not_measured() {
    let dir = common::out_dir("cut_short_refused");
    for (what, path, _) in artifacts(&dir) {
        let whole = std::fs::read(&path).expect("read the artifact");
        // 200 bytes is D-184's case: the header survives, every sample is gone.
        // One byte short is the smallest cut there is.
        for keep in [200, whole.len() / 2, whole.len() - 1] {
            let cut = dir.join(format!("cut-{keep}.wav"));
            std::fs::write(&cut, &whole[..keep]).expect("write the cut copy");

            match measure(&common::tools(), &cut) {
                Err(MediaError::UnusableInput { detail, .. }) => assert!(
                    detail.contains("header declares"),
                    "{what} cut to {keep} bytes was refused for the wrong reason: {detail}"
                ),
                Err(other) => panic!("{what} cut to {keep} bytes: unexpected error {other}"),
                Ok(m) => panic!(
                    "{what} cut to {keep} of {} bytes was accepted as {:.4} s",
                    whole.len(),
                    m.duration
                ),
            }
        }
    }
}
