//! A missing tool, described as something to *press* rather than something to
//! read (D-105).
//!
//! Every external program this application needs — `ffmpeg`, `ffprobe`,
//! `edge-tts` — can be absent, and until now being absent produced one flat
//! `String`:
//!
//! ```text
//! `edge-tts` is not on this machine. Install it with `pip install edge-tts`
//! (or `brew install edge-tts`), press Install in Settings, or point
//! SPOONSTILL_EDGE_TTS at it.
//! ```
//!
//! That sentence is four instructions, three of which require a terminal and
//! one of which is an environment variable. It was shown on the Voice screen
//! above an empty list, with no button anywhere near it. The operator it was
//! written for does not have a terminal open and should not need one: the
//! window is already a program that can run a package manager, and D-092
//! settled that it should.
//!
//! So a missing tool is three separate things, and the difference matters
//! because they go to three different places:
//!
//! - [`Remedy::need`] — one plain sentence, no paths, no flags, no backticks.
//!   This is what an operator reads.
//! - [`Remedy::install`] — the tool's id, when this application can fetch it
//!   itself. This is what the window turns into a button, next to the
//!   sentence, on the screen where the problem appeared.
//! - [`Remedy::detail`] — the located path, the exit status, the last line of
//!   stderr. This is what goes in the diagnostics bundle and behind a
//!   disclosure triangle, and it is never the first thing anybody sees.
//!
//! It lives in `spoonstill-core` because all three producers
//! (`spoonstill_media::tools`, `spoonstill_tts::edge`, and
//! [`crate::project::ProblemKind`]) need one shape, and this crate is the one
//! they all already depend on. It is data and nothing else — no `Command`, no
//! serde, no knowledge of who will draw it.

use std::fmt;

/// What is missing, what to say about it, and whether we can fix it.
///
/// Constructed by whichever layer discovered the absence, because only that
/// layer knows the detail. Rendered by whichever control surface is in front
/// of the operator, because only it knows whether it has a button.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Remedy {
    /// One sentence an operator can act on, in words rather than syntax.
    ///
    /// No absolute paths, no command lines, no environment variables — those
    /// are all [`Remedy::detail`]. "Spoonstill needs FFmpeg to turn your
    /// photos into video. It is not installed on this Mac yet." is the whole
    /// job of this field.
    pub need: String,

    /// The id to hand back to install this, when that is possible.
    ///
    /// `"ffmpeg"` or a provider id such as `"edge"`. [`None`] means there is
    /// nothing to press — the operator has to act, and [`Remedy::need`] has to
    /// carry the whole answer on its own.
    pub install: Option<String>,

    /// What actually happened, for the bundle and for the disclosure triangle.
    ///
    /// The path that was tried, the exit code, the last line of stderr. May be
    /// empty when the absence is the entire story.
    pub detail: String,
}

impl Remedy {
    /// A missing tool this application can fetch itself.
    pub fn installable(
        need: impl Into<String>,
        tool: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            need: need.into(),
            install: Some(tool.into()),
            detail: detail.into(),
        }
    }

    /// A problem the operator has to resolve, with no button to offer.
    pub fn manual(need: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            need: need.into(),
            install: None,
            detail: detail.into(),
        }
    }

    /// Whether a control surface has a button to draw for this.
    #[must_use]
    pub fn is_installable(&self) -> bool {
        self.install.is_some()
    }
}

/// The whole thing on one line: the sentence, then the detail in parentheses.
///
/// This is what the CLI prints and what every existing `to_string()` caller
/// gets, so a terminal keeps the technical half it has always had while the
/// window is free to split the two apart. A `Remedy` never loses its detail by
/// being formatted — it only loses its *button*, and a terminal never had one.
impl fmt::Display for Remedy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.need)?;
        if !self.detail.is_empty() {
            write!(f, " ({})", self.detail)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sentence an operator reads carries none of the syntax that made the
    /// old one unusable. This is the whole of D-105 as an assertion: if a
    /// `need` ever grows a backtick or a flag, it has become a detail.
    #[test]
    fn the_plain_sentence_holds_no_syntax() {
        let remedy = Remedy::installable(
            "Spoonstill needs FFmpeg to turn your photos into video. \
             It is not installed on this Mac yet.",
            "ffmpeg",
            "tried /usr/bin/ffprobe",
        );
        for forbidden in ['`', '$', '<', '>'] {
            assert!(
                !remedy.need.contains(forbidden),
                "{forbidden:?} is syntax, and syntax belongs in `detail`: {}",
                remedy.need
            );
        }
        assert!(
            !remedy.need.contains("--"),
            "a flag belongs in `detail`: {}",
            remedy.need
        );
        assert!(remedy.is_installable());
    }

    /// A terminal keeps everything. The window is what splits them.
    #[test]
    fn formatting_keeps_the_detail() {
        let remedy = Remedy::manual("FFmpeg is not installed.", "tried /usr/bin/ffmpeg");
        let shown = remedy.to_string();
        assert!(shown.contains("FFmpeg is not installed."), "{shown}");
        assert!(shown.contains("tried /usr/bin/ffmpeg"), "{shown}");
        assert!(!remedy.is_installable());
    }

    /// An absence with nothing to add reads as one clean sentence rather than
    /// as a sentence with an empty bracket after it.
    #[test]
    fn an_empty_detail_adds_no_brackets() {
        assert_eq!(
            Remedy::manual("Nothing to add.", "").to_string(),
            "Nothing to add."
        );
    }
}
