//! Subtitles: what to show, when, and in what style (D-106). Pure. No I/O.
//!
//! This module owns three separable things, and keeping them separate is the
//! point:
//!
//! - **[`SubtitleTheme`]** — the six looks an operator can choose between, each
//!   resolving to a [`ThemeStyle`] of plain numbers and colours. Every length
//!   is a *fraction* of the output frame, never a pixel, so one theme means the
//!   same thing at 720p and at 4K and in all three of D-070's aspects.
//! - **[`cues`]** — one scene's caption text, cut into timed pieces.
//! - **[`SubtitleSpec`]** — what the cache key hashes, so that changing a theme
//!   re-renders and changing nothing does not (D-043).
//!
//! What is deliberately **not** here is anything that needs a font. Line
//! wrapping by real glyph metrics, and the rasterizing itself, live in
//! `spoonstill_media::caption`, because a font file is concrete and D-010 keeps
//! this crate free of concrete things. The division shows up in [`ThemeStyle`]:
//! `max_chars` is a *character budget* used to cut cues without measuring
//! anything, and the renderer re-wraps the result properly once it has metrics.
//!
//! # Why the timings are proportional
//!
//! A cue's share of the scene is its share of the characters. That is an
//! approximation of speech rate, and it is the honest one available to us: the
//! only source of true word timing is the provider's own word-boundary events,
//! which D-072 files under V1.1 and which a *supplied recording* does not have
//! at all. Proportional timing is stable, needs no network, and works
//! identically for spoken, recorded and silent scenes.

use core::fmt;

/// Shortest a cue may stay on screen, in seconds.
///
/// Below this a caption reads as a flicker rather than as words. It is not a
/// clamp applied afterwards — it *bounds the cue count* in [`cues`], so the
/// text is cut into fewer, longer pieces rather than into pieces that are then
/// silently stretched into overlapping each other.
pub const MIN_CUE_SECONDS: f64 = 0.9;

/// Most cues one scene may produce.
///
/// Each cue costs the renderer one FFmpeg input and one `overlay` filter, so
/// this is a real resource bound rather than a taste one. It binds only on the
/// pathological scene D-095 describes — an hour of narration in a single row —
/// where cues become long rather than absent. A scene of ordinary length is
/// nowhere near it: at [`ThemeStyle::max_chars`] of around ninety, sixty cues
/// is some five thousand characters, about six minutes of speech.
pub const MAX_CUES: usize = 60;

/// A colour, straight 8-bit RGBA, not premultiplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgba {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
    /// Alpha. `0` means the element is off, which is how a theme declines an
    /// outline or a shadow without a second boolean to disagree with.
    pub a: u8,
}

impl Rgba {
    /// A colour from its four channels.
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba { r, g, b, a }
    }

    /// Fully transparent — the "this theme has no such element" value.
    pub const NONE: Rgba = Rgba::new(0, 0, 0, 0);

    /// Whether this colour draws anything at all.
    #[must_use]
    pub const fn is_visible(self) -> bool {
        self.a > 0
    }
}

impl fmt::Display for Rgba {
    /// `#rrggbbaa`, which is what goes into the cache key and the logs.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#{:02x}{:02x}{:02x}{:02x}",
            self.r, self.g, self.b, self.a
        )
    }
}

/// Which of the three bundled weights a theme draws with.
///
/// Three, not a numeric axis: the renderer holds three real font files, and a
/// weight that does not exist would otherwise have to be faked by dilating the
/// glyph mask — which is what makes synthetic bold look like a mistake.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    /// The lightest bundled weight.
    Regular,
    /// The default for captions: heavy enough to hold an edge against a busy
    /// photograph without reading as shouting.
    SemiBold,
    /// For themes whose whole idea is emphasis.
    Bold,
}

impl Weight {
    /// Stable word for the cache key and the logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Weight::Regular => "regular",
            Weight::SemiBold => "semibold",
            Weight::Bold => "bold",
        }
    }
}

impl fmt::Display for Weight {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What sits behind the words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backdrop {
    /// Nothing. The text carries its own legibility through outline or shadow.
    None,
    /// A rounded rectangle hugging the text block.
    Fitted,
    /// A bar spanning the full frame width.
    Band,
}

impl Backdrop {
    /// Stable word for the cache key and the logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Backdrop::None => "none",
            Backdrop::Fitted => "fitted",
            Backdrop::Band => "band",
        }
    }
}

/// Where the caption block sits vertically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Placement {
    /// Bottom of the frame — where a viewer looks for subtitles.
    Bottom,
    /// Top. For footage whose subject lives along the bottom edge.
    Top,
}

impl Placement {
    /// Stable word for the cache key, the logs and `project.yaml`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Placement::Bottom => "bottom",
            Placement::Top => "top",
        }
    }

    /// Parse the `subtitles.position` cell. Case-insensitive.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "bottom" => Some(Placement::Bottom),
            "top" => Some(Placement::Top),
            _ => None,
        }
    }

    /// Every placement, in the order a chooser should offer them.
    pub const ALL: [Placement; 2] = [Placement::Bottom, Placement::Top];
}

impl fmt::Display for Placement {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The six looks.
///
/// Six because they span the actual decisions rather than the actual colours:
/// outlined or boxed, dark or light, quiet or loud. An operator picking between
/// them is choosing how much the caption is allowed to cover the photograph,
/// which is the only question the theme really asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SubtitleTheme {
    /// White text with a hard black edge and a soft drop shadow, no box. The
    /// broadcast default, and the one that covers least of the image.
    Classic,
    /// White text on a rounded translucent black plate that hugs the words.
    /// The most legible thing here over a bright or busy still.
    Boxed,
    /// White text on a black bar across the full width, flush to the edge.
    /// Documentary.
    Band,
    /// Near-black text on a warm off-white card. Editorial, and the only light
    /// theme — it reads as a printed caption rather than as television.
    Card,
    /// Heavy yellow with a thick black edge. Loud on purpose: social video,
    /// watched muted on a phone in daylight.
    Punch,
    /// Small, light, close to the edge, shadow only. For footage that should
    /// not have to share the frame.
    Minimal,
}

impl SubtitleTheme {
    /// Every theme, in the order a chooser should offer them.
    ///
    /// [`SubtitleTheme::Classic`] is first because it is the default.
    pub const ALL: [SubtitleTheme; 6] = [
        SubtitleTheme::Classic,
        SubtitleTheme::Boxed,
        SubtitleTheme::Band,
        SubtitleTheme::Card,
        SubtitleTheme::Punch,
        SubtitleTheme::Minimal,
    ];

    /// The name in `project.yaml`, on the command line, and in the cache key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            SubtitleTheme::Classic => "classic",
            SubtitleTheme::Boxed => "boxed",
            SubtitleTheme::Band => "band",
            SubtitleTheme::Card => "card",
            SubtitleTheme::Punch => "punch",
            SubtitleTheme::Minimal => "minimal",
        }
    }

    /// One line an operator can choose between, for `still subtitles` and the
    /// window's theme list.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            SubtitleTheme::Classic => {
                "White with a black edge and a soft shadow. No box — covers the least image."
            }
            SubtitleTheme::Boxed => {
                "White on a rounded translucent black plate. The most legible over a busy still."
            }
            SubtitleTheme::Band => "White on a black bar across the full width. Documentary.",
            SubtitleTheme::Card => {
                "Near-black on a warm off-white card. The one light theme; reads as print."
            }
            SubtitleTheme::Punch => {
                "Heavy yellow with a thick black edge. For social video watched muted."
            }
            SubtitleTheme::Minimal => "Small, light, shadow only, close to the edge. Understated.",
        }
    }

    /// Parse the `subtitles.theme` cell. Case-insensitive, whitespace-tolerant.
    ///
    /// Returns `None` rather than a default, because D-055's rule holds here as
    /// everywhere: present-but-wrong is not the same as absent, and a
    /// misspelled theme is a problem to report rather than a look to substitute.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let wanted = text.trim().to_ascii_lowercase();
        SubtitleTheme::ALL
            .into_iter()
            .find(|theme| theme.as_str() == wanted)
    }

    /// Every theme name, comma-separated — for the `expected` half of a
    /// problem message, so the list can never drift from [`SubtitleTheme::ALL`].
    #[must_use]
    pub fn names() -> String {
        SubtitleTheme::ALL
            .iter()
            .map(|t| t.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// The numbers behind the look.
    #[must_use]
    pub const fn style(self) -> ThemeStyle {
        // Every length below is a fraction: of the frame height for anything
        // vertical, of the frame width for anything horizontal, of the type
        // size for anything belonging to the type itself. Nothing here is a
        // pixel, because a pixel would pin the design to 1080p.
        match self {
            SubtitleTheme::Classic => ThemeStyle {
                weight: Weight::SemiBold,
                size: 0.050,
                line_spacing: 1.22,
                fill: Rgba::new(255, 255, 255, 255),
                outline: Rgba::new(0, 0, 0, 235),
                outline_width: 0.115,
                shadow: Rgba::new(0, 0, 0, 150),
                shadow_offset: 0.055,
                shadow_blur: 0.075,
                backdrop: Backdrop::None,
                backdrop_fill: Rgba::NONE,
                padding_x: 0.0,
                padding_y: 0.0,
                radius: 0.0,
                margin: 0.075,
                max_width: 0.86,
                max_lines: 2,
                max_chars: 88,
            },
            SubtitleTheme::Boxed => ThemeStyle {
                weight: Weight::SemiBold,
                size: 0.046,
                line_spacing: 1.24,
                fill: Rgba::new(255, 255, 255, 255),
                outline: Rgba::NONE,
                outline_width: 0.0,
                shadow: Rgba::NONE,
                shadow_offset: 0.0,
                shadow_blur: 0.0,
                backdrop: Backdrop::Fitted,
                backdrop_fill: Rgba::new(0, 0, 0, 172),
                padding_x: 0.62,
                padding_y: 0.34,
                radius: 0.30,
                margin: 0.070,
                max_width: 0.82,
                max_lines: 2,
                max_chars: 84,
            },
            SubtitleTheme::Band => ThemeStyle {
                weight: Weight::Regular,
                size: 0.044,
                line_spacing: 1.26,
                fill: Rgba::new(255, 255, 255, 250),
                outline: Rgba::NONE,
                outline_width: 0.0,
                shadow: Rgba::NONE,
                shadow_offset: 0.0,
                shadow_blur: 0.0,
                backdrop: Backdrop::Band,
                backdrop_fill: Rgba::new(0, 0, 0, 205),
                padding_x: 0.0,
                padding_y: 0.62,
                radius: 0.0,
                // Flush to the edge: a band that floats is a band with a
                // stripe of photograph under it, which reads as a mistake.
                margin: 0.0,
                max_width: 0.80,
                max_lines: 2,
                max_chars: 80,
            },
            SubtitleTheme::Card => ThemeStyle {
                weight: Weight::SemiBold,
                size: 0.043,
                line_spacing: 1.28,
                fill: Rgba::new(24, 23, 21, 255),
                outline: Rgba::NONE,
                outline_width: 0.0,
                // The shadow belongs to the card, not to the type: it is what
                // lifts an off-white plate off a pale sky.
                shadow: Rgba::new(0, 0, 0, 90),
                shadow_offset: 0.10,
                shadow_blur: 0.22,
                backdrop: Backdrop::Fitted,
                backdrop_fill: Rgba::new(249, 247, 243, 242),
                padding_x: 0.72,
                padding_y: 0.42,
                radius: 0.26,
                margin: 0.075,
                max_width: 0.78,
                max_lines: 2,
                max_chars: 80,
            },
            SubtitleTheme::Punch => ThemeStyle {
                weight: Weight::Bold,
                size: 0.058,
                line_spacing: 1.18,
                fill: Rgba::new(255, 214, 10, 255),
                outline: Rgba::new(0, 0, 0, 255),
                outline_width: 0.165,
                shadow: Rgba::new(0, 0, 0, 130),
                shadow_offset: 0.06,
                shadow_blur: 0.06,
                backdrop: Backdrop::None,
                backdrop_fill: Rgba::NONE,
                padding_x: 0.0,
                padding_y: 0.0,
                radius: 0.0,
                margin: 0.090,
                max_width: 0.84,
                max_lines: 2,
                max_chars: 62,
            },
            SubtitleTheme::Minimal => ThemeStyle {
                weight: Weight::Regular,
                size: 0.037,
                line_spacing: 1.30,
                fill: Rgba::new(255, 255, 255, 238),
                outline: Rgba::NONE,
                outline_width: 0.0,
                shadow: Rgba::new(0, 0, 0, 170),
                shadow_offset: 0.055,
                shadow_blur: 0.16,
                backdrop: Backdrop::None,
                backdrop_fill: Rgba::NONE,
                padding_x: 0.0,
                padding_y: 0.0,
                radius: 0.0,
                margin: 0.055,
                max_width: 0.72,
                max_lines: 2,
                max_chars: 76,
            },
        }
    }
}

impl fmt::Display for SubtitleTheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A theme as plain numbers.
///
/// Fractions throughout — see [`SubtitleTheme::style`]. The renderer multiplies
/// them by the real frame and rounds once, at the end, so a theme is the same
/// design at every output size rather than the same pixel counts.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ThemeStyle {
    /// Which bundled font weight.
    pub weight: Weight,
    /// Type size, as a fraction of frame height.
    pub size: f64,
    /// Baseline-to-baseline distance, as a multiple of the type size.
    pub line_spacing: f64,
    /// The letterforms themselves.
    pub fill: Rgba,
    /// The hard edge around them. [`Rgba::NONE`] for no outline.
    pub outline: Rgba,
    /// Outline thickness, as a fraction of the type size.
    pub outline_width: f64,
    /// The soft shadow. [`Rgba::NONE`] for none.
    pub shadow: Rgba,
    /// How far the shadow falls, down and right, as a fraction of type size.
    pub shadow_offset: f64,
    /// How soft it is, as a fraction of type size. `0.0` is a hard shadow.
    pub shadow_blur: f64,
    /// What sits behind the words.
    pub backdrop: Backdrop,
    /// The backdrop's colour.
    pub backdrop_fill: Rgba,
    /// Backdrop padding left and right, as a fraction of type size.
    pub padding_x: f64,
    /// Backdrop padding above and below, as a fraction of type size.
    pub padding_y: f64,
    /// Corner radius, as a fraction of type size.
    pub radius: f64,
    /// Distance from the frame edge the caption sits against, as a fraction of
    /// frame height. Which edge is [`Placement`]'s business, not the theme's.
    pub margin: f64,
    /// Widest the text may run, as a fraction of frame width.
    pub max_width: f64,
    /// Most lines one cue may wrap to before the renderer shrinks the type.
    pub max_lines: u32,
    /// Character budget per cue, used by [`cues`] before any font exists.
    pub max_chars: usize,
}

/// One caption, and the window it is on screen.
///
/// Times are seconds from the start of the **scene**, and they are closed at
/// the start and open at the end: consecutive cues share a boundary and are
/// never both drawn, because the renderer's `enable` window is `between(t, ...)`
/// on the earlier one and the later one wins the tie by being drawn second.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// The words, whitespace already normalized.
    pub text: String,
    /// When it appears, in seconds from the start of the scene.
    pub start: f64,
    /// When it goes.
    pub end: f64,
}

impl Cue {
    /// How long it is on screen.
    #[must_use]
    pub fn duration(&self) -> f64 {
        (self.end - self.start).max(0.0)
    }
}

/// Cut one scene's caption text into timed cues.
///
/// `seconds` is the **narration** duration, not the padded segment duration
/// (D-022): the last cue leaves the screen when the speaking stops, so the
/// frames of silence at the end of a scene are silent in both senses.
///
/// The cue count is bounded twice — by [`MIN_CUE_SECONDS`], so nothing flickers,
/// and by [`MAX_CUES`], so nothing turns one scene into sixty FFmpeg inputs.
/// When either bound binds, the text is re-cut into fewer, longer cues rather
/// than truncated: every word an operator wrote reaches the screen.
///
/// Returns an empty vector for text that is blank, or for a duration that is
/// not positive — a caption with nowhere to go is not an error, it is nothing.
#[must_use]
pub fn cues(text: &str, seconds: f64, budget: usize) -> Vec<Cue> {
    let words = normalize(text);
    if words.is_empty() || !seconds.is_finite() || seconds <= 0.0 {
        return Vec::new();
    }

    // How many cues this scene can hold: enough time for each to be read, and
    // never more than the renderer will spend inputs on.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let by_time = (seconds / MIN_CUE_SECONDS).floor().max(1.0) as usize;
    let allowed = by_time.clamp(1, MAX_CUES);

    // Grow the budget until the split fits. Each pass at least doubles the
    // shortfall away, so this terminates in a handful of rounds even for the
    // hour-long single scene of D-095; the guard is belt and braces.
    let mut budget = budget.max(8);
    let mut pieces = split(&words, budget);
    for _ in 0..8 {
        if pieces.len() <= allowed {
            break;
        }
        let total: usize = words.chars().count();
        budget = budget.max(total.div_ceil(allowed)) + 1;
        pieces = split(&words, budget);
    }
    // A single word longer than any budget cannot be split further, so the
    // count can still exceed `allowed` in theory. Joining the tail is better
    // than dropping it.
    if pieces.len() > allowed {
        let tail = pieces.split_off(allowed.saturating_sub(1).max(1));
        pieces.push(tail.join(" "));
    }

    // Time is shared out by character count, which is the best proxy for speech
    // rate available without word boundaries (D-072). Boundaries are computed
    // from a running total rather than accumulated, so the last cue ends
    // exactly on `seconds` instead of a rounding error short of it.
    let lengths: Vec<usize> = pieces.iter().map(|p| p.chars().count().max(1)).collect();
    let total: usize = lengths.iter().sum();
    #[allow(clippy::cast_precision_loss)]
    let total_f = total as f64;

    let mut out = Vec::with_capacity(pieces.len());
    let mut before = 0usize;
    for (piece, length) in pieces.into_iter().zip(lengths) {
        #[allow(clippy::cast_precision_loss)]
        let start = seconds * (before as f64) / total_f;
        before += length;
        #[allow(clippy::cast_precision_loss)]
        let end = seconds * (before as f64) / total_f;
        out.push(Cue {
            text: piece,
            start,
            end,
        });
    }
    out
}

/// Collapse every run of whitespace to one space and trim the ends.
///
/// A script is a text file an operator typed, so it arrives with hard wraps,
/// tabs, and the occasional non-breaking space. None of that is layout — the
/// renderer wraps to the frame, and a line break the operator happened to type
/// at column 72 would otherwise become a line break on screen.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Greedily pack words into pieces of at most `budget` characters, preferring
/// to end a piece where a sentence ends.
///
/// The sentence preference is what stops a caption reading
/// "…and then he left. The next" — a break placed after `left.` costs a few
/// characters of packing and buys a cue that is a complete thought.
fn split(words: &str, budget: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut current = String::new();

    for word in words.split(' ') {
        let would_be = if current.is_empty() {
            word.chars().count()
        } else {
            current.chars().count() + 1 + word.chars().count()
        };

        if !current.is_empty() && would_be > budget {
            pieces.push(core::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);

        // A sentence that has ended and already fills most of the budget is a
        // better place to break than wherever the next word runs out of room.
        if ends_sentence(word) && current.chars().count() * 2 >= budget {
            pieces.push(core::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        pieces.push(current);
    }
    if pieces.is_empty() {
        pieces.push(String::new());
    }
    carry_weak_endings(&mut pieces);
    pieces
}

/// Words that must not be the last thing on screen before a cut.
///
/// The subtitler's rule: a cue ending on a conjunction, preposition or article
/// leaves the viewer holding an unfinished phrase across a cut. Seen on real
/// narration — *"Those born with poor aptitude could pour twenty-four hours a
/// day into training and"* / *"still fall short…"* — where the break lands
/// wherever the character budget ran out rather than where the sentence
/// breathes.
///
/// Deliberately short and closed-class. It is a list of function words, not a
/// grammar: anything longer starts making judgements about content, and
/// anything cleverer needs a parser we are not going to carry.
const WEAK_ENDINGS: [&str; 34] = [
    "a", "an", "the", "and", "or", "but", "nor", "so", "yet", "of", "to", "in", "on", "at", "for",
    "with", "from", "by", "as", "into", "onto", "over", "under", "than", "that", "which", "who",
    "if", "when", "while", "because", "though", "although", "before",
];

/// Move a trailing function word onto the next piece.
///
/// Only ever moves **one** word, and never the only word in a piece: a cue must
/// not be emptied to spare the one before it, and moving a run of them just
/// relocates the same problem.
fn carry_weak_endings(pieces: &mut [String]) {
    for index in 0..pieces.len().saturating_sub(1) {
        let last = match pieces[index].rsplit(' ').next() {
            Some(word) => word.to_owned(),
            None => continue,
        };
        // A piece of one word has nothing to give away.
        if !pieces[index].contains(' ') {
            continue;
        }
        let bare = last.trim_matches(|c: char| !c.is_alphanumeric());
        if !WEAK_ENDINGS.contains(&bare.to_ascii_lowercase().as_str()) {
            continue;
        }
        // A word carrying punctuation is ending something, not dangling.
        if last.ends_with([',', ';', ':', '.', '!', '?']) {
            continue;
        }
        pieces[index].truncate(pieces[index].len() - last.len() - 1);
        let next = std::mem::take(&mut pieces[index + 1]);
        pieces[index + 1] = if next.is_empty() {
            last
        } else {
            format!("{last} {next}")
        };
    }
}

/// Whether a word ends a sentence — allowing for the closing quote or bracket
/// that so often follows the stop.
fn ends_sentence(word: &str) -> bool {
    let trimmed = word.trim_end_matches(['"', '\'', ')', ']', '»', '”', '’']);
    trimmed.ends_with(['.', '!', '?', '…'])
}

/// Everything about a scene's subtitles that changes the pixels (D-043).
///
/// This is what the segment cache key hashes. It is a *rendered* description
/// rather than the structs themselves, for the same reason [`crate::project::TtsSettings`]
/// is a list of strings: a key derived from a `#[derive(Hash)]` on a struct
/// changes silently when a field is added, and a key derived from text changes
/// only when the text does.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleSpec {
    /// The look.
    pub theme: SubtitleTheme,
    /// Which edge it sits against.
    pub placement: Placement,
    /// The cues, already timed.
    pub cues: Vec<Cue>,
}

impl SubtitleSpec {
    /// Whether this draws anything. A scene with no caption text resolves to a
    /// spec with no cues, and the renderer must treat that as "no subtitles"
    /// rather than as "an empty subtitle".
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// The lines a cache key is built from — the theme, the placement, and
    /// every cue's text and timing to the millisecond.
    ///
    /// Millisecond precision, not full float precision: two runs that differ
    /// only in the last bits of a measured duration would otherwise miss the
    /// cache for a difference no viewer could see. A millisecond is far below
    /// one frame at any frame rate we render.
    #[must_use]
    pub fn key_fields(&self) -> Vec<String> {
        let mut fields = Vec::with_capacity(self.cues.len() + 2);
        fields.push(format!("theme={}", self.theme.as_str()));
        fields.push(format!("place={}", self.placement.as_str()));
        for cue in &self.cues {
            fields.push(format!(
                "{:.3}>{:.3}:{}",
                cue.start,
                cue.end,
                cue.text.as_str()
            ));
        }
        fields
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_round_trips_through_its_name() {
        for theme in SubtitleTheme::ALL {
            assert_eq!(SubtitleTheme::parse(theme.as_str()), Some(theme));
            assert_eq!(
                SubtitleTheme::parse(&format!("  {}  ", theme.as_str().to_uppercase())),
                Some(theme),
                "the name is case- and whitespace-insensitive"
            );
        }
        assert_eq!(SubtitleTheme::parse("clasic"), None, "D-055: no fallback");
        assert_eq!(SubtitleTheme::parse(""), None);
    }

    #[test]
    fn every_theme_name_is_distinct_and_listed() {
        let mut names: Vec<&str> = SubtitleTheme::ALL.iter().map(|t| t.as_str()).collect();
        let count = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), count, "two themes share a name");

        let listed = SubtitleTheme::names();
        for theme in SubtitleTheme::ALL {
            assert!(listed.contains(theme.as_str()), "{listed} omits {theme}");
        }
    }

    /// A theme is a design, and these are the properties that make it one that
    /// can be rendered at all. Caught here rather than as a strange-looking
    /// frame six hundred scenes into a film.
    #[test]
    fn every_theme_is_a_usable_design() {
        for theme in SubtitleTheme::ALL {
            let s = theme.style();
            assert!(
                s.size > 0.02 && s.size < 0.12,
                "{theme}: type size {} is not a caption",
                s.size
            );
            assert!(s.line_spacing >= 1.0, "{theme}: lines would overlap");
            assert!(s.fill.is_visible(), "{theme}: invisible text");
            assert!(
                s.max_width > 0.4 && s.max_width <= 1.0,
                "{theme}: unusable text width"
            );
            assert!(s.max_lines >= 1, "{theme}: no room for a line");
            assert!(s.max_chars >= 20, "{theme}: cues would be single words");
            assert!(
                s.margin >= 0.0 && s.margin < 0.4,
                "{theme}: margin puts the caption off the frame"
            );
            assert!(
                (s.backdrop == Backdrop::None) != s.backdrop_fill.is_visible(),
                "{theme}: a backdrop and its colour must agree"
            );
            // Legibility is not optional. Every theme has to survive a white
            // sky and a black shadow, so plain fill with nothing behind or
            // around it is a design that will be unreadable in some scene.
            assert!(
                s.backdrop != Backdrop::None || s.outline.is_visible() || s.shadow.is_visible(),
                "{theme}: no outline, shadow or backdrop — unreadable over a bright still"
            );
        }
    }

    #[test]
    fn blank_or_timeless_text_produces_nothing() {
        assert!(cues("", 10.0, 80).is_empty());
        assert!(cues("   \n\t ", 10.0, 80).is_empty());
        assert!(cues("hello", 0.0, 80).is_empty());
        assert!(cues("hello", -1.0, 80).is_empty());
        assert!(cues("hello", f64::NAN, 80).is_empty());
    }

    #[test]
    fn a_short_line_is_one_cue_spanning_the_whole_scene() {
        let c = cues("A quiet morning on the water.", 6.0, 80);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].text, "A quiet morning on the water.");
        assert!((c[0].start - 0.0).abs() < 1e-9);
        assert!((c[0].end - 6.0).abs() < 1e-9, "{:?}", c[0]);
    }

    /// The operator's line breaks are theirs, not the screen's.
    #[test]
    fn whitespace_is_normalized_before_anything_else() {
        let c = cues("  one\n\ttwo   three\r\nfour  ", 8.0, 80);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].text, "one two three four");
    }

    /// Every word reaches the screen. This is the property that matters most:
    /// a caption that silently drops the second half of a sentence is worse
    /// than no caption at all.
    #[test]
    fn no_word_is_ever_lost() {
        let text = "The tide came in before dawn. \
                    By seven the harbour was full, and the boats \
                    that had waited all winter went out one after another \
                    into a flat grey sea that gave nothing away.";
        for seconds in [2.0, 5.0, 30.0, 600.0] {
            for budget in [20, 40, 80, 200] {
                let joined = cues(text, seconds, budget)
                    .into_iter()
                    .map(|c| c.text)
                    .collect::<Vec<_>>()
                    .join(" ");
                assert_eq!(
                    joined,
                    normalize(text),
                    "words lost at {seconds}s, budget {budget}"
                );
            }
        }
    }

    /// Cues tile the scene exactly: no gap, no overlap, ending on the measured
    /// narration duration rather than near it.
    #[test]
    fn cues_tile_the_scene_without_gap_or_overlap() {
        let text = "One. Two three four five six seven. Eight nine ten eleven twelve.";
        let c = cues(text, 12.0, 24);
        assert!(c.len() > 1, "this text should split: {c:?}");
        assert!((c[0].start).abs() < 1e-9, "the first cue starts at zero");
        for pair in c.windows(2) {
            assert!(
                (pair[0].end - pair[1].start).abs() < 1e-9,
                "gap or overlap between {:?} and {:?}",
                pair[0],
                pair[1]
            );
        }
        assert!(
            (c.last().expect("at least one cue").end - 12.0).abs() < 1e-9,
            "the last cue ends on the narration, not near it"
        );
    }

    /// Nothing flickers: a short scene gets few cues, not many tiny ones.
    #[test]
    fn a_short_scene_never_produces_a_flicker() {
        let text = "One two three four five six seven eight nine ten eleven twelve thirteen.";
        let c = cues(text, 2.0, 12);
        assert!(
            c.len() <= 2,
            "2s cannot hold more than two readable cues: {c:?}"
        );
        for cue in &c {
            assert!(
                cue.duration() >= MIN_CUE_SECONDS - 1e-9,
                "{cue:?} is a flicker"
            );
        }
    }

    /// D-095's hour-long single scene, which is the only input that reaches
    /// MAX_CUES. It must still caption every word, with a bounded number of
    /// FFmpeg inputs.
    #[test]
    fn an_absurdly_long_line_stays_within_the_input_budget() {
        let text = "word ".repeat(12_000);
        let c = cues(&text, 3_600.0, 80);
        assert!(c.len() <= MAX_CUES, "{} cues is too many inputs", c.len());
        assert!(!c.is_empty());
        let words: usize = c.iter().map(|x| x.text.split(' ').count()).sum();
        assert_eq!(words, 12_000, "every word still reaches the screen");
    }

    /// A break after a full stop reads as a thought; a break three words later
    /// reads as a fault. Cheap to prefer, so it is preferred.
    #[test]
    fn a_break_is_preferred_where_a_sentence_ends() {
        let c = cues(
            "The boat came in. The harbour was already full of them.",
            10.0,
            34,
        );
        assert!(c.len() >= 2, "{c:?}");
        assert_eq!(c[0].text, "The boat came in.");
    }

    /// A cue that ends on "and" hands the viewer half a phrase across a cut.
    /// Taken from real narration in the author's own project.
    #[test]
    fn a_cue_never_ends_on_a_dangling_function_word() {
        let text = "Those born with poor aptitude could pour twenty-four hours a day \
                    into training and still fall short of what someone else achieved \
                    in a single hour.";
        let c = cues(text, 9.0, 84);
        assert!(c.len() > 1, "this should split: {c:?}");
        for cue in &c[..c.len() - 1] {
            let last = cue.text.rsplit(' ').next().expect("a word");
            let bare = last
                .trim_matches(|ch: char| !ch.is_alphanumeric())
                .to_ascii_lowercase();
            assert!(
                !WEAK_ENDINGS.contains(&bare.as_str()) || last.ends_with(','),
                "cue ends on {last:?}: {cue:?}"
            );
        }
    }

    /// The carry must never empty a cue or lose a word — checked across every
    /// budget, because the piece that gives a word away can be short already.
    #[test]
    fn carrying_a_weak_word_never_empties_a_cue() {
        let text = "It was a long day and a longer night, and the of the to the \
                    in the on the at the for the with the from the by the as the";
        for budget in [12, 16, 24, 40, 80] {
            let c = cues(text, 60.0, budget);
            for cue in &c {
                assert!(!cue.text.trim().is_empty(), "empty cue at budget {budget}");
            }
            let joined = c
                .iter()
                .map(|x| x.text.as_str())
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(joined, normalize(text), "words lost at budget {budget}");
        }
    }

    /// One word longer than the whole budget has nowhere to break, and must
    /// still survive.
    #[test]
    fn an_unbreakable_word_survives() {
        let c = cues(
            "Llanfairpwllgwyngyllgogerychwyrndrobwllllantysiliogogogoch",
            5.0,
            10,
        );
        assert_eq!(c.len(), 1);
        assert!(c[0].text.starts_with("Llanfair"));
    }

    /// The cache key must move when the look moves and stand still otherwise
    /// (D-043).
    #[test]
    fn the_key_fields_follow_everything_that_changes_the_pixels() {
        let base = SubtitleSpec {
            theme: SubtitleTheme::Classic,
            placement: Placement::Bottom,
            cues: cues("The tide came in before dawn.", 6.0, 80),
        };
        let same = SubtitleSpec {
            cues: cues("The tide came in before dawn.", 6.0, 80),
            ..base.clone()
        };
        assert_eq!(base.key_fields(), same.key_fields());

        for changed in [
            SubtitleSpec {
                theme: SubtitleTheme::Punch,
                ..base.clone()
            },
            SubtitleSpec {
                placement: Placement::Top,
                ..base.clone()
            },
            SubtitleSpec {
                cues: cues("The tide came in before noon.", 6.0, 80),
                ..base.clone()
            },
            SubtitleSpec {
                cues: cues("The tide came in before dawn.", 7.0, 80),
                ..base.clone()
            },
        ] {
            assert_ne!(
                base.key_fields(),
                changed.key_fields(),
                "a change that moves the pixels must move the key"
            );
        }
    }

    #[test]
    fn an_empty_spec_knows_it_draws_nothing() {
        let spec = SubtitleSpec {
            theme: SubtitleTheme::Classic,
            placement: Placement::Bottom,
            cues: Vec::new(),
        };
        assert!(spec.is_empty());
    }

    #[test]
    fn a_placement_round_trips_and_refuses_nonsense() {
        for placement in Placement::ALL {
            assert_eq!(Placement::parse(placement.as_str()), Some(placement));
        }
        assert_eq!(Placement::parse("middle"), None);
    }

    #[test]
    fn a_colour_prints_as_the_hex_a_key_can_hash() {
        assert_eq!(Rgba::new(255, 214, 10, 255).to_string(), "#ffd60aff");
        assert!(!Rgba::NONE.is_visible());
    }
}
