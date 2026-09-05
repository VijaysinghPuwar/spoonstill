//! What language a line is written in, read off the script it uses (D-158).
//!
//! # Why this is here at all
//!
//! An operator who types Hindi into a scene and presses Render used to get a
//! failure, not a film: the default voice is `en-US-AvaNeural`, the service
//! accepts a request for it, returns **no audio**, and the message that came
//! back said the line had *"no speakable words in it — only punctuation,
//! digits or symbols"*. Every word was speakable. Nothing was wrong with the
//! text and nothing in the message pointed at the voice.
//!
//! So the script picks the language when — and **only** when — nobody has
//! picked a voice. A voice the operator named is obeyed, right or wrong; that
//! is D-076's rule about an explicit `--jobs` applied to a different setting.
//!
//! # What it can and cannot know
//!
//! A script is not a language. Devanagari is Hindi, Marathi, Nepali, Sanskrit
//! and more; Cyrillic is Russian and a dozen others; Han is Chinese and half of
//! Japanese. This module answers with the **most widely written** language of
//! each script and says so, because the alternative — refusing to guess, which
//! is this project's usual rule (D-111, D-152) — means the render fails, and a
//! render that fails is worse than one an operator retunes with `--voice`. The
//! one ambiguity worth resolving properly is resolved: Han **with kana** is
//! Japanese, Han without is Chinese.
//!
//! Latin is deliberately not an answer. A line of English gets no language
//! from here and falls through to the provider's own default, which is what
//! every film made before this did — so nothing already made changes.

/// The language a line is in, as a BCP-47 primary subtag, or `None`.
///
/// `None` means *nothing here settles it*: an empty line, digits, punctuation,
/// or Latin script — for which the caller's existing default is already right.
///
/// The dominant non-Latin script wins by character count, so one English word
/// inside a Hindi sentence does not change the answer, and neither does a
/// number or a comma. Ties are broken by the order of [`SCRIPTS`], which is
/// fixed, so the same text always gives the same answer — a voice reaches the
/// audio cache key, and an answer that moved would re-speak a whole film.
#[must_use]
pub fn of(text: &str) -> Option<&'static str> {
    let mut counts = [0usize; SCRIPTS.len()];
    for ch in text.chars() {
        if let Some(index) = script_of(ch) {
            counts[index] += 1;
        }
    }

    let (index, best) = counts
        .iter()
        .enumerate()
        .max_by_key(|(order, count)| (**count, std::cmp::Reverse(*order)))?;
    if *best == 0 {
        return None;
    }

    // Japanese and Chinese share the Han characters, and only the kana
    // separate them. Checked here rather than by giving kana a bigger weight,
    // because a sentence can be mostly Han and still plainly Japanese.
    if SCRIPTS[index].0 == "zh" && counts[KANA] > 0 {
        return Some("ja");
    }
    Some(SCRIPTS[index].0)
}

/// Which of [`SCRIPTS`] this character belongs to.
fn script_of(ch: char) -> Option<usize> {
    let c = ch as u32;
    SCRIPTS
        .iter()
        .position(|(_, ranges)| ranges.iter().any(|(lo, hi)| c >= *lo && c <= *hi))
}

/// Index of the kana entry, which is Japanese on its own and also the thing
/// that tells Japanese from Chinese.
const KANA: usize = 17;

/// Each script this can recognise, and the language it stands for.
///
/// The blocks are the ones that carry a script's letters. Combining marks live
/// inside the same block for every script here, which is why a matra counts
/// towards its own script rather than towards nothing.
///
/// Ordered, and the order is load-bearing twice: [`KANA`] indexes into it, and
/// a tie is broken by it.
const SCRIPTS: &[(&str, &[(u32, u32)])] = &[
    // South Asia. Devanagari is Hindi here — also Marathi, Nepali and
    // Sanskrit, and an operator writing those names a voice.
    ("hi", &[(0x0900, 0x097F), (0xA8E0, 0xA8FF)]),
    ("bn", &[(0x0980, 0x09FF)]),
    ("pa", &[(0x0A00, 0x0A7F)]),
    ("gu", &[(0x0A80, 0x0AFF)]),
    ("or", &[(0x0B00, 0x0B7F)]),
    ("ta", &[(0x0B80, 0x0BFF)]),
    ("te", &[(0x0C00, 0x0C7F)]),
    ("kn", &[(0x0C80, 0x0CFF)]),
    ("ml", &[(0x0D00, 0x0D7F)]),
    ("si", &[(0x0D80, 0x0DFF)]),
    // The rest of the world, in Unicode order.
    ("el", &[(0x0370, 0x03FF), (0x1F00, 0x1FFF)]),
    ("ru", &[(0x0400, 0x04FF), (0x0500, 0x052F)]),
    ("hy", &[(0x0530, 0x058F)]),
    ("he", &[(0x0590, 0x05FF)]),
    // Arabic script, which is also Urdu and Persian. Arabic is the most
    // written of them.
    (
        "ar",
        &[(0x0600, 0x06FF), (0x0750, 0x077F), (0x08A0, 0x08FF)],
    ),
    ("th", &[(0x0E00, 0x0E7F)]),
    ("lo", &[(0x0E80, 0x0EFF)]),
    // Kana. `KANA` is this row; keep them together.
    (
        "ja",
        &[(0x3040, 0x309F), (0x30A0, 0x30FF), (0x31F0, 0x31FF)],
    ),
    ("my", &[(0x1000, 0x109F)]),
    ("ka", &[(0x10A0, 0x10FF)]),
    ("am", &[(0x1200, 0x137F)]),
    ("km", &[(0x1780, 0x17FF)]),
    (
        "ko",
        &[(0x1100, 0x11FF), (0x3130, 0x318F), (0xAC00, 0xD7AF)],
    ),
    // Han, last because the kana check above reads it as Japanese when kana
    // are present.
    (
        "zh",
        &[(0x3400, 0x4DBF), (0x4E00, 0x9FFF), (0xF900, 0xFAFF)],
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_script_a_line_is_written_in_names_its_language() {
        assert_eq!(of("नमस्ते दुनिया"), Some("hi"));
        assert_eq!(of("চলো বাংলা"), Some("bn"));
        assert_eq!(of("Καλημέρα κόσμε"), Some("el"));
        assert_eq!(of("Здравствуй мир"), Some("ru"));
        assert_eq!(of("مرحبا بالعالم"), Some("ar"));
        assert_eq!(of("안녕하세요 여러분"), Some("ko"));
        assert_eq!(of("สวัสดีชาวโลก"), Some("th"));
    }

    /// Latin gets no answer, and that is the point: every film made before
    /// this module existed keeps the voice it had.
    #[test]
    fn latin_and_nothing_settle_nothing() {
        assert_eq!(of("Hello world, chapter 3."), None);
        assert_eq!(of(""), None);
        assert_eq!(of("123 — ... !?"), None);
        assert_eq!(of("Café naïve résumé"), None);
    }

    /// One English word in a Hindi sentence does not make it English.
    #[test]
    fn the_dominant_script_wins_not_the_first_one() {
        assert_eq!(of("यह chapter तीन है, और यह हिंदी है"), Some("hi"));
        assert_eq!(of("Chapter 3 — नमस्ते दुनिया, यह हिंदी है"), Some("hi"));
    }

    /// The one ambiguity worth resolving rather than guessing: Japanese is
    /// written with the same Han characters as Chinese, and the kana are what
    /// tell them apart.
    #[test]
    fn han_with_kana_is_japanese_and_han_alone_is_chinese() {
        assert_eq!(of("你好世界"), Some("zh"));
        assert_eq!(of("こんにちは"), Some("ja"));
        assert_eq!(of("私は日本語を話します"), Some("ja"));
        assert_eq!(of("今日は良い天気ですね"), Some("ja"));
    }

    /// The answer reaches an audio cache key, so it must not depend on
    /// anything that can vary between two runs over the same text.
    #[test]
    fn the_same_line_always_gives_the_same_answer() {
        let line = "नमस्ते दुनिया, this is chapter 3, 你好";
        let first = of(line);
        for _ in 0..50 {
            assert_eq!(of(line), first);
        }
    }
}
