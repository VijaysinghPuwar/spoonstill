//! Every external program this application needs, checked and installed in one
//! place (D-105).
//!
//! Two subsystems own binaries — `spoonstill_media::tools` has `ffmpeg` and
//! `ffprobe`, `spoonstill_tts` has whatever its providers shell out to — and
//! before this module a control surface had to know which was which. The
//! window knew about `install_provider` and nothing about FFmpeg, so the
//! screen reporting the *more* serious of the two problems was the one that
//! could do less about it: a missing voice service had a button, and a missing
//! FFmpeg had the string `brew install ffmpeg`.
//!
//! It lives here rather than in the window because D-010 says the shell owns
//! no business logic and may not reach `spoonstill-media` at all — and because
//! "if the CLI cannot do it, it does not exist". `still doctor` and the
//! window's Install buttons are two faces of the functions below.
//!
//! Nothing here downloads anything from us. Every install is the platform's
//! own package manager, run because somebody pressed a button that said
//! install — which is not what D-012 refuses (that is fetching a *build*
//! nobody chose, silently, at render time).

use spoonstill_core::Remedy;

/// The id of the FFmpeg pair, as it crosses into a control surface.
pub use spoonstill_media::tools::FFMPEG_TOOL as FFMPEG;

/// One external program, and whether this machine can run it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolReport {
    /// What to pass back to [`install`]. `"ffmpeg"`, or a provider id.
    pub id: String,
    /// What it is for, in the operator's terms — shown beside the id so a
    /// list of tools reads as a list of capabilities rather than of binaries.
    pub purpose: &'static str,
    /// Nothing to do.
    pub ready: bool,
    /// What is wrong and what fixes it. [`None`] exactly when `ready`.
    pub remedy: Option<Remedy>,
    /// Which build this is, when somebody has paid for the answer (D-151).
    ///
    /// `None` from [`ffmpeg`] and [`provider`] on purpose: those are a stat and
    /// are on the path of every validate, and asking a binary its version is a
    /// spawn. [`check_all`] fills it, because its two callers — `still doctor`
    /// and the window's Settings card — are a person asking *what is on this
    /// machine*, which is the one question a version answers.
    pub version: Option<String>,
}

impl ToolReport {
    /// Whether a control surface has a button to draw for this.
    #[must_use]
    pub fn is_installable(&self) -> bool {
        self.remedy.as_ref().is_some_and(Remedy::is_installable)
    }
}

/// Whether FFmpeg and ffprobe are both runnable on this machine.
///
/// A stat rather than a spawn: this is on the path of every validate, and the
/// spawn itself is still the authority (D-103).
#[must_use]
pub fn ffmpeg() -> ToolReport {
    report(
        FFMPEG,
        "turning your photos into video",
        spoonstill_media::Tools::from_env().ready(),
    )
}

/// Whether a voice provider's tooling is runnable on this machine.
///
/// A provider id this build does not have is itself a report — not ready, and
/// not installable, because the fix is to correct `project.yaml` rather than
/// to fetch anything.
#[must_use]
pub fn provider(id: &str) -> ToolReport {
    const PURPOSE: &str = "reading your written lines aloud";

    match crate::tts::provider(id) {
        Err(error) => ToolReport {
            id: id.to_owned(),
            purpose: PURPOSE,
            ready: false,
            version: None,
            remedy: Some(Remedy::manual(
                "This project asks for a voice service this version of spoonstill does \
                 not have.",
                error.to_string(),
            )),
        },
        Ok(engine) => report(
            engine.id(),
            PURPOSE,
            match engine.availability() {
                crate::tts::Availability::Ready => Ok(()),
                crate::tts::Availability::Missing(remedy) => Err(remedy),
            },
        ),
    }
}

/// Every tool at once, in the order a render needs them.
///
/// FFmpeg first because a machine without it cannot make a film at all, and
/// telling somebody their voice service is missing while the renderer is also
/// missing is answering the second question first. This is what a launch-time
/// check and `still doctor` both read.
#[must_use]
pub fn check_all() -> Vec<ToolReport> {
    let mut ffmpeg = ffmpeg();
    // Only when it runs: asking a binary that is not there for its version is
    // a spawn that can only fail, and the remedy already says what is wrong.
    if ffmpeg.ready {
        ffmpeg.version = Some(spoonstill_media::Tools::from_env().ffmpeg_version());
    }
    let mut all = vec![ffmpeg];
    all.extend(crate::tts::providers().iter().map(|p| provider(p.id())));
    all
}

/// Fetch one tool through this machine's own package manager.
///
/// # Errors
///
/// A [`Remedy`] naming every candidate that was tried and what each said.
/// Deliberately not installable: pressing again would do the same thing, and
/// what the operator needs at that point is the detail.
pub fn install(tool: &str) -> Result<String, Remedy> {
    if tool == FFMPEG {
        return spoonstill_media::tools::install();
    }
    crate::tts::provider(tool)
        .map_err(|error| {
            Remedy::manual(
                "There is nothing to install for that — spoonstill does not have a voice \
                 service by that name.",
                error.to_string(),
            )
        })?
        .install()
        .map_err(|error| {
            Remedy::manual(
                "Spoonstill could not install the voice service for you. Installing it \
                 yourself, then pressing Check again, is the usual fix.",
                error.to_string(),
            )
        })
}

/// Build a report from whatever the owning subsystem answered.
fn report(id: &str, purpose: &'static str, checked: Result<(), Remedy>) -> ToolReport {
    ToolReport {
        id: id.to_owned(),
        purpose,
        ready: checked.is_ok(),
        remedy: checked.err(),
        version: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list a launch check and `still doctor` both read has FFmpeg first,
    /// because a machine that cannot render at all should not be told about
    /// voices first.
    #[test]
    fn ffmpeg_is_reported_before_any_voice_service() {
        let all = check_all();
        assert_eq!(all.first().map(|t| t.id.as_str()), Some(FFMPEG), "{all:?}");
        assert!(all.len() > 1, "there is at least one provider: {all:?}");
    }

    /// Ready and "has a remedy" are the same fact said twice, and they must
    /// never disagree — a ready tool with a remedy would draw an Install
    /// button over a working installation, which is the bug D-104 fixed.
    #[test]
    fn a_ready_tool_has_nothing_to_report() {
        for tool in check_all() {
            assert_eq!(
                tool.ready,
                tool.remedy.is_none(),
                "{} says ready={} and remedy={:?}",
                tool.id,
                tool.ready,
                tool.remedy
            );
            assert!(!tool.purpose.is_empty(), "{} has no purpose", tool.id);
        }
    }

    /// A provider id nothing implements is a report rather than a panic, and
    /// it offers no button — the fix is to correct `project.yaml`, and a
    /// package manager cannot do that.
    #[test]
    fn an_unknown_provider_is_reported_and_not_installable() {
        let unknown = provider("no-such-provider-9d2f1");
        assert!(!unknown.ready);
        assert!(
            !unknown.is_installable(),
            "there is no package manager for a typo: {unknown:?}"
        );
        assert!(
            install("no-such-provider-9d2f1").is_err(),
            "and installing it is refused rather than attempted"
        );
    }
}
