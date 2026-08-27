//! Edge TTS — Microsoft's neural voices, through the maintained `edge-tts`
//! command line tool.
//!
//! ## Why a subprocess rather than a Rust client
//!
//! D-023 already decided what Edge TTS *is* here: the internal and development
//! provider, free, never load-bearing in a sold build, because it speaks to a
//! reverse-engineered endpoint. That endpoint moves — it has an anti-abuse
//! token derived from a clock skew and a shared secret, and it has changed
//! more than once. Reimplementing it in Rust would mean owning a protocol whose
//! specification is "whatever Edge does this month", and shipping a build that
//! stops working on a Tuesday.
//!
//! `edge-tts` is the Python implementation that tracks those changes and is
//! already installed on the machines that want this provider. So this module
//! spawns it — through `spoonstill_media::command`, the one place a process is
//! spawned (D-011), which means argument vectors, a timeout, retained stderr
//! and a paste-ready command line for the diagnostics bundle, all for free.
//!
//! A provider that speaks HTTP itself, as ElevenLabs will, implements the same
//! trait and needs none of this.
//!
//! ## The text goes in a file, never in an argument
//!
//! `edge-tts` takes either `--text` or `--file`. This module always uses
//! `--file`, for three reasons, in increasing order of importance:
//!
//! 1. A paragraph can exceed the operating system's argument length limit.
//! 2. Arguments are visible to every process on the machine through `ps`.
//! 3. **The command line is logged** (D-016) and lands in the bundle the
//!    operator sends us. Their script is their content; it does not belong in
//!    our diagnostics, and the only reliable way to keep it out is for it never
//!    to be an argument.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use spoonstill_media::atomic;
use spoonstill_media::command::FfmpegCommand;

use crate::{Availability, Provider, Request, Spoken, TtsError, Voice, opening};

/// How long to let a package manager run. Installing pulls a dependency tree
/// over a network, so this is minutes rather than seconds.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// The package managers to try, in the order that works most often.
///
/// `pipx` first because it is the one that does not fight a system Python;
/// `brew` next on macOS; a `--user` pip last, which is the advice that fails on
/// a modern Homebrew Python and is therefore the fallback rather than the lead.
#[cfg(target_os = "macos")]
const INSTALLERS: &[(&str, &[&str])] = &[
    ("pipx", &["install", "edge-tts"]),
    ("brew", &["install", "edge-tts"]),
    (
        "python3",
        &["-m", "pip", "install", "--user", "--upgrade", "edge-tts"],
    ),
];

/// Windows has no Homebrew, and its Python is not externally managed, so a
/// plain `pip --user` is the normal answer rather than the last resort.
#[cfg(not(target_os = "macos"))]
const INSTALLERS: &[(&str, &[&str])] = &[
    ("pipx", &["install", "edge-tts"]),
    (
        "python",
        &["-m", "pip", "install", "--user", "--upgrade", "edge-tts"],
    ),
    (
        "python3",
        &["-m", "pip", "install", "--user", "--upgrade", "edge-tts"],
    ),
];

/// The last thing a failing installer said, which is the part that names the
/// cause. Package managers print pages; an error box holds a sentence.
fn last_line(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_owned()
}

/// Environment override for the `edge-tts` binary. Development convenience,
/// and the hook a packaged build would use — same shape as
/// `spoonstill_media::tools` (D-012: no auto-download, no candidate search).
pub const EDGE_TTS_ENV: &str = "SPOONSTILL_EDGE_TTS";

/// The name used in `project.yaml`.
pub const ID: &str = "edge";

/// How long one line may take before it is a failure.
///
/// A short line is about 1.5 s over a working connection (measured on this
/// machine, 2026-08-26). Ninety seconds is not a performance budget — it is the
/// point past which "the network is gone" is a better explanation than "this
/// sentence is long", and it matches the receive timeout the author's own
/// SetupTTS uses against the same service.
const SPEAK_TIMEOUT: Duration = Duration::from_secs(90);

/// Listing voices talks to the same service and returns 300-odd rows.
const LIST_TIMEOUT: Duration = Duration::from_secs(30);

/// The default voice, used when a project names none.
///
/// `edge-tts` has its own default and it changes between releases; naming ours
/// keeps a render reproducible across an upgrade of a tool we do not pin
/// (D-043: a cache key that hashes "default" must mean one voice).
pub const DEFAULT_VOICE: &str = "en-US-AvaNeural";

/// Microsoft's neural voices, via `edge-tts`.
#[derive(Debug, Clone)]
pub struct Edge {
    program: PathBuf,
}

impl Edge {
    /// Use whichever binary the environment names, falling back to `PATH`.
    #[must_use]
    pub fn from_env() -> Self {
        Edge {
            program: std::env::var_os(EDGE_TTS_ENV)
                .map_or_else(|| PathBuf::from("edge-tts"), PathBuf::from),
        }
    }

    /// Use an explicitly located binary — what a packaged build does, and what
    /// a test does.
    #[must_use]
    pub fn at(program: impl Into<PathBuf>) -> Self {
        Edge {
            program: program.into(),
        }
    }

    /// The binary this provider will run.
    #[must_use]
    pub fn program(&self) -> &Path {
        &self.program
    }
}

impl Default for Edge {
    fn default() -> Self {
        Self::from_env()
    }
}

impl Provider for Edge {
    fn id(&self) -> &str {
        ID
    }

    fn availability(&self) -> Availability {
        let mut command = FfmpegCommand::new(&self.program);
        command.arg("--version");
        match command.spawn().and_then(|c| c.wait_until(LIMIT_VERSION)) {
            Ok(finished) if finished.status.success() => Availability::Ready,
            Ok(finished) => Availability::Missing(format!(
                "`{} --version` failed: {}",
                self.program.display(),
                finished.stderr.trim()
            )),
            Err(_) => Availability::Missing(format!(
                "`{}` is not on this machine. Install it with \
                 `pip install edge-tts` (or `brew install edge-tts`), or point \
                 {EDGE_TTS_ENV} at it.",
                self.program.display()
            )),
        }
    }

    fn default_voice(&self) -> &str {
        DEFAULT_VOICE
    }

    /// Fetch `edge-tts` through whichever package manager this machine has.
    ///
    /// Told to run `pip install edge-tts`, an operator on a Homebrew Python
    /// meets `error: externally-managed-environment` and stops. So the
    /// candidates are tried in the order that works most often, and the first
    /// one that both exists and succeeds wins. Nothing is downloaded from us
    /// (D-062, D-012): every candidate is the platform's own package manager,
    /// run because a button that says install was pressed.
    fn install(&self) -> Result<String, TtsError> {
        let mut tried: Vec<String> = Vec::new();

        for (program, args) in INSTALLERS {
            let mut command = FfmpegCommand::new(*program);
            command.args(*args);
            let shown = command.display();

            match command.spawn().and_then(|c| c.wait_until(INSTALL_TIMEOUT)) {
                Ok(finished) if finished.status.success() => {
                    // Present is not the same as runnable: a `pip --user`
                    // install can land somewhere that is not on PATH, and
                    // reporting success there would be reporting a lie.
                    return match self.availability() {
                        Availability::Ready => Ok(shown),
                        Availability::Missing(detail) => Err(TtsError::Unavailable {
                            provider: ID.to_owned(),
                            detail: format!(
                                "`{shown}` succeeded but {} still cannot be run \
                                 — it is probably not on PATH. {detail}",
                                self.program.display()
                            ),
                        }),
                    };
                }
                Ok(finished) => tried.push(format!(
                    "`{shown}` exited {}: {}",
                    finished.status.code().unwrap_or(-1),
                    last_line(&finished.stderr)
                )),
                // Not installed on this machine — the next candidate is the
                // point of having a list.
                Err(_) => tried.push(format!("`{program}` is not on this machine")),
            }
        }

        Err(TtsError::Unavailable {
            provider: ID.to_owned(),
            detail: format!("could not install it. {}", tried.join("; ")),
        })
    }

    fn voices(&self) -> Result<Vec<Voice>, TtsError> {
        let mut command = FfmpegCommand::new(&self.program);
        command.arg("--list-voices");
        let finished = command.spawn()?.wait_until(LIST_TIMEOUT)?.ok()?;
        Ok(parse_voices(&String::from_utf8_lossy(&finished.stdout)))
    }

    fn speak(&self, request: &Request<'_>, destination: &Path) -> Result<Spoken, TtsError> {
        if request.text.trim().is_empty() {
            return Err(TtsError::BadRequest {
                provider: ID.to_owned(),
                detail: "the line is empty".to_owned(),
            });
        }

        let voice = if request.voice.is_empty() || request.voice == "default" {
            DEFAULT_VOICE
        } else {
            request.voice
        };

        // Both temporaries live beside the destination: a rename within one
        // filesystem is atomic, one across two is a copy (D-042).
        let script = atomic::partial_path(&destination.with_extension("txt"));
        let audio = atomic::partial_path(destination);
        let mut file = std::fs::File::create(&script).map_err(|source| TtsError::Io {
            path: script.clone(),
            source,
        })?;
        file.write_all(request.text.as_bytes())
            .and_then(|()| file.sync_all())
            .map_err(|source| TtsError::Io {
                path: script.clone(),
                source,
            })?;
        drop(file);

        let mut command = FfmpegCommand::new(&self.program);
        command
            .arg("--voice")
            .arg(voice)
            .arg("--rate")
            .arg(percent(request.setting("rate"))?)
            .arg("--volume")
            .arg(percent(request.setting("volume"))?)
            .arg("--pitch")
            .arg(hertz(request.setting("pitch"))?)
            .arg("--file")
            .arg(&script)
            .arg("--write-media")
            .arg(&audio);

        let how = command.display();
        let outcome = command
            .spawn()
            .and_then(|child| child.wait_until(SPEAK_TIMEOUT))
            .and_then(spoonstill_media::Finished::ok);

        // The script is the operator's words on our disk. It goes away whether
        // the run worked or not, and before the error is even shaped.
        let _ = std::fs::remove_file(&script);

        let finished = match outcome {
            Ok(finished) => finished,
            Err(e) => {
                let _ = std::fs::remove_file(&audio);
                return Err(e.into());
            }
        };

        // `edge-tts` reports a refused line by exiting zero and writing nothing
        // — the failure mode a bare exit-status check would call success.
        let bytes = std::fs::metadata(&audio).map(|m| m.len()).unwrap_or(0);
        if bytes == 0 {
            let _ = std::fs::remove_file(&audio);
            return Err(TtsError::NoAudio {
                provider: ID.to_owned(),
                text: opening(request.text),
                detail: if finished.stderr.trim().is_empty() {
                    format!("the service accepted the request for {voice} and returned no audio")
                } else {
                    finished.stderr.trim().to_owned()
                },
            });
        }

        atomic::move_into_place(&audio, destination)?;
        Ok(Spoken { bytes, how })
    }
}

/// `--version` should answer immediately or not at all.
const LIMIT_VERSION: Duration = Duration::from_secs(20);

/// Validate a percentage knob into the `+N%` form `edge-tts` requires.
///
/// Absent means `+0%`. A value the tool would reject is caught here, with the
/// scene's name attached, rather than as a stderr line from a subprocess in the
/// middle of a 500-line batch.
fn percent(value: Option<&str>) -> Result<String, TtsError> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok("+0%".to_owned());
    };
    let number = raw.trim_end_matches('%');
    let parsed: i32 = number.parse().map_err(|_| TtsError::BadRequest {
        provider: ID.to_owned(),
        detail: format!("{raw:?} is not a percentage like -10% or +25%"),
    })?;
    if !(-100..=200).contains(&parsed) {
        return Err(TtsError::BadRequest {
            provider: ID.to_owned(),
            detail: format!("{parsed}% is outside the range -100% to +200%"),
        });
    }
    Ok(format!("{parsed:+}%"))
}

/// The same, for pitch, which Edge states in hertz.
fn hertz(value: Option<&str>) -> Result<String, TtsError> {
    let Some(raw) = value.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok("+0Hz".to_owned());
    };
    let number = raw
        .trim_end_matches(|c: char| c.eq_ignore_ascii_case(&'z') || c.eq_ignore_ascii_case(&'h'));
    let parsed: i32 = number.trim().parse().map_err(|_| TtsError::BadRequest {
        provider: ID.to_owned(),
        detail: format!("{raw:?} is not a pitch shift like -20Hz or +50Hz"),
    })?;
    if !(-100..=100).contains(&parsed) {
        return Err(TtsError::BadRequest {
            provider: ID.to_owned(),
            detail: format!("{parsed}Hz is outside the range -100Hz to +100Hz"),
        });
    }
    Ok(format!("{parsed:+}Hz"))
}

/// Read `edge-tts --list-voices`, which prints a fixed-width table.
///
/// Parsed by splitting on runs of two or more spaces rather than by column
/// offset: the widths follow the longest value in each column, so they differ
/// between releases and between locales. A row that does not have at least a
/// name is skipped rather than guessed at.
fn parse_voices(table: &str) -> Vec<Voice> {
    table
        .lines()
        .skip_while(|line| !line.starts_with("---"))
        .skip(1)
        .filter_map(|line| {
            let mut columns = line.split("  ").filter(|c| !c.trim().is_empty());
            let id = columns.next()?.trim().to_owned();
            if id.is_empty() {
                return None;
            }
            // `af-ZA-AdriNeural` -> `af-ZA`. A name that is not in that shape
            // keeps an empty locale rather than a wrong one.
            let locale = id
                .match_indices('-')
                .nth(1)
                .map_or_else(String::new, |(at, _)| id[..at].to_owned());
            Some(Voice {
                id,
                locale,
                gender: columns.next().unwrap_or_default().trim().to_owned(),
                note: columns
                    .map(|c| c.trim())
                    .filter(|c| !c.is_empty())
                    .collect::<Vec<_>>()
                    .join(" · "),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first rows of a real `edge-tts --list-voices` on this machine,
    /// 2026-08-26, `edge-tts 7.2.8`. Recorded rather than described, so that a
    /// change in the tool's output is a test failure and not a silent empty
    /// voice list in the window (plan.md §M2: a recorded fixture from the
    /// first commit).
    const RECORDED: &str = "\
Name                               Gender    ContentCategories      VoicePersonalities
---------------------------------  --------  ---------------------  --------------------------------------
af-ZA-AdriNeural                   Female    General                Friendly, Positive
af-ZA-WillemNeural                 Male      General                Friendly, Positive
en-US-AvaNeural                    Female    Conversation, Copilot  Expressive, Caring, Pleasant, Friendly
en-GB-RyanNeural                   Male      News, Novel            Friendly, Positive
";

    #[test]
    fn the_recorded_voice_table_parses_into_voices() {
        let voices = parse_voices(RECORDED);

        assert_eq!(voices.len(), 4);
        assert_eq!(voices[2].id, "en-US-AvaNeural");
        assert_eq!(voices[2].locale, "en-US");
        assert_eq!(voices[2].gender, "Female");
        assert!(voices[2].note.contains("Expressive"));
        assert!(
            voices.iter().any(|v| v.id == DEFAULT_VOICE),
            "the voice we default to must exist in the service's own list"
        );
    }

    #[test]
    fn a_table_with_no_rows_is_empty_rather_than_a_row_of_dashes() {
        let header = RECORDED.lines().take(2).collect::<Vec<_>>().join("\n");
        assert!(parse_voices(&header).is_empty());
        assert!(parse_voices("").is_empty());
    }

    #[test]
    fn knobs_default_to_no_change_and_normalize_their_sign() {
        assert_eq!(percent(None).expect("absent is fine"), "+0%");
        assert_eq!(percent(Some("")).expect("blank is absent"), "+0%");
        assert_eq!(percent(Some("10")).expect("bare number"), "+10%");
        assert_eq!(percent(Some("+10%")).expect("already signed"), "+10%");
        assert_eq!(percent(Some("-25%")).expect("negative"), "-25%");
        assert_eq!(hertz(None).expect("absent is fine"), "+0Hz");
        assert_eq!(hertz(Some("-20Hz")).expect("negative"), "-20Hz");
        assert_eq!(hertz(Some("5")).expect("bare number"), "+5Hz");
    }

    #[test]
    fn a_knob_that_is_not_a_number_is_refused_before_the_process_starts() {
        let error = percent(Some("fast")).expect_err("not a percentage");
        assert!(error.to_string().contains("fast"), "{error}");
        assert!(percent(Some("400%")).is_err(), "outside the range");
        assert!(hertz(Some("+900Hz")).is_err(), "outside the range");
    }

    #[test]
    fn an_empty_line_is_refused_without_spawning_anything() {
        // The program name is deliberately nonsense: reaching the process
        // boundary at all would fail this test rather than pass it slowly.
        let edge = Edge::at("/nonexistent/edge-tts");
        let error = edge
            .speak(
                &Request {
                    text: "   ",
                    voice: "en-US-AvaNeural",
                    settings: &[],
                },
                Path::new("/nonexistent/out.mp3"),
            )
            .expect_err("an empty line");
        assert!(matches!(error, TtsError::BadRequest { .. }), "{error}");
    }

    #[test]
    fn a_missing_binary_reports_how_to_install_it() {
        let edge = Edge::at("/nonexistent/edge-tts");
        match edge.availability() {
            Availability::Missing(detail) => {
                assert!(detail.contains("pip install edge-tts"), "{detail}");
                assert!(detail.contains(EDGE_TTS_ENV), "{detail}");
            }
            Availability::Ready => panic!("a binary that is not there cannot be ready"),
        }
    }
}
