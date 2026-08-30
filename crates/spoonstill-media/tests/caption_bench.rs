//! Rough timings for the caption rasterizer. `--ignored`, like the live TTS
//! suite: it measures rather than asserts.
use std::time::Instant;

use spoonstill_core::captions::{Placement, SubtitleTheme};
use spoonstill_core::{Aspect, OutputSpec};
use spoonstill_media::caption::render_cue;

const LINE: &str = "And talent mattered even more than strength. Those born with \
                    high aptitude cultivated smoothly and rose fast.";

#[test]
#[ignore]
fn how_long_does_a_cue_take() {
    println!("\n  size    theme     ms/cue   band px      RGBA");
    for (name, edge) in [("720p", 720u32), ("1080p", 1080), ("4K", 2160)] {
        let out = OutputSpec::new(Aspect::Landscape16x9, edge, 30).unwrap();
        for theme in SubtitleTheme::ALL {
            // warm the font cache
            let _ = render_cue(LINE, theme, Placement::Bottom, out);
            let n = 10;
            let t = Instant::now();
            let mut bytes = 0;
            for _ in 0..n {
                let img = render_cue(LINE, theme, Placement::Bottom, out);
                bytes = img.canvas.bytes().len();
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
            let img = render_cue(LINE, theme, Placement::Bottom, out);
            println!(
                "  {name:6}  {:9} {ms:6.1}   {}x{:<5}  {:5} KB",
                format!("{theme:?}"),
                img.canvas.width(),
                img.canvas.height(),
                bytes / 1024
            );
        }
    }
}
