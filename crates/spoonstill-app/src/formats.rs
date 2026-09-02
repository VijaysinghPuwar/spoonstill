//! The output-size chooser, as both control surfaces see it (D-143).
//!
//! The same seam `subtitles` is for themes and `tooling` is for external
//! programs: `spoonstill-core` owns [`Aspect`] and [`Resolution`], and this is
//! where they become a list something can draw. `still resolutions` prints it;
//! the window's Output screen fills two `<select>`s from it.
//!
//! Nothing here decides anything. Every rule about which short edges are legal
//! belongs to [`OutputSpec::new`] (D-114), and this module calls it rather than
//! restating it — a list that claimed a size the constructor refuses would be
//! a chooser offering a render that cannot happen.

use spoonstill_core::{Aspect, OutputSpec, Resolution};

/// One aspect, as something to put in a list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AspectChoice {
    /// The name in `project.yaml` and on the command line — `16:9`.
    pub id: &'static str,
    /// One line saying what it is for, in destinations rather than ratios.
    pub description: &'static str,
    /// Whether this is the one a project gets without asking.
    pub default: bool,
}

/// One named size, already resolved against an aspect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SizeChoice {
    /// The name on the command line — `1080p`, `2160p`.
    pub id: &'static str,
    /// The other spellings it answers to, `4k` among them.
    pub aliases: Vec<&'static str>,
    /// One line saying what it is for.
    pub description: &'static str,
    /// The short edge in pixels — the number `project.yaml` would carry.
    pub short_edge: u32,
    /// Output width in the aspect this list was asked for.
    pub width: u32,
    /// Output height in the same.
    pub height: u32,
    /// Whether this is the one a project gets without asking.
    pub default: bool,
}

impl SizeChoice {
    /// `1920x1080`, for a line of text or a `<option>` label.
    #[must_use]
    pub fn dimensions(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }
}

/// Every aspect, in the order a chooser should offer them.
#[must_use]
pub fn aspects() -> Vec<AspectChoice> {
    let default = crate::import::Settings::default().output_spec.aspect();
    Aspect::ALL
        .into_iter()
        .map(|aspect| AspectChoice {
            id: aspect.as_str(),
            description: aspect.description(),
            default: aspect == default,
        })
        .collect()
}

/// Every named size, with the pixel dimensions it produces in `aspect`.
///
/// A size is only listed if [`OutputSpec::new`] accepts it, so this cannot
/// offer a render that would then be refused. In practice every named size is
/// legal in every aspect — a test in `spoonstill-core` holds that — and the
/// filter here is what keeps a fifth name honest if one is ever added.
#[must_use]
pub fn sizes(aspect: Aspect) -> Vec<SizeChoice> {
    let defaults = crate::import::Settings::default().output_spec;
    Resolution::ALL
        .into_iter()
        .filter_map(|resolution| {
            let spec = OutputSpec::new(aspect, resolution.short_edge(), defaults.fps()).ok()?;
            Some(SizeChoice {
                id: resolution.as_str(),
                aliases: resolution.aliases().to_vec(),
                description: resolution.description(),
                short_edge: resolution.short_edge(),
                width: spec.width(),
                height: spec.height(),
                default: resolution.short_edge() == defaults.short_edge(),
            })
        })
        .collect()
}

/// Resolve what a control surface was given into a short edge.
///
/// Accepts a name (`4k`), a canonical name (`2160p`) or a bare number
/// (`2160`), because all three are things an operator types and refusing one
/// of them is a distinction only this program cares about.
///
/// # Errors
///
/// A sentence naming what is on offer, when the text is neither.
pub fn parse_size(text: &str) -> Result<u32, String> {
    let text = text.trim();
    if let Some(resolution) = Resolution::parse(text) {
        return Ok(resolution.short_edge());
    }
    // A bare number is a short edge, and `OutputSpec::new` judges it — this
    // deliberately does not, because "even and divisible by 9" is a rule that
    // depends on the aspect and belongs where the aspect is known (D-114).
    if let Ok(pixels) = text.parse::<u32>() {
        return Ok(pixels);
    }
    Err(format!(
        "{text:?} is not one of {} (2k and 4k are accepted too), \
         nor a short edge in pixels",
        Resolution::names()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_size_is_one_that_renders() {
        for aspect in Aspect::ALL {
            let sizes = sizes(aspect);
            assert_eq!(sizes.len(), Resolution::ALL.len(), "{aspect}");
            for size in &sizes {
                let spec = OutputSpec::new(aspect, size.short_edge, 30)
                    .unwrap_or_else(|e| panic!("{} at {aspect}: {e}", size.id));
                assert_eq!((spec.width(), spec.height()), (size.width, size.height));
            }
        }
    }

    /// The two headline sizes, in the words the operator asked for them in.
    #[test]
    fn two_k_and_four_k_are_on_the_list_by_those_names() {
        let landscape = sizes(Aspect::Landscape16x9);
        let two_k = landscape
            .iter()
            .find(|s| s.aliases.contains(&"2k"))
            .expect("2k is offered");
        assert_eq!(two_k.dimensions(), "2560x1440");
        let four_k = landscape
            .iter()
            .find(|s| s.aliases.contains(&"4k"))
            .expect("4k is offered");
        assert_eq!(four_k.dimensions(), "3840x2160");

        // The same two names, vertically — a 4K Short is 2160x3840.
        let portrait = sizes(Aspect::Portrait9x16);
        assert_eq!(portrait[3].dimensions(), "2160x3840");
        assert_eq!(portrait[1].dimensions(), "1080x1920");
    }

    #[test]
    fn exactly_one_of_each_list_is_the_default() {
        assert_eq!(aspects().iter().filter(|a| a.default).count(), 1);
        assert_eq!(aspects().iter().find(|a| a.default).unwrap().id, "16:9");
        for aspect in Aspect::ALL {
            let sizes = sizes(aspect);
            assert_eq!(sizes.iter().filter(|s| s.default).count(), 1, "{aspect}");
            assert_eq!(sizes.iter().find(|s| s.default).unwrap().id, "1080p");
        }
    }

    #[test]
    fn a_size_is_a_name_or_a_number() {
        assert_eq!(parse_size("4k"), Ok(2160));
        assert_eq!(parse_size(" 2K "), Ok(1440));
        assert_eq!(parse_size("1080p"), Ok(1080));
        assert_eq!(parse_size("900"), Ok(900), "a number is still a short edge");
        let error = parse_size("enormous").unwrap_err();
        assert!(error.contains("2160p"), "{error}");
        assert!(error.contains("4k"), "{error}");
    }

    /// The chooser's default has to be the *project* default, not a literal
    /// repeated here — those are the two that drift.
    #[test]
    fn the_listed_default_is_the_projects_own() {
        let spec = crate::import::Settings::default().output_spec;
        assert_eq!(
            sizes(spec.aspect())
                .iter()
                .find(|s| s.default)
                .unwrap()
                .short_edge,
            spec.short_edge()
        );
    }
}
