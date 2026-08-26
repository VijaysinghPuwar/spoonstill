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
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().replace('x', ":").as_str() {
            "16:9" | "landscape" => Some(Aspect::Landscape16x9),
            "9:16" | "portrait" | "vertical" => Some(Aspect::Portrait9x16),
            "1:1" | "square" => Some(Aspect::Square1x1),
            _ => None,
        }
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
        /// Why it cannot be used.
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

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GeometryError::UnusableShortEdge { short_edge, reason } => {
                write!(f, "short edge {short_edge} is unusable: {reason}")
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
                reason: "must be positive",
            });
        }
        if !short_edge.is_multiple_of(2) {
            return Err(GeometryError::UnusableShortEdge {
                short_edge,
                reason: "must be even — H.264 4:2:0 cannot represent odd dimensions",
            });
        }
        let long_edge = match aspect {
            Aspect::Landscape16x9 | Aspect::Portrait9x16 => {
                if !short_edge.is_multiple_of(9) {
                    return Err(GeometryError::UnusableShortEdge {
                        short_edge,
                        reason: "16:9 needs a short edge divisible by 9 to stay integer",
                    });
                }
                short_edge / 9 * 16
            }
            Aspect::Square1x1 => short_edge,
        };
        if !long_edge.is_multiple_of(2) {
            return Err(GeometryError::UnusableShortEdge {
                short_edge,
                reason: "produces an odd long edge",
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
}
