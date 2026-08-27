//! The M1 exit gates that are not the motion matrix: the D-033 SAR regression,
//! hostile inputs, and clean cancellation.

mod common;

use std::time::Duration;

use spoonstill_core::{Aspect, OutputSpec};
use spoonstill_media::error::MediaError;
use spoonstill_media::scene::{Cancel, SceneRequest, render_scene};
use spoonstill_media::{SegmentProfile, assert_matches_profile, probe};

/// plan.md M1 exit gate 3, and the reason D-033 exists.
///
/// A 1999x1001 source through `scale` + `zoompan` with no trailing `setsar`
/// produces SAR 30007:30000 and DAR 30007:16875 instead of 16:9 — the
/// `SAR 12160:12159` class of bug `Automated-Video-Generator` records as BUG
/// W2-1. Per D-041, that segment would then concatenate with exit code 0 and no
/// warning.
///
/// The fixture is asserted to be genuinely odd by `common::odd_still` before it
/// is used, because an even one would make this test pass for the wrong reason
/// forever (`ffmpeg-findings.md` §8b).
#[test]
fn odd_dimensions_sar() {
    let tools = common::tools();
    let dir = common::out_dir("odd-dimensions-sar");

    // Every aspect, because the trap is about the source being odd, and the
    // resulting rounding differs per output aspect.
    for aspect in Aspect::ALL {
        let out = dir.join(format!("odd-{}.mp4", aspect.as_str().replace(':', "x")));
        let output = OutputSpec::new(aspect, 360, 30).unwrap();

        let mut request = SceneRequest::new(
            common::odd_still(),
            common::narration(1.0),
            out.clone(),
            output,
        );
        request.encode.preset = "veryfast".to_string();

        render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap_or_else(|e| panic!("odd source into {aspect}: {e}"));

        let probed = probe(&tools, &out, Duration::from_secs(60)).unwrap();
        let video = probed.video().unwrap();

        assert_eq!(
            video.sample_aspect_ratio.as_deref(),
            Some("1:1"),
            "a 1999x1001 source rendered to {aspect} produced SAR {:?}. \
             setsar=1 must be the last filter before format=yuv420p (D-033).",
            video.sample_aspect_ratio
        );
        assert_eq!(
            (video.width, video.height),
            (Some(output.width()), Some(output.height()))
        );

        // And the whole profile, because SAR is only the mismatch we know
        // about — the gate is uniformity, not one field (D-040).
        assert_matches_profile(&SegmentProfile::for_output(output), &probed)
            .unwrap_or_else(|m| panic!("odd source into {aspect}: {m:?}"));
    }
}

/// plan.md M1 exit gate 4. D-052: hostile input is the normal case.
///
/// The real proof that arguments are vectors rather than shell strings is a
/// filename that would be several words, a redirection and a substitution if it
/// ever reached a shell.
#[test]
fn hostile_paths_survive_the_process_boundary() {
    let tools = common::tools();
    let root = common::out_dir("hostile-paths");

    // What counts as a hostile *name* is platform-specific, and Win32 refuses
    // two of these before any of our code runs: `|` is a reserved character,
    // and a directory whose name ends in a space is `InvalidFilename` (error
    // 123). A test cannot assert that we survive a name the operating system
    // will not create — so the shapes they stand for, a shell metacharacter
    // and awkward surrounding whitespace, are covered by names that exist
    // everywhere, and the two POSIX-only ones still run where they are legal.
    // D-090; the macOS coverage is unchanged.
    let mut names = vec![
        "ünïcode spaced 名前",
        "it's a $(pwd) `name`",
        "semi;colon &and& ampersand",
        " leading space",
        "-leading-dash",
    ];
    if cfg!(unix) {
        names.push("semi;colon &and& pipe|");
        names.push("trailing space ");
    }

    for (index, name) in names.iter().enumerate() {
        // The hostile text is in the directory as well as the filenames.
        let dir = root.join(format!("{index} {name}"));
        let image = common::copy_to(&common::still(2000, 2000), &dir, &format!("{name}.jpg"));
        let audio = common::copy_to(&common::narration(0.5), &dir, &format!("{name}.wav"));
        let out = dir.join(format!("{name}.mp4"));

        let output = OutputSpec::new(Aspect::Square1x1, 360, 30).unwrap();
        let mut request = SceneRequest::new(image, audio, out.clone(), output);
        request.encode.preset = "veryfast".to_string();

        let rendered = render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap_or_else(|e| panic!("path {name:?}: {e}"));

        assert!(
            out.exists(),
            "path {name:?}: no segment at {}",
            out.display()
        );
        assert_eq!(rendered.frames, 15, "path {name:?}");

        // Nothing the filename could have done to a shell actually happened.
        assert!(
            !root.join("pwd").exists(),
            "a command substitution in a filename was executed"
        );
    }
}

/// plan.md M1 exit gate 5. D-045: cancellation is graceful, then forced, then
/// clean.
///
/// The requirement is precise: the destination is "absent, or present and
/// marked partial — never a valid-looking stub". spoonstill takes the first
/// branch, because nothing is ever written to the destination path until the
/// segment has passed the profile assertion (D-042).
#[test]
fn cancellation_leaves_no_valid_looking_stub() {
    let tools = common::tools();
    let dir = common::out_dir("cancellation");
    let out = dir.join("segment.mp4");

    // Long enough and large enough that the render is still running when the
    // cancellation lands.
    let output = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap();
    let request = SceneRequest::new(
        common::still(4000, 3000),
        common::narration(20.0),
        out.clone(),
        output,
    );

    let cancel = Cancel::new();
    let trigger = cancel.clone();

    // Cancel once the encode has demonstrably started, rather than after a
    // fixed sleep: a timing-based test that cancels before FFmpeg has opened
    // its output file would pass without exercising anything.
    let mut seen_progress = false;
    let error = render_scene(
        &tools,
        &request,
        &cancel,
        &spoonstill_core::diagnostics::Noop,
        &mut |progress| {
            if !seen_progress && progress.frame.unwrap_or(0) > 0 {
                seen_progress = true;
                trigger.request();
            }
        },
    )
    .expect_err("a cancelled render must not report success");

    assert!(
        matches!(error, MediaError::Cancelled { .. }),
        "expected a cancellation, got: {error}"
    );
    assert!(
        seen_progress,
        "the render was cancelled before it started, so this test proved nothing"
    );

    assert!(
        !out.exists(),
        "a cancelled render left {} behind. Nothing may reach the destination \
         path until it has passed the profile assertion (D-042).",
        out.display()
    );

    // And no partial file is left littering the output directory either.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leftovers.is_empty(),
        "a cancelled render left files behind: {leftovers:?}"
    );
}

/// A truncated image must fail with a named cause, not a panic and not a
/// plausible-looking segment.
#[test]
fn a_truncated_image_is_refused_by_name() {
    let tools = common::tools();
    let dir = common::out_dir("truncated-image");

    let good = common::still(4000, 3000);
    let truncated = dir.join("truncated.jpg");
    let bytes = std::fs::read(&good).unwrap();
    std::fs::write(&truncated, &bytes[..4096]).unwrap();

    let out = dir.join("segment.mp4");
    let request = SceneRequest::new(
        truncated,
        common::narration(0.5),
        out.clone(),
        OutputSpec::new(Aspect::Square1x1, 360, 30).unwrap(),
    );

    let error = render_scene(
        &tools,
        &request,
        &Cancel::new(),
        &spoonstill_core::diagnostics::Noop,
        &mut |_| {},
    )
    .expect_err("a truncated image cannot render");
    assert!(
        error.to_string().contains("truncated.jpg"),
        "the error must name the file: {error}"
    );
    assert!(!out.exists(), "a failed render must leave no segment");
}

/// A zero-byte narration must be rejected with the file named, not turned into
/// a one-frame segment.
#[test]
fn empty_narration_is_refused_by_name() {
    let tools = common::tools();
    let dir = common::out_dir("empty-narration");

    let empty = dir.join("zero_byte.mp3");
    std::fs::write(&empty, b"").unwrap();

    let out = dir.join("segment.mp4");
    let request = SceneRequest::new(
        common::still(2000, 2000),
        empty,
        out.clone(),
        OutputSpec::new(Aspect::Square1x1, 360, 30).unwrap(),
    );

    let error = render_scene(
        &tools,
        &request,
        &Cancel::new(),
        &spoonstill_core::diagnostics::Noop,
        &mut |_| {},
    )
    .expect_err("an empty narration cannot drive a scene");
    assert!(
        error.to_string().contains("zero_byte.mp3"),
        "the error must name the file: {error}"
    );
    assert!(!out.exists());
}

/// D-035, end to end: the same scene identity renders byte-identically.
///
/// This is what makes the cache safe (D-043) and resume meaningful. An
/// unseeded `random.choice()` — `ffmpeg-ai`'s approach — would break both.
#[test]
fn the_same_scene_identity_renders_identically() {
    let tools = common::tools();
    let dir = common::out_dir("deterministic-render");

    let image = common::still(4000, 3000);
    let audio = common::narration(1.0);
    let output = OutputSpec::new(Aspect::Landscape16x9, 360, 30).unwrap();

    let mut bytes = Vec::new();
    for run in 0..2 {
        let out = dir.join(format!("run-{run}.mp4"));
        let mut request = SceneRequest::new(image.clone(), audio.clone(), out.clone(), output);
        request.project_id = "determinism".to_string();
        request.scene_index = 7;
        request.encode.preset = "veryfast".to_string();

        let rendered = render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap();
        // The move is chosen from identity, so both runs must pick the same one.
        assert_eq!(
            rendered.motion.descriptor(),
            spoonstill_core::MotionSpec::seeded(
                "determinism",
                7,
                &format!(
                    "{:016x}",
                    spoonstill_core::hash::fnv1a(&std::fs::read(&image).unwrap())
                )
            )
            .descriptor()
        );
        bytes.push(std::fs::read(&out).unwrap());
    }

    assert_eq!(
        bytes[0].len(),
        bytes[1].len(),
        "two renders of the same scene identity differ in size"
    );
    assert!(
        bytes[0] == bytes[1],
        "two renders of the same scene identity are not byte-identical, so the \
         cache key of D-043 cannot be trusted"
    );
}
