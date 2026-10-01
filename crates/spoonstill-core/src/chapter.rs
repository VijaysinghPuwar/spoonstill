//! Cutting a chapter into scenes, one picture every few seconds (D-193).
//!
//! The operator writes a whole chapter, then makes one picture for every
//! sentence — or for part of a sentence when it runs long — so the film changes
//! picture every three to five seconds. Doing that split by hand, a line at a
//! time into `001.txt`, `002.txt`, … was most of the time a chapter took.
//!
//! This module is the split, and nothing else: text in, pieces out, no files.
//! Where each piece goes is `spoonstill_app::chapter`'s business.
//!
//! ## How a place to cut is chosen
//!
//! Every gap between two words is a candidate, and each has a cost:
//!
//! | the gap | cost |
//! |---|---|
//! | after a full stop, `?`, `!` or `…` | 0 |
//! | after `;` `:` or a dash | 1 |
//! | after `,` | 2 |
//! | before `and`, `but`, `then`, `when`, … | 3 |
//! | anywhere else | 25 |
//!
//! A comma costs more than a dash because a comma is also how a list is
//! written, and "a deep," / "steady gold" is a cut inside a phrase — found in
//! the second chapter this was tried on. And one more cost: **keeping
//! narration and quoted speech in one scene** costs 4, so "His voice was flat
//! and bored." and "\"It will glow.\"" prefer a picture each.
//! | after `the`, `of`, `and`, … — a dangling word | +20 |
//!
//! **The story decides, and the seconds are a guide** — the author's words:
//! *"the cuts should make sense in the story or narration"*. So a mid-phrase
//! cut costs more than a sentence running two seconds long, and a sentence
//! with no pause in it is kept whole well past the maximum.
//!
//! And one shape is refused outright, because it reads as a mistake: **the end
//! of one sentence joined to the start of the next.** A piece may be several
//! whole sentences ("Chu Kingdom. Haonan Province."), or part of one sentence
//! ("In a log cabin in the outer sect,"), but never "playing a game. Then his
//! vision had gone black," — found in the first run over the author's own
//! chapter, and the reason this rule exists.
//!
//! A piece also costs something for every second it falls outside the pacing,
//! so between two natural places to cut, the one nearer three-to-five seconds
//! wins.
//!
//! The cheapest set of cuts over the whole paragraph wins. It is a shortest
//! path over word positions, so a paragraph of `n` words costs `O(n·w)` where
//! `w` is the most words a piece can hold — microseconds for a chapter.
//!
//! **A paragraph break is always a cut.** A blank line is the author saying
//! "new thought", and no piece carries words across it.
//!
//! ## What the seconds are
//!
//! An **estimate** from the character count. Measured on the author's own ten
//! lines in their own voice on 2026-09-30: 15.1 characters a second, spread
//! 12.8–17.0 — a full stop is a pause, so clipped prose reads slower than
//! flowing prose (see `edge.rs`'s `SPEECH_CHARS_PER_SECOND`, which is the fast
//! end on purpose, for a different job). The real length of a scene is only
//! known once its narration is spoken and measured (D-021), and every surface
//! that shows these numbers says so.

/// Characters per second of speech, for estimating how long a piece will take
/// to say.
///
/// Measured, not chosen: ten real lines, `en-US-AndrewMultilingualNeural`,
/// 965 characters over 63.9 s of audio. Not the renderer's
/// `SPEECH_CHARS_PER_SECOND` (17.3), which is the *fastest* observed rate and
/// is used to refuse a line no scene could hold — a ceiling, where this is a
/// typical value.
pub const CHARS_PER_SECOND: f64 = 15.0;

/// How long each piece should take to say.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pacing {
    /// A piece shorter than this is merged with a neighbour when that is cheap.
    pub min_seconds: f64,
    /// A piece longer than this is cut, at the most natural place available.
    pub max_seconds: f64,
}

impl Pacing {
    /// The author's own request: a new picture every three to five seconds.
    pub const DEFAULT: Pacing = Pacing {
        min_seconds: 3.0,
        max_seconds: 5.0,
    };

    /// A pacing, with the two ends put in order and held to something
    /// meaningful. A window slider or a command-line flag can hand over
    /// anything, and a minimum above the maximum is a request with one obvious
    /// meaning rather than an error.
    #[must_use]
    pub fn new(min_seconds: f64, max_seconds: f64) -> Pacing {
        let clean = |s: f64| {
            if s.is_finite() {
                s.clamp(0.5, 60.0)
            } else {
                4.0
            }
        };
        let (a, b) = (clean(min_seconds), clean(max_seconds));
        Pacing {
            min_seconds: a.min(b),
            max_seconds: a.max(b),
        }
    }
}

impl Default for Pacing {
    fn default() -> Self {
        Pacing::DEFAULT
    }
}

/// One piece of the chapter: the words one picture is shown under.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// The words, with whitespace collapsed.
    pub text: String,
    /// How long it will probably take to say. An estimate (see the module
    /// note).
    pub seconds: f64,
}

/// The estimated time to say `text`.
#[must_use]
pub fn estimate(text: &str) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let chars = normalize(text).chars().count() as f64;
    chars / CHARS_PER_SECOND
}

/// Cut a chapter into pieces of about `pacing`'s length.
///
/// Blank text gives no pieces. Every word of the input appears in exactly one
/// piece, in order — nothing is dropped, reordered or rewritten except
/// whitespace, which is collapsed.
#[must_use]
pub fn cut(chapter: &str, pacing: Pacing) -> Vec<Cut> {
    let mut out = Vec::new();
    for paragraph in paragraphs(chapter) {
        let words: Vec<&str> = paragraph.split_whitespace().collect();
        for piece in cut_paragraph(&words, pacing) {
            let seconds = estimate(&piece);
            out.push(Cut {
                text: piece,
                seconds,
            });
        }
    }
    out
}

/// A chapter's paragraphs: runs of lines separated by at least one blank line.
///
/// A single line break is *not* a paragraph — text pasted out of a document or
/// a PDF arrives hard-wrapped at whatever width it was displayed at, and
/// cutting there would put a picture change wherever the author's word
/// processor happened to wrap.
fn paragraphs(chapter: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for line in chapter.lines() {
        if line.trim().is_empty() {
            if !current.trim().is_empty() {
                out.push(std::mem::take(&mut current));
            }
            current.clear();
        } else {
            current.push(' ');
            current.push_str(line);
        }
    }
    if !current.trim().is_empty() {
        out.push(current);
    }
    out
}

/// The cost of a cut placed immediately after `words[i]`.
fn gap_cost(words: &[&str], i: usize) -> f64 {
    let word = words[i];
    let mut cost = if ends_sentence(word) {
        0.0
    } else if ends_clause(word) {
        if without_closers(word).ends_with(',') {
            2.0
        } else {
            1.0
        }
    } else if words.get(i + 1).is_some_and(|next| opens_clause(next)) {
        3.0
    } else {
        25.0
    };
    if dangles(word) {
        cost += 20.0;
    }
    cost
}

/// The cost of a piece whose words are `chars` characters long.
fn length_cost(chars: usize, pacing: Pacing) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let seconds = chars as f64 / CHARS_PER_SECOND;
    if seconds < pacing.min_seconds {
        2.0 * (pacing.min_seconds - seconds)
    } else if seconds > pacing.max_seconds {
        // Gently for the first two and a half seconds over — a whole sentence
        // is worth that — and steeply after, so text with no punctuation at
        // all still changes picture rather than holding one for fifteen
        // seconds.
        let over = seconds - pacing.max_seconds;
        3.0 * over + 17.0 * (over - 2.5).max(0.0)
    } else {
        0.0
    }
}

/// What a piece of `words[a..b]` costs for straddling a sentence boundary: a
/// full stop inside it is fine only when the piece is whole sentences — it
/// starts where a sentence starts and ends where one ends (see the module
/// note).
fn straddle_cost(words: &[&str], a: usize, b: usize) -> f64 {
    let inside = (a..b.saturating_sub(1)).any(|i| ends_sentence(words[i]));
    if !inside {
        return 0.0;
    }
    let starts_whole = a == 0 || ends_sentence(words[a - 1]);
    let ends_whole = b == words.len() || ends_sentence(words[b - 1]);
    if !(starts_whole && ends_whole) {
        return 1000.0;
    }
    // Whole sentences may share a picture — but narration and a character's
    // speech prefer their own.
    let mut cost = 0.0;
    for i in a..b - 1 {
        if ends_sentence(words[i]) && (opens_quote(words[i + 1]) || closes_quote(words[i])) {
            cost += 4.0;
        }
    }
    cost
}

fn opens_quote(word: &str) -> bool {
    word.starts_with(['"', '“', '«', '‘'])
}

fn closes_quote(word: &str) -> bool {
    word.ends_with(['"', '”', '»', '’'])
}

/// The cheapest way to cut one paragraph (see the module note).
fn cut_paragraph(words: &[&str], pacing: Pacing) -> Vec<String> {
    let n = words.len();
    if n == 0 {
        return Vec::new();
    }
    // Characters in words[a..b] joined by single spaces, from a running total.
    let mut prefix = vec![0usize; n + 1];
    for (i, word) in words.iter().enumerate() {
        prefix[i + 1] = prefix[i] + word.chars().count();
    }
    let span = |a: usize, b: usize| prefix[b] - prefix[a] + (b - a).saturating_sub(1);

    // A piece far past the maximum is never the cheapest answer while a cut
    // exists, so the search does not look further than this many characters.
    // A single word longer than it is still a piece of its own.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let reach = (pacing.max_seconds * CHARS_PER_SECOND * 3.0).ceil() as usize + 1;

    // best[b] = cheapest cost of cutting words[..b]; from[b] = where the last
    // piece of that answer starts.
    let mut best = vec![f64::INFINITY; n + 1];
    let mut from = vec![0usize; n + 1];
    best[0] = 0.0;
    for b in 1..=n {
        let mut a = b;
        while a > 0 {
            a -= 1;
            let chars = span(a, b);
            if chars > reach && a + 1 < b {
                break;
            }
            if !best[a].is_finite() {
                continue;
            }
            // The paragraph's end is a free cut; every other end pays for
            // where it falls.
            let gap = if b == n { 0.0 } else { gap_cost(words, b - 1) };
            let cost = best[a] + length_cost(chars, pacing) + gap + straddle_cost(words, a, b);
            // `<` rather than `<=`: between equal answers the longer last
            // piece wins, which means fewer pictures to make.
            if cost < best[b] {
                best[b] = cost;
                from[b] = a;
            }
        }
    }

    let mut bounds = Vec::new();
    let mut b = n;
    while b > 0 {
        bounds.push((from[b], b));
        b = from[b];
    }
    bounds.reverse();
    bounds
        .into_iter()
        .map(|(a, b)| words[a..b].join(" "))
        .collect()
}

/// Collapse every run of whitespace to one space and trim the ends.
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Strip the closing quotes and brackets that so often follow a stop.
fn without_closers(word: &str) -> &str {
    word.trim_end_matches(['"', '\'', ')', ']', '»', '”', '’', '*', '_'])
}

fn ends_sentence(word: &str) -> bool {
    let bare = without_closers(word);
    if !bare.ends_with(['.', '!', '?', '…', '।', '॥', '。', '！', '？']) {
        return false;
    }
    // "Mr. Chen" is not two sentences, and a cut after "Mr." would be the
    // cheapest cut in the paragraph. A title, or a single capital initial
    // ("J. R."), ends nothing.
    if let Some(stem) = bare.strip_suffix('.') {
        let stem = stem.trim_start_matches(|c: char| !c.is_alphanumeric());
        if ABBREVIATIONS.contains(&stem.to_lowercase().as_str()) {
            return false;
        }
        let mut chars = stem.chars();
        if let (Some(only), None) = (chars.next(), chars.next())
            && only.is_uppercase()
        {
            return false;
        }
    }
    true
}

/// Words written with a full stop that do not end a sentence.
///
/// Not `no`: "The answer was no." ends a sentence far more often than "No. 7"
/// begins a reference in a novel.
const ABBREVIATIONS: [&str; 13] = [
    "mr", "mrs", "ms", "dr", "st", "jr", "sr", "prof", "vs", "etc", "e.g", "i.e", "mt",
];

fn ends_clause(word: &str) -> bool {
    let bare = without_closers(word);
    bare.ends_with([',', ';', ':', '—', '–']) || bare == "-" || bare.ends_with("--")
}

/// Words a new clause opens with, so a cut *before* them reads naturally.
const OPENERS: [&str; 16] = [
    "and", "but", "or", "so", "then", "when", "while", "as", "because", "though", "although",
    "until", "before", "after", "where", "which",
];

fn opens_clause(word: &str) -> bool {
    let bare: String = word
        .trim_start_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase();
    OPENERS.contains(&bare.as_str())
}

/// Function words that must not be the last thing before a picture changes —
/// the subtitle rule (`captions::WEAK_ENDINGS`), for the same reason.
const DANGLING: [&str; 30] = [
    "a", "an", "the", "and", "or", "but", "nor", "of", "to", "in", "on", "at", "for", "with",
    "from", "by", "as", "into", "onto", "over", "under", "than", "that", "which", "who", "if",
    "his", "her", "their", "my",
];

fn dangles(word: &str) -> bool {
    // A word carrying punctuation is ending something, not dangling.
    if word.ends_with(|c: char| !c.is_alphanumeric()) {
        return false;
    }
    DANGLING.contains(&word.to_lowercase().as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(cuts: &[Cut]) -> Vec<&str> {
        cuts.iter().map(|c| c.text.as_str()).collect()
    }

    /// The author's own opening, which is the reason this exists.
    const OPENING: &str = "Chu Kingdom. Haonan Province. The Floating Cloud Sect.\n\n\
In a log cabin in the outer sect, Adrian Vale sat on the edge of a hard bed, \
wearing an expression of utter disbelief. Not long ago he had been in his own \
bedroom, playing a game. Then his vision had gone black, and when he opened his \
eyes again, he had crossed over.";

    #[test]
    fn every_word_survives_in_order() {
        let cuts = cut(OPENING, Pacing::DEFAULT);
        let joined = texts(&cuts).join(" ");
        assert_eq!(joined, normalize(OPENING));
    }

    #[test]
    fn a_paragraph_break_is_always_a_cut() {
        let cuts = cut(OPENING, Pacing::DEFAULT);
        assert!(
            cuts.iter()
                .any(|c| c.text.ends_with("The Floating Cloud Sect.")),
            "{:#?}",
            texts(&cuts)
        );
        assert!(
            !cuts.iter().any(|c| c.text.contains("Sect. In a log cabin")),
            "a piece crossed a blank line: {:#?}",
            texts(&cuts)
        );
    }

    #[test]
    fn pieces_land_near_the_pacing() {
        let cuts = cut(OPENING, Pacing::DEFAULT);
        for c in &cuts {
            assert!(
                c.seconds <= 6.0 && c.seconds >= 2.0,
                "{:.1}s: {:?}\nall: {:#?}",
                c.seconds,
                c.text,
                texts(&cuts)
            );
        }
    }

    #[test]
    fn a_long_sentence_is_cut_at_a_comma_rather_than_mid_phrase() {
        let text = "In a log cabin in the outer sect, Adrian Vale sat on the edge \
                    of a hard bed, wearing an expression of utter disbelief.";
        let cuts = cut(text, Pacing::DEFAULT);
        assert!(cuts.len() >= 2, "{:#?}", texts(&cuts));
        for c in &cuts[..cuts.len() - 1] {
            assert!(
                c.text.ends_with(','),
                "cut away from a comma: {:#?}",
                texts(&cuts)
            );
        }
    }

    /// The author's own opening, cut by the first version of this module into
    /// "playing a game. Then his vision had gone black," — half of one sentence
    /// and half of the next.
    #[test]
    fn no_piece_joins_the_end_of_one_sentence_to_the_start_of_the_next() {
        let text = "Not long ago he had been in his own bedroom, playing a game. Then his \
                    vision had gone black, and when he opened his eyes again, he had crossed \
                    over. His cultivation was thoroughly unremarkable — the fourth stage of \
                    Body Refinement. Even among the outer sect disciples, that put him at the \
                    very bottom of the pile.";
        for c in cut(text, Pacing::DEFAULT) {
            let words: Vec<&str> = c.text.split(' ').collect();
            let inside = words[..words.len() - 1].iter().any(|w| ends_sentence(w));
            if inside {
                assert!(
                    ends_sentence(words[words.len() - 1]),
                    "a sentence end inside a piece that ends mid-sentence: {:?}",
                    c.text
                );
            }
        }
    }

    #[test]
    fn a_sentence_with_no_natural_pause_is_kept_whole_past_the_maximum() {
        let text = "Into the body of an outer sect disciple of the Floating Cloud Sect \
                    he had crossed.";
        assert_eq!(cut(text, Pacing::DEFAULT).len(), 1);
    }

    /// Three cuts from the second chapter this was tried on, each inside a
    /// phrase or across narration and speech.
    #[test]
    fn a_list_an_adjective_pair_and_a_quote_are_not_cut_through() {
        let cuts = cut(
            "Then the smell of pine smoke and damp straw reminded him, and the memory \
             came back all at once — the game, the black screen, the body that was not his.",
            Pacing::DEFAULT,
        );
        assert!(
            !cuts.iter().any(|c| c.text.ends_with("the game,")),
            "{:#?}",
            texts(&cuts)
        );

        let cuts = cut(
            "A murmur went through the crowd when a tall girl from the northern valleys \
             made the stone flash a deep, steady gold; she walked back to her place \
             without smiling, as if she had expected nothing less.",
            Pacing::DEFAULT,
        );
        assert!(
            !cuts.iter().any(|c| c.text.ends_with("a deep,")),
            "{:#?}",
            texts(&cuts)
        );

        let cuts = cut(
            "\"Place your hand on the Aptitude Stone,\" Elder Chen said. His voice was flat \
             and bored. \"It will glow. White is poor. Blue is ordinary.\"",
            Pacing::DEFAULT,
        );
        assert!(
            !cuts.iter().any(|c| c.text.contains("bored. \"It")),
            "{:#?}",
            texts(&cuts)
        );
    }

    #[test]
    fn a_title_is_not_the_end_of_a_sentence() {
        let text = "Elder Mr. Chen of the Azure Peak looked down at the boy, and Dr. Lin \
                    and J. R. Vale said nothing at all for a very long time.";
        for c in cut(text, Pacing::DEFAULT) {
            assert!(
                !c.text.ends_with("Mr.") && !c.text.ends_with("Dr.") && !c.text.ends_with("J."),
                "{:?}",
                c.text
            );
        }
    }

    #[test]
    fn short_sentences_are_joined_rather_than_flashed() {
        let cuts = cut(
            "He ran. She followed. The door slammed. Nobody spoke.",
            Pacing::DEFAULT,
        );
        assert!(cuts.len() <= 2, "{:#?}", texts(&cuts));
    }

    #[test]
    fn no_piece_ends_on_a_dangling_word() {
        let text = "Those born with poor aptitude could pour twenty-four hours a day into \
                    training and still fall short of those who were blessed with a natural \
                    talent for the way of the sword and the path of the immortal.";
        for c in cut(text, Pacing::DEFAULT) {
            let last = c.text.rsplit(' ').next().unwrap_or_default();
            assert!(!dangles(last), "ends on {last:?}: {:?}", c.text);
        }
    }

    #[test]
    fn blank_text_is_no_pieces() {
        assert!(cut("", Pacing::DEFAULT).is_empty());
        assert!(cut("  \n\n \t\n", Pacing::DEFAULT).is_empty());
    }

    #[test]
    fn a_hard_wrapped_paragraph_is_one_paragraph() {
        let wrapped = "He ran for the door\nand did not look back\nuntil the sect was gone.";
        let flat = "He ran for the door and did not look back until the sect was gone.";
        assert_eq!(
            texts(&cut(wrapped, Pacing::DEFAULT)),
            texts(&cut(flat, Pacing::DEFAULT))
        );
    }

    #[test]
    fn a_word_longer_than_any_piece_is_kept_whole() {
        let giant = "x".repeat(500);
        let cuts = cut(&format!("Before. {giant} after."), Pacing::DEFAULT);
        assert!(cuts.iter().any(|c| c.text.contains(&giant)));
        assert_eq!(texts(&cuts).join(" "), format!("Before. {giant} after."));
    }

    #[test]
    fn faster_pacing_gives_more_pieces() {
        let fast = cut(OPENING, Pacing::new(1.5, 2.5)).len();
        let slow = cut(OPENING, Pacing::new(6.0, 10.0)).len();
        assert!(fast > slow, "fast {fast}, slow {slow}");
    }

    #[test]
    fn a_backwards_pacing_is_put_in_order() {
        let p = Pacing::new(5.0, 3.0);
        assert_eq!((p.min_seconds, p.max_seconds), (3.0, 5.0));
        let p = Pacing::new(f64::NAN, 0.0);
        assert!(p.min_seconds <= p.max_seconds && p.min_seconds >= 0.5);
    }

    #[test]
    fn a_chapter_without_punctuation_still_cuts_and_keeps_every_word() {
        let words: Vec<String> = (0..2000).map(|i| format!("word{i}")).collect();
        let text = words.join(" ");
        let cuts = cut(&text, Pacing::DEFAULT);
        assert_eq!(texts(&cuts).join(" "), text);
        for c in &cuts {
            assert!(c.seconds < 9.0, "{:.1}s", c.seconds);
        }
    }

    #[test]
    fn non_latin_text_cuts_at_its_own_full_stops() {
        // Devanagari ends a sentence with the danda (D-157, D-158 made Hindi a
        // language this renders), so a cut lands after one.
        let text = "यह एक बहुत लंबा वाक्य है जो कई शब्दों से बना है। \
                    यह दूसरा वाक्य भी काफी लंबा है और इसमें बहुत शब्द हैं।";
        let cuts = cut(text, Pacing::DEFAULT);
        assert_eq!(texts(&cuts).join(" "), normalize(text));
        assert!(cuts[0].text.ends_with('।'), "{:#?}", texts(&cuts));
    }
}

/// Random chapters, thousands of them, against the properties that must hold
/// for every input — not examples, which only prove the cases someone thought
/// of (D-116).
#[cfg(test)]
mod stress {
    use super::*;

    /// A small deterministic generator, so a failure names its seed and
    /// reproduces exactly.
    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
            #[allow(clippy::cast_possible_truncation)]
            let at = (self.next() % from.len() as u64) as usize;
            from[at]
        }
    }

    const WORDS: [&str; 40] = [
        "the",
        "sect",
        "Adrian",
        "Vale",
        "and",
        "cultivation",
        "of",
        "a",
        "when",
        "he",
        "opened",
        "his",
        "eyes",
        "Mr.",
        "Dr.",
        "J.",
        "twenty-four",
        "hours",
        "—",
        "–",
        "…",
        "\"Get",
        "up,\"",
        "(quietly)",
        "'yes'",
        "नमस्ते",
        "दुनिया।",
        "你好。",
        "🙂",
        "3.5",
        "x",
        "which",
        "but",
        "into",
        "Chu",
        "Kingdom.",
        "Province,",
        "strength;",
        "said:",
        "why?",
    ];
    const ENDS: [&str; 8] = ["", "", "", ".", ",", "!", "?", "…"];
    const GAPS: [&str; 8] = [" ", " ", " ", "  ", "\n", "\r\n", "\n\n", "\t"];

    fn chapter(rng: &mut Rng) -> String {
        let mut text = String::new();
        let words = 1 + rng.next() % 400;
        for _ in 0..words {
            text.push_str(rng.pick(&WORDS));
            text.push_str(rng.pick(&ENDS));
            text.push_str(rng.pick(&GAPS));
        }
        if rng.next().is_multiple_of(20) {
            text.push_str(&"y".repeat(300));
        }
        text
    }

    #[test]
    fn every_random_chapter_keeps_every_word_and_every_rule() {
        let mut rng = Rng(0x5eed_cafe_f00d_1234);
        for round in 0..1500_u32 {
            let text = chapter(&mut rng);
            let pacing = if round.is_multiple_of(3) {
                Pacing::new(1.0, 2.0)
            } else {
                Pacing::DEFAULT
            };
            let cuts = cut(&text, pacing);

            // Nothing lost, nothing added, nothing reordered.
            let joined: Vec<String> = cuts.iter().map(|c| c.text.clone()).collect();
            assert_eq!(
                joined.join(" "),
                normalize(&text),
                "round {round}: {text:?}"
            );

            for c in &cuts {
                assert!(!c.text.is_empty(), "round {round}: an empty piece");
                assert!(c.seconds.is_finite() && c.seconds > 0.0, "round {round}");
            }

            // Per paragraph, so "the piece ends where its paragraph ends" is
            // known exactly: a piece holding a full stop inside it must start
            // at a sentence start and end at a sentence end or the paragraph's.
            for paragraph in paragraphs(&text) {
                let words: Vec<&str> = paragraph.split_whitespace().collect();
                let pieces = cut_paragraph(&words, pacing);
                let mut at = 0;
                for piece in &pieces {
                    let n = piece.split(' ').count();
                    let (a, b) = (at, at + n);
                    assert!(
                        straddle_cost(&words, a, b) < 1000.0,
                        "round {round}: straddles: {piece:?}"
                    );
                    at = b;
                }
                assert_eq!(at, words.len(), "round {round}");
            }
        }
    }

    #[test]
    fn a_long_chapter_cuts_in_well_under_a_second() {
        let mut rng = Rng(42);
        let mut text = String::new();
        while text.len() < 1_000_000 {
            text.push_str(&chapter(&mut rng));
            text.push_str("\n\n");
        }
        let started = std::time::Instant::now();
        let cuts = cut(&text, Pacing::DEFAULT);
        let took = started.elapsed();
        assert!(!cuts.is_empty());
        // Generous, because a debug build on a shared machine is slow; the
        // point is linear, not quadratic — a quadratic cut of 2 MB takes
        // minutes.
        assert!(took.as_secs() < 20, "{took:?} for {} bytes", text.len());
    }
}
