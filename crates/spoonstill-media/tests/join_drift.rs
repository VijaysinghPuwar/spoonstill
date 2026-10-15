//! D-206, against the files FFmpeg really writes.
//!
//! An MP4 states a track's length in its edit list, in the *movie* timescale,
//! and FFmpeg's default for that is 1000. A 119-frame segment at 30 fps is
//! 3.966667 s and was declared 3.966 s. The concat demuxer starts each file at
//! the previous one's declared end, so every such join lost 0.67 ms: an
//! 800-scene film came out 38 ms short — every frame present, `nb_frames`
//! exact — and the film's own assertion refused it, four renders running.
//!
//! Two halves, because the defect has two: segments rendered now must declare
//! their real length, and segments cached *before* the fix — which keep their
//! cache key, so that 800 4K scenes are not re-encoded — must still join to a
//! film of exactly the right length.

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_core::diagnostics::Noop;
use spoonstill_core::{Aspect, OutputSpec};
use spoonstill_media::concat::{Expectation, concat};
use spoonstill_media::scene::{Cancel, SceneRequest, render_scene};
use spoonstill_media::{SegmentProfile, probe};

/// 119 frames: the length the author's own film had 52 of.
const FRAMES: u32 = 119;
const FPS: u32 = 30;

fn output() -> OutputSpec {
    OutputSpec::new(Aspect::Landscape16x9, 360, FPS).unwrap()
}

fn render(dir: &Path) -> PathBuf {
    let out = dir.join("seg-0001-aaaaaaaaaaaaaaaa.mp4");
    let mut request = SceneRequest::new(
        common::still(1376, 768),
        common::narration(3.966),
        out.clone(),
        output(),
    );
    request.encode.preset = "veryfast".to_string();
    let rendered = render_scene(
        &common::tools(),
        &request,
        &Cancel::new(),
        &Noop,
        &mut |_| {},
    )
    .expect("render the segment");
    assert_eq!(
        rendered.frames, FRAMES,
        "the fixture must be the 119-frame case"
    );
    out
}

/// The video track's declared length, in its own 1/90000 ticks.
fn declared_ticks(path: &Path) -> u64 {
    let probed = probe(&common::tools(), path, Duration::from_secs(60)).unwrap();
    let video = probed.video().expect("a video stream");
    (video.duration.expect("a declared duration") * 90_000.0).round() as u64
}

#[test]
fn a_rendered_segment_declares_its_real_length() {
    let dir = common::out_dir("join_drift_declared");
    let segment = render(&dir);
    assert_eq!(
        declared_ticks(&segment),
        u64::from(FRAMES) * 3_000,
        "a {FRAMES}-frame segment must declare {FRAMES} frames, not a millisecond-rounded length"
    );
}

#[test]
fn segments_cached_before_the_fix_join_to_the_exact_length() {
    let dir = common::out_dir("join_drift_legacy");
    let fresh = render(&dir);

    // A segment as builds before D-206 wrote 52 of the author's 800: the same
    // packets, with both edit lists 60 ticks (0.67 ms) short of them. Both,
    // because the demuxer advances by the *longer* track — rounding only the
    // video left the audio's real end in charge, and the join did not drift. Written
    // by patching the edit list rather than by re-encoding, because whether
    // FFmpeg rounds a given segment down depends on the whole filter chain —
    // a plain encode of this length rounds up, and a fixture that only
    // sometimes holds the hazard is the trap ffmpeg-findings §8b records.
    let legacy = dir.join("legacy.mp4");
    let mut bytes = std::fs::read(&fresh).unwrap();
    shorten_edit_lists(&mut bytes, 60);
    std::fs::write(&legacy, &bytes).unwrap();
    assert_eq!(
        declared_ticks(&legacy),
        u64::from(FRAMES) * 3_000 - 60,
        "the fixture must declare the defect's own length"
    );
    let file = probe(&common::tools(), &legacy, Duration::from_secs(60))
        .unwrap()
        .format_duration
        .unwrap();
    assert!(
        (file - 3.966).abs() < 1e-6,
        "the file as a whole must declare 3.966 s, which is what the demuxer reads; it declares {file}"
    );

    // Sixty joins lose 40 ms without the fix — more than the one frame the
    // film is allowed — so this fails against the old list.
    const COUNT: usize = 60;
    let joined = dir.join("segments");
    std::fs::create_dir_all(&joined).unwrap();
    let segments: Vec<PathBuf> = (0..COUNT)
        .map(|i| {
            let path = joined.join(format!("seg-{i:04}-aaaaaaaaaaaaaaaa.mp4"));
            std::fs::copy(&legacy, &path).unwrap();
            path
        })
        .collect();
    let segment_frames = vec![u64::from(FRAMES); COUNT];
    let frames: u64 = segment_frames.iter().sum();
    let expected = frames as f64 / f64::from(FPS);

    let film = concat(
        &common::tools(),
        &segments,
        &dir.join("film.mp4"),
        &Expectation {
            profile: &SegmentProfile::for_output(output()),
            frames,
            segment_frames: &segment_frames,
            duration: expected,
            fps: FPS,
        },
        &Cancel::new(),
        &Noop,
    )
    .expect("sixty legacy segments join into a film of the right length");

    let video = film.probe.video().unwrap();
    let actual = video.duration.unwrap();
    assert!(
        (actual - expected).abs() < 0.001,
        "the film is {actual} s against {expected} s: the joins drifted"
    );
}

/// Take `ticks` off every track's edit list. The fix writes the movie
/// timescale as 90 000, so a tick here is the video's own tick. Only the
/// version-0 single-entry form FFmpeg emits is understood; anything else fails
/// the test rather than patching blind.
fn shorten_edit_lists(bytes: &mut [u8], ticks: u32) {
    let mut found = 0;
    let mut from = 0;
    while let Some(offset) = bytes[from..].windows(4).position(|w| w == b"elst") {
        let at = from + offset;
        assert_eq!(bytes[at + 4], 0, "a version-0 edit list");
        let entries = u32::from_be_bytes(bytes[at + 8..at + 12].try_into().unwrap());
        assert_eq!(entries, 1, "a single edit");
        let field = at + 12..at + 16;
        let duration = u32::from_be_bytes(bytes[field.clone()].try_into().unwrap());
        bytes[field].copy_from_slice(&(duration - ticks).to_be_bytes());
        found += 1;
        from = at + 16;
    }
    assert_eq!(
        found, 2,
        "one edit list for the video and one for the audio"
    );
}
