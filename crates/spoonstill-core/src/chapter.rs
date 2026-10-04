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
//! **A paragraph break is the most natural cut there is**, and costs nothing.
//! It is crossed only when a paragraph is too short to hold a picture alone —
//! in a novel that is a line of dialogue, `"Let's do it."`, which held a
//! picture for 0.77 s in the author's chapter — and then only by whole
//! sentences, at a cost, so two paragraphs share a picture only to avoid a
//! flash (D-196). It used to be an absolute cut, and the author's 431-scene
//! chapter had twenty scenes under two seconds because of it.
//!
//! A paragraph with no letters or digits — `√`, `***`, `---` — is a mark
//! between sections and is left out rather than spoken (D-196).
//!
//! ## What the seconds are
//!
//! An **estimate** from the character count: 16.3 characters a second,
//! measured on 2026-10-03 over the author's 431 lines in their own voice —
//! 30 391 characters, 1 868 s of spoken scene (D-196). The first measurement,
//! ten lines on 2026-09-30, said 15.1; at that rate every estimate ran 8% long
//! and the cut aimed short. Clipped prose reads slower than flowing prose, so
//! this is a typical value, not the renderer's `SPEECH_CHARS_PER_SECOND`
//! (`edge.rs`), which is the fast end on purpose, for a different job. The real
//! length of a scene is only known once its narration is spoken and measured
//! (D-021), and every surface that shows these numbers says so.

/// Characters per second of speech, for estimating how long a piece will take
/// to say.
///
/// Measured, not chosen: the author's 431 real lines,
/// `en-US-AndrewMultilingualNeural`, 30 391 characters over 1 868 s (D-196).
/// Not the renderer's
/// `SPEECH_CHARS_PER_SECOND` (17.3), which is the *fastest* observed rate and
/// is used to refuse a line no scene could hold — a ceiling, where this is a
/// typical value.
pub const CHARS_PER_SECOND: f64 = 16.3;

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

/// How a chapter is cut: by length, or by a count of sentences (D-194).
///
/// The author's words: *"select the length of auto cut — one sentence, two
/// sentences — so the user has flexibility"*. Time is the default because it
/// is what was asked for first; a count of sentences is simpler to predict
/// and is what an operator who writes one picture per sentence wants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CutBy {
    /// About this long each, cut where the story allows.
    Time(Pacing),
    /// This many whole sentences each, never part of one.
    Sentences(usize),
}

impl CutBy {
    /// The names a setting or a flag uses: `time`, or `1`, `2`, `3` — a number
    /// of sentences. Anything else is `None`, and a caller that must answer
    /// uses the default rather than guessing.
    #[must_use]
    pub fn parse(name: &str, pacing: Pacing) -> Option<CutBy> {
        let name = name.trim().to_ascii_lowercase();
        if name == "time" || name.is_empty() {
            return Some(CutBy::Time(pacing));
        }
        let count = name
            .trim_end_matches("sentences")
            .trim_end_matches("sentence")
            .trim()
            .parse::<usize>()
            .ok()?;
        (1..=10).contains(&count).then_some(CutBy::Sentences(count))
    }

    /// The name [`CutBy::parse`] reads back.
    #[must_use]
    pub fn name(&self) -> String {
        match self {
            CutBy::Time(_) => "time".to_owned(),
            CutBy::Sentences(n) => n.to_string(),
        }
    }
}

/// Cut a chapter the way `by` says.
#[must_use]
pub fn cut_by(chapter: &str, by: CutBy) -> Vec<Cut> {
    match by {
        CutBy::Time(pacing) => cut(chapter, pacing),
        CutBy::Sentences(count) => cut_sentences(chapter, count),
    }
}

/// Cut a chapter into pieces of `count` whole sentences each (D-194).
///
/// A blank line still always ends a piece, so the last piece of a paragraph
/// may hold fewer. A sentence is never split, however long it runs — that is
/// the promise this mode makes, and the reason to choose it over time.
#[must_use]
pub fn cut_sentences(chapter: &str, count: usize) -> Vec<Cut> {
    let count = count.max(1);
    let mut out = Vec::new();
    for paragraph in paragraphs(chapter) {
        let mut piece: Vec<&str> = Vec::new();
        let mut sentences = 0;
        for word in paragraph.split_whitespace() {
            piece.push(word);
            if ends_sentence(word) {
                sentences += 1;
                if sentences == count {
                    let text = piece.join(" ");
                    out.push(Cut {
                        seconds: estimate(&text),
                        text,
                    });
                    piece.clear();
                    sentences = 0;
                }
            }
        }
        if !piece.is_empty() {
            let text = piece.join(" ");
            out.push(Cut {
                seconds: estimate(&text),
                text,
            });
        }
    }
    out
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
/// piece, in order — nothing is reordered or rewritten except whitespace,
/// which is collapsed, and nothing is dropped except a paragraph with no
/// letters or digits in it, which is a section mark (D-196).
///
/// A blank line is the most natural place to cut, and is crossed only when a
/// paragraph is too short to hold a picture alone — one line of dialogue —
/// and then only by whole sentences (D-196).
#[must_use]
pub fn cut(chapter: &str, pacing: Pacing) -> Vec<Cut> {
    let paragraphs = paragraphs(chapter);
    let (words, breaks) = split_chapter(&paragraphs);
    cut_words(&words, &breaks, pacing)
        .into_iter()
        .map(|piece| Cut {
            seconds: estimate(&piece),
            text: piece,
        })
        .collect()
}

/// Every word of the chapter, and for each whether it ends a paragraph.
fn split_chapter(paragraphs: &[String]) -> (Vec<&str>, Vec<bool>) {
    let mut words: Vec<&str> = Vec::new();
    let mut breaks: Vec<bool> = Vec::new();
    for paragraph in paragraphs {
        let start = words.len();
        words.extend(paragraph.split_whitespace());
        if words.len() > start {
            breaks.resize(words.len(), false);
            if let Some(last) = breaks.last_mut() {
                *last = true;
            }
        }
    }
    (words, breaks)
}

/// A chapter's paragraphs: runs of lines separated by at least one blank line.
///
/// A single line break is *not* a paragraph — text pasted out of a document or
/// a PDF arrives hard-wrapped at whatever width it was displayed at, and
/// cutting there would put a picture change wherever the author's word
/// processor happened to wrap.
///
/// A paragraph with nothing in it to say — `√`, `***`, `---`, a row of dots —
/// is a mark the author put between sections, not a line, and is left out
/// (D-196). Spoken, `√` was a second of the voice reading a symbol under a
/// picture of its own.
fn paragraphs(chapter: &str) -> Vec<String> {
    let mut out = raw_paragraphs(chapter);
    out.retain(|p| p.chars().any(char::is_alphanumeric));
    out
}

fn raw_paragraphs(chapter: &str) -> Vec<String> {
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

/// The cost of a cut placed immediately after `words[i]`. The end of a
/// paragraph is the most natural place there is.
fn gap_cost(words: &[&str], breaks: &[bool], i: usize) -> f64 {
    let word = words[i];
    if breaks[i] {
        return 0.0;
    }
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
        // And steeply below two thirds of the minimum: a picture on screen
        // for under two seconds is a flash, and the author's 431-scene chapter
        // had twenty of them, every one a short line of dialogue (D-196).
        let flash = pacing.min_seconds * 2.0 / 3.0;
        2.0 * (pacing.min_seconds - seconds) + 4.0 * (flash - seconds).max(0.0)
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
///
/// The end of a paragraph counts as the end of a sentence here, whatever its
/// punctuation: a heading with no full stop is still whole. Crossing one costs
/// [`PARAGRAPH_JOIN`], so two paragraphs share a picture only when one of them
/// is too short to hold one alone (D-196).
#[cfg(test)]
fn straddle_cost(words: &[&str], breaks: &[bool], a: usize, b: usize) -> f64 {
    Straddle::new(words, breaks).cost(a, b)
}

/// [`straddle_cost`] in constant time per piece, from running totals — the
/// search asks it for every candidate, and walking each candidate's words
/// made a chapter cost `O(n·w²)` (D-196).
struct Straddle {
    n: usize,
    /// `stop[i]`: how many of `words[..i]` end a sentence or a paragraph.
    stop: Vec<usize>,
    /// `paid[i]`: what crossing each of those stops costs, summed.
    paid: Vec<f64>,
}

/// For each word, whether its paragraph ends a sentence. One that does not —
/// `Chapter 24`, a date, a name on its own line — has no pause in it but the
/// blank lines around it, and joining it to either neighbour reads it as part
/// of one breath: "Chapter 24 The fall air…", "She stopped. 2024" (D-196).
fn paragraph_is_whole(words: &[&str], breaks: &[bool]) -> Vec<bool> {
    let mut whole = vec![true; words.len()];
    let mut current = true;
    for i in (0..words.len()).rev() {
        if breaks[i] {
            current = ends_sentence(words[i]);
        }
        whole[i] = current;
    }
    whole
}

/// What crossing the blank line after `words[i]` costs.
fn join_cost(whole: &[bool], i: usize) -> f64 {
    if whole[i] && whole.get(i + 1).copied().unwrap_or(true) {
        PARAGRAPH_JOIN
    } else {
        1000.0
    }
}

impl Straddle {
    fn new(words: &[&str], breaks: &[bool]) -> Straddle {
        let n = words.len();
        let whole = paragraph_is_whole(words, breaks);
        let mut stop = vec![0usize; n + 1];
        let mut paid = vec![0.0; n + 1];
        for i in 0..n {
            let stops = breaks[i] || ends_sentence(words[i]);
            let mut cost = 0.0;
            if stops {
                if breaks[i] {
                    cost += join_cost(&whole, i);
                }
                if words.get(i + 1).is_some_and(|next| opens_quote(next)) || closes_quote(words[i])
                {
                    cost += 4.0;
                }
            }
            stop[i + 1] = stop[i] + usize::from(stops);
            paid[i + 1] = paid[i] + cost;
        }
        Straddle { n, stop, paid }
    }

    fn stops(&self, i: usize) -> bool {
        self.stop[i + 1] > self.stop[i]
    }

    fn cost(&self, a: usize, b: usize) -> f64 {
        if b < a + 2 || self.stop[b - 1] == self.stop[a] {
            return 0.0;
        }
        let starts_whole = a == 0 || self.stops(a - 1);
        let ends_whole = b == self.n || self.stops(b - 1);
        if !(starts_whole && ends_whole) {
            return 1000.0;
        }
        self.paid[b - 1] - self.paid[a]
    }
}

#[cfg(test)]
fn straddle_cost_slow(words: &[&str], breaks: &[bool], a: usize, b: usize) -> f64 {
    let stops = |i: usize| breaks[i] || ends_sentence(words[i]);
    let whole = paragraph_is_whole(words, breaks);
    let inside = (a..b.saturating_sub(1)).any(stops);
    if !inside {
        return 0.0;
    }
    let starts_whole = a == 0 || stops(a - 1);
    let ends_whole = b == words.len() || stops(b - 1);
    if !(starts_whole && ends_whole) {
        return 1000.0;
    }
    // Whole sentences may share a picture — but narration and a character's
    // speech prefer their own, and so do two paragraphs.
    let mut cost = 0.0;
    for i in a..b - 1 {
        if !stops(i) {
            continue;
        }
        if breaks[i] {
            cost += join_cost(&whole, i);
        }
        if opens_quote(words[i + 1]) || closes_quote(words[i]) {
            cost += 4.0;
        }
    }
    cost
}

/// What it costs a piece to carry words across a blank line (D-196). Less
/// than a two-second flash costs, more than a quote boundary saves.
const PARAGRAPH_JOIN: f64 = 3.0;

fn opens_quote(word: &str) -> bool {
    word.starts_with(['"', '“', '«', '‘'])
}

fn closes_quote(word: &str) -> bool {
    word.ends_with(['"', '”', '»', '’'])
}

/// The cheapest way to cut a run of words (see the module note). `breaks[i]`
/// says `words[i]` ends a paragraph.
fn cut_words(words: &[&str], breaks: &[bool], pacing: Pacing) -> Vec<String> {
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
    let straddle = Straddle::new(words, breaks);
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
            // The chapter's end is a free cut; every other end pays for where
            // it falls.
            let gap = if b == n {
                0.0
            } else {
                gap_cost(words, breaks, b - 1)
            };
            let cost = best[a] + length_cost(chars, pacing) + gap + straddle.cost(a, b);
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

    /// D-196: a paragraph long enough to hold a picture keeps its own.
    #[test]
    fn a_paragraph_that_can_stand_alone_keeps_its_own_picture() {
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
    fn by_sentences_keeps_exactly_that_many_whole_sentences() {
        let text = "One. Two is longer than one. Three! Four? Five.\n\nSix. Seven.";
        assert_eq!(
            texts(&cut_sentences(text, 1)),
            vec![
                "One.",
                "Two is longer than one.",
                "Three!",
                "Four?",
                "Five.",
                "Six.",
                "Seven."
            ]
        );
        assert_eq!(
            texts(&cut_sentences(text, 2)),
            vec![
                "One. Two is longer than one.",
                "Three! Four?",
                "Five.",
                "Six. Seven."
            ],
            "a blank line still ends a piece"
        );
        assert_eq!(
            texts(&cut_sentences(text, 3)),
            vec![
                "One. Two is longer than one. Three!",
                "Four? Five.",
                "Six. Seven."
            ]
        );
        // A title is not a sentence end here either.
        assert_eq!(
            texts(&cut_sentences("Mr. Chen spoke. Then he left.", 1)),
            vec!["Mr. Chen spoke.", "Then he left."]
        );
        // Text with no full stop at all is one piece, never lost.
        assert_eq!(
            texts(&cut_sentences("no stops here at all", 1)),
            vec!["no stops here at all"]
        );
    }

    #[test]
    fn the_cut_setting_reads_back_what_it_writes() {
        let p = Pacing::DEFAULT;
        for by in [
            CutBy::Time(p),
            CutBy::Sentences(1),
            CutBy::Sentences(2),
            CutBy::Sentences(3),
        ] {
            assert_eq!(CutBy::parse(&by.name(), p), Some(by));
        }
        assert_eq!(CutBy::parse("2 sentences", p), Some(CutBy::Sentences(2)));
        assert_eq!(CutBy::parse("0", p), None);
        assert_eq!(CutBy::parse("banana", p), None);
    }

    /// D-196, from the author's 431-scene chapter: every line of dialogue is
    /// its own paragraph, and "Let's do it." held a picture for 0.77 s.
    #[test]
    fn a_line_of_dialogue_too_short_to_hold_a_picture_joins_a_neighbour() {
        let text = "\"You ready?\" Chloe asked with a smile.\n\n\"Let's do it.\"\n\n\
                    Ryan knew how these battles worked. It was all about whose whales \
                    spent more.";
        let cuts = cut(text, Pacing::DEFAULT);
        assert!(
            !cuts.iter().any(|c| c.text == "\"Let's do it.\""),
            "a 0.7 s flash: {:#?}",
            texts(&cuts)
        );
        assert!(
            cuts.iter()
                .any(|c| c.text.contains("smile. \"Let's do it.\"")
                    || c.text.starts_with("\"Let's do it.\" Ryan"))
        );
        assert_eq!(texts(&cuts).join(" "), normalize(text));
    }

    /// D-196, found by its own stress test: a heading has no pause but the
    /// blank line after it, so joining it read "Chapter 24 The fall air…" as
    /// one breath. A paragraph that does not end a sentence keeps its own.
    #[test]
    fn a_heading_or_a_bare_number_is_never_joined_to_the_next_paragraph() {
        let text = "Chapter 24\n\nThe fall air had a little bite to it. Ryan pulled his \
                    jacket tighter.\n\n\"Wait.\"\n\n2024\n\n\"No,\" he said.";
        let cuts = cut(text, Pacing::DEFAULT);
        assert_eq!(cuts[0].text, "Chapter 24", "{:#?}", texts(&cuts));
        assert!(cuts.iter().any(|c| c.text == "2024"), "{:#?}", texts(&cuts));
        assert_eq!(texts(&cuts).join(" "), normalize(text));
    }

    /// D-196: a blank line is crossed only by whole sentences — never by half
    /// of one, whatever the lengths.
    #[test]
    fn a_blank_line_is_never_crossed_mid_sentence() {
        let text = "\"Okay!\"\n\nUpstairs, in her little studio, Mia had tried on five \
                    different outfits and hated every one of them, so she sat down";
        for c in cut(text, Pacing::DEFAULT) {
            if c.text.contains("Okay!") {
                assert!(
                    c.text == "\"Okay!\"" || c.text.ends_with("sat down"),
                    "{:?}",
                    c.text
                );
            }
        }
    }

    /// D-196: `√` between two sections was spoken as a scene of its own.
    #[test]
    fn a_paragraph_with_no_words_is_a_mark_and_is_left_out() {
        let text = "The guy had dropped three hundred million dollars.\n\n√\n\n\
                    Some of them were worth a few hundred million themselves.\n\n***\n\n---";
        let cuts = cut(text, Pacing::DEFAULT);
        for mark in ["√", "***", "---"] {
            assert!(
                !texts(&cuts).join(" ").contains(mark),
                "{:#?}",
                texts(&cuts)
            );
        }
        assert!(
            texts(&cuts).join(" ").contains("dollars. Some")
                || texts(&cuts).iter().any(|c| c.ends_with("dollars."))
        );
        for c in cut_sentences(text, 1) {
            assert!(c.text.chars().any(char::is_alphanumeric), "{:?}", c.text);
        }
        assert!(cut("√\n\n***", Pacing::DEFAULT).is_empty());
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

            // Nothing lost, nothing added, nothing reordered — except a
            // paragraph with no words in it, which is a mark and not a line
            // (D-196).
            let spoken = normalize(&paragraphs(&text).join(" "));
            let joined: Vec<String> = cuts.iter().map(|c| c.text.clone()).collect();
            assert_eq!(joined.join(" "), spoken, "round {round}: {text:?}");

            // The same for every sentence count.
            let by = 1 + (round as usize % 3);
            let sentences: Vec<String> = cut_sentences(&text, by)
                .into_iter()
                .map(|c| c.text)
                .collect();
            assert_eq!(sentences.join(" "), spoken, "round {round} by {by}");
            assert!(sentences.iter().all(|p| !p.is_empty()), "round {round}");

            for c in &cuts {
                assert!(!c.text.is_empty(), "round {round}: an empty piece");
                assert!(c.seconds.is_finite() && c.seconds > 0.0, "round {round}");
            }

            // Over the whole chapter, so "the piece ends where its paragraph
            // ends" is known exactly: a piece holding a full stop or a blank
            // line inside it must start and end whole (D-196).
            let paragraphs = paragraphs(&text);
            let (words, breaks) = split_chapter(&paragraphs);
            let pieces = cut_words(&words, &breaks, pacing);
            let mut at = 0;
            for piece in &pieces {
                let n = piece.split(' ').count();
                let (a, b) = (at, at + n);
                assert!(
                    straddle_cost(&words, &breaks, a, b) < 1000.0,
                    "round {round}: straddles: {piece:?}"
                );
                assert!(
                    (straddle_cost(&words, &breaks, a, b)
                        - straddle_cost_slow(&words, &breaks, a, b))
                    .abs()
                        < 1e-9,
                    "round {round}: the running totals disagree with the walk"
                );
                at = b;
            }
            assert_eq!(at, words.len(), "round {round}");
            // And on spans the search never picked, which is where a running
            // total that is off by one would hide.
            let sampled = if round.is_multiple_of(10) { 30 } else { 0 };
            for a in 0..words.len().min(sampled) {
                for b in a + 1..=words.len().min(a + 30) {
                    assert!(
                        (straddle_cost(&words, &breaks, a, b)
                            - straddle_cost_slow(&words, &breaks, a, b))
                        .abs()
                            < 1e-9,
                        "round {round}: {a}..{b}"
                    );
                }
            }
        }
    }

    /// `cut` grows with the chapter, not with its square.
    ///
    /// This asserted `took.as_secs() < 20` for one 1 MB chapter, and its own
    /// comment said what it was for: "the point is linear, not quadratic".
    /// A wall clock does not say that. It says how fast the machine is — so on
    /// a 15 W Core i3 it **failed**, having passed on the same tree minutes
    /// earlier on an idle one (D-198). Raising the bound is the wrong repair:
    /// a 19-second quadratic cut would pass a 20-second bound just as happily,
    /// which is D-116's trap — a green assertion that asserts nothing.
    ///
    /// So it measures the *ratio* instead. Both sizes are cut on the same
    /// machine under the same load, which is what makes the comparison a
    /// property of the algorithm rather than of the hardware: at 4x the input,
    /// linear is about 4x the time and quadratic is about 16x. The bound is 8x
    /// — comfortably above linear's 4 and comfortably below quadratic's 16.
    ///
    /// Fixed overhead only ever makes the ratio *smaller*, so it cannot make
    /// this pass for a bad reason; and the small run is sized to take long
    /// enough to measure, since a ratio against nearly zero is noise. The
    /// absolute backstop is kept, but loose enough to mean "hung" rather than
    /// "slow laptop".
    #[test]
    fn a_long_chapter_cuts_linearly_not_quadratically() {
        fn text_of(at_least: usize) -> String {
            let mut rng = Rng(42);
            let mut text = String::new();
            while text.len() < at_least {
                text.push_str(&chapter(&mut rng));
                text.push_str("\n\n");
            }
            text
        }
        fn time_to_cut(text: &str) -> std::time::Duration {
            let started = std::time::Instant::now();
            let cuts = cut(text, Pacing::DEFAULT);
            let took = started.elapsed();
            assert!(!cuts.is_empty(), "cut {} bytes into nothing", text.len());
            took
        }

        let small = text_of(250_000);
        let large = text_of(small.len() * 4);
        // Warm the path once so neither timing pays first-call costs.
        let _ = time_to_cut(&small);

        let small_took = time_to_cut(&small);
        let large_took = time_to_cut(&large);

        let grew = large.len() as f64 / small.len() as f64;
        let slowed = large_took.as_secs_f64() / small_took.as_secs_f64().max(1e-6);
        assert!(
            slowed < 8.0,
            "{grew:.1}x the text took {slowed:.1}x the time \
             ({small_took:?} for {} bytes, {large_took:?} for {} bytes) — \
             linear would be about {grew:.0}x, quadratic about {:.0}x",
            small.len(),
            large.len(),
            grew * grew,
        );
        assert!(
            large_took.as_secs() < 120,
            "{large_took:?} for {} bytes is a hang, not a slow machine",
            large.len()
        );
    }
}
