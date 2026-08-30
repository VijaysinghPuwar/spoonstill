//! The subtitle chooser, as both control surfaces see it (D-106).
//!
//! D-010 keeps the window out of `spoonstill-media`, and the preview it needs
//! is drawn there — so this module is the seam, exactly as `tooling` is the
//! seam for external programs and `tts` for providers. The CLI reaches it too:
//! `still subtitles` prints the same list this returns.
//!
//! The preview is drawn by the renderer that draws the film. A chooser whose
//! swatches are hand-made — CSS that resembles the theme, a screenshot taken
//! once — is a chooser that can be wrong, and being wrong about legibility is
//! the only way it can fail an operator.

use spoonstill_core::captions::{Placement, SubtitleTheme};
use spoonstill_core::{Aspect, OutputSpec};

use crate::import::settings::DEFAULT_THEME;

/// One theme, as something to put in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeChoice {
    /// The name in `project.yaml` and on the command line.
    pub id: &'static str,
    /// One line saying what it is for.
    pub description: &'static str,
    /// Whether this is the one a project gets without asking.
    pub default: bool,
}

/// Every theme, in the order a chooser should offer them.
#[must_use]
pub fn themes() -> Vec<ThemeChoice> {
    SubtitleTheme::ALL
        .into_iter()
        .map(|theme| ThemeChoice {
            id: theme.as_str(),
            description: theme.description(),
            default: theme == DEFAULT_THEME,
        })
        .collect()
}

/// The sentence a preview shows when the caller has nothing better.
///
/// Long enough to wrap to two lines in every theme, which is the case worth
/// looking at: one short line tells an operator nothing about how a theme
/// handles the caption they will actually have.
pub const SAMPLE: &str = "The tide came in before dawn, and by seven the harbour was full.";

/// A drawn preview frame.
#[derive(Debug, Clone)]
pub struct Preview {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Straight RGBA, `width * height * 4` bytes. Opaque throughout.
    pub rgba: Vec<u8>,
}

/// Draw one theme over a stand-in photograph.
///
/// `short_edge` is the preview's own size, not the project's — the themes are
/// defined in fractions of the frame (see `ThemeStyle`), so a small preview is
/// a faithful scale model of the real thing rather than a different design.
///
/// # Errors
///
/// The geometry, if `short_edge` is one `OutputSpec` will not accept — odd, or
/// beyond the bounds of D-032.
pub fn preview(
    text: &str,
    theme: &str,
    placement: &str,
    short_edge: u32,
) -> Result<Preview, String> {
    // An empty theme is "no subtitles" — one of the choices in the window's
    // list, and a chooser that answers it with a picture of a caption is
    // telling the operator the opposite of what the setting does.
    let chosen = if theme.trim().is_empty() {
        None
    } else {
        Some(
            SubtitleTheme::parse(theme)
                .ok_or_else(|| format!("{theme:?} is not one of {}", SubtitleTheme::names()))?,
        )
    };
    let placement =
        Placement::parse(placement).ok_or_else(|| format!("{placement:?} is not bottom or top"))?;

    let output =
        OutputSpec::new(Aspect::Landscape16x9, short_edge, 30).map_err(|e| e.to_string())?;
    let text = if text.trim().is_empty() { SAMPLE } else { text };

    let canvas = match chosen {
        Some(theme) => spoonstill_media::caption::preview(text, theme, placement, output),
        None => spoonstill_media::caption::preview_frame(output),
    };
    Ok(Preview {
        width: canvas.width(),
        height: canvas.height(),
        rgba: canvas.into_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever the window lists is what the renderer can draw, and exactly one
    /// of them is the default. A list that drifts from `SubtitleTheme::ALL` is
    /// a chooser offering something that cannot be rendered.
    #[test]
    fn the_chooser_offers_every_theme_and_one_default() {
        let listed = themes();
        assert_eq!(listed.len(), SubtitleTheme::ALL.len());
        assert_eq!(listed.iter().filter(|t| t.default).count(), 1);
        for theme in SubtitleTheme::ALL {
            assert!(
                listed.iter().any(|t| t.id == theme.as_str()),
                "{theme} is renderable but not offered"
            );
        }
    }

    #[test]
    fn every_offered_theme_previews() {
        for choice in themes() {
            let preview = preview(SAMPLE, choice.id, "bottom", 270)
                .unwrap_or_else(|e| panic!("{}: {e}", choice.id));
            assert_eq!((preview.width, preview.height), (480, 270));
            assert_eq!(preview.rgba.len(), 480 * 270 * 4);
            assert!(
                preview.rgba.chunks_exact(4).all(|p| p[3] == 255),
                "{}: the preview must be an opaque frame",
                choice.id
            );
        }
    }

    /// D-055 in the window: a name that is not a theme is refused with the
    /// list, not substituted with the default.
    #[test]
    fn an_unknown_theme_is_refused_with_the_list() {
        let error = preview(SAMPLE, "clasic", "bottom", 270).expect_err("not a theme");
        assert!(error.contains("clasic"), "{error}");
        assert!(
            error.contains("classic"),
            "the message must list them: {error}"
        );

        let error = preview(SAMPLE, "classic", "middle", 270).expect_err("not a placement");
        assert!(error.contains("middle"), "{error}");
    }

    /// "No subtitles" is one of the choices, so it has to be one of the
    /// pictures — the bare frame, with nothing burned into it.
    #[test]
    fn no_theme_previews_the_bare_frame() {
        let bare = preview(SAMPLE, "", "bottom", 270).expect("previews");
        let boxed = preview(SAMPLE, "boxed", "bottom", 270).expect("previews");
        assert_eq!((bare.width, bare.height), (480, 270));
        assert!(
            bare.rgba.chunks_exact(4).all(|p| p[3] == 255),
            "still a frame"
        );
        assert_ne!(bare.rgba, boxed.rgba, "the bare frame has no caption on it");

        // And it is the same photograph the themes are drawn over, so moving
        // between them changes the caption and never the picture.
        assert_eq!(
            &bare.rgba[..480 * 60 * 4],
            &boxed.rgba[..480 * 60 * 4],
            "the stand-in photograph must not change"
        );
    }

    /// An empty box on screen is a request for the sample, not for a blank
    /// preview that makes the theme look broken.
    #[test]
    fn empty_text_falls_back_to_the_sample() {
        let blank = preview("   ", "boxed", "bottom", 270).expect("previews");
        let sample = preview(SAMPLE, "boxed", "bottom", 270).expect("previews");
        assert_eq!(blank.rgba, sample.rgba);
    }
}
