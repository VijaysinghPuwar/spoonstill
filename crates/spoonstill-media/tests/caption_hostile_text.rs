//! Hostile text through the caption path, which is the one place an operator's
//! own words reach code that draws pixels.
//!
//! **What this is and is not about.** `cargo audit` records `ttf-parser` —
//! reached through `fontdue` — as unmaintained at its latest release, shipping
//! in every binary on every platform (D-129). It is tempting to call that "an
//! unmaintained font parser in the path that handles operator input", which
//! would be the project's one security-shaped exposure. It is not that, and
//! the distinction is the point:
//!
//! - `ttf-parser` parses **font files**. Every font here is `include_bytes!`d
//!   from this crate's own assets — three fixed Inter weights, byte-identical
//!   in every build (D-106, D-124). Nothing loads a font from a path, a config
//!   file, an environment variable, or a project. A parser that only ever sees
//!   three trusted, compile-time-fixed inputs is a much smaller risk than one
//!   fed attacker bytes, whatever its maintenance status.
//! - What operator text actually reaches is **our own** wrapping, shrinking,
//!   mask and blur code, plus glyph *lookup* by codepoint. Bugs there are ours.
//!
//! So this file leaves the dependency alone and hammers the code we wrote, with
//! the input an operator can actually supply: a `.txt` beside a photograph, or
//! a line typed into the window.
//!
//! Every case asserts the same three things — it returns, it does not panic,
//! and what it produces fits the canvas it was drawn on.

use spoonstill_core::captions::{Placement, SubtitleTheme};
use spoonstill_core::geometry::{Aspect, OutputSpec};
use spoonstill_media::caption::render_cue;

fn output() -> OutputSpec {
    OutputSpec::new(Aspect::Landscape16x9, 1080, 30).expect("1080p is a valid output")
}

/// The inputs. Each is something a real `.txt` file can contain.
fn hostile() -> Vec<(&'static str, String)> {
    vec![
        ("empty", String::new()),
        ("one space", " ".to_owned()),
        ("only whitespace", " \t \n  \t ".to_owned()),
        ("only newlines", "\n\n\n\n".to_owned()),
        // D-118's separator. Not whitespace, so it survives normalization.
        ("unit separator", "alpha\u{1f}beta".to_owned()),
        (
            "control characters",
            (0u8..0x20).map(|b| b as char).collect(),
        ),
        // No break opportunity anywhere: wrapping cannot help, so the line is
        // as wide as the text. This is the shape a long URL has too.
        ("one very long word", "A".repeat(1200)),
        (
            "a long URL",
            format!("https://example.com/{}", "a/".repeat(200)),
        ),
        // CJK has no spaces, so every sentence is one "word" to a naive wrap.
        ("CJK, no spaces", "日本語の字幕".repeat(60)),
        ("RTL Arabic", "مرحبا بالعالم ".repeat(40)),
        ("RTL Hebrew", "שלום עולם ".repeat(40)),
        // Multi-codepoint graphemes: ZWJ families, skin tones, flags.
        ("emoji ZWJ", "👨‍👩‍👧‍👦🏳️‍🌈👍🏽 ".repeat(20)),
        (
            "combining marks",
            "e\u{301}\u{302}\u{303}\u{304}".repeat(100),
        ),
        ("zero width", "a\u{200b}b\u{200d}c\u{feff}".repeat(100)),
        // Almost certainly not in Inter: exercises the .notdef path.
        ("no glyph in Inter", "\u{10450}\u{1D400}\u{2E80}".repeat(80)),
        ("astral plane", "𝕳𝖊𝖑𝖑𝖔 ".repeat(80)),
        // Size is not this test's job — `a_legal_caption_file_on_a_short_scene`
        // below carries the 256 KiB case on its own, once, because rendering it
        // across twelve theme/placement pairs cost five minutes for no extra
        // information: the shapes are what vary here, not the length.
        (
            "mixed everything",
            "Hello \u{1f}👨‍👩‍👧 مرحبا 日本語 e\u{301} \u{200b}".repeat(100),
        ),
    ]
}

#[test]
fn hostile_text_never_panics_and_always_fits_the_canvas() {
    let output = output();
    let frame_w = output.width();
    let frame_h = output.height();

    for theme in SubtitleTheme::ALL {
        for placement in Placement::ALL {
            for (name, text) in hostile() {
                let image = render_cue(&text, theme, placement, output);
                let w = image.canvas.width();
                let h = image.canvas.height();

                // The band is the full frame width by contract, and `x` is
                // always zero, so the only way to run off the picture is
                // vertically — which is exactly what an unbreakable word or a
                // 256 KiB cue would do by forcing more lines than fit.
                assert_eq!(
                    w, frame_w,
                    "{theme:?}/{placement:?}/{name}: band is {w}px on a {frame_w}px frame",
                );
                assert!(
                    image.y + h <= frame_h,
                    "{theme:?}/{placement:?}/{name}: {h}px band at y={} runs off a \
                     {frame_h}px frame",
                    image.y,
                );

                // A composite reads width * height * 4 bytes. A buffer that
                // disagrees with its own dimensions is a panic or a garbled
                // frame later, far away from here.
                assert_eq!(
                    image.canvas.bytes().len(),
                    w as usize * h as usize * 4,
                    "{theme:?}/{placement:?}/{name}: buffer does not match its dimensions",
                );
            }
        }
    }
}

/// The smallest frame the geometry allows is where a long unbreakable word is
/// most likely to overflow, because the shrink floor is reached soonest.
#[test]
fn a_word_that_cannot_be_broken_still_fits_a_small_frame() {
    for short_edge in [360, 540, 720] {
        let output =
            OutputSpec::new(Aspect::Portrait9x16, short_edge, 30).expect("a valid short edge");
        for theme in SubtitleTheme::ALL {
            let image = render_cue(&"W".repeat(2000), theme, Placement::Bottom, output);
            assert!(
                image.y + image.canvas.height() <= output.height(),
                "{theme:?} at {short_edge}: a {}px band at y={} on a {}px frame",
                image.canvas.height(),
                image.y,
                output.height(),
            );
        }
    }
}

/// The case that is reachable through the documented pipeline, kept as its own
/// test because the numbers are the argument (D-139).
///
/// D-126 permits a script file of 256 KiB and calls it valid. D-106 makes a
/// `.txt` beside a recording the caption for that scene. So a photograph, a
/// three-second recording and a 256 KiB `.txt` is a project `still validate`
/// accepts — and `cues()` bounds the *number* of cues by the scene's duration,
/// so a short scene forces the text into very few, very long ones.
///
/// Measured against the unfixed renderer: 3 cues, the longest 87 334
/// characters, drawn as a **1920x39 839** band — 291.8 MB of RGBA for one cue
/// on a 1080p frame, and 49 seconds. With the clamp: 1920x1053, 7.7 MB, 7.8 s.
#[test]
fn a_legal_caption_file_on_a_short_scene_stays_inside_the_frame() {
    let text = "word ".repeat(52_400); // ~256 KiB, the D-126 ceiling
    let theme = SubtitleTheme::Classic;
    let cues = spoonstill_core::captions::cues(&text, 3.0, theme.style().max_chars);

    let longest = cues
        .iter()
        .max_by_key(|c| c.text.len())
        .expect("a 256 KiB caption produces at least one cue");
    assert!(
        longest.text.chars().count() > 50_000,
        "the premise has changed: the longest cue is only {} chars, so this test \
         no longer exercises the case it was written for",
        longest.text.chars().count(),
    );

    let output = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).expect("1080p");
    let image = render_cue(&longest.text, theme, Placement::Bottom, output);

    assert!(
        image.y + image.canvas.height() <= output.height(),
        "a {}px band at y={} on a {}px frame",
        image.canvas.height(),
        image.y,
        output.height(),
    );

    // The allocation is the other half of the defect. One cue is now single
    // figures of megabytes rather than most of a gigabyte, and `--jobs`
    // multiplies whichever it is.
    let mb = image.canvas.bytes().len() as f64 / 1_048_576.0;
    assert!(
        mb < 32.0,
        "one cue allocated {mb:.1} MB; it was 291.8 MB before the clamp",
    );
}
