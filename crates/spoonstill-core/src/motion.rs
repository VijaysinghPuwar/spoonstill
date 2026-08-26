//! The Ken Burns filter graph (D-030 … D-035). Pure. No I/O.
//!
//! This module is the whole reason the project has measurements. Every
//! constant in it is a number from `ffmpeg-findings.md`, taken on this machine,
//! and every ordering rule below removes a failure mode rather than bounding
//! one. The reference implementations in `plan/` each get at least one of these
//! wrong; see `ffmpeg-findings.md` §1–§3.
//!
//! # The emitted chain
//!
//! ```text
//! scale=<3*OUT_W>:<3*OUT_H>:force_original_aspect_ratio=increase:out_range=tv,
//! crop=<3*OUT_W>:<3*OUT_H>,
//! zoompan=z='<f(on)>':x='<expr>':y='<expr>':d=<N>:s=<OUT_W>x<OUT_H>:fps=<FPS>,
//! setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,
//! setsar=1,
//! format=yuv420p
//! ```
//!
//! Why each position is fixed:
//!
//! - **Cover-fit before `zoompan`** (D-034). The prescale canvas is already at
//!   or above the output aspect in both axes, so `zoompan`'s window is
//!   structurally incapable of walking off the image. This is stronger than
//!   clamping the pan expressions afterwards.
//! - **`setsar=1` last, before `format`** (D-033). A 1999x1001 source produces
//!   SAR 30007:30000 without it, and a `setsar` placed earlier does not survive
//!   the motion filters.
//! - **The frame count is structural** (D-030). One still, no `-loop`, `d=N`,
//!   `-frames:v N`. The unbounded `-loop 1` form is the five-hour "hang".
//! - **`setparams` and `out_range=tv`** (D-037). Measured while building M1:
//!   a JPEG is full-range, and that range flag survives `format=yuv420p` all
//!   the way into the encoder, which then reports `yuvj420p`. Two segments from
//!   differently-ranged sources concat with exit 0 and different colour.

use crate::geometry::{OutputSpec, SourceGeometry};
use crate::hash::fnv1a_fields;
use core::fmt;

/// Colour normalization pinned into every segment (D-037).
///
/// Not cosmetic. `ffprobe` reporting `yuvj420p` instead of `yuv420p` is a
/// segment-profile mismatch, and per D-041 the concat demuxer will not mention
/// it.
const COLOUR: &str = "setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709";

/// How the camera moves across the still.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MotionKind {
    /// Push in: zoom rises from 1 to `1 + amount`.
    ZoomIn,
    /// Pull out: zoom falls from `1 + amount` to 1.
    ZoomOut,
    /// Track left: the window travels toward the left edge at fixed zoom.
    PanLeft,
    /// Track right: the window travels toward the right edge at fixed zoom.
    PanRight,
    /// Track up.
    PanUp,
    /// Track down.
    PanDown,
}

impl MotionKind {
    /// Every kind, in a fixed order.
    ///
    /// The order is load-bearing: [`MotionSpec::seeded`] indexes into it, so
    /// reordering this array silently re-rolls the motion of every existing
    /// scene in every existing project and invalidates the cache (D-035,
    /// D-043). Append, never reorder.
    pub const ALL: [MotionKind; 6] = [
        MotionKind::ZoomIn,
        MotionKind::ZoomOut,
        MotionKind::PanLeft,
        MotionKind::PanRight,
        MotionKind::PanUp,
        MotionKind::PanDown,
    ];

    /// Stable name, as accepted on the command line and stored in state.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            MotionKind::ZoomIn => "zoom-in",
            MotionKind::ZoomOut => "zoom-out",
            MotionKind::PanLeft => "pan-left",
            MotionKind::PanRight => "pan-right",
            MotionKind::PanUp => "pan-up",
            MotionKind::PanDown => "pan-down",
        }
    }

    /// Parse the command-line form.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let normalized = text.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        MotionKind::ALL
            .into_iter()
            .find(|k| k.as_str() == normalized)
    }

    /// Whether this kind holds zoom constant and translates instead.
    #[must_use]
    pub const fn is_pan(self) -> bool {
        matches!(
            self,
            MotionKind::PanLeft | MotionKind::PanRight | MotionKind::PanUp | MotionKind::PanDown
        )
    }
}

impl fmt::Display for MotionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The focal point a zoom converges on, and the cross-axis position of a pan.
///
/// Expressed as a fraction of the legal window travel in each axis, which is
/// what makes D-034 structural: `0.0` is flush with one edge, `1.0` flush with
/// the other, and there is no value that addresses a pixel outside the canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Anchor {
    /// Dead centre — the classic Ken Burns push.
    Center,
    /// Top edge, horizontally centred.
    North,
    /// Bottom edge, horizontally centred.
    South,
    /// Left edge, vertically centred.
    West,
    /// Right edge, vertically centred.
    East,
    /// Top-left corner.
    NorthWest,
    /// Top-right corner.
    NorthEast,
    /// Bottom-left corner.
    SouthWest,
    /// Bottom-right corner.
    SouthEast,
}

impl Anchor {
    /// Every anchor, in a fixed order. Indexed by [`MotionSpec::seeded`], so
    /// this order is as load-bearing as [`MotionKind::ALL`].
    pub const ALL: [Anchor; 9] = [
        Anchor::Center,
        Anchor::North,
        Anchor::South,
        Anchor::West,
        Anchor::East,
        Anchor::NorthWest,
        Anchor::NorthEast,
        Anchor::SouthWest,
        Anchor::SouthEast,
    ];

    /// Position as `(x, y)` fractions of the legal travel, each in `[0, 1]`.
    #[must_use]
    pub const fn fractions(self) -> (f64, f64) {
        match self {
            Anchor::Center => (0.5, 0.5),
            Anchor::North => (0.5, 0.0),
            Anchor::South => (0.5, 1.0),
            Anchor::West => (0.0, 0.5),
            Anchor::East => (1.0, 0.5),
            Anchor::NorthWest => (0.0, 0.0),
            Anchor::NorthEast => (1.0, 0.0),
            Anchor::SouthWest => (0.0, 1.0),
            Anchor::SouthEast => (1.0, 1.0),
        }
    }

    /// Parse the manifest's `zoom_anchor` cell, or the command-line form.
    ///
    /// Compass names are canonical because they are what [`Self::as_str`]
    /// writes into state. The corner and edge synonyms are accepted because
    /// operators hand-write manifests (D-013) and `top_left` is what a person
    /// types; refusing it would be a validation error for a cell whose meaning
    /// was never in doubt. Separators are interchangeable for the same reason.
    ///
    /// `center`/`centre` both work. Nothing else is guessed at — an
    /// unrecognised cell is [`crate::ProblemKind::UnknownZoomAnchor`], never a
    /// silent fall back to centre (D-035).
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let normalized = text.trim().to_ascii_lowercase().replace(['_', ' '], "-");
        Some(match normalized.as_str() {
            "center" | "centre" | "middle" | "c" => Anchor::Center,
            "north" | "top" | "n" => Anchor::North,
            "south" | "bottom" | "s" => Anchor::South,
            "west" | "left" | "w" => Anchor::West,
            "east" | "right" | "e" => Anchor::East,
            "north-west" | "top-left" | "left-top" | "nw" => Anchor::NorthWest,
            "north-east" | "top-right" | "right-top" | "ne" => Anchor::NorthEast,
            "south-west" | "bottom-left" | "left-bottom" | "sw" => Anchor::SouthWest,
            "south-east" | "bottom-right" | "right-bottom" | "se" => Anchor::SouthEast,
            _ => return None,
        })
    }

    /// Stable name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Anchor::Center => "center",
            Anchor::North => "north",
            Anchor::South => "south",
            Anchor::West => "west",
            Anchor::East => "east",
            Anchor::NorthWest => "north-west",
            Anchor::NorthEast => "north-east",
            Anchor::SouthWest => "south-west",
            Anchor::SouthEast => "south-east",
        }
    }
}

impl fmt::Display for Anchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Smallest zoom span worth rendering — below this the move reads as a static
/// frame with encoder noise.
pub const MIN_AMOUNT: f64 = 0.02;
/// Largest zoom span. Beyond ~50% the crop window is small enough that a
/// 3x prescale no longer guarantees unique frames (D-032 was measured at 10%).
pub const MAX_AMOUNT: f64 = 0.50;
/// The zoom span every benchmark in `ffmpeg-findings.md` was taken at.
pub const DEFAULT_AMOUNT: f64 = 0.10;

/// One camera move: what it does, how far, and about which point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionSpec {
    /// Which move.
    pub kind: MotionKind,
    /// Zoom span as a fraction, clamped to `[MIN_AMOUNT, MAX_AMOUNT]`.
    pub amount: f64,
    /// Focal point for zooms; cross-axis position for pans.
    pub anchor: Anchor,
    /// The seed this spec was derived from, carried so it can be written into
    /// state and the cache key (D-035, D-043). Zero when hand-specified.
    pub seed: u64,
}

impl MotionSpec {
    /// A hand-specified move. `amount` is clamped rather than rejected: an
    /// out-of-range zoom is a slider that went too far, not a corrupt project.
    #[must_use]
    pub fn new(kind: MotionKind, amount: f64, anchor: Anchor) -> Self {
        Self {
            kind,
            amount: clamp_amount(amount),
            anchor,
            seed: 0,
        }
    }

    /// Derive a move deterministically from stable scene identity (D-035).
    ///
    /// The same project, the same scene index and the same source bytes always
    /// produce the same move — on this machine, on the operator's machine, and
    /// after a re-render six months later. Unseeded `random.choice()`, as in
    /// `ffmpeg-ai`, breaks resume and makes every cache entry a miss.
    ///
    /// `content_hash` is the hash of the source image bytes, not its path
    /// (D-043): moving a file must not re-roll its motion, and two scenes
    /// pointing at the same bytes must not share a move by accident — which is
    /// why the scene index is in the seed too.
    #[must_use]
    pub fn seeded(project_id: &str, scene_index: u32, content_hash: &str) -> Self {
        let seed = fnv1a_fields(&[
            project_id.as_bytes(),
            &scene_index.to_le_bytes(),
            content_hash.as_bytes(),
        ]);

        // Three independent byte ranges of the same seed, so the kind, the
        // anchor and the amount do not move in lockstep across scenes.
        let kind = MotionKind::ALL[(seed % MotionKind::ALL.len() as u64) as usize];
        let anchor = Anchor::ALL[((seed >> 20) % Anchor::ALL.len() as u64) as usize];
        // 0.08, 0.09, 0.10, 0.11, 0.12 — centred on the measured default.
        let amount = 0.08 + ((seed >> 40) % 5) as f64 * 0.01;

        Self {
            kind,
            amount: clamp_amount(amount),
            anchor,
            seed,
        }
    }

    /// A short stable description, for logs, state, and the cache key.
    #[must_use]
    pub fn descriptor(&self) -> String {
        format!(
            "{}@{}:{:.4}",
            self.kind.as_str(),
            self.anchor.as_str(),
            self.amount
        )
    }
}

fn clamp_amount(amount: f64) -> f64 {
    if amount.is_nan() {
        return DEFAULT_AMOUNT;
    }
    amount.clamp(MIN_AMOUNT, MAX_AMOUNT)
}

/// Format a float for an FFmpeg expression.
///
/// Fixed precision, never scientific notation, never locale-dependent. An
/// `f64` that reached FFmpeg as `1e-1` would parse, and one that reached it as
/// `0,1` would not — and the second is what a locale-aware formatter produces
/// in half of Europe.
fn expr_num(value: f64) -> String {
    format!("{value:.6}")
}

/// Build the complete video filter chain for one scene. Pure: no I/O, no
/// clock, no randomness.
///
/// `frames` is the *structural* frame count from D-030 — the same number that
/// goes to `d=` and to `-frames:v`. It is computed from the measured audio
/// duration (D-021, D-022), never estimated.
///
/// # Panics
///
/// Does not panic. A `frames` of 0 is treated as 1, because a zero-frame
/// segment is a caller bug that should surface as a failed profile assertion
/// with a real file, not as an arithmetic panic inside a string builder.
#[must_use]
pub fn build_filter(
    source: SourceGeometry,
    output: OutputSpec,
    motion: MotionSpec,
    frames: u32,
) -> String {
    let frames = frames.max(1);
    let (pw, ph) = (output.prescale_width(), output.prescale_height());

    let mut chain = String::with_capacity(320);

    // Square-pixel correction, emitted only when the source needs it. `scale`
    // works on stored pixel dimensions and knows nothing about SAR, so an
    // anamorphic source would otherwise be squashed by the cover-fit below.
    // Computed here as integers rather than left to an `iw*sar` expression, so
    // the emitted chain is fully determined by values we probed.
    if !source.has_square_pixels() {
        let (num, den) = source.sar();
        let corrected = (u64::from(source.width()) * u64::from(num) / u64::from(den)).max(2);
        // Even, because every downstream filter and the encoder assume it.
        let corrected = corrected.div_ceil(2) * 2;
        chain.push_str(&format!("scale={corrected}:{},", source.height()));
    }

    // D-034: cover-fit into the prescale canvas, before any motion.
    // D-037: out_range=tv here, where the full-range JPEG first meets a scaler.
    chain.push_str(&format!(
        "scale={pw}:{ph}:force_original_aspect_ratio=increase:out_range=tv,\
         crop={pw}:{ph},"
    ));

    // D-030: d=N on a single non-looped still. The frame count is structural.
    chain.push_str(&format!(
        "zoompan=z='{z}':x='{x}':y='{y}':d={frames}:s={w}x{h}:fps={fps}",
        z = zoom_expr(motion, frames),
        x = axis_expr(Axis::X, motion, frames),
        y = axis_expr(Axis::Y, motion, frames),
        w = output.width(),
        h = output.height(),
        fps = output.fps(),
    ));

    // D-037, then D-033, then the pixel format. This tail is not negotiable.
    chain.push(',');
    chain.push_str(COLOUR);
    chain.push_str(",setsar=1,format=yuv420p");

    chain
}

/// Progress through the move as an FFmpeg expression in `on`, always in `[0,1]`.
///
/// The denominator is `frames - 1` so the final frame lands exactly on the end
/// of the move rather than one step short. It is computed in Rust and emitted
/// as a literal, because a `frames` of 1 would otherwise divide by zero inside
/// the expression evaluator and yield `nan` — which FFmpeg propagates silently
/// into the crop window.
fn progress_expr(frames: u32) -> String {
    if frames <= 1 {
        // A one-frame segment has no travel. Freeze at the start of the move.
        return "0".to_string();
    }
    format!("min(on/{},1)", frames - 1)
}

fn zoom_expr(motion: MotionSpec, frames: u32) -> String {
    let a = expr_num(motion.amount);
    let p = progress_expr(frames);
    match motion.kind {
        MotionKind::ZoomIn => format!("1+{a}*{p}"),
        MotionKind::ZoomOut => format!("1+{a}-{a}*{p}"),
        // A pan needs room to travel, and the room is exactly what the zoom
        // creates: at zoom 1 the legal window travel is zero in both axes.
        MotionKind::PanLeft | MotionKind::PanRight | MotionKind::PanUp | MotionKind::PanDown => {
            format!("1+{a}")
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    X,
    Y,
}

/// One axis of the crop window, as a fraction of its own legal travel.
///
/// This is where D-034 becomes structural rather than clamped. The emitted form
/// is always `(<f>)*(iw-iw/zoom)`, and `iw-iw/zoom` **is** the full legal range
/// of `x` for the current zoom: 0 puts the window flush left, the whole
/// expression puts it flush right. Since `f` is confined to `[0,1]` by
/// construction — a constant anchor fraction, or `min(on/D,1)` which cannot
/// exceed 1 — no reachable value of `on` addresses a pixel outside the canvas.
/// There is no clamp because there is nothing to clamp.
fn axis_expr(axis: Axis, motion: MotionSpec, frames: u32) -> String {
    let (ax, ay) = motion.anchor.fractions();
    let p = progress_expr(frames);

    let fraction = match (axis, motion.kind) {
        // Camera tracks right: the window travels from the left edge to the
        // right edge. "Left" and "right" name the camera move, not the image.
        (Axis::X, MotionKind::PanRight) => p,
        (Axis::X, MotionKind::PanLeft) => format!("1-{p}"),
        (Axis::Y, MotionKind::PanDown) => p,
        (Axis::Y, MotionKind::PanUp) => format!("1-{p}"),
        // Every other case is the anchor: the fixed point of a zoom, or the
        // cross-axis position of a pan.
        (Axis::X, _) => expr_num(ax),
        (Axis::Y, _) => expr_num(ay),
    };

    match axis {
        Axis::X => format!("({fraction})*(iw-iw/zoom)"),
        Axis::Y => format!("({fraction})*(ih-ih/zoom)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Aspect;

    fn out(aspect: Aspect) -> OutputSpec {
        OutputSpec::new(aspect, 1080, 30).unwrap()
    }
    fn src() -> SourceGeometry {
        SourceGeometry::new(4000, 3000, 1, 1).unwrap()
    }

    /// The whole chain, pinned. If this string changes, every cached segment in
    /// every existing project is stale (D-043) and the change belongs in
    /// decisions.md in the same commit as the code.
    #[test]
    fn the_emitted_chain_is_exactly_the_documented_recipe() {
        let filter = build_filter(
            src(),
            out(Aspect::Landscape16x9),
            MotionSpec::new(MotionKind::ZoomIn, 0.10, Anchor::Center),
            112,
        );
        assert_eq!(
            filter,
            "scale=5760:3240:force_original_aspect_ratio=increase:out_range=tv,\
             crop=5760:3240,\
             zoompan=z='1+0.100000*min(on/111,1)':\
             x='(0.500000)*(iw-iw/zoom)':\
             y='(0.500000)*(ih-ih/zoom)':\
             d=112:s=1920x1080:fps=30,\
             setparams=range=tv:color_primaries=bt709:color_trc=bt709:colorspace=bt709,\
             setsar=1,format=yuv420p"
        );
    }

    /// D-033. Asserted positionally, not by "contains" — a `setsar` that has
    /// drifted into the middle of the chain still "contains".
    #[test]
    fn setsar_is_the_last_filter_before_format() {
        for aspect in Aspect::ALL {
            for kind in MotionKind::ALL {
                let filter = build_filter(
                    src(),
                    out(aspect),
                    MotionSpec::new(kind, 0.1, Anchor::Center),
                    90,
                );
                let filters: Vec<&str> = filter.split(',').collect();
                let n = filters.len();
                assert_eq!(filters[n - 1], "format=yuv420p", "in {filter}");
                assert_eq!(filters[n - 2], "setsar=1", "in {filter}");
                assert_eq!(
                    filter.matches("setsar").count(),
                    1,
                    "exactly one setsar, or an earlier one is being relied on"
                );
            }
        }
    }

    /// D-034. The cover-fit must precede the motion filter, or a landscape
    /// source in a 9:16 frame grows black edges.
    #[test]
    fn cover_fit_precedes_zoompan() {
        let filter = build_filter(
            src(),
            out(Aspect::Portrait9x16),
            MotionSpec::new(MotionKind::PanRight, 0.1, Anchor::Center),
            60,
        );
        let scale = filter.find("scale=3240:5760").expect("prescale present");
        let crop = filter.find("crop=3240:5760").expect("crop present");
        let zoompan = filter.find("zoompan=").expect("zoompan present");
        assert!(scale < crop && crop < zoompan, "wrong order in {filter}");
    }

    /// D-032, restated as a property of the emitted string rather than of the
    /// geometry type, because this is the form FFmpeg actually receives.
    #[test]
    fn prescale_in_the_emitted_chain_is_three_times_the_output() {
        for aspect in Aspect::ALL {
            let o = out(aspect);
            let filter = build_filter(
                src(),
                o,
                MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
                30,
            );
            assert!(
                filter.starts_with(&format!(
                    "scale={}:{}:force_original_aspect_ratio=increase",
                    o.width() * 3,
                    o.height() * 3
                )),
                "in {filter}"
            );
            assert!(
                !filter.contains("8000"),
                "scale=8000:-1 is superseded by D-032"
            );
        }
    }

    /// D-030. `d` and `-frames:v` are the same number, and no `-loop` form can
    /// be reached from here.
    #[test]
    fn frame_count_is_structural() {
        for frames in [1_u32, 2, 15, 112, 1800] {
            let filter = build_filter(
                src(),
                out(Aspect::Landscape16x9),
                MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
                frames,
            );
            assert!(filter.contains(&format!(":d={frames}:")), "in {filter}");
        }
    }

    /// A one-frame segment must not divide by zero inside FFmpeg's expression
    /// evaluator. `nan` there does not fail the render — it silently produces a
    /// garbage crop window.
    #[test]
    fn a_single_frame_segment_emits_no_division() {
        let filter = build_filter(
            src(),
            out(Aspect::Square1x1),
            MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
            1,
        );
        assert!(!filter.contains("on/0"), "division by zero in {filter}");
        assert!(filter.contains(":d=1:"));
    }

    /// The structural black-edge guarantee, checked on the emitted expressions:
    /// every axis expression is a fraction of `(iw-iw/zoom)`, which is exactly
    /// the legal travel. Nothing multiplies the canvas, and nothing adds a
    /// constant offset to it.
    #[test]
    fn every_axis_expression_is_a_fraction_of_the_legal_travel() {
        for kind in MotionKind::ALL {
            for anchor in Anchor::ALL {
                let filter = build_filter(
                    src(),
                    out(Aspect::Portrait9x16),
                    MotionSpec::new(kind, 0.5, anchor),
                    120,
                );
                assert!(
                    filter.contains(")*(iw-iw/zoom)") && filter.contains(")*(ih-ih/zoom)"),
                    "{kind}/{anchor} escaped the legal-travel form: {filter}"
                );
            }
        }
    }

    /// Fractions stay inside `[0,1]` for every anchor — the arithmetic half of
    /// the guarantee above.
    #[test]
    fn anchor_fractions_are_bounded() {
        for anchor in Anchor::ALL {
            let (x, y) = anchor.fractions();
            assert!(
                (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y),
                "{anchor}"
            );
        }
    }

    /// A pan at zoom 1 has zero travel and would render as a still. Every pan
    /// must therefore carry a zoom above 1.
    #[test]
    fn pans_carry_enough_zoom_to_have_somewhere_to_go() {
        for kind in MotionKind::ALL.into_iter().filter(|k| k.is_pan()) {
            let filter = build_filter(
                src(),
                out(Aspect::Landscape16x9),
                MotionSpec::new(kind, 0.1, Anchor::Center),
                60,
            );
            assert!(filter.contains("z='1+0.100000'"), "{kind}: {filter}");
        }
    }

    /// D-037. The colour normalization is present and sits before `setsar`.
    #[test]
    fn colour_is_pinned_before_setsar() {
        let filter = build_filter(
            src(),
            out(Aspect::Landscape16x9),
            MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
            30,
        );
        assert!(filter.contains("out_range=tv"), "in {filter}");
        let params = filter.find("setparams=").expect("setparams present");
        let setsar = filter.find("setsar=1").expect("setsar present");
        assert!(params < setsar, "in {filter}");
    }

    /// An anamorphic source gains a square-pixel correction; a normal one does
    /// not, so the common chain stays byte-identical to the documented recipe.
    #[test]
    fn square_pixel_correction_appears_only_when_needed() {
        let normal = build_filter(
            SourceGeometry::new(4000, 3000, 1, 1).unwrap(),
            out(Aspect::Landscape16x9),
            MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
            30,
        );
        assert!(normal.starts_with("scale=5760:3240:"), "in {normal}");

        let anamorphic = build_filter(
            SourceGeometry::new(720, 576, 16, 11).unwrap(),
            out(Aspect::Landscape16x9),
            MotionSpec::new(MotionKind::ZoomIn, 0.1, Anchor::Center),
            30,
        );
        assert!(
            anamorphic.starts_with("scale=1048:576,scale=5760:3240:"),
            "in {anamorphic}"
        );
    }

    /// D-035. Same identity, same move — forever, and across processes.
    #[test]
    fn seeded_motion_is_reproducible() {
        let a = MotionSpec::seeded("proj-alpha", 7, "deadbeef");
        let b = MotionSpec::seeded("proj-alpha", 7, "deadbeef");
        assert_eq!(a, b);
        assert_eq!(a.descriptor(), b.descriptor());
    }

    /// Different scenes in one project must not all get the same move, and the
    /// same scene index in different projects must not either.
    #[test]
    fn seeded_motion_varies_across_identity() {
        let moves: Vec<String> = (0..12)
            .map(|i| MotionSpec::seeded("proj", i, "hash").descriptor())
            .collect();
        let distinct: std::collections::BTreeSet<&String> = moves.iter().collect();
        assert!(
            distinct.len() > 3,
            "12 consecutive scenes produced only {} distinct moves: {moves:?}",
            distinct.len()
        );

        assert_ne!(
            MotionSpec::seeded("proj-a", 3, "hash").seed,
            MotionSpec::seeded("proj-b", 3, "hash").seed
        );
        assert_ne!(
            MotionSpec::seeded("proj", 3, "hash-a").seed,
            MotionSpec::seeded("proj", 3, "hash-b").seed
        );
    }

    /// Every seeded move must be renderable — the amount inside the measured
    /// range, and the kind and anchor real variants.
    #[test]
    fn seeded_motion_is_always_in_range() {
        for i in 0..500 {
            let m = MotionSpec::seeded("p", i, "h");
            assert!(
                (MIN_AMOUNT..=MAX_AMOUNT).contains(&m.amount),
                "scene {i}: amount {} out of range",
                m.amount
            );
            assert!(MotionKind::ALL.contains(&m.kind));
            assert!(Anchor::ALL.contains(&m.anchor));
        }
    }

    /// An out-of-range amount is a slider that went too far, not a corrupt
    /// project — clamp it, and never emit `nan` into a filter expression.
    #[test]
    fn amounts_are_clamped_not_rejected() {
        assert_eq!(
            MotionSpec::new(MotionKind::ZoomIn, 9.0, Anchor::Center).amount,
            MAX_AMOUNT
        );
        assert_eq!(
            MotionSpec::new(MotionKind::ZoomIn, -1.0, Anchor::Center).amount,
            MIN_AMOUNT
        );
        assert_eq!(
            MotionSpec::new(MotionKind::ZoomIn, f64::NAN, Anchor::Center).amount,
            DEFAULT_AMOUNT
        );
        let filter = build_filter(
            src(),
            out(Aspect::Square1x1),
            MotionSpec::new(MotionKind::ZoomIn, f64::NAN, Anchor::Center),
            30,
        );
        assert!(
            !filter.contains("NaN") && !filter.contains("nan"),
            "{filter}"
        );
    }

    /// Expression numbers never reach FFmpeg in scientific notation.
    #[test]
    fn expression_numbers_are_plain_decimals() {
        assert_eq!(expr_num(0.5), "0.500000");
        assert_eq!(expr_num(0.0000001), "0.000000");
        assert!(!expr_num(1e-12).contains('e'));
    }

    #[test]
    fn motion_kinds_parse_the_forms_an_operator_will_type() {
        assert_eq!(MotionKind::parse("zoom-in"), Some(MotionKind::ZoomIn));
        assert_eq!(MotionKind::parse("Zoom_In"), Some(MotionKind::ZoomIn));
        assert_eq!(MotionKind::parse("pan right"), Some(MotionKind::PanRight));
        assert_eq!(MotionKind::parse("dolly"), None);
    }

    /// Every anchor round-trips through its own stable name. Without this,
    /// a `zoom_anchor` written back into state could fail to parse on the way
    /// in, and the scene would silently re-seed on the next run (D-035).
    #[test]
    fn every_anchor_round_trips_through_its_stable_name() {
        for anchor in Anchor::ALL {
            assert_eq!(
                Anchor::parse(anchor.as_str()),
                Some(anchor),
                "{anchor} must survive as_str -> parse"
            );
        }
    }

    #[test]
    fn anchors_parse_the_forms_an_operator_will_type() {
        assert_eq!(Anchor::parse("top_left"), Some(Anchor::NorthWest));
        assert_eq!(Anchor::parse("Bottom Right"), Some(Anchor::SouthEast));
        assert_eq!(Anchor::parse("centre"), Some(Anchor::Center));
        assert_eq!(Anchor::parse("ne"), Some(Anchor::NorthEast));

        // D-035: an unrecognised cell is a validation problem, never a quiet
        // fall back to centre.
        assert_eq!(Anchor::parse("upper-leftish"), None);
        assert_eq!(Anchor::parse(""), None);
    }
}
