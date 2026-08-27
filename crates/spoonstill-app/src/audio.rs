//! Resolving an [`AudioSource`] to `(normalized path, measured duration)`
//! (D-020, D-021, D-043).
//!
//! This is the join between the domain's three audio sources and the one shape
//! the renderer accepts. Every source ends at the same place:
//!
//! ```text
//! AudioSource::File   -> normalize the operator's recording into the cache
//! AudioSource::Silent -> generate a silent track of an exact sample count
//! AudioSource::Tts    -> synthesize with a provider, then normalize
//!         all three -> ffprobe the artifact -> (path, duration)
//! ```
//!
//! The renderer never learns which branch a scene took. That is D-020's actual
//! test — "adding a fourth source must not touch the renderer" — and slice 4
//! is the proof: speaking a line was added here, and nothing downstream of
//! `(path, duration)` was touched to make it work.
//!
//! ## The cache is content-addressed, and that is a correctness requirement
//!
//! D-043: never key on a path. A file that moves must keep its cache entry,
//! and two scenes pointing at the same recording must share one — with BYOK a
//! cache miss costs the operator money, and at M2 it already costs them a
//! re-encode per scene.
//!
//! So the key is the *content* plus the normalization profile:
//!
//! | Source | Key |
//! |---|---|
//! | `File` | `hash(file bytes, profile)` |
//! | `Silent` | `hash("silent", sample count, profile)` |
//! | `Tts` | `hash(text, provider, voice, settings, profile)` |
//!
//! The TTS key is the one that matters most: with BYOK a miss costs the
//! operator money, so it must hit for a line that has not changed even when the
//! project has been renamed, moved, or re-imported. Nothing in it is a path.
//! The provider's own raw output is kept beside the normalized artifact for the
//! same reason — re-normalizing must never mean speaking the line again.
//!
//! ## What lives here and what waits for M3
//!
//! The cache is a **directory of content-named files**, with no index. That is
//! deliberate for M2: `.spoonstill/state.db` is M3's deliverable, and a cache
//! that needs a database to be readable cannot be inspected with `ls` when
//! something goes wrong. A cache hit is still *measured* rather than trusted
//! (D-021) — the artifact is probed on every run, which is what catches a
//! half-written or hand-corrupted entry.

use std::io::Read;
use std::path::{Path, PathBuf};

use spoonstill_core::diagnostics::Diagnostics;
use spoonstill_core::hash::{Fnv1a, fnv1a_fields};
use spoonstill_core::project::MAX_SCENE_SECONDS;
use spoonstill_core::{AudioSource, SAMPLE_RATE, STATE_DIR};
use spoonstill_media::audio::{self, NORMALIZED_EXT, NORMALIZED_PROFILE, Shape, Trim};
use spoonstill_media::{MediaError, Tools};
use spoonstill_tts::TtsError;

/// Where normalized audio lives, under [`STATE_DIR`].
pub const AUDIO_CACHE_DIR: &str = "cache/audio";

/// How much provider padding a spoken scene keeps, and the fact that a
/// supplied recording keeps all of its own.
///
/// This is the project's policy, resolved once and passed down, rather than a
/// constant read inside the resolver — the values are settings (`tts.trim_head`
/// and `tts.trim_tail`) and they are part of the cache key, so they have to
/// travel with the request that uses them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioPolicy {
    /// Applied to synthesized speech only.
    pub trim: Trim,
}

/// Seconds of a provider's leading silence to keep (D-084).
pub const DEFAULT_TRIM_HEAD: f64 = 0.10;

/// Seconds of its trailing silence to keep.
///
/// Not zero: a cut landing on the final consonant sounds like a dropped frame.
/// A quarter of a beat reads as punctuation.
pub const DEFAULT_TRIM_TAIL: f64 = 0.25;

impl Default for AudioPolicy {
    fn default() -> Self {
        AudioPolicy {
            trim: Trim {
                head_seconds: DEFAULT_TRIM_HEAD,
                tail_seconds: DEFAULT_TRIM_TAIL,
            },
        }
    }
}

/// Extension for a provider's raw output, before normalization.
///
/// Deliberately not `.mp3`: providers differ, and FFmpeg picks a demuxer by
/// sniffing content rather than by reading an extension, so a name that claims
/// a format would be a lie with no upside.
const SPOKEN_EXT: &str = "spoken";

/// Chunk size for hashing a source file.
///
/// Big enough that the syscall overhead disappears, small enough that hashing
/// eight narrations at once is 512 KB of buffers rather than eight whole files.
const HASH_CHUNK: usize = 64 * 1024;

/// A resolved narration: the pair the renderer consumes, and nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedAudio {
    /// The normalized artifact, in the cache.
    pub path: PathBuf,
    /// Its duration in seconds, measured on that artifact (D-021).
    pub duration: f64,
    /// Whether the artifact was already there (D-043's whole point).
    pub reused: bool,
    /// Which source this came from, for the log and the review grid.
    pub kind: &'static str,
}

/// Why a scene's narration could not be resolved.
#[derive(Debug)]
pub enum AudioError {
    /// The line could not be spoken.
    ///
    /// A named, typed refusal rather than a panic or a silent silent-scene:
    /// substituting silence for a line somebody wrote would produce a film
    /// that looks finished and is not (D-020).
    Tts(Box<TtsError>),
    /// A `File` scene reached this layer without a resolved path. A bug, not
    /// operator error — import returns one for every `File` scene it keeps.
    NoResolvedPath,
    /// The measured duration is not one we will render (D-021).
    DurationOutOfRange {
        /// What was measured, or declared.
        seconds: f64,
    },
    /// Anything from the process boundary.
    Media(Box<MediaError>),
    /// Reading the source in order to key the cache.
    Io {
        /// The file being read.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::Tts(e) => write!(f, "cannot speak its line — {e}"),
            AudioError::NoResolvedPath => {
                f.write_str("has a supplied narration whose path was never resolved")
            }
            AudioError::DurationOutOfRange { seconds } => write!(
                f,
                "resolves to {seconds:.3}s of narration, which is not renderable — \
                 it must be above zero and at most {MAX_SCENE_SECONDS:.0}s (D-021)"
            ),
            AudioError::Media(e) => write!(f, "{e}"),
            AudioError::Io { path, source } => {
                write!(f, "could not read {}: {source}", path.display())
            }
        }
    }
}

impl std::error::Error for AudioError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AudioError::Tts(e) => Some(e),
            AudioError::Media(e) => Some(e),
            AudioError::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<TtsError> for AudioError {
    fn from(e: TtsError) -> Self {
        AudioError::Tts(Box::new(e))
    }
}

impl From<MediaError> for AudioError {
    fn from(e: MediaError) -> Self {
        AudioError::Media(Box::new(e))
    }
}

/// The project's normalized-audio cache.
#[derive(Debug, Clone)]
pub struct AudioCache {
    directory: PathBuf,
}

impl AudioCache {
    /// The cache belonging to a project root (D-013).
    #[must_use]
    pub fn in_project(root: &Path) -> Self {
        AudioCache {
            directory: root.join(STATE_DIR).join(AUDIO_CACHE_DIR),
        }
    }

    /// Where the cache is, for the diagnostics bundle and for `ls`.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The artifact path for one key.
    #[must_use]
    pub fn path_for(&self, kind: &str, key: u64) -> PathBuf {
        self.directory
            .join(format!("{kind}-{key:016x}.{NORMALIZED_EXT}"))
    }

    /// Where a provider's raw output lives.
    ///
    /// **Keyed on the words, not on the normalization.** This is the whole
    /// point of keeping it: changing the normalization profile, or the trim, or
    /// the loudness target must re-*normalize* every line and re-*speak* none
    /// of them. With BYOK the difference is the operator's money; with any
    /// provider it is their afternoon.
    #[must_use]
    pub fn spoken_path(&self, key: u64) -> PathBuf {
        self.directory
            .join(format!("spoken-{key:016x}.{SPOKEN_EXT}"))
    }
}

/// The sample line a voice audition speaks when the operator has not written
/// one of their own.
///
/// Short on purpose: an audition is a decision about timbre, and a long line
/// is a long wait for the same decision.
pub const PREVIEW_LINE: &str = "This is how this voice will read your narration.";

/// Speak one line in a voice the operator is considering, and hand back the
/// artifact to play.
///
/// This is an **audition**, not a render: nothing about the project changes,
/// `project.yaml` is not written (D-013), and no scene is bound to the voice.
/// It exists because choosing a voice by reading `en-GB-RyanNeural` off a list
/// is not choosing a voice.
///
/// It goes through the same cache and the same normalization as a real scene
/// (D-084), for two reasons. The audition sounds like the film will sound,
/// levelled and trimmed the same way — an audition at a different loudness is
/// a lie about the result. And auditioning the same voice twice costs nothing,
/// which is the difference between comparing six voices and settling for the
/// first one that works.
///
/// # Errors
///
/// [`AudioError`] — an unreachable provider, an unknown voice, an empty line.
pub fn preview(
    root: &Path,
    provider: &str,
    voice: &str,
    text: &str,
) -> Result<ResolvedAudio, AudioError> {
    // The trim is the project's, read from its own settings, so that the
    // audition is trimmed exactly as the render would trim it. A project whose
    // settings will not parse still auditions — with the defaults, and with the
    // problem left for `validate` to report, because a broken `fps` is not a
    // reason to refuse to play a voice.
    let settings = crate::import::settings::load(root)
        .map(|(settings, _)| settings)
        .unwrap_or_default();
    let policy = AudioPolicy {
        trim: Trim {
            head_seconds: settings.trim_head,
            tail_seconds: settings.trim_tail,
        },
    };

    let source = AudioSource::Tts {
        text: text.trim().to_owned(),
        provider: spoonstill_core::project::ProviderId(provider.to_owned()),
        voice: spoonstill_core::project::VoiceId(voice.to_owned()),
        settings: spoonstill_core::project::TtsSettings::default(),
    };

    // The log is opened before the work, like a render's is (D-016): an
    // audition that fails is exactly as diagnosable as a scene that fails.
    let log = spoonstill_state::FileLog::open(root).ok();
    let sink: &dyn Diagnostics = log
        .as_ref()
        .map_or(&spoonstill_core::diagnostics::Noop, |l| {
            l as &dyn Diagnostics
        });

    resolve(
        &AudioCache::in_project(root),
        &Tools::from_env(),
        &source,
        None,
        &policy,
        sink,
    )
}

/// The cache key of the raw speech for a spoken source, or `None` for a source
/// that is not spoken.
///
/// Exposed so that a caller reasoning about what is already spoken — the
/// pre-flight check of D-094, a diagnostic, a future queue — asks the same
/// function [`resolve`] asks, rather than growing a second copy of the key
/// that drifts from this one on the next change to what a key contains.
#[must_use]
pub fn speech_key(source: &AudioSource) -> Option<u64> {
    match source {
        AudioSource::Tts {
            text,
            provider,
            voice,
            settings,
        } => Some(key_for_speech(
            text,
            &provider.0,
            &voice.0,
            &settings.sorted(),
        )),
        _ => None,
    }
}

/// Which provider this scene will actually have to call, if any (D-094).
///
/// `None` for a recording, for silence, and — the case that makes this worth a
/// function — for a line whose raw speech is already in the cache. A re-render
/// of a project whose lines are all spoken needs no voice service at all, and
/// must not be refused by a check that only looked at whether the scene *has*
/// a script.
///
/// The key is the same one [`resolve`] would compute, from the same function.
/// A pre-flight check that guessed at the key would be a second cache with its
/// own bugs.
#[must_use]
pub fn provider_needed<'a>(cache: &AudioCache, source: &'a AudioSource) -> Option<&'a str> {
    let AudioSource::Tts { provider, .. } = source else {
        return None;
    };
    let key = speech_key(source)?;
    if cache.spoken_path(key).exists() {
        return None;
    }
    Some(&provider.0)
}

/// Resolve one scene's narration.
///
/// `file` is the canonical, contained path import produced for an
/// [`AudioSource::File`] scene (D-054); it is ignored for the other sources.
///
/// # Errors
///
/// [`AudioError`], which the caller attaches a scene id to. Nothing here
/// panics on operator input, and nothing here substitutes a working default
/// for a source it cannot produce.
pub fn resolve(
    cache: &AudioCache,
    tools: &Tools,
    source: &AudioSource,
    file: Option<&Path>,
    policy: &AudioPolicy,
    log: &dyn Diagnostics,
) -> Result<ResolvedAudio, AudioError> {
    let kind = source.kind();

    // The key and the work are decided together, before anything touches the
    // disk: an unresolvable source must fail here rather than after a cache
    // lookup that could never have hit.
    let (key, work) = match source {
        AudioSource::File { .. } => {
            let original = file.ok_or(AudioError::NoResolvedPath)?.to_path_buf();
            (key_for_file(&original)?, Work::Normalize(original))
        }
        AudioSource::Silent { seconds } => {
            let seconds = *seconds;
            if !(seconds.is_finite() && seconds > 0.0 && seconds <= MAX_SCENE_SECONDS) {
                return Err(AudioError::DurationOutOfRange { seconds });
            }
            let samples = samples_for(seconds);
            (key_for_silence(samples), Work::Silence(samples))
        }
        AudioSource::Tts {
            text,
            provider,
            voice,
            settings,
        } => {
            if text.trim().is_empty() {
                return Err(AudioError::Tts(Box::new(TtsError::BadRequest {
                    provider: provider.to_string(),
                    detail: "the line is empty".to_owned(),
                })));
            }
            // Sorted, so that two projects that spell the same settings in a
            // different order share one cache entry rather than paying twice.
            let settings = settings.sorted();
            let speech = key_for_speech(text, &provider.0, &voice.0, &settings);
            (
                // Two keys, deliberately. The words decide what is spoken; the
                // words *and* the normalization decide what is rendered.
                key_for_normalized_speech(speech, &policy.trim),
                Work::Speak {
                    speech,
                    text,
                    provider: &provider.0,
                    voice: &voice.0,
                    settings,
                },
            )
        }
    };

    let path = cache.path_for(kind, key);

    // A hit is measured, not assumed. `measure` re-asserts the normalization
    // profile too, so an artifact from an older profile — or a truncated one —
    // is regenerated rather than believed.
    if path.exists() {
        match audio::measure(tools, &path) {
            Ok(measured) => return finish(kind, measured.path, measured.duration, true),
            Err(_) => {
                // Unusable: remove it and make it again. Not an error yet —
                // the operator does not need to know that a cache entry was
                // bad, only that their scene rendered.
                let _ = std::fs::remove_file(&path);
            }
        }
    }

    let made = match work {
        Work::Normalize(original) => {
            // `as_supplied`: the operator's own recording keeps its own ends.
            // Trimming it would be the "we fixed this for you" behaviour
            // plan.md §M2 rules out — their padding is a decision, a
            // provider's is an artifact.
            audio::normalize(tools, &original, &path, &Shape::as_supplied(), log)?
        }
        Work::Silence(samples) => audio::silence(tools, samples, &path, log)?,
        Work::Speak {
            speech,
            text,
            provider,
            voice,
            settings,
        } => {
            let spoken = cache.spoken_path(speech);
            if !spoken.exists() {
                // The provider is found before anything is written: a project
                // that names one this build does not have must say so, not
                // create a cache directory first (D-002).
                let engine = spoonstill_tts::provider(provider)?;
                std::fs::create_dir_all(cache.directory()).map_err(|source| AudioError::Io {
                    path: cache.directory().to_path_buf(),
                    source,
                })?;
                let request = spoonstill_tts::Request {
                    text,
                    voice,
                    settings: &settings.values,
                };
                let said = engine.speak(&request, &spoken)?;
                log.record(
                    &spoonstill_core::diagnostics::Event::info("tts", "spoke one line")
                        .with("provider", provider)
                        .with("voice", voice)
                        .with("bytes", said.bytes.to_string())
                        // The command, not the words: the script is the
                        // operator's content and never enters a bundle (D-016).
                        .with("command", said.how),
                );
            }
            audio::normalize(tools, &spoken, &path, &Shape::spoken(policy.trim), log)?
        }
    };
    finish(kind, made.path, made.duration, false)
}

/// What resolving this source will take, once the cache has been asked.
///
/// Borrowed from the [`AudioSource`] rather than cloned: at n=500 the lines are
/// already in memory once and there is no reason for them to be there twice.
enum Work<'a> {
    /// Normalize the operator's own recording (D-021).
    Normalize(PathBuf),
    /// Generate this many samples of silence (D-020).
    Silence(u64),
    /// Say a line, then normalize what came back (D-023).
    Speak {
        /// Key of the raw output, which depends on the words and nothing else.
        speech: u64,
        /// The words.
        text: &'a str,
        /// Which provider says them.
        provider: &'a str,
        /// In which voice.
        voice: &'a str,
        /// The provider's knobs, already in canonical order.
        settings: spoonstill_core::project::TtsSettings,
    },
}

fn finish(
    kind: &'static str,
    path: PathBuf,
    duration: f64,
    reused: bool,
) -> Result<ResolvedAudio, AudioError> {
    if !(duration.is_finite() && duration > 0.0 && duration <= MAX_SCENE_SECONDS) {
        return Err(AudioError::DurationOutOfRange { seconds: duration });
    }
    Ok(ResolvedAudio {
        path,
        duration,
        reused,
        kind,
    })
}

/// Samples for a declared duration, rounded to the nearest one.
///
/// Rounding rather than truncating: 3.0 s at 48 kHz is 144000 samples exactly,
/// and floating point is entitled to hand back 143999.99999999997.
#[must_use]
pub fn samples_for(seconds: f64) -> u64 {
    (seconds * f64::from(SAMPLE_RATE)).round().max(0.0) as u64
}

/// `hash(file bytes, normalization profile)` (D-043).
fn key_for_file(path: &Path) -> Result<u64, AudioError> {
    let mut file = std::fs::File::open(path).map_err(|source| AudioError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    // Streamed rather than read whole: a narration can be hundreds of
    // megabytes, and eight workers reading eight of them into memory to
    // compute a 64-bit number is not a trade worth making.
    let mut hash = Fnv1a::new();
    let mut buffer = vec![0_u8; HASH_CHUNK];
    loop {
        let read = file.read(&mut buffer).map_err(|source| AudioError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        if read == 0 {
            break;
        }
        hash.write(&buffer[..read]);
    }

    // The profile is mixed in with a separator, so that two files whose bytes
    // differ only where the profile string sits cannot collide.
    Ok(fnv1a_fields(&[
        &hash.finish().to_be_bytes(),
        NORMALIZED_PROFILE.as_bytes(),
    ]))
}

/// `hash("silent", samples, profile)` (D-043).
fn key_for_silence(samples: u64) -> u64 {
    fnv1a_fields(&[
        b"silent",
        &samples.to_be_bytes(),
        NORMALIZED_PROFILE.as_bytes(),
    ])
}

/// `hash(text, provider, voice, settings, profile)` (D-043).
///
/// Every field that changes the audio is in it and nothing else is — no path,
/// no scene id, no project name. Two scenes with the same line in the same
/// voice are one cache entry, in one project or in twenty.
///
/// The fields are length-prefixed rather than concatenated: without that,
/// `voice="ab", text="c"` and `voice="a", text="bc"` would hash alike, and the
/// second render would quietly reuse the first one's audio.
fn key_for_speech(
    text: &str,
    provider: &str,
    voice: &str,
    settings: &spoonstill_core::project::TtsSettings,
) -> u64 {
    let mut hash = Fnv1a::new();
    let mut field = |bytes: &[u8]| {
        hash.write(&(bytes.len() as u64).to_be_bytes());
        hash.write(bytes);
    };
    field(b"tts");
    field(text.as_bytes());
    field(provider.as_bytes());
    field(voice.as_bytes());
    for (name, value) in &settings.values {
        field(name.as_bytes());
        field(value.as_bytes());
    }
    hash.finish()
}

/// The normalized artifact's key: the speech, plus everything done to it after.
///
/// Separate from [`key_for_speech`] so that a change to the normalization —
/// the profile, the loudness target, the trim — misses this cache and hits the
/// other one. That is the property the raw file exists for.
fn key_for_normalized_speech(speech: u64, trim: &Trim) -> u64 {
    fnv1a_fields(&[
        b"tts-normalized",
        &speech.to_be_bytes(),
        &trim.head_seconds.to_bits().to_be_bytes(),
        &trim.tail_seconds.to_bits().to_be_bytes(),
        NORMALIZED_PROFILE.as_bytes(),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoonstill_core::project::{ProviderId, TtsSettings, VoiceId};

    fn tools() -> Tools {
        // Never executed by these tests: every one of them stops before a
        // process would be spawned.
        Tools::at("/nonexistent/ffmpeg", "/nonexistent/ffprobe")
    }

    fn cache() -> AudioCache {
        AudioCache::in_project(Path::new("/projects/demo"))
    }

    /// D-013: the cache is machine state, so it sits under the state dir and
    /// never beside the operator's files.
    #[test]
    fn the_cache_lives_under_the_state_directory() {
        let cache = cache();
        assert!(cache.directory().starts_with("/projects/demo/.spoonstill"));
        assert!(
            cache
                .path_for("file", 0x1234_5678_9abc_def0)
                .ends_with("file-123456789abcdef0.wav")
        );
    }

    /// The artifact name is what reaches the concat list eventually, and that
    /// list is a text format (D-052). Content-addressed names are ASCII by
    /// construction; assert it, because the alternative is discovered at
    /// render time on somebody's Unicode filename.
    #[test]
    fn cache_names_are_ascii_whatever_the_source_was_called() {
        let name = cache()
            .path_for("file", u64::MAX)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert!(name.is_ascii(), "{name}");
        assert_eq!(name, "file-ffffffffffffffff.wav");
    }

    fn spoken(text: &str, provider: &str, voice: &str) -> AudioSource {
        AudioSource::Tts {
            text: text.to_owned(),
            provider: ProviderId(provider.to_owned()),
            voice: VoiceId(voice.to_owned()),
            settings: TtsSettings::default(),
        }
    }

    /// D-020, made visible: TTS is not silently swapped for silence. A scene
    /// whose provider this build does not have must fail loudly, name the
    /// reason, and say what does exist.
    #[test]
    fn a_line_no_provider_can_speak_is_refused_by_name_rather_than_muted() {
        let error = resolve(
            &cache(),
            &tools(),
            &spoken("A line over the opening still.", "elevenlabs", "default"),
            None,
            &AudioPolicy::default(),
            &spoonstill_core::diagnostics::Noop,
        )
        .expect_err("elevenlabs is not built yet");

        assert!(matches!(error, AudioError::Tts(_)), "{error}");
        let message = error.to_string();
        assert!(message.contains("elevenlabs"), "{message}");
        assert!(
            message.contains("edge"),
            "it names what does exist: {message}"
        );
    }

    /// An empty line stops here rather than becoming a zero-length artifact
    /// that the concat would then join without complaint (D-041's shape).
    #[test]
    fn an_empty_line_is_refused_before_any_provider_is_asked() {
        let error = resolve(
            &cache(),
            &tools(),
            &spoken("   \n  ", "edge", "en-US-AvaNeural"),
            None,
            &AudioPolicy::default(),
            &spoonstill_core::diagnostics::Noop,
        )
        .expect_err("an empty line");
        assert!(error.to_string().contains("empty"), "{error}");
    }

    /// D-043: the key is the content and nothing else. These are the four
    /// changes that must move it, and the two that must not.
    #[test]
    fn the_tts_key_moves_with_everything_that_changes_the_audio() {
        let settings = TtsSettings::default();
        let base = key_for_speech("the words", "edge", "en-US-AvaNeural", &settings);

        for (what, key) in [
            (
                "text",
                key_for_speech("other words", "edge", "en-US-AvaNeural", &settings),
            ),
            (
                "provider",
                key_for_speech("the words", "other", "en-US-AvaNeural", &settings),
            ),
            (
                "voice",
                key_for_speech("the words", "edge", "en-GB-RyanNeural", &settings),
            ),
            (
                "settings",
                key_for_speech(
                    "the words",
                    "edge",
                    "en-US-AvaNeural",
                    &TtsSettings {
                        values: vec![("rate".to_owned(), "+10%".to_owned())],
                    },
                ),
            ),
        ] {
            assert_ne!(base, key, "changing the {what} must change the key");
        }

        // The same line asked for twice is one entry, whatever project it is
        // in — there is no path in the key.
        assert_eq!(
            base,
            key_for_speech("the words", "edge", "en-US-AvaNeural", &settings)
        );
    }

    /// Fields are length-prefixed, so a boundary cannot be moved without
    /// changing the key. Without this, `voice="ab", text="c"` and
    /// `voice="a", text="bc"` would silently share one artifact.
    #[test]
    fn the_tts_key_cannot_be_confused_by_moving_a_field_boundary() {
        let settings = TtsSettings::default();
        assert_ne!(
            key_for_speech("c", "edge", "ab", &settings),
            key_for_speech("bc", "edge", "a", &settings)
        );
    }

    /// A declared duration that cannot be rendered is refused before FFmpeg is
    /// asked to generate an hour of silence from a typo.
    #[test]
    fn an_impossible_silent_duration_is_refused_before_spawning() {
        for seconds in [0.0, -3.0, f64::NAN, MAX_SCENE_SECONDS + 1.0] {
            let error = resolve(
                &cache(),
                &tools(),
                &AudioSource::Silent { seconds },
                None,
                &AudioPolicy::default(),
                &spoonstill_core::diagnostics::Noop,
            )
            .expect_err("refused");
            assert!(
                matches!(error, AudioError::DurationOutOfRange { .. }),
                "{seconds} gave {error}"
            );
        }
    }

    /// D-052: a `File` scene with no resolved path is our bug, and it says so
    /// rather than reaching FFmpeg as an empty argument.
    #[test]
    fn a_supplied_scene_with_no_resolved_path_is_a_named_bug() {
        let source = AudioSource::File {
            original_path: PathBuf::from("002.wav"),
        };
        let error = resolve(
            &cache(),
            &tools(),
            &source,
            None,
            &AudioPolicy::default(),
            &spoonstill_core::diagnostics::Noop,
        )
        .expect_err("no path");
        assert!(matches!(error, AudioError::NoResolvedPath), "{error}");
    }

    /// The arithmetic D-021 depends on: a declared duration becomes an exact
    /// sample count, not a rounded decimal.
    #[test]
    fn a_declared_duration_becomes_an_exact_sample_count() {
        assert_eq!(samples_for(3.0), 144_000);
        assert_eq!(samples_for(4.5), 216_000);
        assert_eq!(samples_for(0.0), 0);
        // The floating-point case this rounds for.
        assert_eq!(samples_for(0.1 + 0.2), samples_for(0.3));
    }

    /// D-043: the key changes when the normalization profile changes, so a
    /// profile bump misses the cache rather than reusing artifacts made under
    /// the old one.
    #[test]
    fn the_key_covers_the_profile_and_the_content() {
        let three = key_for_silence(samples_for(3.0));
        let four = key_for_silence(samples_for(4.0));
        assert_ne!(three, four, "different lengths must be different entries");
        assert_eq!(three, key_for_silence(samples_for(3.0)), "and stable");

        // The profile really is inside the key: recomputing with a different
        // one must not land on the same file.
        let with_other_profile = fnv1a_fields(&[
            b"silent",
            &samples_for(3.0).to_be_bytes(),
            b"pcm_s24le/96000/2/v2",
        ]);
        assert_ne!(three, with_other_profile);
    }

    /// D-043's headline: the key is the content, not the path. Two identical
    /// recordings under different names share one cache entry, and moving a
    /// file does not invalidate it.
    #[test]
    fn identical_recordings_at_different_paths_share_one_entry() {
        let dir = std::env::temp_dir().join(format!("spoonstill-audio-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (a, b, c) = (dir.join("a.wav"), dir.join("b.wav"), dir.join("c.wav"));
        std::fs::write(&a, b"RIFF....the same bytes").unwrap();
        std::fs::write(&b, b"RIFF....the same bytes").unwrap();
        std::fs::write(&c, b"RIFF....other bytes...").unwrap();

        assert_eq!(key_for_file(&a).unwrap(), key_for_file(&b).unwrap());
        assert_ne!(key_for_file(&a).unwrap(), key_for_file(&c).unwrap());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file bigger than one hash chunk must hash the same as the same bytes
    /// hashed at once — the boundary this streaming loop could get wrong.
    #[test]
    fn hashing_is_independent_of_the_chunk_boundary() {
        let dir = std::env::temp_dir().join(format!("spoonstill-audio-big-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.wav");
        let bytes: Vec<u8> = (0..=255u8).cycle().take(HASH_CHUNK * 2 + 137).collect();
        std::fs::write(&path, &bytes).unwrap();

        let expected = fnv1a_fields(&[
            &spoonstill_core::hash::fnv1a(&bytes).to_be_bytes(),
            NORMALIZED_PROFILE.as_bytes(),
        ]);
        assert_eq!(key_for_file(&path).unwrap(), expected);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_missing_source_file_names_itself() {
        let error = key_for_file(Path::new("/nonexistent/scene-42.wav")).unwrap_err();
        assert!(error.to_string().contains("scene-42.wav"), "{error}");
    }

    /// The message identifies a row without printing an essay into a terminal.
    /// The shortening itself now belongs to `spoonstill_tts::opening`, which
    /// tests it; this asserts that the *scene error* still uses it.
    #[test]
    fn a_long_line_is_shortened_in_the_error() {
        let long = "word ".repeat(40);
        let error = resolve(
            &cache(),
            &tools(),
            &spoken(&long, "nosuchprovider", "default"),
            None,
            &AudioPolicy::default(),
            &spoonstill_core::diagnostics::Noop,
        )
        .expect_err("no such provider");
        assert!(error.to_string().len() < long.len(), "{error}");
    }
}
