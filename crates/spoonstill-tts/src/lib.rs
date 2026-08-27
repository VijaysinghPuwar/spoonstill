//! Text-to-speech providers behind one trait (D-023).
//!
//! One module per provider. A giant `match` on a provider name string is the
//! mistake this crate is shaped to avoid — adding a provider must not require
//! editing a dispatch table that every other provider also lives in.
//!
//! Defaults differ by distribution: Edge TTS internally (free, and it emits the
//! word-boundary events karaoke captions will need), ElevenLabs BYOK in a sold
//! build — because no reverse-engineered endpoint may be load-bearing in a
//! shipped product. Keys come from `keyring-rs` (D-014) and never leave the
//! machine.
//!
//! ## What a provider is responsible for, and what it is not
//!
//! A provider turns a line of text into **one audio file on disk**, and answers
//! what voices it has. That is the whole contract. It does not:
//!
//! - decide the file's format — whatever it writes is normalized downstream by
//!   `spoonstill_media::audio::normalize`, so an Edge mp3 and an ElevenLabs mp3
//!   converge on the same 48 kHz stereo PCM (D-075);
//! - decide how long the scene is — the duration is *measured* on the
//!   normalized artifact by `ffprobe`, never taken from a provider's word count
//!   or a container header (D-021);
//! - cache anything — the cache key is the caller's, computed from the text,
//!   the provider, the voice, the settings and the profile (D-043).
//!
//! Which is why a provider that shells out and a provider that speaks HTTP can
//! sit behind the same three methods.

#![warn(missing_docs)]

pub mod edge;

use std::path::Path;

/// This crate's package name, resolved at compile time.
pub const CRATE_NAME: &str = env!("CARGO_PKG_NAME");

/// One line to speak, and how to speak it.
///
/// Borrowed rather than owned: the text belongs to the project model and a
/// 500-scene batch should not copy every line to ask for it.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The words, exactly as written. Never logged in full (see [`opening`]).
    pub text: &'a str,
    /// The voice's own name, as the provider spells it.
    pub voice: &'a str,
    /// Provider-specific knobs, as `(name, value)` pairs — the shape
    /// `spoonstill_core::project::TtsSettings` already carries, passed through
    /// without this crate inventing a schema every provider then has to
    /// pretend to share.
    pub settings: &'a [(String, String)],
}

impl Request<'_> {
    /// The value of one setting, if it was given.
    #[must_use]
    pub fn setting(&self, name: &str) -> Option<&str> {
        self.settings
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

/// One voice a provider offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Voice {
    /// What to put in `project.yaml` or the manifest.
    pub id: String,
    /// BCP-47 locale, e.g. `en-US`, when the provider states one.
    pub locale: String,
    /// `Female`, `Male`, or whatever else the provider says.
    pub gender: String,
    /// A human line — personality, content category — or empty.
    pub note: String,
}

/// What a provider wrote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spoken {
    /// Bytes written. Zero is a failure, not an empty line: providers report
    /// service errors by producing nothing.
    pub bytes: u64,
    /// The command or endpoint used, with the text redacted — safe to log
    /// (D-016).
    pub how: String,
}

/// Whether a provider can be used right now, and if not, what to do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// Ready.
    Ready,
    /// Not ready, with a sentence naming the fix.
    Missing(String),
}

/// Everything that can go wrong on the way to an audio file.
#[derive(Debug)]
pub enum TtsError {
    /// The provider named in the project has no implementation here.
    NoSuchProvider {
        /// What was asked for.
        provider: String,
        /// What exists.
        known: Vec<String>,
    },
    /// The provider exists but cannot run — a missing binary, a missing key.
    Unavailable {
        /// Which provider.
        provider: String,
        /// The sentence naming the fix.
        detail: String,
    },
    /// The line will not synthesize: empty, or longer than the provider takes.
    BadRequest {
        /// Which provider.
        provider: String,
        /// Why.
        detail: String,
    },
    /// The provider ran and produced nothing usable.
    NoAudio {
        /// Which provider.
        provider: String,
        /// The opening of the line, to identify the row.
        text: String,
        /// What the provider said about it.
        detail: String,
    },
    /// Anything from the process boundary — spawn, timeout, non-zero exit.
    Process(Box<spoonstill_media::MediaError>),
    /// A file could not be written or read.
    Io {
        /// Which file.
        path: std::path::PathBuf,
        /// The operating system's reason.
        source: std::io::Error,
    },
}

impl std::fmt::Display for TtsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TtsError::NoSuchProvider { provider, known } => write!(
                f,
                "no text-to-speech provider called {provider:?} — this build has {}",
                known.join(", ")
            ),
            TtsError::Unavailable { provider, detail } => {
                write!(f, "the {provider} voice service is not usable: {detail}")
            }
            TtsError::BadRequest { provider, detail } => {
                write!(f, "{provider} will not speak this line: {detail}")
            }
            TtsError::NoAudio {
                provider,
                text,
                detail,
            } => write!(f, "{provider} produced no audio for {text:?}: {detail}"),
            TtsError::Process(e) => write!(f, "{e}"),
            TtsError::Io { path, source } => {
                write!(f, "{}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for TtsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TtsError::Process(e) => Some(e),
            TtsError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<spoonstill_media::MediaError> for TtsError {
    fn from(e: spoonstill_media::MediaError) -> Self {
        TtsError::Process(Box::new(e))
    }
}

/// A thing that can say a line.
///
/// `Send + Sync` because the audio pool of D-044 speaks several lines at once
/// and every worker shares one provider.
pub trait Provider: Send + Sync {
    /// The name used in `project.yaml` and in the manifest.
    fn id(&self) -> &str;

    /// Whether this provider can run right now, and the fix if it cannot.
    ///
    /// Called before a batch rather than per scene: discovering at scene 340 of
    /// 500 that a binary is missing has wasted forty minutes (D-002).
    fn availability(&self) -> Availability;

    /// Every voice on offer.
    ///
    /// # Errors
    ///
    /// [`TtsError::Unavailable`] or [`TtsError::Process`] if the list cannot be
    /// fetched.
    fn voices(&self) -> Result<Vec<Voice>, TtsError>;

    /// The voice this provider uses when a project names none.
    ///
    /// `tts.voice` defaults to the literal string `default`, and every provider
    /// resolves that to something concrete — Edge's own default changes between
    /// releases, so ours is named rather than inherited (see `edge::DEFAULT_VOICE`).
    /// A control surface has to be able to **say** what that something is:
    /// "default" is not an answer to "whose voice will I hear", and an operator
    /// who cannot answer that picks the first row in a list of three hundred.
    ///
    /// Empty when a provider genuinely has no opinion.
    fn default_voice(&self) -> &str {
        ""
    }

    /// Install this provider's tooling, and say what was run.
    ///
    /// Telling an operator to open a terminal and run `pip install edge-tts` is
    /// telling them the program cannot do the one thing it just asked for. A
    /// provider that is a command line tool knows how to fetch itself; one that
    /// is an API key does not, and says so.
    ///
    /// This is **not** D-012's forbidden runtime download. That rule is about
    /// the *renderer* quietly fetching an encoder mid-run, producing output
    /// nobody can reproduce. This runs only when an operator presses a button
    /// that says install, before any project is rendered, and it fetches
    /// through the platform's own package manager rather than pulling a binary
    /// from us.
    ///
    /// # Errors
    ///
    /// [`TtsError::Unavailable`] when the provider cannot install itself, with
    /// the sentence saying what the operator must do instead.
    fn install(&self) -> Result<String, TtsError> {
        Err(TtsError::Unavailable {
            provider: self.id().to_owned(),
            detail: "this provider cannot install itself".to_owned(),
        })
    }

    /// Say `request` into `destination`, which the provider creates.
    ///
    /// The destination's parent already exists. On any error the provider
    /// leaves no file behind — a partial artifact is worse than none, because
    /// the caller's cache would treat it as a hit forever (D-042's shape).
    ///
    /// # Errors
    ///
    /// Any [`TtsError`].
    fn speak(&self, request: &Request<'_>, destination: &Path) -> Result<Spoken, TtsError>;
}

/// The providers this build has.
///
/// A `Vec` of trait objects rather than a `match` on a name: adding one is
/// adding a line here and a module, and no provider's code is edited to make
/// room for another (the `MoneyPrinterTurbo/app/services/voice.py` mistake).
#[must_use]
pub fn providers() -> Vec<Box<dyn Provider>> {
    vec![Box::new(edge::Edge::from_env())]
}

/// Find one by the name a project used.
///
/// # Errors
///
/// [`TtsError::NoSuchProvider`], naming every provider that does exist —
/// because the operator's next question is always "then what can I write here".
pub fn provider(id: &str) -> Result<Box<dyn Provider>, TtsError> {
    let all = providers();
    let known: Vec<String> = all.iter().map(|p| p.id().to_owned()).collect();
    all.into_iter()
        .find(|p| p.id().eq_ignore_ascii_case(id))
        .ok_or_else(|| TtsError::NoSuchProvider {
            provider: id.to_owned(),
            known,
        })
}

/// The first few words of a line, for an error message or a log.
///
/// **Never the whole line.** A diagnostics bundle is a file the operator sends
/// to us (D-016), and their script is their content — enough to identify the
/// row, not enough to reconstruct the film.
#[must_use]
pub fn opening(text: &str) -> String {
    const LIMIT: usize = 48;
    let trimmed = text.trim();
    if trimmed.chars().count() <= LIMIT {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(LIMIT).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// D-073 renamed every crate in this workspace. A crate whose `[package]`
    /// name drifts out of the family is a rename that was applied by hand and
    /// missed a spot — which is the exact failure that lost the name once
    /// already. Cheap to assert, so assert it.
    #[test]
    fn crate_is_part_of_the_spoonstill_family() {
        assert!(
            CRATE_NAME.starts_with("spoonstill-"),
            "expected a spoonstill-* crate, found {CRATE_NAME:?}"
        );
    }

    #[test]
    fn an_unknown_provider_names_the_ones_that_exist() {
        let message = match provider("elevenlabs") {
            Err(error) => error.to_string(),
            Ok(found) => panic!("{} is not built yet", found.id()),
        };
        assert!(message.contains("elevenlabs"), "{message}");
        assert!(
            message.contains("edge"),
            "the message lists what does exist"
        );
    }

    #[test]
    fn the_default_provider_is_findable_by_name_in_any_case() {
        assert_eq!(provider("edge").expect("built in").id(), "edge");
        assert_eq!(provider("Edge").expect("built in").id(), "edge");
    }

    #[test]
    fn an_opening_identifies_the_row_without_quoting_the_script() {
        let long = "The harbour was empty by the time we arrived, and the light \
                    had gone out of the water entirely.";
        let shown = opening(long);
        assert!(shown.ends_with('…'));
        assert!(shown.chars().count() < long.chars().count());
        assert_eq!(opening("  short line  "), "short line");
    }

    #[test]
    fn settings_are_looked_up_by_name() {
        let values = vec![("rate".to_owned(), "+10%".to_owned())];
        let request = Request {
            text: "hello",
            voice: "en-US-AvaNeural",
            settings: &values,
        };
        assert_eq!(request.setting("rate"), Some("+10%"));
        assert_eq!(request.setting("pitch"), None);
    }
}
