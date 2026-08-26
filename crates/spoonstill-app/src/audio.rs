//! Resolving an [`AudioSource`] to `(normalized path, measured duration)`
//! (D-020, D-021, D-043).
//!
//! This is the join between the domain's three audio sources and the one shape
//! the renderer accepts. Every source ends at the same place:
//!
//! ```text
//! AudioSource::File   -> normalize the operator's recording into the cache
//! AudioSource::Silent -> generate a silent track of an exact sample count
//! AudioSource::Tts    -> synthesize, then normalize   [M2 slice 4]
//!         all three -> ffprobe the artifact -> (path, duration)
//! ```
//!
//! The renderer never learns which branch a scene took. That is D-020's actual
//! test — "adding a fourth source must not touch the renderer" — and it is why
//! the TTS arm being unimplemented shows up here as one typed error rather
//! than as a hole somewhere downstream.
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
//! | `Tts` | `hash(text, provider, voice, settings, profile)` — slice 4 |
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
use spoonstill_media::audio::{self, NORMALIZED_EXT, NORMALIZED_PROFILE};
use spoonstill_media::{MediaError, Tools};

/// Where normalized audio lives, under [`STATE_DIR`].
pub const AUDIO_CACHE_DIR: &str = "cache/audio";

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
    /// The scene needs TTS, which arrives in M2 slice 4.
    ///
    /// A named, typed refusal rather than a panic or a silent silent-scene:
    /// substituting silence for a line somebody wrote would produce a film
    /// that looks finished and is not.
    TtsNotAvailable {
        /// Which provider the scene asked for.
        provider: String,
        /// The opening of the line, to identify the row.
        text: String,
    },
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
            AudioError::TtsNotAvailable { provider, text } => write!(
                f,
                "needs text-to-speech ({provider}) for {text:?}, which is not built yet \
                 — M2 slice 4. Give the scene an `audio_file` or a `duration` to render \
                 it today."
            ),
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

impl std::error::Error for AudioError {}

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
        AudioSource::Tts { text, provider, .. } => {
            return Err(AudioError::TtsNotAvailable {
                provider: provider.to_string(),
                text: opening(text),
            });
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
        Work::Normalize(original) => audio::normalize(tools, &original, &path, log)?,
        Work::Silence(samples) => audio::silence(tools, samples, &path, log)?,
    };
    finish(kind, made.path, made.duration, false)
}

/// What resolving this source will take, once the cache has been asked.
enum Work {
    /// Normalize the operator's own recording (D-021).
    Normalize(PathBuf),
    /// Generate this many samples of silence (D-020).
    Silence(u64),
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

/// The first few words of a line, for a message that has to identify a row
/// without printing a paragraph into a terminal.
fn opening(text: &str) -> String {
    let trimmed = text.trim();
    let mut out: String = trimmed.chars().take(48).collect();
    if trimmed.chars().count() > 48 {
        out.push('…');
    }
    out
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

    /// D-020, made visible: TTS is not silently swapped for silence. A scene
    /// with a line nobody can speak yet must fail loudly and name the reason.
    #[test]
    fn a_tts_scene_is_refused_by_name_rather_than_silently_muted() {
        let source = AudioSource::Tts {
            text: "A line to be spoken over the opening still.".to_owned(),
            provider: ProviderId("elevenlabs".to_owned()),
            voice: VoiceId("default".to_owned()),
            settings: TtsSettings::default(),
        };
        let error = resolve(
            &cache(),
            &tools(),
            &source,
            None,
            &spoonstill_core::diagnostics::Noop,
        )
        .expect_err("TTS is slice 4");

        assert!(matches!(error, AudioError::TtsNotAvailable { .. }));
        let message = error.to_string();
        assert!(message.contains("elevenlabs"), "{message}");
        assert!(message.contains("A line to be spoken"), "{message}");
        // And it says what an operator can do instead, right now.
        assert!(message.contains("audio_file"), "{message}");
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
    #[test]
    fn a_long_line_is_shortened_for_the_message() {
        let long = "word ".repeat(40);
        let shown = opening(&long);
        assert!(shown.chars().count() <= 49, "{shown}");
        assert!(shown.ends_with('…'));
        assert_eq!(opening("  short line  "), "short line");
    }
}
