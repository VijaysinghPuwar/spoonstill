//! A provider's voice catalogue, remembered from the last time it was fetched
//! (D-213).
//!
//! `edge-tts --list-voices` crosses the same network as a line of speech, so
//! when Microsoft's service turns this machine away after a few hundred lines
//! (D-199) the **Voice screen** goes with it. The author's own `runs.csv`, for
//! 2026-11-01: fourteen `voices` failures over four hours, every one while a
//! render was waiting out a throttle — so the screen whose whole job is
//! choosing a voice was unusable for exactly as long as the render was, and
//! choosing a voice needs no network at all.
//!
//! So a catalogue that arrives is written beside `settings.yaml`, and one that
//! does not is answered from that copy — **said out loud**, never presented as
//! fresh. The saved copy is only ever what the provider itself once printed: a
//! voice is never invented, and a machine that has never fetched one still
//! gets the error.
//!
//! The file is tab-separated, one voice per line, because a voice id, a
//! locale and a gender carry no tab and the note has its tabs and newlines
//! folded to spaces on the way in.

use std::path::{Path, PathBuf};

use spoonstill_tts::{Provider, TtsError, Voice};

/// What the catalogue screen is drawn from.
#[derive(Debug, Clone)]
pub struct Catalogue {
    /// Every voice the provider offers, as it last said.
    pub voices: Vec<Voice>,
    /// `None` when the provider answered just now. Otherwise why this is the
    /// saved copy — one sentence an operator can read, with the provider's own
    /// error after it.
    pub remembered: Option<String>,
}

/// The file a provider's catalogue is remembered in.
fn path_in(dir: &Path, provider: &str) -> PathBuf {
    dir.join(format!("voices-{provider}.tsv"))
}

/// Ask `provider` for its voices, remembering the answer in this machine's
/// state directory and falling back to it when the provider cannot answer.
///
/// # Errors
///
/// The provider's own error, when it cannot answer **and** nothing was ever
/// remembered.
pub fn voices(provider: &dyn Provider) -> Result<Catalogue, TtsError> {
    voices_in(
        spoonstill_state::runs::config_dir().as_deref(),
        provider.id(),
        provider.voices(),
    )
}

/// [`voices`], with the directory and the provider's answer as arguments so a
/// test reaches both branches without a network or the operator's own state
/// (D-144's shape).
///
/// # Errors
///
/// As [`voices`].
pub fn voices_in(
    dir: Option<&Path>,
    provider: &str,
    fetched: Result<Vec<Voice>, TtsError>,
) -> Result<Catalogue, TtsError> {
    match fetched {
        Ok(voices) => {
            // Best effort: a catalogue that could not be saved is still the
            // catalogue, and a preference must never fail over a cache.
            if let Some(dir) = dir {
                let _ = std::fs::create_dir_all(dir);
                let _ = crate::machine::replace_file(&path_in(dir, provider), &encode(&voices));
            }
            Ok(Catalogue {
                voices,
                remembered: None,
            })
        }
        Err(error) => {
            let saved = dir
                .and_then(|dir| std::fs::read_to_string(path_in(dir, provider)).ok())
                .map(|text| decode(&text))
                .filter(|voices| !voices.is_empty());
            match saved {
                Some(voices) => Ok(Catalogue {
                    voices,
                    remembered: Some(format!(
                        "{provider} could not list its voices just now, so this is the list \
                         it gave last time — choosing one works the same. ({error})"
                    )),
                }),
                None => Err(error),
            }
        }
    }
}

fn clean(field: &str) -> String {
    field.replace(['\t', '\n', '\r'], " ")
}

fn encode(voices: &[Voice]) -> String {
    let mut out = String::new();
    for voice in voices {
        out.push_str(&clean(&voice.id));
        out.push('\t');
        out.push_str(&clean(&voice.locale));
        out.push('\t');
        out.push_str(&clean(&voice.gender));
        out.push('\t');
        out.push_str(&clean(&voice.note));
        out.push('\n');
    }
    out
}

/// A line that does not have four fields is skipped rather than guessed at: a
/// file damaged by hand loses its damaged rows and keeps the rest.
fn decode(text: &str) -> Vec<Voice> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let id = fields.next()?.trim();
            let locale = fields.next()?;
            let gender = fields.next()?;
            let note = fields.next()?;
            (!id.is_empty() && fields.next().is_none()).then(|| Voice {
                id: id.to_owned(),
                locale: locale.to_owned(),
                gender: gender.to_owned(),
                note: note.to_owned(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(id: &str, note: &str) -> Voice {
        Voice {
            id: id.to_owned(),
            locale: id.split('-').take(2).collect::<Vec<_>>().join("-"),
            gender: "Female".to_owned(),
            note: note.to_owned(),
        }
    }

    fn refused() -> TtsError {
        TtsError::Unavailable {
            provider: "edge".to_owned(),
            detail: "Microsoft's voice service stopped accepting connections".to_owned(),
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "spoonstill-catalogue-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_refused_list_is_answered_from_the_last_one_and_says_so() {
        let dir = scratch("refused");
        let fresh = vec![
            voice("en-US-AvaNeural", "Friendly,\tPositive"),
            voice("hi-IN-MadhurNeural", ""),
        ];
        let first = voices_in(Some(&dir), "edge", Ok(fresh.clone())).expect("fetched");
        assert!(
            first.remembered.is_none(),
            "a fresh list is not a remembered one"
        );

        let later = voices_in(Some(&dir), "edge", Err(refused())).expect("remembered");
        assert_eq!(later.voices.len(), 2);
        assert_eq!(later.voices[0].id, "en-US-AvaNeural");
        assert_eq!(
            later.voices[0].note, "Friendly, Positive",
            "a tab cannot split a row"
        );
        assert_eq!(later.voices[1].note, "");
        let said = later.remembered.expect("it says it is the saved copy");
        assert!(said.contains("last time"), "{said}");
        assert!(
            said.contains("stopped accepting"),
            "the provider's own error is kept: {said}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn nothing_remembered_is_still_the_error() {
        let dir = scratch("never");
        assert!(voices_in(Some(&dir), "edge", Err(refused())).is_err());
        assert!(voices_in(None, "edge", Err(refused())).is_err());
    }

    #[test]
    fn providers_do_not_share_a_memory() {
        let dir = scratch("separate");
        voices_in(Some(&dir), "edge", Ok(vec![voice("en-US-AvaNeural", "")])).expect("fetched");
        assert!(voices_in(Some(&dir), "other", Err(refused())).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_damaged_row_is_dropped_and_the_rest_kept() {
        let rows = decode("en-US-AvaNeural\ten-US\tFemale\t\nbroken line\n\t\t\t\n");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "en-US-AvaNeural");
    }
}
