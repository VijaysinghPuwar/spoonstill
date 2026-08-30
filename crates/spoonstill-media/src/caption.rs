//! Drawing a caption: text, outline, shadow and backdrop into RGBA pixels (D-106).
//!
//! # Why this exists at all
//!
//! FFmpeg can burn subtitles two ways, and this machine's FFmpeg can do
//! neither. Homebrew core split the formula: `brew install ffmpeg` — what
//! `README.md`, `still doctor --install` and D-105's Install button all reach
//! for — now yields a slim build with no `libass` and no `libfreetype`, so
//! there is no `subtitles` filter and no `drawtext` filter. Measured here on
//! 2026-08-29 against ffmpeg 8.0.1: `No such filter: 'drawtext'`.
//!
//! So we draw the pixels and hand FFmpeg an `overlay`, which is a core filter
//! present in every build there is. The cost is this module. The benefit is
//! that subtitles work on the FFmpeg the operator already has, which is the
//! difference between a feature and a feature request.
//!
//! # What comes out
//!
//! One [`CaptionImage`] per cue: a straight (non-premultiplied) RGBA band the
//! full width of the frame, plus the `y` it is overlaid at. Full width because
//! it makes the filter-graph half trivial — every overlay is at `x=0` — and
//! because [`spoonstill_core::captions::Backdrop::Band`] needs it anyway.
//!
//! # Order of operations, which is the whole design
//!
//! ```text
//! wrap to the frame  ->  shrink if it still will not fit
//!   -> shadow   (cast by the backdrop if there is one, by the text if not)
//!     -> backdrop
//!       -> outline   (the glyph mask, dilated)
//!         -> fill
//! ```
//!
//! The shadow rule is the one worth knowing: a theme with a plate wants the
//! plate to lift off the photograph, and a theme without one wants the letters
//! to. Casting both would be a drop shadow on a drop shadow.

use spoonstill_core::OutputSpec;
use spoonstill_core::captions::{Backdrop, Placement, Rgba, SubtitleTheme, ThemeStyle, Weight};

/// The three bundled weights, embedded rather than looked up on the machine.
///
/// A system font would make a theme mean something different on macOS than on
/// Windows, and would make the rendered film depend on what the operator has
/// installed — which is the opposite of every other thing in this program,
/// where the output is a function of the inputs and nothing else (D-077).
///
/// Inter, SIL Open Font License 1.1. The licence travels with the files, in
/// `assets/fonts/LICENSE-Inter.txt`.
const REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");
const BOLD: &[u8] = include_bytes!("../assets/fonts/Inter-Bold.ttf");

/// Smallest the type may be shrunk to when a cue will not fit in
/// [`ThemeStyle::max_lines`], as a fraction of the theme's own size.
///
/// Below this the caption stops being the theme the operator chose. A cue that
/// still will not fit at this size is allowed its extra line instead — an
/// overflowing caption is a design that slipped, a truncated one is a sentence
/// the viewer never got.
const MIN_SHRINK: f64 = 0.62;

/// A rectangle of straight (non-premultiplied) RGBA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Canvas {
    width: u32,
    height: u32,
    /// `width * height * 4` bytes, row-major, R G B A.
    pixels: Vec<u8>,
}

impl Canvas {
    /// A fully transparent canvas.
    #[must_use]
    pub fn new(width: u32, height: u32) -> Self {
        Canvas {
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }

    /// The raw RGBA bytes, as FFmpeg's `rawvideo` demuxer wants them.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.pixels
    }

    /// Consume the canvas for its bytes.
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.pixels
    }

    /// The pixel at `(x, y)`, or transparent black outside.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Rgba {
        if x >= self.width || y >= self.height {
            return Rgba::NONE;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        Rgba::new(
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        )
    }

    /// Composite `colour` over the pixel at `(x, y)`, scaled by `coverage`.
    ///
    /// Source-over on straight alpha. Written out rather than reached for from
    /// a crate because it is eight lines and because getting premultiplication
    /// wrong here would show up as a dark halo around every letter — the kind
    /// of defect that survives review by looking like antialiasing.
    fn blend(&mut self, x: u32, y: u32, colour: Rgba, coverage: u8) {
        if x >= self.width || y >= self.height || coverage == 0 || colour.a == 0 {
            return;
        }
        let sa = (u32::from(colour.a) * u32::from(coverage) / 255) as u8;
        if sa == 0 {
            return;
        }
        let i = ((y as usize) * (self.width as usize) + (x as usize)) * 4;
        let da = self.pixels[i + 3];

        let sa32 = u32::from(sa);
        let da32 = u32::from(da);
        // out_a = sa + da*(1-sa)
        let out_a = sa32 + da32 * (255 - sa32) / 255;
        if out_a == 0 {
            self.pixels[i..i + 4].fill(0);
            return;
        }
        for c in 0..3 {
            let sc = u32::from(match c {
                0 => colour.r,
                1 => colour.g,
                _ => colour.b,
            });
            let dc = u32::from(self.pixels[i + c]);
            // Straight-alpha source-over, rounded rather than truncated so a
            // long chain of blends does not drift dark.
            let num = sc * sa32 * 255 + dc * da32 * (255 - sa32);
            self.pixels[i + c] = u8::try_from((num / 255 + out_a / 2) / out_a).unwrap_or(255);
        }
        self.pixels[i + 3] = u8::try_from(out_a).unwrap_or(255);
    }
}

/// One cue, drawn, and where it belongs on the frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptionImage {
    /// The pixels. Always the full frame width.
    pub canvas: Canvas,
    /// Where the top of the band sits on the frame. `x` is always zero.
    pub y: u32,
}

/// An 8-bit coverage mask — one channel, the shape of something.
#[derive(Debug, Clone)]
struct Mask {
    width: usize,
    height: usize,
    a: Vec<u8>,
}

/// Box blur passes in a drop shadow, and therefore the shadow's reach.
///
/// Three passes of radius `r` spread `3r`, so this is both how the shadow is
/// built and how much room the canvas has to reserve for it. One constant
/// rather than a `3` in the blur and a `2` in the margin, because that
/// disagreement is exactly what D-117 was.
const SHADOW_BLUR_PASSES: usize = 3;

impl Mask {
    fn new(width: usize, height: usize) -> Self {
        Mask {
            width,
            height,
            a: vec![0; width * height],
        }
    }

    fn get(&self, x: usize, y: usize) -> u8 {
        if x >= self.width || y >= self.height {
            0
        } else {
            self.a[y * self.width + x]
        }
    }

    /// Take the maximum of what is there and what is being drawn.
    ///
    /// Maximum rather than addition: two glyphs whose antialiased edges touch
    /// must not sum to a bright seam between them.
    fn raise(&mut self, x: usize, y: usize, value: u8) {
        if x < self.width && y < self.height {
            let slot = &mut self.a[y * self.width + x];
            *slot = (*slot).max(value);
        }
    }

    /// Grow the shape by `radius` pixels in every direction.
    ///
    /// A disc, not a square. A square dilation is separable and twice as fast,
    /// and it puts a visible corner on the outside of every `o` — which is
    /// exactly the sort of thing that reads as "cheap" without the viewer being
    /// able to say why.
    fn dilate(&self, radius: usize) -> Mask {
        if radius == 0 {
            return self.clone();
        }

        // Decomposed into the disc's horizontal chords (D-130).
        //
        // The disc used to be a list of ~pi*r^2 offsets walked for every pixel,
        // which is `O(area * r^2)` — and `r` grows with the frame, so the cost
        // grows with the *fourth* power of the resolution. Measured: `punch` at
        // 4K took **604 ms for one cue**, against 41 ms at 1080p, a 14.6x step
        // for a 4x area. At 500 scenes that is ten minutes of drawing text.
        //
        // A disc is the union of its rows, and each row is an interval. So
        // dilating by the disc is the maximum, over each row offset `dy`, of a
        // *one-dimensional* dilation by that row's half-width — and a 1-D
        // dilation is `O(1)` per pixel with a sliding-window maximum. That
        // makes the whole thing `O(area * r)`, and it is the **same kernel**:
        // the half-widths are the same integers the offset list was built from,
        // so the output is identical, which is asserted rather than assumed.
        let r = radius as isize;
        let mut out = Mask::new(self.width, self.height);

        // Rows sharing a half-width share their work: `dy` and `-dy` always do,
        // and near the equator several do.
        let mut cache: std::collections::HashMap<usize, Vec<u8>> = std::collections::HashMap::new();

        for dy in -r..=r {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let half = ((r * r - dy * dy) as f64).sqrt().floor() as usize;
            let widened = cache
                .entry(half)
                .or_insert_with(|| self.widen_rows(half))
                .clone();

            for y in 0..self.height {
                let source = y as isize + dy;
                if source < 0 || source >= self.height as isize {
                    continue;
                }
                let row = source as usize * self.width;
                let target = y * self.width;
                for x in 0..self.width {
                    let value = widened[row + x];
                    if value > out.a[target + x] {
                        out.a[target + x] = value;
                    }
                }
            }
        }
        out
    }

    /// Every row dilated horizontally by `half`, with a sliding-window maximum.
    ///
    /// The inner half of [`Mask::dilate`] (D-130). A monotonic deque holds the
    /// indices whose values could still be the maximum of the window, so each
    /// pixel is pushed and popped at most once and the pass is `O(width)` per
    /// row however wide the window is.
    fn widen_rows(&self, half: usize) -> Vec<u8> {
        let mut out = vec![0u8; self.width * self.height];
        if self.width == 0 {
            return out;
        }
        let half = half as isize;
        let mut window: std::collections::VecDeque<usize> = std::collections::VecDeque::new();

        for y in 0..self.height {
            let row = y * self.width;
            window.clear();
            // Prime the window with everything the first pixel can see.
            let mut next = 0isize;
            for x in 0..self.width {
                let last = (x as isize + half).min(self.width as isize - 1);
                while next <= last {
                    let value = self.a[row + next as usize];
                    while window.back().is_some_and(|&i| self.a[row + i] <= value) {
                        window.pop_back();
                    }
                    window.push_back(next as usize);
                    next += 1;
                }
                // Drop what has fallen off the left edge of the window.
                let first = x as isize - half;
                while window.front().is_some_and(|&i| (i as isize) < first) {
                    window.pop_front();
                }
                out[row + x] = window.front().map_or(0, |&i| self.a[row + i]);
            }
        }
        out
    }

    /// Offset the shape down and to the right, then soften it.
    ///
    /// [`SHADOW_BLUR_PASSES`] box blurs, which is the standard cheap
    /// approximation of a Gaussian and is indistinguishable from one at the
    /// radii a drop shadow uses.
    fn shadow(&self, offset: usize, blur: usize) -> Mask {
        let mut out = Mask::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                if x >= offset && y >= offset {
                    out.a[y * self.width + x] = self.get(x - offset, y - offset);
                }
            }
        }
        for _ in 0..SHADOW_BLUR_PASSES {
            if blur > 0 {
                out = out.box_blur(blur);
            }
        }
        out
    }

    /// One separable box blur of the given radius, on a running sum (D-130).
    ///
    /// Separable already — a horizontal pass then a vertical one — but each
    /// pass re-added all `2r+1` samples for every pixel, so a pass was
    /// `O(area * r)` and the three of a shadow were `O(area * r)` again. A box
    /// blur does not need that: consecutive windows differ by one sample at
    /// each end, so the sum carries across and a pass becomes `O(area)`.
    ///
    /// Measured on the theme it matters most to, `card` at 4K, whose shadow
    /// radius is the largest any theme asks for: 245 ms a cue before, and the
    /// dilation fix alone did nothing for it because the blur was the cost.
    ///
    /// **The edges keep their old behaviour deliberately.** Samples off the
    /// end count as zero and the divisor stays `2r+1`, so a shadow fades into
    /// the border exactly as it did. That is not obviously the *best* rule, but
    /// changing it would change every rendered frame, and this is a
    /// performance decision — a test asserts the output is byte-identical.
    fn box_blur(&self, radius: usize) -> Mask {
        let span = (radius * 2 + 1) as u32;

        let mut mid = Mask::new(self.width, self.height);
        for y in 0..self.height {
            let row = y * self.width;
            // The window for x = 0 reaches from -r to +r; everything left of
            // the mask is zero, so it starts as the first r+1 samples.
            let mut sum: u32 = (0..=radius.min(self.width.saturating_sub(1)))
                .map(|x| u32::from(self.a[row + x]))
                .sum();
            for x in 0..self.width {
                mid.a[row + x] = u8::try_from(sum / span).unwrap_or(255);
                // Slide: the sample entering on the right, the one leaving on
                // the left. Both are zero when they fall outside.
                let entering = x + radius + 1;
                if entering < self.width {
                    sum += u32::from(self.a[row + entering]);
                }
                if x + 1 >= radius + 1 {
                    let leaving = x + 1 - (radius + 1);
                    sum -= u32::from(self.a[row + leaving]);
                }
            }
        }

        let mut out = Mask::new(self.width, self.height);
        for x in 0..self.width {
            let mut sum: u32 = (0..=radius.min(self.height.saturating_sub(1)))
                .map(|y| u32::from(mid.a[y * self.width + x]))
                .sum();
            for y in 0..self.height {
                out.a[y * self.width + x] = u8::try_from(sum / span).unwrap_or(255);
                let entering = y + radius + 1;
                if entering < self.height {
                    sum += u32::from(mid.a[entering * self.width + x]);
                }
                if y + 1 >= radius + 1 {
                    let leaving = y + 1 - (radius + 1);
                    sum -= u32::from(mid.a[leaving * self.width + x]);
                }
            }
        }
        out
    }

    /// Paint the whole mask onto a canvas in one colour.
    fn paint(&self, canvas: &mut Canvas, colour: Rgba) {
        if !colour.is_visible() {
            return;
        }
        for y in 0..self.height {
            for x in 0..self.width {
                let a = self.a[y * self.width + x];
                if a > 0 {
                    canvas.blend(x as u32, y as u32, colour, a);
                }
            }
        }
    }
}

/// One parsed weight, kept for the life of the process.
///
/// Parsing a 320 KB TTF is not free, and `render_cue` is called once per cue —
/// several times per scene, five hundred scenes at the design point. Parsing
/// per cue cost measured seconds across a hundred-scene film. There are three
/// of these and they never change, so they are parsed at most once each.
fn font(weight: Weight) -> &'static fontdue::Font {
    use std::sync::OnceLock;
    static FONTS: [OnceLock<fontdue::Font>; 3] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new()];

    let (slot, bytes) = match weight {
        Weight::Regular => (&FONTS[0], REGULAR),
        Weight::SemiBold => (&FONTS[1], SEMIBOLD),
        Weight::Bold => (&FONTS[2], BOLD),
    };
    slot.get_or_init(|| {
        // The bytes are `include_bytes!`d from this crate's own assets, so a
        // parse failure is a corrupt checkout, not an operator's input. There
        // is no useful recovery and no caller who could act on the error.
        fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
            .expect("a bundled Inter weight must parse; the checkout is damaged if it does not")
    })
}

/// A weight, plus the measurements taken from it so far.
///
/// The cache is what makes [`balance`] affordable. Bisecting for the narrowest
/// width that holds the same number of lines re-wraps the same words a dozen
/// times, and every wrap measures every candidate line — so one two-line
/// caption asked fontdue for the same glyph's advance some hundreds of times.
/// Keyed by `(char, px)` because the shrink loop changes `px`.
struct Fonts {
    font: &'static fontdue::Font,
    advance: std::cell::RefCell<std::collections::HashMap<(char, u32), f32>>,
    kern: std::cell::RefCell<std::collections::HashMap<(char, char, u32), f32>>,
}

impl Fonts {
    fn for_weight(weight: Weight) -> Fonts {
        Fonts {
            font: font(weight),
            advance: std::cell::RefCell::new(std::collections::HashMap::new()),
            kern: std::cell::RefCell::new(std::collections::HashMap::new()),
        }
    }

    fn advance_of(&self, ch: char, px: f32) -> f32 {
        *self
            .advance
            .borrow_mut()
            .entry((ch, px.to_bits()))
            .or_insert_with(|| self.font.metrics(ch, px).advance_width)
    }

    fn kern_between(&self, left: char, right: char, px: f32) -> f32 {
        *self
            .kern
            .borrow_mut()
            .entry((left, right, px.to_bits()))
            .or_insert_with(|| self.font.horizontal_kern(left, right, px).unwrap_or(0.0))
    }

    /// Advance width of one line at `px`, kerning included.
    fn measure(&self, line: &str, px: f32) -> f32 {
        let mut width = 0.0;
        let mut previous: Option<char> = None;
        for ch in line.chars() {
            if let Some(p) = previous {
                width += self.kern_between(p, ch, px);
            }
            width += self.advance_of(ch, px);
            previous = Some(ch);
        }
        width
    }
}

/// Greedily wrap `text` into lines no wider than `limit`.
///
/// A word wider than the limit on its own gets its own line and is allowed to
/// overflow, rather than being broken mid-word: a hyphen we invented is a
/// spelling mistake on screen.
fn wrap(fonts: &Fonts, text: &str, px: f32, limit: f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if current.is_empty() || fonts.measure(&candidate, px) <= limit {
            current = candidate;
        } else {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// Re-wrap to the same number of lines, as evenly as they will go.
///
/// Greedy wrapping fills each line to the brim and leaves whatever is left on
/// the last one, which is how "…and by seven the harbour was" ends up above a
/// line reading "full." Two lines of similar length read as a caption; one full
/// line over an orphan reads as a mistake.
///
/// The trick is the standard one: the narrowest width that still produces the
/// same number of lines is the width at which those lines are most nearly
/// equal. Found by bisection, which costs a dozen measurements of text that is
/// at most a couple of lines long.
fn balance(fonts: &Fonts, text: &str, px: f32, limit: f32, lines: usize) -> Vec<String> {
    if lines < 2 {
        return wrap(fonts, text, px, limit);
    }
    let (mut lo, mut hi) = (0.0_f32, limit);
    for _ in 0..12 {
        let mid = (lo + hi) / 2.0;
        if wrap(fonts, text, px, mid).len() <= lines {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    let balanced = wrap(fonts, text, px, hi);
    // Never accept a rebalance that added a line, which bisection can do when
    // the text has a word wider than `hi`.
    if balanced.len() == lines {
        balanced
    } else {
        wrap(fonts, text, px, limit)
    }
}

/// Draw one cue.
///
/// The returned image is always the full width of `output`, so the caller
/// overlays it at `x=0`.
///
/// # Panics
///
/// Does not panic on any caption text. It will panic only if a bundled font
/// fails to parse, which means the binary's own assets are damaged.
#[must_use]
pub fn render_cue(
    text: &str,
    theme: SubtitleTheme,
    placement: Placement,
    output: OutputSpec,
) -> CaptionImage {
    let style = theme.style();
    let frame_w = output.width();
    let frame_h = output.height();
    let fonts = Fonts::for_weight(style.weight);

    let limit = (style.max_width * f64::from(frame_w)) as f32;

    // Fit the cue into the theme's line budget by shrinking the type, not by
    // narrowing the text. Six percent a step is small enough that the operator
    // reads it as "this line is a bit long" rather than as a different theme.
    let base_px = (style.size * f64::from(frame_h)).max(8.0);
    let mut px = base_px;
    let mut lines = wrap(&fonts, text, px as f32, limit);
    while lines.len() as u32 > style.max_lines && px > base_px * MIN_SHRINK {
        px = (px * 0.94).max(base_px * MIN_SHRINK);
        lines = wrap(&fonts, text, px as f32, limit);
    }
    let px_f = px as f32;
    let mut lines = balance(&fonts, text, px_f, limit, lines.len());

    // The loop above tries to reach `max_lines` by shrinking the type and gives
    // up at `MIN_SHRINK`. Nothing then stopped `draw` from allocating a band as
    // tall as however many lines were left over — and a band taller than the
    // frame cannot be shown, so those pixels are cost with no picture in them.
    //
    // Measured before this clamp (D-139): a three-second scene with a caption
    // file of 256 KiB — which D-126 explicitly permits, and which D-106 makes a
    // caption by putting a `.txt` beside a recording — split into three cues,
    // the longest 87 334 characters, and the first rendered as a **1920x39 839**
    // band: 291.8 MB of RGBA for one cue on a 1080p frame, 49 seconds to draw,
    // multiplied by `--jobs`. This is D-114's defect one layer down: there the
    // *output* geometry had no ceiling, here a geometry *derived* from it had
    // none.
    //
    // The clamp is the frame, not `style.max_lines`. Every theme declares two
    // lines, and enforcing that as a hard cap would silently shorten captions
    // that render three or four lines today and look fine — a change to films
    // that already exist. The frame is the honest limit: past it there is
    // nothing to see either way.
    // Mirrors `draw`'s own geometry rather than approximating it, because an
    // estimate that forgets the outline and the shadow puts the band back over
    // the edge — measured at 1094px on a 1080px frame before these terms were
    // included.
    let line_height = (px_f * style.line_spacing as f32).round().max(1.0);
    let pad_y = (style.padding_y * f64::from(px_f)) as f32;
    let outline_r = (style.outline_width * f64::from(px_f)).round().max(0.0) as f32;
    let shadow_off = (style.shadow_offset * f64::from(px_f)).round().max(0.0) as f32;
    let shadow_blur = (style.shadow_blur * f64::from(px_f)).round().max(0.0) as f32;
    #[allow(clippy::cast_precision_loss)]
    let margin = outline_r + shadow_off + shadow_blur * SHADOW_BLUR_PASSES as f32;

    let metrics = fonts
        .font
        .horizontal_line_metrics(px_f)
        .expect("a horizontal font has horizontal line metrics");
    let first_line = metrics.ascent - metrics.descent;

    // `draw` builds a band of
    //   line_height * (n - 1) + ascent + descent + 2 * pad_y + 2 * margin
    // so this is that, solved for n.
    #[allow(clippy::cast_precision_loss)]
    let frame_h_f = output.height() as f32;
    // `draw` also offsets the band from the frame edge by `style.margin`, so
    // that room is not available for text either. Subtracted once, not twice:
    // the band is pushed away from one edge, not both.
    #[allow(clippy::cast_precision_loss)]
    let placement_margin = (style.margin * f64::from(frame_h_f)) as f32;
    let spare = frame_h_f - first_line - 2.0 * pad_y - 2.0 * margin - placement_margin;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let fits = if spare <= 0.0 {
        1
    } else {
        ((spare / line_height).floor() as usize)
            .saturating_add(1)
            .max(1)
    };
    if lines.len() > fits {
        lines.truncate(fits);
    }

    draw(&fonts, &lines, px_f, &style, placement, output)
}

/// Everything after the text has been wrapped: geometry, masks, compositing.
#[allow(clippy::too_many_lines)]
fn draw(
    fonts: &Fonts,
    lines: &[String],
    px: f32,
    style: &ThemeStyle,
    placement: Placement,
    output: OutputSpec,
) -> CaptionImage {
    let frame_w = output.width();
    let frame_h = output.height();

    let metrics = fonts
        .font
        .horizontal_line_metrics(px)
        .expect("a horizontal font has horizontal line metrics");
    let ascent = metrics.ascent;
    let descent = -metrics.descent; // fontdue reports descent as negative

    let line_height = (px * style.line_spacing as f32).round();
    let text_h = (line_height * (lines.len().saturating_sub(1)) as f32 + ascent + descent).ceil();
    let text_w = lines
        .iter()
        .map(|l| fonts.measure(l, px))
        .fold(0.0_f32, f32::max)
        .ceil();

    let pad_x = (style.padding_x * f64::from(px)) as f32;
    let pad_y = (style.padding_y * f64::from(px)) as f32;
    let outline_r = (style.outline_width * f64::from(px)).round().max(0.0) as usize;
    let shadow_off = (style.shadow_offset * f64::from(px)).round().max(0.0) as usize;
    let shadow_blur = (style.shadow_blur * f64::from(px)).round().max(0.0) as usize;

    // Room for what spills outside the box: the outline in every direction,
    // the shadow down and to the right, the blur in every direction. Kept
    // asymmetric on purpose — a theme whose band is flush to the frame edge
    // (Backdrop::Band, margin 0) must have nothing at all below it, or a strip
    // of photograph shows under the band and reads as a rendering fault.
    //
    // `* SHADOW_BLUR_PASSES`, not `* 2` (D-117): `Mask::shadow` runs **three**
    // box blurs, and three passes of radius r spread 3r, so reserving 2r cut
    // the outermost ring off against the canvas edge. Downward is the binding
    // direction, because the offset moves the shadow that way.
    let bleed_up = outline_r + shadow_blur * SHADOW_BLUR_PASSES;
    let bleed_down = outline_r + shadow_off + shadow_blur * SHADOW_BLUR_PASSES;

    let box_h = (text_h + pad_y * 2.0).ceil() as usize;
    let box_w = match style.backdrop {
        Backdrop::Band => frame_w as usize,
        Backdrop::Fitted | Backdrop::None => {
            (text_w + pad_x * 2.0).ceil().min(f64::from(frame_w) as f32) as usize
        }
    };

    let canvas_w = frame_w as usize;
    let canvas_h = box_h + bleed_up + bleed_down;

    let box_x = match style.backdrop {
        Backdrop::Band => 0usize,
        Backdrop::Fitted | Backdrop::None => canvas_w.saturating_sub(box_w) / 2,
    };
    let box_y = bleed_up;

    // The text block, centred inside the box.
    let text_left = |line_w: f32| -> f32 { (canvas_w as f32 - line_w) / 2.0 };
    let text_top = box_y as f32 + pad_y;

    // --- the glyph mask ------------------------------------------------------
    let mut glyphs = Mask::new(canvas_w, canvas_h);
    for (row, line) in lines.iter().enumerate() {
        let line_w = fonts.measure(line, px);
        let mut pen = text_left(line_w);
        let baseline = text_top + ascent + line_height * row as f32;
        let mut previous: Option<char> = None;

        for ch in line.chars() {
            if let Some(p) = previous {
                pen += fonts.kern_between(p, ch, px);
            }
            let (m, bitmap) = fonts.font.rasterize(ch, px);
            // fontdue hands back a bitmap positioned by `xmin` from the pen and
            // `ymin` from the baseline, measured upward — so the top edge is
            // the baseline less the height above it.
            let gx = (pen + m.xmin as f32).round() as isize;
            let gy = (baseline - (m.ymin + m.height as i32) as f32).round() as isize;

            for by in 0..m.height {
                for bx in 0..m.width {
                    let v = bitmap[by * m.width + bx];
                    if v == 0 {
                        continue;
                    }
                    let tx = gx + bx as isize;
                    let ty = gy + by as isize;
                    if tx >= 0 && ty >= 0 {
                        glyphs.raise(tx as usize, ty as usize, v);
                    }
                }
            }
            pen += m.advance_width;
            previous = Some(ch);
        }
    }

    // --- the backdrop mask ---------------------------------------------------
    let radius = (style.radius * f64::from(px)).round().max(0.0) as f32;
    let plate = match style.backdrop {
        Backdrop::None => None,
        Backdrop::Band | Backdrop::Fitted => Some(rounded_rect(
            canvas_w,
            canvas_h,
            box_x as f32,
            box_y as f32,
            box_w as f32,
            box_h as f32,
            if style.backdrop == Backdrop::Band {
                0.0
            } else {
                radius
            },
        )),
    };

    // --- composite -----------------------------------------------------------
    let mut canvas = Canvas::new(canvas_w as u32, canvas_h as u32);

    // The shadow belongs to whatever the theme's outermost shape is: the plate
    // when there is one, the letters when there is not.
    if style.shadow.is_visible() {
        let caster = plate.as_ref().unwrap_or(&glyphs);
        caster
            .shadow(shadow_off, shadow_blur)
            .paint(&mut canvas, style.shadow);
    }
    if let Some(plate) = &plate {
        plate.paint(&mut canvas, style.backdrop_fill);
    }
    if style.outline.is_visible() && outline_r > 0 {
        glyphs.dilate(outline_r).paint(&mut canvas, style.outline);
    }
    glyphs.paint(&mut canvas, style.fill);

    // --- place it on the frame ----------------------------------------------
    let margin = (style.margin * f64::from(frame_h)).round() as i64;
    let box_bottom_in_canvas = (box_y + box_h) as i64;
    let y = match placement {
        Placement::Bottom => i64::from(frame_h) - margin - box_bottom_in_canvas,
        Placement::Top => margin - box_y as i64,
    };
    // Never off the top, and never so low that the band's own pixels leave the
    // frame. The bleed may leave it — it is transparent.
    let lowest = i64::from(frame_h) - box_bottom_in_canvas;
    let y = y.clamp(-(box_y as i64), lowest.max(0)).max(0);

    CaptionImage {
        canvas,
        y: u32::try_from(y).unwrap_or(0),
    }
}

/// A rounded rectangle as a coverage mask, antialiased over one pixel.
fn rounded_rect(width: usize, height: usize, x: f32, y: f32, w: f32, h: f32, radius: f32) -> Mask {
    let mut mask = Mask::new(width, height);
    let r = radius.min(w / 2.0).min(h / 2.0).max(0.0);

    let x0 = x.floor().max(0.0) as usize;
    let y0 = y.floor().max(0.0) as usize;
    let x1 = ((x + w).ceil() as usize).min(width);
    let y1 = ((y + h).ceil() as usize).min(height);

    let (half_w, half_h) = (w / 2.0, h / 2.0);
    let (cx0, cy0) = (x + half_w, y + half_h);

    for py in y0..y1 {
        for px_ in x0..x1 {
            // The signed distance from the pixel centre to the rounded
            // rectangle. `qx`/`qy` are the offsets outside the straight part of
            // each edge, so the first term is the distance around a corner and
            // the second is the (negative) distance from an interior pixel to
            // the nearest edge. Without that second term the distance is zero
            // everywhere inside instead of negative, and the whole plate draws
            // at half its intended alpha — which is what
            // `the_band_theme_sits_flush_against_the_frame_edge` caught.
            let cx = px_ as f32 + 0.5;
            let cy = py as f32 + 0.5;
            let qx = (cx - cx0).abs() - (half_w - r);
            let qy = (cy - cy0).abs() - (half_h - r);
            let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
            let inside = qx.max(qy).min(0.0);
            let distance = outside + inside - r;

            // A one-pixel ramp across zero is the antialiasing.
            let coverage = (0.5 - distance).clamp(0.0, 1.0);
            if coverage > 0.0 {
                mask.raise(px_, py, (coverage * 255.0).round() as u8);
            }
        }
    }
    mask
}

/// A whole frame showing what a theme looks like, for the window's chooser.
///
/// Rendered by the code that renders the film — not by CSS that resembles it.
/// A preview drawn a second way is a preview that can be wrong, and the only
/// thing an operator uses it for is deciding whether the real one will be
/// legible.
///
/// The backdrop is deliberately not flat: a translucent plate over a flat grey
/// tells you nothing, and half of these themes are translucent.
#[must_use]
pub fn preview(
    text: &str,
    theme: SubtitleTheme,
    placement: Placement,
    output: OutputSpec,
) -> Canvas {
    let mut canvas = preview_frame(output);
    let caption = render_cue(text, theme, placement, output);
    for y in 0..caption.canvas.height() {
        for x in 0..caption.canvas.width() {
            let p = caption.canvas.pixel(x, y);
            if p.a > 0 {
                canvas.blend(x, caption.y + y, Rgba::new(p.r, p.g, p.b, 255), p.a);
            }
        }
    }
    canvas
}

/// The stand-in photograph on its own, with nothing burned into it.
///
/// This is what "no subtitles" looks like, and the window shows exactly it when
/// that row is selected. Shared with [`preview`] so that switching between a
/// theme and none changes the caption and never the picture underneath.
#[must_use]
pub fn preview_frame(output: OutputSpec) -> Canvas {
    let mut canvas = Canvas::new(output.width(), output.height());
    let w = f64::from(output.width());
    let h = f64::from(output.height());

    for y in 0..output.height() {
        for x in 0..output.width() {
            let fx = f64::from(x) / w;
            let fy = f64::from(y) / h;
            // A sky over a shoreline: bright at the top where white type has to
            // survive, dark at the bottom where dark type does, and a band of
            // mid-tone across the middle where most captions actually land.
            let sky = 232.0 - 90.0 * fy;
            let land = 78.0 - 26.0 * (fy - 0.55);
            let blend = ((fy - 0.52) * 9.0).clamp(0.0, 1.0);
            let base = sky * (1.0 - blend) + land * blend;
            // A little structure, so a translucent plate has something to sit on.
            let ripple = 14.0 * ((fx * 11.0).sin() * (fy * 7.0).cos());
            let v = (base + ripple).clamp(0.0, 255.0) as u8;
            let warm = (f64::from(v) * 1.04).min(255.0) as u8;
            let cool = (f64::from(v) * 0.94) as u8;
            canvas.blend(x, y, Rgba::new(cool, v, warm, 255), 255);
        }
    }
    canvas
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoonstill_core::Aspect;

    fn out() -> OutputSpec {
        OutputSpec::new(Aspect::Landscape16x9, 1080, 30).expect("1080p")
    }

    /// The property the whole module exists for: ink appears, and it appears
    /// where a subtitle belongs.
    #[test]
    fn a_cue_puts_ink_on_the_canvas() {
        let image = render_cue(
            "The tide came in before dawn.",
            SubtitleTheme::Classic,
            Placement::Bottom,
            out(),
        );
        assert_eq!(image.canvas.width(), 1920, "always the full frame width");
        assert!(image.canvas.height() > 0);

        let ink = image
            .canvas
            .bytes()
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .count();
        assert!(
            ink > 500,
            "only {ink} non-transparent pixels — nothing drew"
        );

        // Bottom placement, on a 1080-high frame, belongs in the bottom third.
        assert!(
            image.y > 1080 / 2,
            "a bottom caption at y={} is not at the bottom",
            image.y
        );
        assert!(
            image.y + image.canvas.height() <= 1080,
            "the band runs off the frame: y={} h={}",
            image.y,
            image.canvas.height()
        );
    }

    #[test]
    fn top_placement_goes_to_the_top() {
        let image = render_cue("Up here.", SubtitleTheme::Classic, Placement::Top, out());
        assert!(image.y < 1080 / 3, "top caption at y={}", image.y);
    }

    /// Every theme has to survive every aspect (D-070) without running off the
    /// frame or drawing nothing.
    #[test]
    fn every_theme_draws_inside_every_frame() {
        let text = "The harbour was already full of them, and the light was going.";
        for aspect in [
            Aspect::Landscape16x9,
            Aspect::Portrait9x16,
            Aspect::Square1x1,
        ] {
            let output = OutputSpec::new(aspect, 720, 30).expect("720 is valid at every aspect");
            for theme in SubtitleTheme::ALL {
                for placement in Placement::ALL {
                    let image = render_cue(text, theme, placement, output);
                    assert_eq!(image.canvas.width(), output.width(), "{theme} {aspect}");
                    assert!(
                        image.y + image.canvas.height() <= output.height() + 64,
                        "{theme} {placement} at {aspect}: y={} h={} frame={}",
                        image.y,
                        image.canvas.height(),
                        output.height()
                    );
                    let ink = image
                        .canvas
                        .bytes()
                        .chunks_exact(4)
                        .filter(|p| p[3] > 8)
                        .count();
                    assert!(ink > 200, "{theme} {placement} at {aspect} drew nothing");
                }
            }
        }
    }

    /// The band theme is flush to the frame edge. A gap under it is the exact
    /// defect the asymmetric bleed calculation exists to prevent, and it is
    /// invisible in a unit test unless something asserts it.
    #[test]
    fn the_band_theme_sits_flush_against_the_frame_edge() {
        let image = render_cue("Flush.", SubtitleTheme::Band, Placement::Bottom, out());
        assert_eq!(
            image.y + image.canvas.height(),
            1080,
            "the band must reach the bottom edge exactly"
        );
        // And the bottom row must actually be painted, not transparent bleed.
        let bottom = image.canvas.height() - 1;
        let opaque = (0..image.canvas.width())
            .filter(|&x| image.canvas.pixel(x, bottom).a > 128)
            .count();
        assert!(
            opaque > 1900,
            "the bottom row of the band is not solid: {opaque} of 1920"
        );
    }

    /// Long text wraps rather than running off the side.
    #[test]
    fn a_long_cue_wraps_within_the_theme_width() {
        let long = "By seven the harbour was full, and the boats that had waited \
                    all winter went out one after another into a flat grey sea.";
        let image = render_cue(long, SubtitleTheme::Boxed, Placement::Bottom, out());

        // Find the leftmost and rightmost painted columns.
        let mut left = u32::MAX;
        let mut right = 0u32;
        for y in 0..image.canvas.height() {
            for x in 0..image.canvas.width() {
                if image.canvas.pixel(x, y).a > 16 {
                    left = left.min(x);
                    right = right.max(x);
                }
            }
        }
        assert!(left < right, "nothing drew");
        assert!(left > 4, "the caption touches the left edge at x={left}");
        assert!(
            right < 1920 - 4,
            "the caption touches the right edge at x={right}"
        );
        // Two lines of this theme, at 0.046 of 1080, is a band of a few hundred
        // pixels — not one enormous line.
        assert!(
            image.canvas.height() > 90 && image.canvas.height() < 400,
            "unexpected band height {}",
            image.canvas.height()
        );
    }

    /// A cue with more words than the theme's line budget shrinks the type
    /// rather than dropping the words.
    #[test]
    fn an_overlong_cue_shrinks_rather_than_truncates() {
        let short = render_cue("Short.", SubtitleTheme::Classic, Placement::Bottom, out());
        let long = render_cue(
            "An unusually long single caption that has considerably more words in \
             it than this theme's two-line budget was ever designed to hold at all.",
            SubtitleTheme::Classic,
            Placement::Bottom,
            out(),
        );
        let short_ink = short
            .canvas
            .bytes()
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .count();
        let long_ink = long
            .canvas
            .bytes()
            .chunks_exact(4)
            .filter(|p| p[3] > 0)
            .count();
        assert!(
            long_ink > short_ink * 3,
            "the long cue drew {long_ink} against {short_ink} — words were dropped"
        );
    }

    /// The light theme really is light, and the loud one really is yellow.
    /// Cheap, and it catches a theme table edited into nonsense.
    #[test]
    fn a_themes_colours_reach_the_pixels() {
        let card = render_cue("Print.", SubtitleTheme::Card, Placement::Bottom, out());
        let bright = card
            .canvas
            .bytes()
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] > 220 && p[1] > 220 && p[2] > 220)
            .count();
        assert!(bright > 1000, "the card theme has no light plate: {bright}");

        let punch = render_cue("Loud.", SubtitleTheme::Punch, Placement::Bottom, out());
        let yellow = punch
            .canvas
            .bytes()
            .chunks_exact(4)
            .filter(|p| p[3] > 200 && p[0] > 200 && p[1] > 160 && p[2] < 90)
            .count();
        assert!(yellow > 200, "the punch theme is not yellow: {yellow}");
    }

    /// Same input, same pixels — the property the segment cache rests on
    /// (D-043) and that D-077 asserts for the film as a whole.
    #[test]
    fn rendering_is_deterministic() {
        let once = render_cue("Twice.", SubtitleTheme::Boxed, Placement::Bottom, out());
        let again = render_cue("Twice.", SubtitleTheme::Boxed, Placement::Bottom, out());
        assert_eq!(once, again);
    }

    /// Text the operator can type: an emoji the font has no glyph for, a
    /// right-to-left fragment, a very long unbreakable token. None of these
    /// may panic, and none may produce an empty band.
    #[test]
    fn awkward_text_does_not_panic() {
        for text in [
            "Caf\u{e9} \u{2014} na\u{ef}ve r\u{e9}sum\u{e9}",
            "\u{1f680} launch \u{1f30a}",
            "\u{5317}\u{4eac} \u{306e} \u{5199}\u{771f}",
            "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch",
            "\"Quoted.\" (Bracketed.) [Also.]",
            "   ",
        ] {
            let image = render_cue(text, SubtitleTheme::Classic, Placement::Bottom, out());
            assert!(image.canvas.height() > 0, "empty band for {text:?}");
        }
    }

    /// The preview is a real frame drawn by the real renderer, so it must be
    /// fully opaque and must actually contain the caption.
    #[test]
    fn the_preview_is_an_opaque_frame_with_the_caption_in_it() {
        let small = OutputSpec::new(Aspect::Landscape16x9, 360, 30).expect("360p");
        let canvas = preview(
            "A quiet morning.",
            SubtitleTheme::Band,
            Placement::Bottom,
            small,
        );
        assert_eq!((canvas.width(), canvas.height()), (640, 360));
        assert!(
            canvas.bytes().chunks_exact(4).all(|p| p[3] == 255),
            "a preview frame must be opaque"
        );
        // The band theme paints a near-black bar; the gradient never is.
        let dark = canvas
            .bytes()
            .chunks_exact(4)
            .filter(|p| p[0] < 40 && p[1] < 40 && p[2] < 40)
            .count();
        assert!(
            dark > 2000,
            "the caption is missing from the preview: {dark}"
        );
    }

    /// Source-over on straight alpha, checked at the two ends and the middle.
    #[test]
    fn blending_is_source_over() {
        let mut canvas = Canvas::new(3, 1);
        canvas.blend(0, 0, Rgba::new(255, 0, 0, 255), 255);
        assert_eq!(canvas.pixel(0, 0), Rgba::new(255, 0, 0, 255));

        // Opaque white over opaque red is white.
        canvas.blend(0, 0, Rgba::new(255, 255, 255, 255), 255);
        assert_eq!(canvas.pixel(0, 0), Rgba::new(255, 255, 255, 255));

        // Zero coverage changes nothing.
        canvas.blend(1, 0, Rgba::new(255, 255, 255, 255), 0);
        assert_eq!(canvas.pixel(1, 0), Rgba::NONE);

        // Half-covered black over nothing is half-alpha black.
        canvas.blend(2, 0, Rgba::new(0, 0, 0, 255), 128);
        assert_eq!(canvas.pixel(2, 0).a, 128);
    }

    /// A two-line caption is two comparable lines, not a full line over an
    /// orphan. Measured on the mask, because "looks balanced" is exactly the
    /// kind of claim that needs a number.
    #[test]
    fn a_two_line_cue_is_balanced() {
        let image = render_cue(
            "The tide came in before dawn, and by seven the harbour was full.",
            SubtitleTheme::Classic,
            Placement::Bottom,
            out(),
        );
        // Painted width of the top half against the bottom half of the band.
        let half = image.canvas.height() / 2;
        let mut widths = [0u32; 2];
        for (band, width) in widths.iter_mut().enumerate() {
            let (mut left, mut right) = (u32::MAX, 0u32);
            let rows = if band == 0 {
                0..half
            } else {
                half..image.canvas.height()
            };
            for y in rows {
                for x in 0..image.canvas.width() {
                    if image.canvas.pixel(x, y).a > 16 {
                        left = left.min(x);
                        right = right.max(x);
                    }
                }
            }
            *width = right.saturating_sub(if left == u32::MAX { right } else { left });
        }
        let (a, b) = (widths[0].min(widths[1]), widths[0].max(widths[1]));
        assert!(a > 0 && b > 0, "expected two painted lines: {widths:?}");
        assert!(
            f64::from(a) / f64::from(b) > 0.55,
            "line widths {widths:?} are a full line over an orphan"
        );
    }

    /// A disc, not a square: the corner of the bounding box must stay empty.
    #[test]
    fn dilation_is_round() {
        let mut mask = Mask::new(21, 21);
        mask.raise(10, 10, 255);
        let grown = mask.dilate(6);
        assert_eq!(grown.get(10, 4), 255, "straight up, within the radius");
        assert_eq!(grown.get(4, 4), 0, "the diagonal corner is outside a disc");
    }

    /// D-130. The fast dilation is the **same** dilation.
    ///
    /// `dilate` used to walk a list of every offset inside the disc, for every
    /// pixel. It now walks the disc's horizontal chords, which is the same set
    /// of pixels reached a different way — so this asserts the two agree
    /// exactly, on shapes chosen to catch the ways a decomposition goes wrong:
    /// the edges, a single dot, a diagonal, and radii either side of a whole
    /// number of pixels.
    #[test]
    fn the_decomposed_dilation_matches_the_disc_it_replaced() {
        /// The original implementation, kept here as the definition of right.
        fn by_offsets(mask: &Mask, radius: usize) -> Mask {
            if radius == 0 {
                return mask.clone();
            }
            let r = radius as isize;
            let mut disc = Vec::new();
            for dy in -r..=r {
                for dx in -r..=r {
                    if dx * dx + dy * dy <= r * r {
                        disc.push((dx, dy));
                    }
                }
            }
            let mut out = Mask::new(mask.width, mask.height);
            for y in 0..mask.height {
                for x in 0..mask.width {
                    let mut best = 0u8;
                    for &(dx, dy) in &disc {
                        let sx = x as isize + dx;
                        let sy = y as isize + dy;
                        if sx < 0 || sy < 0 {
                            continue;
                        }
                        best = best.max(mask.get(sx as usize, sy as usize));
                    }
                    out.a[y * mask.width + x] = best;
                }
            }
            out
        }

        let mut shapes = Vec::new();

        // A single dot in the middle: the disc's own shape, drawn.
        let mut dot = Mask::new(31, 31);
        dot.a[15 * 31 + 15] = 255;
        shapes.push(("a dot", dot));

        // Ink in every corner and along both edges, where an offset can fall
        // outside the mask and a chord can be clipped wrongly.
        let mut edges = Mask::new(24, 18);
        for x in 0..24 {
            edges.a[x] = 200;
            edges.a[17 * 24 + x] = 90;
        }
        for y in 0..18 {
            edges.a[y * 24] = 255;
            edges.a[y * 24 + 23] = 40;
        }
        shapes.push(("the edges", edges));

        // A diagonal, and a range of greys — dilation is a maximum, not a
        // presence test, so the values have to travel too.
        let mut diagonal = Mask::new(29, 29);
        for i in 0..29 {
            diagonal.a[i * 29 + i] = (i * 8) as u8;
        }
        shapes.push(("a diagonal", diagonal));

        // Nothing at all, which must stay nothing.
        shapes.push(("an empty band", Mask::new(20, 12)));

        for (name, shape) in &shapes {
            for radius in [0usize, 1, 2, 3, 5, 8, 13] {
                let fast = shape.dilate(radius);
                let slow = by_offsets(shape, radius);
                assert_eq!(
                    fast.a, slow.a,
                    "{name} at radius {radius}: the decomposition is not the \
                     same kernel"
                );
            }
        }
    }

    /// D-130. The running-sum blur is the **same** blur, edges included.
    ///
    /// A box blur on a running sum is only equal to the naive one if the
    /// window is treated identically at the borders — where samples fall off
    /// the end, and where the divisor stays `2r+1` rather than shrinking. Those
    /// are exactly the places an off-by-one hides, and getting one wrong would
    /// change every shadow in every rendered frame by a shade nobody would
    /// notice until they compared two builds.
    #[test]
    fn the_running_sum_blur_matches_the_one_it_replaced() {
        /// The original implementation, kept as the definition of right.
        fn by_resumming(mask: &Mask, radius: usize) -> Mask {
            let mut mid = Mask::new(mask.width, mask.height);
            let span = (radius * 2 + 1) as u32;
            for y in 0..mask.height {
                for x in 0..mask.width {
                    let mut sum = 0u32;
                    for k in 0..=radius * 2 {
                        let sx = x as isize + k as isize - radius as isize;
                        if sx >= 0 {
                            sum += u32::from(mask.get(sx as usize, y));
                        }
                    }
                    mid.a[y * mask.width + x] = u8::try_from(sum / span).unwrap_or(255);
                }
            }
            let mut out = Mask::new(mask.width, mask.height);
            for y in 0..mask.height {
                for x in 0..mask.width {
                    let mut sum = 0u32;
                    for k in 0..=radius * 2 {
                        let sy = y as isize + k as isize - radius as isize;
                        if sy >= 0 {
                            sum += u32::from(mid.get(x, sy as usize));
                        }
                    }
                    out.a[y * mask.width + x] = u8::try_from(sum / span).unwrap_or(255);
                }
            }
            out
        }

        let mut shapes = Vec::new();

        let mut block = Mask::new(23, 19);
        for y in 5..14 {
            for x in 6..17 {
                block.a[y * 23 + x] = 255;
            }
        }
        shapes.push(("a block", block));

        // Ink hard against all four edges, which is where the window runs off.
        let mut edges = Mask::new(17, 13);
        for x in 0..17 {
            edges.a[x] = 255;
            edges.a[12 * 17 + x] = 128;
        }
        for y in 0..13 {
            edges.a[y * 17] = 200;
            edges.a[y * 17 + 16] = 64;
        }
        shapes.push(("the edges", edges));

        let mut dot = Mask::new(15, 15);
        dot.a[7 * 15 + 7] = 255;
        shapes.push(("a dot", dot));

        shapes.push(("nothing", Mask::new(11, 9)));

        for (name, shape) in &shapes {
            // Radii past the mask's own size too: the window is then wider than
            // the data, which is the case most likely to be wrong.
            for radius in [0usize, 1, 2, 3, 6, 10, 20] {
                assert_eq!(
                    shape.box_blur(radius).a,
                    by_resumming(shape, radius).a,
                    "{name} at radius {radius}: the running sum is not the same blur"
                );
            }
        }
    }

    /// D-124. The font is compiled into the binary, so the licence has to be
    /// distributed with the binary — the OFL's condition 2 asks that *"each
    /// copy contains the above copyright notice and this license"*.
    ///
    /// Asserted against the licence file the fonts came with, not against a
    /// copy of the words, so the notice cannot drift away from what is actually
    /// embedded. Replace the fonts and this fails until the notice is updated,
    /// which is the point: `include_bytes!` makes the obligation invisible at
    /// the call site, and nothing else in the build would notice.
    #[test]
    fn the_notices_file_carries_the_licence_of_the_font_that_is_embedded() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("the workspace root");

        let licence = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/fonts/LICENSE-Inter.txt"),
        )
        .expect("the fonts ship with their licence");

        let notices = std::fs::read_to_string(root.join("THIRD-PARTY-NOTICES.md"))
            .expect("THIRD-PARTY-NOTICES.md is at the workspace root");

        assert!(
            notices.contains(licence.trim()),
            "the notices file no longer contains the licence the embedded fonts \
             are under — update THIRD-PARTY-NOTICES.md from \
             crates/spoonstill-media/assets/fonts/LICENSE-Inter.txt"
        );

        // The copyright line specifically, because the OFL names it separately
        // from the licence body.
        assert!(
            notices.contains("Copyright 2020 The Inter Project Authors"),
            "the copyright notice is missing"
        );

        // And every weight that is actually embedded is one the notice covers.
        for weight in ["Inter-Regular.ttf", "Inter-SemiBold.ttf", "Inter-Bold.ttf"] {
            assert!(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/fonts")
                    .join(weight)
                    .exists(),
                "{weight} is embedded by name in this file"
            );
        }
    }
}
