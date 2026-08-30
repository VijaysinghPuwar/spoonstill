//! Burned-in subtitles, against real FFmpeg (D-106).
//!
//! The unit tests in `src/caption.rs` prove the pixels are drawn correctly.
//! This file proves the other half, which is the half that can silently not
//! happen: that the drawn pixels reach the encoded segment, at the right times,
//! **without disturbing the segment profile**. FFmpeg will happily accept an
//! `overlay` that changes the pixel format or drops the SAR and exit 0 about it
//! — `ffmpeg-findings.md` §5 is the same lesson about concat.
//!
//! Nothing here asserts the profile by hand. `render_scene` refuses to move a
//! segment into place unless it matches `SegmentProfile` (D-041, D-042), so a
//! render that *returns* is a render that passed every colour, SAR, pixel
//! format, timescale and frame-count check. That is why these tests mostly
//! assert about pixels and timing: the rest is already guarded.

mod common;

use std::path::Path;

use spoonstill_core::captions::{Cue, Placement, SubtitleSpec, SubtitleTheme};
use spoonstill_core::{Aspect, OutputSpec};
use spoonstill_media::command::FfmpegCommand;
use spoonstill_media::scene::{Cancel, SceneRequest, render_scene};

/// A three-second scene at 640x360 — small enough to be quick, big enough for
/// a caption to be a caption.
fn output() -> OutputSpec {
    OutputSpec::new(Aspect::Landscape16x9, 360, 30).expect("360p is a valid output")
}

const NARRATION: f64 = 3.0;

fn render(name: &str, subtitles: Option<SubtitleSpec>) -> std::path::PathBuf {
    let tools = common::tools();
    let dir = common::out_dir(name);
    let out = dir.join("segment.mp4");

    let mut request = SceneRequest::new(
        common::still(1600, 900),
        common::narration(NARRATION),
        out.clone(),
        output(),
    );
    request.encode.preset = "veryfast".to_string();
    request.subtitles = subtitles;

    render_scene(
        &tools,
        &request,
        &Cancel::new(),
        &spoonstill_core::diagnostics::Noop,
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"));

    out
}

/// Mean luma of the bottom strip of one frame, at time `t`.
///
/// The bottom strip is where every theme in [`Placement::Bottom`] puts its
/// caption, so this is the one number that says "there is a caption on screen".
fn bottom_luma(path: &Path, t: f64) -> f64 {
    let raw = path.with_extension(format!("t{}.gray", (t * 1000.0) as u32));
    let mut command = FfmpegCommand::new(common::tools().ffmpeg());
    command
        .args(["-hide_banner", "-v", "error", "-y"])
        .arg("-ss")
        .arg(format!("{t:.3}"))
        .input(path)
        .args(["-frames:v", "1"])
        // The bottom eighth of the frame, full width.
        .args(["-vf", "crop=640:45:0:315"])
        .args(["-f", "rawvideo", "-pix_fmt", "gray"])
        .arg(&raw);

    let finished = command
        .spawn()
        .expect("ffmpeg starts")
        .wait()
        .expect("ffmpeg runs");
    assert!(
        finished.status.success(),
        "sampling {} at {t}s: {}",
        path.display(),
        finished.stderr
    );

    let bytes = std::fs::read(&raw).expect("the sampled frame");
    assert!(!bytes.is_empty(), "no pixels sampled at {t}s");
    let total: u64 = bytes.iter().map(|&b| u64::from(b)).sum();
    total as f64 / bytes.len() as f64
}

fn one_cue(text: &str, start: f64, end: f64, theme: SubtitleTheme) -> SubtitleSpec {
    SubtitleSpec {
        theme,
        placement: Placement::Bottom,
        cues: vec![Cue {
            text: text.to_string(),
            start,
            end,
        }],
    }
}

/// The headline: a caption reaches the encoded segment, and the segment still
/// passes every profile assertion — which `render_scene` returning at all
/// already proves (D-041).
#[test]
fn a_caption_is_burned_into_the_segment() {
    let plain = render("subtitles-plain", None);
    let captioned = render(
        "subtitles-burned",
        Some(one_cue("A caption.", 0.0, NARRATION, SubtitleTheme::Band)),
    );

    let without = bottom_luma(&plain, 1.0);
    let with = bottom_luma(&captioned, 1.0);
    assert!(
        with < without - 30.0,
        "the band theme did not darken the bottom of the frame: \
         {with:.1} with subtitles against {without:.1} without"
    );
}

/// The `enable` window is real: the caption is there while the cue runs and
/// gone afterwards, in the *same* segment. A subtitle that is simply burned
/// over the whole scene would pass the test above and fail this one.
#[test]
fn a_cue_appears_and_leaves_on_time() {
    let path = render(
        "subtitles-timing",
        // On for the first second of a three-second scene.
        Some(one_cue("Only at the start.", 0.0, 1.0, SubtitleTheme::Band)),
    );

    let during = bottom_luma(&path, 0.5);
    let after = bottom_luma(&path, 2.0);
    assert!(
        during < after - 30.0,
        "the cue did not leave the screen: {during:.1} at 0.5s against {after:.1} at 2.0s"
    );
}

/// Consecutive cues tile without a frame that shows both or neither.
///
/// The half-open `gte(t,start)*lt(t,end)` window is what makes this true;
/// `between(t,start,end)` is closed at both ends and would draw the shorter
/// band under the taller one on every shared frame.
#[test]
fn consecutive_cues_tile_without_a_seam() {
    let spec = SubtitleSpec {
        theme: SubtitleTheme::Band,
        placement: Placement::Bottom,
        cues: vec![
            Cue {
                text: "First half.".into(),
                start: 0.0,
                end: 1.5,
            },
            Cue {
                text: "Second half, which is a good deal longer than the first.".into(),
                start: 1.5,
                end: NARRATION,
            },
        ],
    };
    let path = render("subtitles-tiling", Some(spec));

    // Every sample across the scene must have a caption on screen, including
    // the frames either side of the boundary at 1.5s.
    for t in [0.1, 0.7, 1.4, 1.5, 1.6, 2.2, 2.9] {
        let luma = bottom_luma(&path, t);
        assert!(
            luma < 90.0,
            "no caption on screen at {t}s (luma {luma:.1}) — the cues left a gap"
        );
    }
}

/// Every theme survives a real encode. This is the matrix that would catch a
/// theme whose band is taller than the frame, or whose colours make FFmpeg
/// choose a different pixel format.
#[test]
fn every_theme_renders_a_conforming_segment() {
    for theme in SubtitleTheme::ALL {
        // A returning render is a passing profile assertion (D-041).
        render(
            &format!("subtitles-theme-{}", theme.as_str()),
            Some(one_cue(
                "The tide came in before dawn, and by seven the harbour was full.",
                0.0,
                NARRATION,
                theme,
            )),
        );
    }
}

/// The bands are transient. A project rendered twice must not accumulate a
/// directory of loose `.rgba` files beside its segments.
#[test]
fn the_caption_bands_are_cleaned_up() {
    let path = render(
        "subtitles-cleanup",
        Some(one_cue("Tidy.", 0.0, NARRATION, SubtitleTheme::Boxed)),
    );
    let dir = path.parent().expect("the output directory");

    let leftovers: Vec<_> = std::fs::read_dir(dir)
        .expect("readable output directory")
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains("rgba") || n.contains("partial"))
        .collect();

    assert!(
        leftovers.is_empty(),
        "the render left temporary files behind: {leftovers:?}"
    );
}

/// A spec with no cues must produce the segment a project without subtitles
/// produces — byte for byte. That equality is what D-043's cache key rests on:
/// turning subtitles on for a scene that has no text must not invalidate it.
#[test]
fn a_spec_with_no_cues_renders_the_same_bytes_as_none() {
    let none = render("subtitles-none", None);
    let empty = render(
        "subtitles-empty",
        Some(SubtitleSpec {
            theme: SubtitleTheme::Punch,
            placement: Placement::Bottom,
            cues: Vec::new(),
        }),
    );
    assert_eq!(
        std::fs::read(&none).expect("segment"),
        std::fs::read(&empty).expect("segment"),
        "an empty subtitle spec changed the segment"
    );
}

/// D-117. A shadow must fit inside the band it is drawn on.
///
/// `Mask::shadow` runs three box blurs, and three passes of radius `r` spread
/// `3r` — but the canvas reserved `2r`, so the outermost ring of every soft
/// shadow was cut off against the canvas edge. Measured before the fix: `card`
/// and `minimal` left alpha at the bottom row at 720p, 1080p and 4K alike,
/// which is the scale-invariance of D-106 faithfully reproducing a bug.
///
/// The rule is stated as a property of the *output*, not of the arithmetic:
/// **a theme that casts a shadow leaves the edge of its canvas transparent.**
/// A theme with no shadow is excluded because its plate is *meant* to reach the
/// edge — `band` is the full frame width by design, and `boxed` runs flush top
/// and bottom.
#[test]
fn a_soft_shadow_is_not_clipped_by_the_edge_of_its_own_canvas() {
    // 4K was measured too, and clipped identically — but a 3840-wide triple
    // box blur is most of a minute in a debug build, and the property is
    // scale-invariant by construction (every length is a fraction of the
    // frame), so two sizes are the evidence and the third is in D-117.
    for short_edge in [720u32, 1080] {
        let output = OutputSpec::new(Aspect::Landscape16x9, short_edge, 30).expect("geometry");
        for theme in SubtitleTheme::ALL {
            if theme.style().shadow_blur <= 0.0 {
                continue;
            }
            for placement in [Placement::Bottom, Placement::Top] {
                let image = spoonstill_media::caption::render_cue(
                    "A line long enough to need the whole band, and then some more.",
                    theme,
                    placement,
                    output,
                );
                let (w, h) = (image.canvas.width(), image.canvas.height());
                let alpha = |x: u32, y: u32| image.canvas.pixel(x, y).a;

                let worst = (0..w)
                    .map(|x| alpha(x, 0).max(alpha(x, h - 1)))
                    .chain((0..h).map(|y| alpha(0, y).max(alpha(w - 1, y))))
                    .max()
                    .unwrap_or(0);
                assert_eq!(
                    worst, 0,
                    "{theme:?} at {short_edge}p {placement:?} leaves alpha {worst} on the \
                     edge of its {w}x{h} canvas — the shadow is being cut off there"
                );
            }
        }
    }
}
