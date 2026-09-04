//! Output geometry, source geometry, and the prescale rule (D-032, D-034).

use core::fmt;

/// The aspect ratios spoonstill renders (D-070).
///
/// D-070 was Open with a recorded default of "yes, all three in V1". The
/// default is what is implemented here, and it is nearly free: D-034's
/// cover-fit into the prescale canvas is aspect-agnostic, so the marginal cost
/// of the two extra ratios is test-matrix breadth rather than new code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aspect {
    /// 16:9 landscape.
    Landscape16x9,
    /// 9:16 vertical.
    Portrait9x16,
    /// 1:1 square.
    Square1x1,
}

impl Aspect {
    /// Every aspect, in a fixed order. Used to drive the motion matrix.
    pub const ALL: [Aspect; 3] = [
        Aspect::Landscape16x9,
        Aspect::Portrait9x16,
        Aspect::Square1x1,
    ];

    /// Short stable name, as accepted on the command line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Aspect::Landscape16x9 => "16:9",
            Aspect::Portrait9x16 => "9:16",
            Aspect::Square1x1 => "1:1",
        }
    }

    /// Parse the command-line form. Case-insensitive, `x` accepted for `:`.
    ///
    /// The vertical aliases are deliberate (D-143): an operator making a
    /// YouTube Short, an Instagram Reel or a TikTok is not thinking "9:16",
    /// they are thinking of the place the film is going. All three are the
    /// same frame, so all three are the same word here.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace('x', ":").as_str() {
            "16:9" | "landscape" | "wide" | "youtube" => Some(Aspect::Landscape16x9),
            "9:16" | "portrait" | "vertical" | "shorts" | "short" | "youtube-shorts" | "reel"
            | "reels" | "tiktok" | "story" | "stories" => Some(Aspect::Portrait9x16),
            "1:1" | "square" => Some(Aspect::Square1x1),
            _ => None,
        }
    }

    /// What this aspect is *for*, in the words an operator uses.
    ///
    /// Shown by `still resolutions` and in the window's chooser, so that the
    /// person picking one does not have to know which ratio a Short is.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Aspect::Landscape16x9 => "landscape — YouTube, a television, a laptop",
            Aspect::Portrait9x16 => "portrait — YouTube Shorts, Reels, TikTok, Stories",
            Aspect::Square1x1 => "square — a feed post",
        }
    }
}

/// A named output size, which is a short edge with a name on it (D-143).
///
/// The short edge is still the parameter — [`OutputSpec::new`] takes one, and
/// every rule about evenness and divisibility lives there. This is the layer
/// above it, because "4K" is what an operator asks for and 2160 is what the
/// renderer needs, and asking a person to know that 4K portrait is 2160 wide
/// rather than 3840 wide is asking them to do arithmetic to get a common case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Resolution {
    /// 720 short edge — 1280x720 landscape.
    Hd720,
    /// 1080 short edge — 1920x1080 landscape. The default.
    Fhd1080,
    /// 1440 short edge — 2560x1440 landscape. Commonly called 2K.
    Qhd1440,
    /// 2160 short edge — 3840x2160 landscape. 4K UHD, and the ceiling (D-114).
    Uhd2160,
}

impl Resolution {
    /// Every named size, smallest first.
    pub const ALL: [Resolution; 4] = [
        Resolution::Hd720,
        Resolution::Fhd1080,
        Resolution::Qhd1440,
        Resolution::Uhd2160,
    ];

    /// The short edge in pixels — the number [`OutputSpec::new`] wants.
    ///
    /// Every one of these is even and divisible by 9, which is what 16:9 needs
    /// to stay integer. A test asserts that, because a fifth entry that is not
    /// would be a name that only works in two of the three aspects.
    #[must_use]
    pub const fn short_edge(self) -> u32 {
        match self {
            Resolution::Hd720 => 720,
            Resolution::Fhd1080 => 1080,
            Resolution::Qhd1440 => 1440,
            Resolution::Uhd2160 => 2160,
        }
    }

    /// The canonical name, as accepted on the command line and written back.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Resolution::Hd720 => "720p",
            Resolution::Fhd1080 => "1080p",
            Resolution::Qhd1440 => "1440p",
            Resolution::Uhd2160 => "2160p",
        }
    }

    /// The other spellings this size answers to, for help text and listings.
    ///
    /// `2k` means 2560x1440 here, which is the consumer usage. DCI 2K is
    /// 2048x1080 and is a different number; it is not offered, and this
    /// comment is the whole of the reason the alias is documented rather than
    /// left for someone to discover.
    #[must_use]
    pub const fn aliases(self) -> &'static [&'static str] {
        match self {
            Resolution::Hd720 => &["720", "hd"],
            Resolution::Fhd1080 => &["1080", "fhd", "full-hd"],
            Resolution::Qhd1440 => &["1440", "2k", "qhd"],
            Resolution::Uhd2160 => &["2160", "4k", "uhd"],
        }
    }

    /// One line describing what this size is for.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Resolution::Hd720 => "small and quick — a draft, or a preview to send",
            Resolution::Fhd1080 => "the default — what almost every screen shows",
            Resolution::Qhd1440 => "2K — sharper than 1080p and about half the size of 4K",
            Resolution::Uhd2160 => "4K — the largest this renders (D-114)",
        }
    }

    /// Parse a name or one of its aliases. Case-insensitive.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        Resolution::ALL
            .iter()
            .copied()
            .find(|r| r.as_str() == text || r.aliases().contains(&text.as_str()))
    }

    /// The named size a short edge corresponds to, if it has a name.
    ///
    /// Used to *report* a project's geometry in the words it was chosen in.
    /// A project that sets `short_edge: 900` has no name, and is shown as its
    /// number — the numbers are not going away.
    #[must_use]
    pub fn from_short_edge(short_edge: u32) -> Option<Self> {
        Resolution::ALL
            .iter()
            .copied()
            .find(|r| r.short_edge() == short_edge)
    }

    /// Every canonical name, comma-separated, for an error message.
    #[must_use]
    pub fn names() -> String {
        Resolution::ALL
            .iter()
            .map(|r| r.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for Resolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Display for Aspect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What went wrong when a caller asked for geometry we will not render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GeometryError {
    /// The short edge does not produce integer, even dimensions.
    UnusableShortEdge {
        /// The short edge the caller asked for.
        short_edge: u32,
        /// What it is not, as a noun phrase: this reads after "is not" both in
        /// this error's own `Display` and in the `still validate` line that
        /// embeds it (D-114).
        reason: &'static str,
    },
    /// Frame rate outside the supported range.
    UnusableFps(u32),
    /// A source with a zero dimension — a truncated or unreadable image.
    DegenerateSource {
        /// Source width as probed.
        width: u32,
        /// Source height as probed.
        height: u32,
    },
}

impl GeometryError {
    /// The noun phrase alone, without the subject this error would print.
    ///
    /// For a caller that has its own subject to put in front of it —
    /// `ProblemKind::UnusableSetting` renders `` `field`: "value" is not
    /// {expected} ``, and handing it the whole `Display` produced
    /// *"…" is not short edge 4294967292 is not a size this renders* (D-114).
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            GeometryError::UnusableShortEdge { reason, .. } => reason,
            GeometryError::UnusableFps(_) => "a frame rate between 1 and 120",
            GeometryError::DegenerateSource { .. } => "an image with two non-zero dimensions",
        }
    }
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Phrased as a noun so it reads correctly both alone and after
            // "is not", which is how `ProblemKind::UnusableSetting` composes it
            // into a `still validate` line (D-114).
            GeometryError::UnusableShortEdge { short_edge, reason } => {
                write!(f, "short edge {short_edge} is not {reason}")
            }
            GeometryError::UnusableFps(fps) => {
                write!(f, "frame rate {fps} is outside the supported range 1..=120")
            }
            GeometryError::DegenerateSource { width, height } => write!(
                f,
                "source geometry {width}x{height} has a zero dimension — the image is \
                 unreadable or truncated"
            ),
        }
    }
}

impl core::error::Error for GeometryError {}

/// The output frame: pixel dimensions and frame rate.
///
/// Constructed rather than assembled field-by-field, because H.264 4:2:0 cannot
/// represent odd dimensions and every downstream assertion assumes even ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutputSpec {
    width: u32,
    height: u32,
    fps: u32,
    aspect: Aspect,
}

/// The largest frame this program will render, in 16x16 macroblocks (D-114).
///
/// **Derived, not chosen.** 36 864 is `MaxFS` for H.264 level 5.2, the highest
/// level `spoonstill_media::profile::h264_level` models — and past the table it
/// returns 52 regardless, so a larger frame would be *labelled* 5.2 while being
/// something no 5.2 decoder can play. D-041 rests on the segment profile being
/// a true description of the file, so the ceiling is the largest frame that
/// description stays true for.
///
/// In practice: 4K UHD is 32 400 macroblocks and renders; 8K is 129 600 and is
/// refused. A test in `spoonstill-media` asserts this number still agrees with
/// the level table, because they live in different crates and only a test can
/// keep them from drifting.
pub const MAX_MACROBLOCKS: u64 = 36_864;

/// One reason string, used by every refusal that means "past the ceiling".
const TOO_LARGE: &str =
    "a size this renders — H.264 level 5.2 tops out at 36864 macroblocks, which is 4K";

/// Macroblocks a frame occupies, counted exactly as the level table counts
/// them: partial blocks at the right and bottom edges are whole blocks.
const fn macroblocks(width: u32, height: u32) -> u64 {
    (width.div_ceil(16) as u64) * (height.div_ceil(16) as u64)
}

impl OutputSpec {
    /// Build an output spec from an aspect and its **short edge**.
    ///
    /// The short edge is the stable parameter across aspects: 1080 gives
    /// 1920x1080, 1080x1920 and 1080x1080 — the three sizes an operator
    /// actually names. Deriving from the long edge instead would make "1080"
    /// mean three different pixel counts.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError`] when the short edge cannot produce integer,
    /// even dimensions in the requested aspect, or when the frame rate is
    /// outside 1..=120.
    pub fn new(aspect: Aspect, short_edge: u32, fps: u32) -> Result<Self, GeometryError> {
        if short_edge == 0 {
            return Err(GeometryError::UnusableShortEdge {
                short_edge,
                reason: "a positive number",
            });
        }
        if !short_edge.is_multiple_of(2) {
            return Err(GeometryError::UnusableShortEdge {
                short_edge,
                reason: "even — H.264 4:2:0 cannot represent odd dimensions",
            });
        }
        let long_edge = match aspect {
            Aspect::Landscape16x9 | Aspect::Portrait9x16 => {
                if !short_edge.is_multiple_of(9) {
                    return Err(GeometryError::UnusableShortEdge {
                        short_edge,
                        reason: "divisible by 9, which 16:9 needs to stay integer",
                    });
                }
                // Checked: `short_edge / 9 * 16` overflows `u32` above about
                // 2.4 billion, and an overflow here panicked in debug and
                // **wrapped in release** — 4294967292 was accepted as a valid
                // 3340530112x4294967292 output with no problem reported
                // (D-114). The division happens first, so only the multiply
                // can go.
                (short_edge / 9)
                    .checked_mul(16)
                    .ok_or(GeometryError::UnusableShortEdge {
                        short_edge,
                        reason: TOO_LARGE,
                    })?
            }
            Aspect::Square1x1 => short_edge,
        };
        if !long_edge.is_multiple_of(2) {
            return Err(GeometryError::UnusableShortEdge {
                short_edge,
                reason: "a size whose long edge is even",
            });
        }
        if fps == 0 || fps > 120 {
            return Err(GeometryError::UnusableFps(fps));
        }

        let (width, height) = match aspect {
            Aspect::Landscape16x9 => (long_edge, short_edge),
            Aspect::Portrait9x16 => (short_edge, long_edge),
            Aspect::Square1x1 => (short_edge, short_edge),
        };

        // The ceiling, and it is derived rather than picked (D-114). See
        // [`MAX_MACROBLOCKS`]: past it the segment profile would have to
        // declare an H.264 level no decoder honours, and the prescale canvas
        // would be nine times a frame that is already too big.
        if macroblocks(width, height) > MAX_MACROBLOCKS {
            return Err(GeometryError::UnusableShortEdge {
                short_edge,
                reason: TOO_LARGE,
            });
        }

        Ok(Self {
            width,
            height,
            fps,
            aspect,
        })
    }

    /// Output width in pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    /// Output height in pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    /// The short edge, which is what the operator actually chose — width and
    /// height fall out of it and the aspect (D-070).
    #[must_use]
    pub const fn short_edge(&self) -> u32 {
        if self.width < self.height {
            self.width
        } else {
            self.height
        }
    }
    /// Output frame rate in whole frames per second.
    #[must_use]
    pub const fn fps(&self) -> u32 {
        self.fps
    }
    /// The aspect this spec was built from.
    #[must_use]
    pub const fn aspect(&self) -> Aspect {
        self.aspect
    }

    /// Prescale canvas width — **3x the output width** (D-032).
    ///
    /// Derived, never a constant. `scale=8000:-1` is superseded: 8000 is 7.4x
    /// for 1080p and 4.2x for 1080x1920, the same number meaning different
    /// things per aspect (`ffmpeg-findings.md` §1).
    ///
    /// Cannot overflow, and that is an invariant of the constructor rather
    /// than of this line: [`MAX_MACROBLOCKS`] caps an edge far below
    /// `u32::MAX / 3`. It used to overflow — `short_edge: 1431655776` produced
    /// a prescale canvas of 3340530176x**32**, the height having wrapped
    /// (D-114) — which is why the cap exists and why a test pins the boundary.
    #[must_use]
    pub const fn prescale_width(&self) -> u32 {
        self.width * PRESCALE_FACTOR
    }

    /// Prescale canvas height — **3x the output height** (D-032).
    ///
    /// Measured floor *and* ceiling: at 2x, 280 of 300 frames were unique and
    /// motion visibly stepped; at 3x, 300 of 300; at 4x and 6x, nothing
    /// improved and the encode got slower.
    #[must_use]
    pub const fn prescale_height(&self) -> u32 {
        self.height * PRESCALE_FACTOR
    }
}

/// D-032. Both the floor and the ceiling, measured on this machine.
pub const PRESCALE_FACTOR: u32 = 3;

/// The geometry of a source still, as probed — never as assumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SourceGeometry {
    width: u32,
    height: u32,
    sar_num: u32,
    sar_den: u32,
}

impl SourceGeometry {
    /// Build source geometry from probed values.
    ///
    /// A SAR of `0:1` or `0:0` — which is what `ffprobe` reports for a still
    /// with no aspect metadata — normalizes to `1:1`, because "unspecified"
    /// means square pixels for every format we accept.
    ///
    /// # Errors
    ///
    /// Returns [`GeometryError::DegenerateSource`] when either dimension is
    /// zero. `truncated.jpg` probes as `0x0`, and it must be named as a bad
    /// image rather than divided by later.
    pub fn new(width: u32, height: u32, sar_num: u32, sar_den: u32) -> Result<Self, GeometryError> {
        if width == 0 || height == 0 {
            return Err(GeometryError::DegenerateSource { width, height });
        }
        let (sar_num, sar_den) = if sar_num == 0 || sar_den == 0 {
            (1, 1)
        } else {
            (sar_num, sar_den)
        };
        Ok(Self {
            width,
            height,
            sar_num,
            sar_den,
        })
    }

    /// Source width in stored pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        self.width
    }
    /// Source height in stored pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        self.height
    }
    /// Sample aspect ratio as `(numerator, denominator)`.
    #[must_use]
    pub const fn sar(&self) -> (u32, u32) {
        (self.sar_num, self.sar_den)
    }

    /// Whether the source has square pixels.
    ///
    /// When it does — the overwhelmingly common case — the filter chain is
    /// exactly the one D-034 documents. When it does not, the chain gains a
    /// leading square-pixel correction, because `scale` works on stored pixel
    /// dimensions and would otherwise squash an anamorphic source.
    #[must_use]
    pub const fn has_square_pixels(&self) -> bool {
        self.sar_num == self.sar_den
    }

    /// Source width in **display** pixels — stored width corrected for SAR.
    ///
    /// This, not [`Self::width`], is what D-034's cover-fit sees: an
    /// anamorphic source is squared up by the leading correction in the filter
    /// chain before anything is scaled, so a 720x576 frame at 16:15 covers
    /// 768 pixels of frame and not 720.
    #[must_use]
    pub fn display_width(&self) -> u32 {
        let scaled = u64::from(self.width) * u64::from(self.sar_num) / u64::from(self.sar_den);
        u32::try_from(scaled).unwrap_or(u32::MAX)
    }

    /// Source height in display pixels. SAR is horizontal, so this is the
    /// stored height — named for symmetry, so a caller never has to remember
    /// which of the two the ratio applies to.
    #[must_use]
    pub const fn display_height(&self) -> u32 {
        self.height
    }

    /// Whether this source covers `out` without being enlarged (D-034).
    ///
    /// Cover-fit scales the source until it fills the frame in both axes, so
    /// it is enlarged unless it is at least as wide **and** at least as tall
    /// as the frame. `true` means every output pixel comes from at least one
    /// source pixel; `false` means the film is an upscale of the photograph.
    ///
    /// Deliberately ignores the Ken Burns zoom, which samples a sub-region and
    /// so enlarges further: this answers "is the frame itself an upscale",
    /// which is the question an operator can act on by choosing a size.
    #[must_use]
    pub fn fills(&self, out: &OutputSpec) -> bool {
        self.display_width() >= out.width() && self.display_height() >= out.height()
    }

    /// The largest output short edge this source covers without being enlarged.
    ///
    /// Rounded **down** to a short edge [`OutputSpec::new`] accepts in that
    /// aspect — a multiple of 18 for the two 16:9 shapes, which is what "even
    /// and divisible by 9" comes to, and a multiple of 2 for square. `0` means
    /// no legal size renders this source natively.
    ///
    /// The answer is per aspect because the frame's shape decides which edge
    /// binds: a 1376x768 still is 756 landscape and 432 portrait, because
    /// portrait asks its 768 pixels of height to carry the *long* edge.
    #[must_use]
    pub fn native_short_edge(&self, aspect: Aspect) -> u32 {
        let dw = u64::from(self.display_width());
        let dh = u64::from(self.display_height());
        // `s` is the short edge; the long edge is `s * 16 / 9` in the two 16:9
        // shapes. Both edges have to fit, so the answer is the smaller bound.
        let raw = match aspect {
            Aspect::Landscape16x9 => core::cmp::min(dw * 9 / 16, dh),
            Aspect::Portrait9x16 => core::cmp::min(dw, dh * 9 / 16),
            Aspect::Square1x1 => core::cmp::min(dw, dh),
        };
        let step = match aspect {
            Aspect::Landscape16x9 | Aspect::Portrait9x16 => 18,
            Aspect::Square1x1 => 2,
        };
        u32::try_from(raw / step * step).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_v1_aspects_at_1080_are_the_expected_sizes() {
        let l = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap();
        assert_eq!((l.width(), l.height()), (1920, 1080));
        let p = OutputSpec::new(Aspect::Portrait9x16, 1080, 30).unwrap();
        assert_eq!((p.width(), p.height()), (1080, 1920));
        let s = OutputSpec::new(Aspect::Square1x1, 1080, 30).unwrap();
        assert_eq!((s.width(), s.height()), (1080, 1080));
    }

    /// D-032, the headline number: prescale is derived from the output, not a
    /// constant, so it means the same thing in every aspect.
    #[test]
    fn prescale_is_three_times_the_output_in_every_aspect() {
        for aspect in Aspect::ALL {
            let out = OutputSpec::new(aspect, 1080, 30).unwrap();
            assert_eq!(out.prescale_width(), out.width() * 3);
            assert_eq!(out.prescale_height(), out.height() * 3);
        }
    }

    /// The superseded `scale=8000:-1` meant 7.4x in one aspect and 4.2x in
    /// another. This asserts the thing that replaced it does not.
    #[test]
    fn prescale_ratio_does_not_drift_between_aspects() {
        let ratios: Vec<u32> = Aspect::ALL
            .iter()
            .map(|&a| {
                let o = OutputSpec::new(a, 1080, 30).unwrap();
                o.prescale_height() / o.height()
            })
            .collect();
        assert_eq!(ratios, vec![3, 3, 3]);
    }

    #[test]
    fn odd_and_zero_short_edges_are_refused() {
        assert!(OutputSpec::new(Aspect::Square1x1, 1081, 30).is_err());
        assert!(OutputSpec::new(Aspect::Square1x1, 0, 30).is_err());
        // 16:9 needs divisibility by 9 as well as evenness.
        assert!(OutputSpec::new(Aspect::Landscape16x9, 1000, 30).is_err());
    }

    #[test]
    fn frame_rate_is_bounded() {
        assert!(OutputSpec::new(Aspect::Square1x1, 1080, 0).is_err());
        assert!(OutputSpec::new(Aspect::Square1x1, 1080, 121).is_err());
        assert!(OutputSpec::new(Aspect::Square1x1, 1080, 120).is_ok());
    }

    /// `truncated.jpg` probes as 0x0. It must be named, not divided by.
    #[test]
    fn a_degenerate_source_is_refused_by_name() {
        let err = SourceGeometry::new(0, 0, 1, 1).unwrap_err();
        assert!(matches!(err, GeometryError::DegenerateSource { .. }));
        assert!(err.to_string().contains("truncated"));
    }

    /// ffprobe reports `0:1` for a still with no aspect metadata.
    #[test]
    fn unspecified_source_sar_means_square_pixels() {
        let g = SourceGeometry::new(4000, 3000, 0, 1).unwrap();
        assert_eq!(g.sar(), (1, 1));
        assert!(g.has_square_pixels());
    }

    /// D-143. Every named size has to work in **all three** aspects, which
    /// means every short edge here is even and divisible by 9 — 16:9's rule is
    /// the strict one, and a name that only worked in portrait and square
    /// would be a trap rather than a shortcut.
    #[test]
    fn every_named_resolution_renders_in_every_aspect() {
        for resolution in Resolution::ALL {
            for aspect in Aspect::ALL {
                OutputSpec::new(aspect, resolution.short_edge(), 30)
                    .unwrap_or_else(|e| panic!("{resolution} at {aspect} is not renderable: {e}"));
            }
        }
    }

    /// The four sizes, spelled out. These are the numbers an operator is
    /// picturing when they type the name, and nothing else in the tree
    /// asserts them.
    #[test]
    fn the_named_sizes_are_the_sizes_people_mean() {
        let landscape = |r: Resolution| {
            let s = OutputSpec::new(Aspect::Landscape16x9, r.short_edge(), 30).unwrap();
            (s.width(), s.height())
        };
        assert_eq!(landscape(Resolution::Hd720), (1280, 720));
        assert_eq!(landscape(Resolution::Fhd1080), (1920, 1080));
        assert_eq!(landscape(Resolution::Qhd1440), (2560, 1440), "2K");
        assert_eq!(landscape(Resolution::Uhd2160), (3840, 2160), "4K UHD");

        // Portrait is the same short edge, not the same long edge: 4K vertical
        // is 2160x3840. Getting this backwards is the arithmetic the name
        // exists to remove.
        let shorts = OutputSpec::new(Aspect::Portrait9x16, 1080, 30).unwrap();
        assert_eq!((shorts.width(), shorts.height()), (1080, 1920));
        let shorts_4k =
            OutputSpec::new(Aspect::Portrait9x16, Resolution::Uhd2160.short_edge(), 30).unwrap();
        assert_eq!((shorts_4k.width(), shorts_4k.height()), (2160, 3840));
    }

    #[test]
    fn a_resolution_parses_by_name_and_by_alias() {
        for (text, want) in [
            ("720p", Resolution::Hd720),
            ("720", Resolution::Hd720),
            ("1080p", Resolution::Fhd1080),
            ("1080", Resolution::Fhd1080),
            ("2K", Resolution::Qhd1440),
            ("1440p", Resolution::Qhd1440),
            ("qhd", Resolution::Qhd1440),
            ("4k", Resolution::Uhd2160),
            (" 4K ", Resolution::Uhd2160),
            ("2160p", Resolution::Uhd2160),
            ("UHD", Resolution::Uhd2160),
        ] {
            assert_eq!(Resolution::parse(text), Some(want), "parsing {text:?}");
        }
        assert_eq!(Resolution::parse("8k"), None, "past the D-114 ceiling");
        assert_eq!(Resolution::parse(""), None);
        assert_eq!(Resolution::parse("1081p"), None);
    }

    /// Round trip: a name resolves to a short edge, and a short edge is
    /// reported back under the same name. The window shows a project's
    /// geometry through `from_short_edge`, so a mismatch here would display a
    /// size the project does not have.
    #[test]
    fn a_named_size_round_trips_through_its_short_edge() {
        for resolution in Resolution::ALL {
            assert_eq!(
                Resolution::from_short_edge(resolution.short_edge()),
                Some(resolution)
            );
            assert_eq!(Resolution::parse(resolution.as_str()), Some(resolution));
        }
        assert_eq!(
            Resolution::from_short_edge(900),
            None,
            "no name, still legal"
        );
    }

    /// No two names may share an alias, and no alias may be another name —
    /// `parse` returns the first match, so a duplicate would silently make one
    /// of the two unreachable.
    #[test]
    fn no_two_resolutions_answer_to_the_same_word() {
        let mut seen: Vec<String> = Vec::new();
        for resolution in Resolution::ALL {
            let mut words = vec![resolution.as_str().to_owned()];
            words.extend(resolution.aliases().iter().map(|a| (*a).to_owned()));
            for word in words {
                assert!(!seen.contains(&word), "{word:?} is claimed twice");
                assert_eq!(
                    Resolution::parse(&word),
                    Some(resolution),
                    "{word:?} must reach {resolution}"
                );
                seen.push(word);
            }
        }
    }

    /// D-143. A YouTube Short, a Reel and a TikTok are one frame, so they are
    /// one word here — an operator naming the destination gets the geometry.
    #[test]
    fn the_vertical_destinations_are_all_one_aspect() {
        for text in [
            "shorts",
            "Shorts",
            "youtube-shorts",
            "reel",
            "reels",
            "tiktok",
            "story",
            "vertical",
            "portrait",
        ] {
            assert_eq!(
                Aspect::parse(text),
                Some(Aspect::Portrait9x16),
                "{text:?} is a vertical film"
            );
        }
        assert_eq!(Aspect::parse("youtube"), Some(Aspect::Landscape16x9));
    }

    #[test]
    fn aspect_parses_the_forms_an_operator_will_type() {
        for (text, want) in [
            ("16:9", Aspect::Landscape16x9),
            ("16x9", Aspect::Landscape16x9),
            ("Portrait", Aspect::Portrait9x16),
            ("9:16", Aspect::Portrait9x16),
            ("1:1", Aspect::Square1x1),
            ("square", Aspect::Square1x1),
        ] {
            assert_eq!(Aspect::parse(text), Some(want), "parsing {text:?}");
        }
        assert_eq!(Aspect::parse("4:3"), None);
    }

    /// D-114. The constructor is reachable from `project.yaml`, from
    /// `still render-scene --short-edge`, and from the window's
    /// `subtitle_preview` IPC command, so every one of these is operator input.
    #[test]
    fn a_short_edge_too_large_to_render_is_refused_rather_than_wrapped() {
        // The two that overflowed. In debug this panicked with "attempt to
        // multiply with overflow"; in release it *wrapped* and was accepted —
        // 4294967292 became a 3340530112x4294967292 output that `validate`
        // reported as "no problems", and 1431655776 produced a prescale canvas
        // of 3340530176x32, the height having wrapped to 32.
        for edge in [4_294_967_292u32, 1_431_655_776] {
            for aspect in [
                Aspect::Landscape16x9,
                Aspect::Portrait9x16,
                Aspect::Square1x1,
            ] {
                assert!(
                    OutputSpec::new(aspect, edge, 30).is_err(),
                    "{aspect:?} accepted a short edge of {edge}"
                );
            }
        }

        // Not an overflow, and still refused: 160000x90000 is 43 200 000
        // macroblocks and a raw RGBA frame of it is 57 GB.
        assert!(OutputSpec::new(Aspect::Landscape16x9, 90_000, 30).is_err());

        // The boundary, both sides. 4K is the largest real size and renders;
        // 8K is past what the profile could honestly label.
        let uhd = OutputSpec::new(Aspect::Landscape16x9, 2160, 30).expect("4K renders");
        assert_eq!((uhd.width(), uhd.height()), (3840, 2160));
        assert!(macroblocks(uhd.width(), uhd.height()) <= MAX_MACROBLOCKS);
        assert!(
            OutputSpec::new(Aspect::Landscape16x9, 4320, 30).is_err(),
            "8K would be declared level 5.2 and no 5.2 decoder can play it"
        );
    }

    /// The invariant `prescale_width`/`prescale_height` rely on: the cap keeps
    /// every accepted frame far below `u32::MAX / 3`, so tripling cannot
    /// overflow. Asserted at the largest frame the cap admits, which is the
    /// only place it could ever fail.
    #[test]
    fn the_prescale_canvas_cannot_overflow_at_the_ceiling() {
        for aspect in [
            Aspect::Landscape16x9,
            Aspect::Portrait9x16,
            Aspect::Square1x1,
        ] {
            // Walk down to the biggest short edge this aspect accepts.
            let mut edge = 4608;
            let spec = loop {
                if let Ok(spec) = OutputSpec::new(aspect, edge, 30) {
                    break spec;
                }
                edge -= 18;
                assert!(edge > 0, "{aspect:?} accepted nothing");
            };
            assert_eq!(
                spec.prescale_width(),
                spec.width()
                    .checked_mul(PRESCALE_FACTOR)
                    .expect("no overflow"),
            );
            assert_eq!(
                spec.prescale_height(),
                spec.height()
                    .checked_mul(PRESCALE_FACTOR)
                    .expect("no overflow"),
            );
        }
    }

    /// F-13. The author's whole corpus is 1376x768 into a 1920x1080 frame:
    /// every scene of every chapter is an upscale, and nothing said so.
    #[test]
    fn a_source_smaller_than_the_frame_does_not_fill_it() {
        let out = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap();
        let real = SourceGeometry::new(1376, 768, 1, 1).unwrap();
        assert!(!real.fills(&out));
        assert!(SourceGeometry::new(1920, 1080, 1, 1).unwrap().fills(&out));
        assert!(SourceGeometry::new(4000, 3000, 1, 1).unwrap().fills(&out));
    }

    /// Cover-fit needs **both** edges, so a source that is wide enough and
    /// short is still enlarged — the failure a width-only check would miss.
    #[test]
    fn one_edge_long_enough_is_not_enough() {
        let out = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap();
        assert!(!SourceGeometry::new(4000, 1000, 1, 1).unwrap().fills(&out));
        assert!(!SourceGeometry::new(1900, 4000, 1, 1).unwrap().fills(&out));
    }

    /// SAR is not decoration: an anamorphic source covers more frame than its
    /// stored width, and `fills` reads the display size D-034 actually scales.
    #[test]
    fn an_anamorphic_source_is_measured_after_its_pixels_are_squared_up() {
        let out = OutputSpec::new(Aspect::Landscape16x9, 720, 30).unwrap();
        // 1000x720 stored at 4:3 is 1333x720 displayed — wide enough for
        // 1280x720, which the stored width alone is not.
        let anamorphic = SourceGeometry::new(1000, 720, 4, 3).unwrap();
        assert_eq!(anamorphic.display_width(), 1333);
        assert_eq!(anamorphic.display_height(), 720);
        assert!(anamorphic.fills(&out));
        assert!(!SourceGeometry::new(1000, 720, 1, 1).unwrap().fills(&out));
    }

    /// The number the warning offers has to be a size `OutputSpec` accepts,
    /// or the message tells the operator to type something that is refused.
    #[test]
    fn every_suggested_short_edge_is_a_size_that_renders() {
        for aspect in Aspect::ALL {
            for (w, h) in [
                (1376, 768),
                (640, 480),
                (4000, 3000),
                (1920, 1080),
                (17, 19),
                (3, 3),
            ] {
                let edge = SourceGeometry::new(w, h, 1, 1)
                    .unwrap()
                    .native_short_edge(aspect);
                if edge == 0 {
                    continue;
                }
                let spec = OutputSpec::new(aspect, edge, 30)
                    .unwrap_or_else(|e| panic!("{aspect:?} {w}x{h} suggested {edge}: {e}"));
                assert!(
                    SourceGeometry::new(w, h, 1, 1).unwrap().fills(&spec),
                    "{aspect:?} {w}x{h} suggested {edge}, which it does not fill"
                );
            }
        }
    }

    /// The author's own stills, in the two shapes they might be cut for.
    #[test]
    fn the_native_size_depends_on_the_shape_it_is_cut_to() {
        let real = SourceGeometry::new(1376, 768, 1, 1).unwrap();
        assert_eq!(real.native_short_edge(Aspect::Landscape16x9), 756);
        assert_eq!(real.native_short_edge(Aspect::Portrait9x16), 432);
        assert_eq!(real.native_short_edge(Aspect::Square1x1), 768);
    }

    /// A source too small for any legal frame answers 0 rather than a size
    /// that would be refused a line later.
    #[test]
    fn a_source_smaller_than_the_smallest_legal_frame_suggests_nothing() {
        let tiny = SourceGeometry::new(10, 10, 1, 1).unwrap();
        assert_eq!(tiny.native_short_edge(Aspect::Landscape16x9), 0);
        assert_eq!(tiny.native_short_edge(Aspect::Square1x1), 10);
    }
}
