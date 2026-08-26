//! `ffmpeg-findings.md` §9, ported into CI.
//!
//! > "A measurement that only ever ran once, by hand, on one machine, is a fact
//! > with a short shelf life."
//!
//! plan.md M1 calls this "the M1 test that matters" and specifies the matrix:
//! three durations x two frame rates x every V1 aspect ratio x
//! landscape/portrait/square sources x ASCII/Unicode/spaced paths. For each
//! combination it asserts exact frame count, exact duration, SAR 1:1, and no
//! black edge on the first, middle and last frame.
//!
//! # What runs, and what is sampled
//!
//! The full cross product is 3 x 2 x 3 x 3 x 3 = 162 renders. This test runs
//! the 54-render cross product of duration x frame rate x aspect x source
//! shape, and cycles the three path styles across those 54 rather than
//! multiplying by them — every path style is exercised against every aspect and
//! every source shape, without rendering the same pixels three times.
//!
//! That is a deliberate sampling and it is stated here rather than left to be
//! inferred, because a bounded test that reads as exhaustive is worse than an
//! honestly bounded one. Set `SPOONSTILL_FULL_MATRIX=1` to render all 162.
//!
//! # Why the outputs are small
//!
//! The matrix renders at a 360-pixel short edge, not 1080. Every property it
//! asserts — frame count, duration, SAR, black edges — is scale-invariant, and
//! the prescale rule under test is a *ratio* (D-032), which is preserved. The
//! production size is covered separately by
//! [`the_production_recipe_at_1080p_is_frame_exact`], which renders the exact
//! case from `ffmpeg-findings.md` §7 and checks it against the measured answer.

mod common;

use std::time::Duration;

use spoonstill_core::{Aspect, OutputSpec};
use spoonstill_media::scene::{Cancel, SceneRequest, render_scene};
use spoonstill_media::{SegmentProfile, assert_matches_profile, probe_counting_frames};

/// The three durations. 3.717 s is the reference from `ffmpeg-findings.md` §7,
/// chosen because it is deliberately *not* a frame multiple at any of our
/// frame rates, so the D-022 padding arithmetic is exercised rather than
/// coincidentally satisfied.
const DURATIONS: [f64; 3] = [0.5, 1.0, 3.717];
/// Two frame rates, both dividing 48 kHz exactly.
const FRAME_RATES: [u32; 2] = [24, 30];
/// Source shapes: landscape, portrait, square.
const SOURCES: [(u32, u32); 3] = [(4000, 3000), (3000, 4000), (2000, 2000)];
/// D-052: hostile paths are the normal case, not an edge case.
const PATH_STYLES: [&str; 3] = ["ascii", "ünïcode 名前", "spaced name"];

/// Short edge for the matrix renders.
const SHORT_EDGE: u32 = 360;

struct Case {
    duration: f64,
    fps: u32,
    aspect: Aspect,
    source: (u32, u32),
    path_style: &'static str,
}

fn cases() -> Vec<Case> {
    let full = std::env::var_os("SPOONSTILL_FULL_MATRIX").is_some();
    let mut out = Vec::new();
    let mut cycle = 0;
    for &duration in &DURATIONS {
        for &fps in &FRAME_RATES {
            for aspect in Aspect::ALL {
                for &source in &SOURCES {
                    if full {
                        for &path_style in &PATH_STYLES {
                            out.push(Case {
                                duration,
                                fps,
                                aspect,
                                source,
                                path_style,
                            });
                        }
                    } else {
                        out.push(Case {
                            duration,
                            fps,
                            aspect,
                            source,
                            path_style: PATH_STYLES[cycle % PATH_STYLES.len()],
                        });
                        cycle += 1;
                    }
                }
            }
        }
    }
    out
}

#[test]
fn motion_matrix() {
    let tools = common::tools();
    let root = common::out_dir("motion-matrix");
    let cases = cases();

    println!(
        "motion matrix: {} renders at a {SHORT_EDGE}px short edge \
         ({} durations x {} frame rates x {} aspects x {} source shapes, \
         path styles cycled across them).",
        cases.len(),
        DURATIONS.len(),
        FRAME_RATES.len(),
        Aspect::ALL.len(),
        SOURCES.len(),
    );
    if std::env::var_os("SPOONSTILL_FULL_MATRIX").is_none() {
        println!(
            "  path styles are cycled, not crossed — set SPOONSTILL_FULL_MATRIX=1 \
             for all {} combinations.",
            DURATIONS.len()
                * FRAME_RATES.len()
                * Aspect::ALL.len()
                * SOURCES.len()
                * PATH_STYLES.len()
        );
    }

    for (index, case) in cases.iter().enumerate() {
        let (source_width, source_height) = case.source;
        let output = OutputSpec::new(case.aspect, SHORT_EDGE, case.fps)
            .expect("the matrix only uses renderable geometry");

        // Each case gets its own directory so the path style applies to the
        // directory as well as the file — a space in a parent directory has
        // broken more pipelines than a space in a filename.
        let dir = root.join(format!("{index:03}-{}", case.path_style));
        let image = common::copy_to(
            &common::still(source_width, source_height),
            &dir,
            &format!("still {}.jpg", case.path_style),
        );
        let audio = common::copy_to(
            &common::narration(case.duration),
            &dir,
            &format!("narration {}.wav", case.path_style),
        );
        let out = dir.join(format!("segment {}.mp4", case.path_style));

        let label = format!(
            "case {index}: {}s @ {}fps, {} out of a {source_width}x{source_height} source, \
             path style {:?}",
            case.duration, case.fps, case.aspect, case.path_style
        );

        let mut request = SceneRequest::new(image, audio, out.clone(), output);
        request.project_id = "motion-matrix".to_string();
        request.scene_index = index as u32;
        // The matrix is about geometry, not encoder tuning; a faster preset
        // changes none of the asserted fields and keeps the suite usable.
        request.encode.preset = "veryfast".to_string();

        let rendered = render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap_or_else(|e| panic!("{label}\n{e}"));

        // --- exact frame count, structurally (D-030) ---------------------
        let expected_frames = spoonstill_core::frames_for_duration(case.duration, case.fps);
        assert_eq!(rendered.frames, expected_frames, "{label}: frame count");

        // Counted by decoding, not read from a header.
        let probe = probe_counting_frames(&tools, &out, Duration::from_secs(60))
            .unwrap_or_else(|e| panic!("{label}\n{e}"));
        let video = probe.video().expect("the segment has a video stream");
        assert_eq!(
            video.nb_read_frames,
            Some(u64::from(expected_frames)),
            "{label}: decoded frame count"
        );

        // --- exact duration, and the D-022 pad ---------------------------
        let expected_duration = f64::from(expected_frames) / f64::from(case.fps);
        assert!(
            (rendered.duration - expected_duration).abs() < 1e-9,
            "{label}: duration {} != {expected_duration}",
            rendered.duration
        );
        assert!(
            rendered.pad >= 0.0 && rendered.pad < 1.0 / f64::from(case.fps),
            "{label}: pad {} is not within one frame — narration must be padded \
             up to the grid, never trimmed (D-022)",
            rendered.pad
        );

        // --- SAR 1:1 and the whole profile (D-033, D-040) ----------------
        assert_eq!(
            video.sample_aspect_ratio.as_deref(),
            Some("1:1"),
            "{label}: SAR"
        );
        let profile = SegmentProfile::for_output(output);
        assert_matches_profile(&profile, &probe)
            .unwrap_or_else(|m| panic!("{label}: profile mismatch: {m:?}"));

        // --- no black edge, first / middle / last (D-034) ----------------
        let last = expected_frames - 1;
        let probe_frames = [0, last / 2, last];
        let minima =
            common::border_luma_minima(&out, output.width(), output.height(), &probe_frames);
        for (frame, minimum) in probe_frames.iter().zip(&minima) {
            assert!(
                *minimum > 0,
                "{label}: frame {frame} has a black border (min luma {minimum}). \
                 Cover-fit into the prescale canvas happens before zoompan \
                 precisely so this cannot occur (D-034)."
            );
        }
    }
}

/// The probe used above must be capable of failing.
///
/// `ffmpeg-findings.md` §8b's lesson, applied to a probe rather than a fixture:
/// a check that encodes a hazard must assert that it still encodes it. A
/// border probe that always returned a positive number would make every
/// black-edge assertion in `motion_matrix` vacuous, and nothing else in the
/// suite would notice.
#[test]
fn the_black_edge_probe_catches_a_real_letterbox() {
    use spoonstill_media::command::FfmpegCommand;

    let tools = common::tools();
    let dir = common::out_dir("black-edge-control");
    let letterboxed = dir.join("letterboxed.mp4");

    // The exact failure D-034 removes: fit *inside* the frame and pad, instead
    // of cover-fitting into the prescale canvas.
    let mut c = FfmpegCommand::new(tools.ffmpeg());
    c.args(["-hide_banner", "-loglevel", "error", "-y"])
        .input(&common::still(4000, 3000))
        .arg("-vf")
        .arg(
            "scale=360:640:force_original_aspect_ratio=decrease,\
             pad=360:640:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p",
        )
        // One frame: a still with no `-loop` emits exactly one, whatever
        // `-frames:v` asks for (D-030, form 4). One is all this control needs.
        .args([
            "-frames:v",
            "1",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "18",
        ])
        .arg(&letterboxed);
    let finished = c
        .spawn()
        .expect("spawn ffmpeg")
        .wait_until(common::RENDER_TIMEOUT)
        .expect("within timeout");
    assert!(finished.status.success(), "{}", finished.stderr);

    let minima = common::border_luma_minima(&letterboxed, 360, 640, &[0]);
    assert!(
        minima.contains(&0),
        "the black-edge probe reported {minima:?} on a deliberately letterboxed \
         file. It must read 0 there, or every black-edge assertion in this file \
         is vacuous."
    );
}

/// The production recipe at the production size, against the one case in
/// `ffmpeg-findings.md` §7 whose answer was measured by hand on this machine.
///
/// The matrix runs small for speed; this proves the same code path produces the
/// documented result at 1920x1080 with the D-036 encoder settings.
#[test]
fn the_production_recipe_at_1080p_is_frame_exact() {
    let tools = common::tools();
    let dir = common::out_dir("production-1080p");
    let out = dir.join("segment.mp4");

    let output = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap();
    let request = SceneRequest::new(
        common::still(4000, 3000),
        common::narration(3.717),
        out.clone(),
        output,
    );

    let rendered = render_scene(
        &tools,
        &request,
        &Cancel::new(),
        &spoonstill_core::diagnostics::Noop,
        &mut |_| {},
    )
    .expect("the production recipe renders");

    // The measured answer: ceil(3.717 x 30) = 112 frames = 3.733333 s.
    assert_eq!(rendered.frames, 112);
    assert!((rendered.duration - 3.733_333_333).abs() < 1e-6);

    let probe = probe_counting_frames(&tools, &out, Duration::from_secs(120)).unwrap();
    let video = probe.video().unwrap();
    assert_eq!(video.nb_read_frames, Some(112));
    assert_eq!((video.width, video.height), (Some(1920), Some(1080)));
    assert_eq!(video.sample_aspect_ratio.as_deref(), Some("1:1"));
    assert_eq!(video.time_base.as_deref(), Some("1/90000"));
    assert_eq!(video.pix_fmt.as_deref(), Some("yuv420p"));

    // The full profile, including the colour fields D-037 pins.
    assert_matches_profile(&SegmentProfile::for_output(output), &probe)
        .expect("the production segment matches the profile");

    // Default encoder settings, not the matrix's faster preset (D-036).
    assert_eq!(request.encode.preset, "medium");
    assert_eq!(request.encode.crf, 18);
}
