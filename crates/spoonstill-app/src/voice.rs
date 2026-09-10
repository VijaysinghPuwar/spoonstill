//! Which voice a render will use, and who chose it (D-162, D-164).
//!
//! One rule, reached by both control surfaces. It lived in `apps/desktop` for
//! exactly one session, which was long enough to show why it cannot: the
//! machine's fallback voice is read here, so a rule that only the window could
//! call was a setting only the window could honour.

use serde::Serialize;

/// Which of the four answers decided the voice, so a surface can say *whose*
/// choice the operator is looking at (D-162).
///
/// The window has always been able to name the voice and never able to say
/// where the name came from, and the two that matter most looked identical:
/// a project that asks for a voice and a project that asks for nothing both
/// displayed a real voice id, so `en-US-AvaNeural` read as somebody's decision
/// when it was the absence of one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum VoiceOrigin {
    /// The Voice screen's pick, for this render only.
    Run,
    /// `project.yaml`'s own `tts.voice`.
    Project,
    /// The machine's fallback, from Settings (D-092).
    Fallback,
    /// Nothing named one. The renderer picks per line from the script it is
    /// written in (D-158), so there is no single answer to display.
    Unchosen,
}

/// The voice a render will use, and who said so.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VoiceChoice {
    /// The voice that will speak. Empty only when nobody named one *and* the
    /// provider could not be reached to say what it would fall back to.
    pub voice: String,
    /// Which of the four answers decided it.
    pub origin: VoiceOrigin,
    /// The `voice` field of a render request: `Some` only for the two answers
    /// that are the window's to make. `None` hands the question back to the
    /// renderer, which reads `project.yaml` and then D-158's script rule —
    /// so the window never has to restate a rule it does not own.
    ///
    /// **This is why the type exists.** The Render screen used to display
    /// `effectiveVoice()` and the render request used to build
    /// `chosenVoice || (projectNamesNoVoice() ? appDefaultVoice : null)` —
    /// two spellings of one rule, in two files, with nothing asserting they
    /// agreed. What is shown is now computed from the same value that is sent.
    #[serde(rename = "overrideForRender")]
    pub override_for_render: Option<String>,
    /// Where it came from, in three or four words, for a tag beside the name.
    pub said: String,
    /// The same fact as a sentence, for the operator who wants to know what to
    /// do about it. `Remedy`'s shape (D-105): a short form to read at a glance
    /// and a long one to act on.
    pub detail: String,
    /// Whether [`Self::voice`] is *already* this machine's fallback (D-164).
    ///
    /// Not the same question as `origin == Fallback`: a voice picked for this
    /// run can also be the machine's, and the Voice screen's "use for every
    /// project" control has to know which, or it offers to set something that
    /// is already set.
    #[serde(rename = "isFallback")]
    pub is_fallback: bool,
}

/// A voice named by nobody. `project.yaml` spells that `default`
/// (`spoonstill_app::import::settings::DEFAULT_VOICE`), and an empty string is
/// the same statement from a page that has not loaded one yet.
pub fn names_a_voice(value: &str) -> bool {
    let trimmed = value.trim();
    !trimmed.is_empty() && trimmed != crate::import::settings::DEFAULT_VOICE
}

/// Resolve the four answers into one, in the order D-092 set: this run's pick,
/// then the project's own, then the machine's fallback, and only then nothing.
///
/// Pure, and every input is a string the page already holds — the point is not
/// that Rust can reach them, it is that the rule is written **once** and can be
/// tested (D-010).
pub fn resolve(
    chosen: Option<&str>,
    project_voice: &str,
    fallback: Option<&str>,
    provider_default: &str,
) -> VoiceChoice {
    // Kept for the whole function: whether the voice being *shown* is the
    // machine's is a different question from whether the machine's is the one
    // being used, and only the second is what `origin` answers.
    let fallback_names = fallback.map(str::trim).filter(|v| names_a_voice(v));

    if let Some(voice) = chosen.filter(|v| names_a_voice(v)) {
        return VoiceChoice {
            voice: voice.trim().to_owned(),
            origin: VoiceOrigin::Run,
            override_for_render: Some(voice.trim().to_owned()),
            is_fallback: fallback_names.is_some_and(|f| f == voice.trim()),
            said: "Chosen for this render".to_owned(),
            detail: "This voice reads every written line in the next render. \
                     Nothing here writes to project.yaml, so it lasts until you \
                     choose again."
                .to_owned(),
        };
    }

    if names_a_voice(project_voice) {
        return VoiceChoice {
            voice: project_voice.trim().to_owned(),
            origin: VoiceOrigin::Project,
            override_for_render: None,
            is_fallback: fallback_names.is_some_and(|f| f == project_voice.trim()),
            said: "Named in project.yaml".to_owned(),
            detail: "This project asks for this voice itself, so every render of \
                     it sounds the same on any machine."
                .to_owned(),
        };
    }

    if let Some(voice) = fallback.filter(|v| names_a_voice(v)) {
        return VoiceChoice {
            voice: voice.trim().to_owned(),
            origin: VoiceOrigin::Fallback,
            override_for_render: Some(voice.trim().to_owned()),
            is_fallback: true,
            said: "Your fallback voice".to_owned(),
            detail: "This project names no voice, so the one set in Settings is \
                     used. Every project that names none gets this same voice."
                .to_owned(),
        };
    }

    // The one case the window could not say out loud. `provider_default` is
    // what an English line gets; a line in another script gets that script's
    // voice instead (D-158), so naming one voice here would be a guess at the
    // project's content. The sentence says what to do about it, because this
    // is the state that renders ten parts of one film in ten voices.
    let detail = if provider_default.is_empty() {
        "Nothing has named a voice. Each line will be read in whatever voice \
         its own script suggests. Choose one here, or set a fallback in \
         Settings, to keep several projects sounding the same."
            .to_owned()
    } else {
        format!(
            "Nothing has named a voice. English lines will be read by \
             {provider_default}, and a line written in another script gets that \
             script's voice. Choose one here, or set a fallback in Settings, to \
             keep several projects sounding the same."
        )
    };
    VoiceChoice {
        voice: provider_default.trim().to_owned(),
        origin: VoiceOrigin::Unchosen,
        override_for_render: None,
        is_fallback: false,
        said: "Nobody chose this".to_owned(),
        detail,
    }
}

/// The same question, with the machine's fallback read from disk (D-164).
///
/// Every control surface asks it this way. The fallback is **read here rather
/// than passed in**, and that is a fix: the window filled its copy only when
/// the Settings screen had been opened, so on any launch that went straight
/// from Home into a project the setting was ignored — saved, displayed, and
/// doing nothing. A caller cannot forget to load what it never holds.
#[must_use]
pub fn for_run(chosen: Option<&str>, project_voice: &str, provider_default: &str) -> VoiceChoice {
    resolve(
        chosen,
        project_voice,
        crate::machine::load().default_voice.as_deref(),
        provider_default,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four answers, in the order D-092 set them, each one displacing the
    /// one below it (D-162).
    ///
    /// Written as a table because the defect this exists to prevent is a
    /// *precedence* change — someone adding a fifth answer, or moving the
    /// fallback above the project — and a precedence change is invisible in
    /// any test that supplies only one input at a time.
    #[test]
    fn the_voice_is_decided_by_the_first_answer_that_names_one() {
        let cases = [
            // chosen        project         fallback      provider   → voice, origin
            (
                Some("run"),
                "proj",
                Some("fall"),
                "prov",
                "run",
                VoiceOrigin::Run,
            ),
            (
                None,
                "proj",
                Some("fall"),
                "prov",
                "proj",
                VoiceOrigin::Project,
            ),
            (
                None,
                "default",
                Some("fall"),
                "prov",
                "fall",
                VoiceOrigin::Fallback,
            ),
            (None, "default", None, "prov", "prov", VoiceOrigin::Unchosen),
            // `default` is the word `project.yaml` uses for "nobody said", and
            // it is not a voice (D-086). An empty string is the same statement
            // from a page that has not loaded the project yet.
            (
                None,
                "",
                Some("fall"),
                "prov",
                "fall",
                VoiceOrigin::Fallback,
            ),
            (Some(""), "proj", None, "prov", "proj", VoiceOrigin::Project),
            (
                Some("default"),
                "default",
                None,
                "prov",
                "prov",
                VoiceOrigin::Unchosen,
            ),
            (
                None,
                "default",
                Some(" "),
                "prov",
                "prov",
                VoiceOrigin::Unchosen,
            ),
            // A voice id is not a path, so trimming one is safe — unlike a
            // folder name, which D-089 says is never trimmed.
            (Some(" run "), "proj", None, "prov", "run", VoiceOrigin::Run),
        ];

        for (chosen, project, fallback, provider, voice, origin) in cases {
            let got = resolve(chosen, project, fallback, provider);
            assert_eq!(
                (got.voice.as_str(), got.origin),
                (voice, origin),
                "chosen={chosen:?} project={project:?} fallback={fallback:?}"
            );
        }
    }

    /// What is displayed and what is sent come out of one call, and the two
    /// answers the window does not own are handed back rather than restated.
    ///
    /// `Project` and `Unchosen` both send `None`: the renderer reads
    /// `project.yaml` itself, and D-158 picks a voice per line from the script
    /// it is written in. Sending `provider_default` for `Unchosen` would look
    /// identical on this screen and would speak a Hindi project in English.
    #[test]
    fn only_the_two_answers_the_window_owns_are_sent_as_an_override() {
        assert_eq!(
            resolve(Some("run"), "proj", Some("fall"), "prov").override_for_render,
            Some("run".to_owned())
        );
        assert_eq!(
            resolve(None, "default", Some("fall"), "prov").override_for_render,
            Some("fall".to_owned())
        );
        assert_eq!(
            resolve(None, "proj", Some("fall"), "prov").override_for_render,
            None,
            "project.yaml's own voice is the renderer's to read"
        );
        assert_eq!(
            resolve(None, "default", None, "prov").override_for_render,
            None,
            "sending the provider's default would overrule D-158's script rule"
        );

        // And an override, when there is one, is the voice being displayed.
        for (chosen, project, fallback) in [
            (Some("run"), "proj", Some("fall")),
            (None, "default", Some("fall")),
        ] {
            let got = resolve(chosen, project, fallback, "prov");
            assert_eq!(
                got.override_for_render.as_deref(),
                Some(got.voice.as_str()),
                "the render would use a voice the screen does not name"
            );
        }
    }

    /// Every answer says which it is, and the one that means "nobody decided"
    /// does not read like a decision.
    ///
    /// This is the defect (D-091's class): the Voice screen tagged an
    /// unchosen voice `From project.yaml`, which is a *false* statement about
    /// a file, and the ten-parts-of-one-film case is exactly the case where
    /// believing it costs a re-render.
    #[test]
    fn an_unchosen_voice_does_not_claim_anybody_chose_it() {
        let unchosen = resolve(None, "default", None, "en-US-AvaNeural");
        assert!(
            !unchosen.said.contains("project.yaml") && !unchosen.detail.contains("asks for"),
            "an unchosen voice is claiming the project named it: {unchosen:?}"
        );
        assert!(
            unchosen.detail.contains("Settings"),
            "the state that renders ten projects in ten voices has to say what \
             to do about it: {}",
            unchosen.detail
        );
        // The provider's own fallback is named, because an operator who does
        // nothing will hear it — but as an example, not as the whole answer.
        assert!(
            unchosen.detail.contains("en-US-AvaNeural"),
            "{}",
            unchosen.detail
        );
        assert!(
            unchosen.detail.contains("script"),
            "D-158 means there is no single voice to promise here: {}",
            unchosen.detail
        );

        // No two answers share a tag, or the tag distinguishes nothing.
        let said: std::collections::BTreeSet<String> = [
            resolve(Some("run"), "proj", Some("fall"), "prov"),
            resolve(None, "proj", Some("fall"), "prov"),
            resolve(None, "default", Some("fall"), "prov"),
            unchosen,
        ]
        .into_iter()
        .map(|choice| choice.said)
        .collect();
        assert_eq!(said.len(), 4, "two origins read the same: {said:?}");
    }

    /// "Already your fallback" is a different question from "chosen because it
    /// is your fallback", and the pin control needs the first (D-164).
    #[test]
    fn a_voice_can_be_the_machines_without_being_the_reason_it_was_chosen() {
        // Picked for this run, and it happens to be the machine's too. The
        // control must not offer to set what is already set.
        let both = resolve(Some("fall"), "proj", Some("fall"), "prov");
        assert_eq!(both.origin, VoiceOrigin::Run);
        assert!(both.is_fallback, "the pin would offer to set it again");

        // Picked for this run, and it is not.
        assert!(!resolve(Some("run"), "proj", Some("fall"), "prov").is_fallback);

        // The project's own voice, which also happens to be the machine's.
        assert!(resolve(None, "fall", Some("fall"), "prov").is_fallback);
        assert!(!resolve(None, "proj", Some("fall"), "prov").is_fallback);

        // Reached *because* it is the fallback — trivially true, and the case
        // that used to be the only one anybody would have thought to check.
        assert!(resolve(None, "default", Some("fall"), "prov").is_fallback);

        // Nobody chose, so nothing shown is anybody's fallback.
        assert!(!resolve(None, "default", None, "prov").is_fallback);
    }

    /// A provider that cannot be reached has no default voice to offer, and
    /// the sentence still has to be a sentence.
    ///
    /// Reachable: `provider_status` leaves `default_voice` empty when
    /// `edge-tts` is missing (D-105), which is the first run on a new machine.
    #[test]
    fn an_unreachable_provider_still_gets_a_readable_answer() {
        let choice = resolve(None, "default", None, "");
        assert_eq!(choice.origin, VoiceOrigin::Unchosen);
        assert!(choice.voice.is_empty());
        assert!(
            !choice.detail.contains("  ") && !choice.detail.contains("read by ,"),
            "an empty provider default left a hole in the sentence: {}",
            choice.detail
        );
        assert!(choice.detail.contains("Settings"), "{}", choice.detail);
    }
}
