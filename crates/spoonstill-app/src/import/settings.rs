//! `project.yaml` — the human-owned input (D-013, D-056).
//!
//! **The renderer never writes to this file.** Nothing in this module opens it
//! for writing, and M3 has a gate that asserts its hash and mtime are
//! unchanged after a full render.
//!
//! Everything in it is optional, including the file itself: a folder holding
//! nothing but `001.png` and `001.txt` is a valid project (D-050's convention
//! mode), and these defaults are what it gets.
//!
//! ## Why a typo is an error rather than a default
//!
//! The raw form uses `deny_unknown_fields`, so `apsect: 9:16` fails to load
//! instead of quietly rendering 500 scenes at 16:9. That is D-055's rule
//! applied to the settings file: present-but-wrong is not the same as absent,
//! and the whole cost of the strictness is one clear error message the first
//! time an operator misspells a key.
//!
//! ## Two kinds of failure, deliberately handled differently
//!
//! - **The file will not parse** — bad YAML, an unknown key, a string where a
//!   number belongs. That aborts, because there is nothing left to validate,
//!   and `serde_yaml_ng` gives a line and column that point straight at it.
//! - **A value will not do** — `aspect: 4:3`, `fps: 0`, a negative default
//!   duration. Those become [`Problem`]s and collect, so the operator sees all
//!   of them in one run (plan.md §M2).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use spoonstill_core::captions::{Placement, SubtitleTheme};
use spoonstill_core::motion::{DEFAULT_AMOUNT, MAX_AMOUNT, MIN_AMOUNT};
use spoonstill_core::project::{MAX_SCENE_SECONDS, Problem, ProblemKind, ProviderId, VoiceId};
use spoonstill_core::{Aspect, MANIFEST_FILE, OutputSpec, Resolution};

/// Default TTS provider (D-023, D-081).
///
/// D-023 makes this differ by distribution — Edge TTS internally, ElevenLabs
/// in a sold build, because no reverse-engineered endpoint may be load-bearing
/// in a shipped product. There is still no build flag, and `edge` is the only
/// provider that exists, so the default is the one that works: a project that
/// says nothing gets the voice service this build can actually reach, rather
/// than a name that fails on the first spoken scene.
pub const DEFAULT_PROVIDER: &str = "edge";

/// Default voice, meaning "whatever that provider calls its default".
pub const DEFAULT_VOICE: &str = "default";

/// Seconds of a provider's leading silence a spoken scene keeps (D-084).
pub const DEFAULT_TRIM_HEAD: f64 = 0.10;

/// Seconds of its trailing silence to keep.
pub const DEFAULT_TRIM_TAIL: f64 = 0.25;

/// Longest padding worth keeping, in seconds.
///
/// A sanity bound in the shape of D-055's duration check: `trim_tail: 30` is a
/// typo far more often than it is a request for half a minute of room tone.
const MAX_TRIM_SECONDS: f64 = 10.0;

/// Default name of the rendered film, inside the project folder.
pub const DEFAULT_OUTPUT: &str = "out.mp4";

/// Subtitles are **off** unless the project asks for them (D-106).
///
/// Off because burning text into the picture is irreversible: it is in the
/// pixels, and an operator who did not want it re-renders the whole film. The
/// reverse mistake costs one flag. D-072 filed captions as V1.1 on the
/// assumption they would be a sidecar `.srt`; burned-in is what was asked for,
/// and burned-in is the one that has to be asked for.
pub const DEFAULT_SUBTITLES: bool = false;

/// The look a project gets when it turns subtitles on without naming one.
pub const DEFAULT_THEME: SubtitleTheme = SubtitleTheme::Classic;

/// How long an image with no narration holds, in seconds (D-050, D-056).
///
/// D-050 says an unpaired image becomes a silent scene of "default duration"
/// without fixing a number. Four seconds is long enough to read a title card
/// and short enough that a folder of them is not punishing. An operator who
/// disagrees sets `defaults.duration` once, for the whole project.
pub const DEFAULT_SILENT_SECONDS: f64 = 4.0;

/// Everything the project as a whole decides.
///
/// Per-scene overrides live in the manifest (D-050); this is the layer above
/// them, and every field here has a working default so that the smallest
/// possible project — a folder of images — renders with no configuration at
/// all.
#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    /// Where the finished film goes, relative to the project root.
    pub output: PathBuf,
    /// Geometry and frame rate, already validated (D-032, D-070).
    pub output_spec: OutputSpec,
    /// The CSV manifest to read, if the operator named one explicitly. `None`
    /// means "look for the default manifest name, and fall back to convention
    /// mode if it is not there" (D-050).
    pub manifest: Option<String>,
    /// Default TTS provider for scenes that do not name one (D-023).
    pub provider: ProviderId,
    /// Default voice.
    pub voice: VoiceId,
    /// Seconds of a provider's leading silence to keep (D-084).
    pub trim_head: f64,
    /// Seconds of its trailing silence to keep.
    pub trim_tail: f64,
    /// x264 preset (D-036).
    pub preset: String,
    /// x264 CRF (D-036).
    pub crf: u32,
    /// Hold time for an image with no narration (D-050).
    pub silent_seconds: f64,
    /// Default zoom span, as a fraction (D-035's amount).
    pub amount: f64,
    /// Whether to burn subtitles into the picture (D-106).
    pub subtitles: bool,
    /// Which look, when they are on.
    pub subtitle_theme: SubtitleTheme,
    /// Which edge they sit against.
    pub subtitle_placement: Placement,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            output: PathBuf::from(DEFAULT_OUTPUT),
            // 1080 at 16:9 and 30 fps is the one geometry an operator gets
            // without asking; D-070 keeps all three aspects available.
            output_spec: OutputSpec::new(Aspect::Landscape16x9, 1080, 30)
                .expect("1080 at 16:9 and 30 fps is a valid output spec"),
            manifest: None,
            provider: ProviderId(DEFAULT_PROVIDER.to_owned()),
            voice: VoiceId(DEFAULT_VOICE.to_owned()),
            trim_head: DEFAULT_TRIM_HEAD,
            trim_tail: DEFAULT_TRIM_TAIL,
            preset: "medium".to_owned(),
            crf: 18,
            silent_seconds: DEFAULT_SILENT_SECONDS,
            amount: DEFAULT_AMOUNT,
            subtitles: DEFAULT_SUBTITLES,
            subtitle_theme: DEFAULT_THEME,
            subtitle_placement: Placement::Bottom,
        }
    }
}

/// The file as written, before any value is judged.
///
/// Every field is optional so that a partial file is legal, and
/// `deny_unknown_fields` so that a misspelled one is not.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    output: Option<String>,
    aspect: Option<String>,
    /// A named size — `1080p`, `1440p`/`2k`, `2160p`/`4k` (D-143). The same
    /// thing as `short_edge`, said in the words an operator uses; naming both
    /// is a problem rather than a precedence rule.
    resolution: Option<String>,
    short_edge: Option<u32>,
    fps: Option<u32>,
    manifest: Option<String>,
    tts: Option<RawTts>,
    encode: Option<RawEncode>,
    defaults: Option<RawDefaults>,
    subtitles: Option<RawSubtitles>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSubtitles {
    enabled: Option<bool>,
    theme: Option<String>,
    position: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTts {
    provider: Option<String>,
    voice: Option<String>,
    trim_head: Option<f64>,
    trim_tail: Option<f64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEncode {
    preset: Option<String>,
    crf: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDefaults {
    duration: Option<f64>,
    zoom_amount: Option<f64>,
}

/// The file could not be read or parsed at all.
///
/// Distinct from a [`Problem`] on purpose: a problem is something to add to
/// the list and keep going, and this is the case where there is no list.
#[derive(Debug)]
pub enum SettingsError {
    /// The file exists but could not be read.
    Unreadable {
        /// Which file.
        path: PathBuf,
        /// The OS error.
        source: std::io::Error,
    },
    /// The file is not valid YAML, or holds a key or type we do not accept.
    Malformed {
        /// Which file.
        path: PathBuf,
        /// The parser's message, which carries a line and column.
        detail: String,
    },
}

impl std::fmt::Display for SettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsError::Unreadable { path, source } => {
                write!(f, "could not read {}: {source}", path.display())
            }
            SettingsError::Malformed { path, detail } => {
                write!(f, "{}: {detail}", path.display())
            }
        }
    }
}

impl std::error::Error for SettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SettingsError::Unreadable { source, .. } => Some(source),
            SettingsError::Malformed { .. } => None,
        }
    }
}

/// The largest `project.yaml` this will read (D-126).
///
/// Not derived from anything the way `MAX_SCRIPT_BYTES` is, because a
/// settings file has no natural length — so it is a round number chosen to be
/// far past any real one. The fixtures' are under 200 bytes.
const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

/// Read `project.yaml` from a project root, or return the defaults if there is
/// none.
///
/// A folder with no manifest is a legal project (D-050), so an absent file is
/// not a problem and does not appear in the returned list.
///
/// # Errors
///
/// [`SettingsError`] when the file exists but cannot be read or parsed. Value
/// problems are returned alongside the settings instead, so that a run reports
/// every one of them at once.
pub fn load(root: &Path) -> Result<(Settings, Vec<Problem>), SettingsError> {
    let path = root.join(MANIFEST_FILE);

    // Size first (D-126). `project.yaml` holds a dozen keys; anything past a
    // megabyte is a file that landed under that name, and reading it whole to
    // discover that is the cost this avoids.
    match std::fs::metadata(&path) {
        Ok(meta) if meta.len() > MAX_SETTINGS_BYTES => {
            return Err(SettingsError::Malformed {
                path,
                detail: format!(
                    "is {} MB — a project's settings are a dozen lines, so this is \
                     some other file under that name",
                    meta.len() / (1024 * 1024)
                ),
            });
        }
        _ => {}
    }

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok((Settings::default(), Vec::new()));
        }
        Err(source) => return Err(SettingsError::Unreadable { path, source }),
    };
    parse(&text).map_err(|detail| SettingsError::Malformed { path, detail })
}

/// Parse the text of a `project.yaml`.
///
/// Separate from [`load`] so the tests below need no filesystem.
///
/// # Errors
///
/// The parser's message when the document will not deserialize.
pub fn parse(text: &str) -> Result<(Settings, Vec<Problem>), String> {
    // An empty document deserializes to null, not to an empty mapping, and a
    // project.yaml holding only comments is a perfectly ordinary file.
    let raw: RawSettings = serde_yaml_ng::from_str(text)
        .map_err(|e| e.to_string())
        .or_else(|error| {
            if text.trim().is_empty() {
                Ok(RawSettings::default())
            } else {
                Err(error)
            }
        })?;
    Ok(resolve(raw))
}

/// Turn accepted syntax into accepted values, collecting everything that will
/// not do.
fn resolve(raw: RawSettings) -> (Settings, Vec<Problem>) {
    let mut settings = Settings::default();
    let mut problems = Vec::new();

    if let Some(output) = non_empty(raw.output) {
        settings.output = PathBuf::from(output);
    }

    // Geometry is validated as a unit, because it is one: a short edge is only
    // usable in the context of an aspect (D-032), and OutputSpec::new already
    // knows every rule. Re-deriving them here is how the two drift apart.
    let aspect = match non_empty(raw.aspect) {
        None => Some(settings.output_spec.aspect()),
        Some(text) => match Aspect::parse(&text) {
            Some(aspect) => Some(aspect),
            None => {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "aspect",
                    value: text,
                    expected: "one of 16:9, 9:16, 1:1",
                }));
                None
            }
        },
    };
    // `resolution` is `short_edge` with a name on it (D-143). Naming both is
    // refused rather than resolved by precedence: whichever one lost would be
    // an operator's stated intention silently discarded, which is the same
    // failure D-055 refuses for a misspelled key.
    let named = match non_empty(raw.resolution) {
        None => None,
        Some(text) => match Resolution::parse(&text) {
            Some(resolution) if raw.short_edge.is_some() => {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "resolution",
                    value: text,
                    expected: "named on its own — `short_edge` says the same thing, \
                               so set one or the other",
                }));
                let _ = resolution;
                None
            }
            Some(resolution) => Some(resolution.short_edge()),
            None => {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "resolution",
                    value: text,
                    expected: leak(format!("one of {}", Resolution::names())),
                }));
                None
            }
        },
    };

    if let Some(aspect) = aspect {
        let short_edge = raw
            .short_edge
            .or(named)
            .unwrap_or(settings.output_spec.short_edge());
        let fps = raw.fps.unwrap_or(settings.output_spec.fps());
        match OutputSpec::new(aspect, short_edge, fps) {
            Ok(spec) => settings.output_spec = spec,
            Err(error) => problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                field: if raw.fps.is_some() && short_edge == settings.output_spec.short_edge() {
                    "fps"
                } else if named.is_some() {
                    "resolution"
                } else {
                    "short_edge"
                },
                value: format!("{short_edge} at {aspect}, {fps} fps"),
                expected: error.reason(),
            })),
        }
    }

    settings.manifest = non_empty(raw.manifest);

    if let Some(tts) = raw.tts {
        if let Some(provider) = non_empty(tts.provider) {
            settings.provider = ProviderId(provider);
        }
        if let Some(voice) = non_empty(tts.voice) {
            settings.voice = VoiceId(voice);
        }
        // A negative number means "keep it all", which is how an operator turns
        // the trim off without a second boolean to disagree with these two.
        for (field, value, slot) in [
            ("tts.trim_head", tts.trim_head, &mut settings.trim_head),
            ("tts.trim_tail", tts.trim_tail, &mut settings.trim_tail),
        ] {
            let Some(seconds) = value else { continue };
            if !seconds.is_finite() || seconds > MAX_TRIM_SECONDS {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field,
                    value: seconds.to_string(),
                    expected: "a number of seconds no greater than 10, or negative \
                               to keep the provider's padding as it is",
                }));
                continue;
            }
            *slot = seconds;
        }
    }

    if let Some(encode) = raw.encode {
        if let Some(preset) = non_empty(encode.preset) {
            settings.preset = preset;
        }
        if let Some(crf) = encode.crf {
            // x264's range. 0 is lossless and enormous; 51 is unwatchable.
            // D-036 pins 18 as the default and does not license a value the
            // encoder will reject.
            if crf > 51 {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "encode.crf",
                    value: crf.to_string(),
                    expected: "between 0 and 51 (D-036 uses 18)",
                }));
            } else {
                settings.crf = crf;
            }
        }
    }

    if let Some(defaults) = raw.defaults {
        if let Some(duration) = defaults.duration {
            if duration.is_finite() && duration > 0.0 && duration <= MAX_SCENE_SECONDS {
                settings.silent_seconds = duration;
            } else {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "defaults.duration",
                    value: duration.to_string(),
                    expected: "a positive number of seconds",
                }));
            }
        }
        if let Some(amount) = defaults.zoom_amount {
            if amount.is_finite() && (MIN_AMOUNT..=MAX_AMOUNT).contains(&amount) {
                settings.amount = amount;
            } else {
                problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "defaults.zoom_amount",
                    value: amount.to_string(),
                    expected: leak(format!("between {MIN_AMOUNT} and {MAX_AMOUNT}")),
                }));
            }
        }
    }

    if let Some(subtitles) = raw.subtitles {
        // Naming a theme is not the same as asking for subtitles, and neither
        // implies the other: `enabled: false` with a theme set is how an
        // operator keeps their choice while turning the feature off for one
        // render.
        if let Some(enabled) = subtitles.enabled {
            settings.subtitles = enabled;
        }
        if let Some(theme) = non_empty(subtitles.theme) {
            match SubtitleTheme::parse(&theme) {
                Some(parsed) => settings.subtitle_theme = parsed,
                None => problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "subtitles.theme",
                    value: theme,
                    expected: leak(format!("one of {}", SubtitleTheme::names())),
                })),
            }
        }
        if let Some(position) = non_empty(subtitles.position) {
            match Placement::parse(&position) {
                Some(parsed) => settings.subtitle_placement = parsed,
                None => problems.push(Problem::in_project(ProblemKind::UnusableSetting {
                    field: "subtitles.position",
                    value: position,
                    expected: "bottom or top",
                })),
            }
        }
    }

    (settings, problems)
}

/// `Some(trimmed)` for a cell with content, `None` for one that is blank.
///
/// A key present with an empty value means the operator cleared it, which is
/// the same request as leaving it out.
fn non_empty(value: Option<String>) -> Option<String> {
    value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

/// `ProblemKind::UnusableSetting::expected` is a `&'static str` because every
/// other producer of it has a fixed phrase. These two are assembled from
/// constants that could change, so they are leaked rather than duplicated as
/// literals that would silently drift from the constants they describe.
///
/// Bounded: this is only reachable from settings validation, which runs once
/// per project load, and only on the error path.
fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(text: &str) -> Settings {
        let (settings, problems) = parse(text).expect("parses");
        assert!(problems.is_empty(), "unexpected problems: {problems:?}");
        settings
    }

    fn problems(text: &str) -> Vec<String> {
        let (_, problems) = parse(text).expect("parses");
        problems.iter().map(ToString::to_string).collect()
    }

    /// The smallest legal project has no `project.yaml` at all, so the
    /// defaults have to be a complete, working configuration on their own.
    #[test]
    fn the_defaults_are_a_complete_configuration() {
        let s = Settings::default();
        assert_eq!(s.output, PathBuf::from("out.mp4"));
        assert_eq!(s.output_spec.width(), 1920);
        assert_eq!(s.output_spec.height(), 1080);
        assert_eq!(s.output_spec.fps(), 30);
        assert_eq!(s.crf, 18, "D-036");
        assert_eq!(s.preset, "medium", "D-036");
        assert_eq!(s.silent_seconds, DEFAULT_SILENT_SECONDS, "D-050");
        assert_eq!(settings(""), s, "an empty file is the same as no file");
        assert_eq!(settings("# just a comment\n"), s);
    }

    #[test]
    fn a_partial_file_overrides_only_what_it_names() {
        let s = settings("fps: 24\n");
        assert_eq!(s.output_spec.fps(), 24);
        assert_eq!(s.output_spec.width(), 1920, "aspect untouched");
        assert_eq!(s.crf, 18, "encode untouched");
    }

    #[test]
    fn a_full_file_is_read_in_full() {
        let s = settings(
            "output: film.mp4\n\
             aspect: 9:16\n\
             short_edge: 1080\n\
             fps: 30\n\
             manifest: rows.csv\n\
             tts:\n  provider: edge\n  voice: en-GB-SoniaNeural\n\
             encode:\n  preset: slow\n  crf: 20\n\
             defaults:\n  duration: 2.5\n  zoom_amount: 0.2\n",
        );
        assert_eq!(s.output, PathBuf::from("film.mp4"));
        assert_eq!(s.output_spec.aspect(), Aspect::Portrait9x16);
        assert_eq!(
            (s.output_spec.width(), s.output_spec.height()),
            (1080, 1920)
        );
        assert_eq!(s.manifest.as_deref(), Some("rows.csv"));
        assert_eq!(s.provider, ProviderId("edge".into()));
        assert_eq!(s.voice, VoiceId("en-GB-SoniaNeural".into()));
        assert_eq!(s.preset, "slow");
        assert_eq!(s.crf, 20);
        assert_eq!(s.silent_seconds, 2.5);
        assert_eq!(s.amount, 0.2);
    }

    /// D-055 applied to the settings file. A misspelled key must not be a
    /// silent default — that is 500 scenes rendered at the wrong aspect with
    /// nothing on screen to say so.
    #[test]
    fn a_misspelled_key_is_refused_rather_than_ignored() {
        let error = parse("apsect: 9:16\n").expect_err("unknown key");
        assert!(
            error.contains("apsect") || error.contains("unknown field"),
            "the message must name the key: {error}"
        );
    }

    #[test]
    fn a_nested_misspelled_key_is_refused_too() {
        assert!(parse("encode:\n  quality: 20\n").is_err());
        assert!(parse("tts:\n  vioce: rachel\n").is_err());
    }

    #[test]
    fn broken_yaml_reports_where() {
        let error = parse("aspect: [16:9\n").expect_err("unterminated sequence");
        assert!(
            error.contains("line") || error.contains("column"),
            "the parser must locate it: {error}"
        );
    }

    /// Value problems collect rather than aborting, so one run shows the whole
    /// list (plan.md §M2).
    #[test]
    fn unusable_values_are_collected_not_thrown() {
        let found = problems(
            "aspect: 4:3\n\
             encode:\n  crf: 99\n\
             defaults:\n  duration: -1\n  zoom_amount: 5\n",
        );
        assert_eq!(found.len(), 4, "every bad value, in one pass: {found:?}");
        assert!(found[0].contains("aspect"), "{found:?}");
        assert!(found[1].contains("crf"), "{found:?}");
        assert!(found[2].contains("duration"), "{found:?}");
        assert!(found[3].contains("zoom_amount"), "{found:?}");
    }

    /// Geometry is validated by `OutputSpec::new`, not re-derived here — so
    /// the D-032/D-033 rules about even and integer dimensions apply to the
    /// manifest exactly as they apply to the command line.
    #[test]
    fn geometry_is_held_to_the_same_rules_as_the_command_line() {
        assert_eq!(problems("short_edge: 1001\n").len(), 1);
        assert_eq!(problems("fps: 0\n").len(), 1);
        assert_eq!(problems("fps: 500\n").len(), 1);
        assert!(problems("short_edge: 720\n").is_empty());
        assert!(problems("aspect: square\nshort_edge: 1000\n").is_empty());
    }

    /// D-143. A named size is the same request as a short edge, and it has to
    /// mean the same thing in every aspect.
    #[test]
    fn a_named_resolution_is_a_short_edge_with_a_name() {
        let two_k = settings("resolution: 2k\n");
        assert_eq!(
            (two_k.output_spec.width(), two_k.output_spec.height()),
            (2560, 1440)
        );

        let four_k = settings("resolution: 4k\n");
        assert_eq!(
            (four_k.output_spec.width(), four_k.output_spec.height()),
            (3840, 2160)
        );

        // A Short, named as the thing it is for rather than as a ratio.
        let short = settings("aspect: shorts\nresolution: 1080p\n");
        assert_eq!(short.output_spec.aspect(), Aspect::Portrait9x16);
        assert_eq!(
            (short.output_spec.width(), short.output_spec.height()),
            (1080, 1920)
        );

        // 4K vertical. The short edge is the one that stays put.
        let short_4k = settings("aspect: 9:16\nresolution: 4k\n");
        assert_eq!(
            (short_4k.output_spec.width(), short_4k.output_spec.height()),
            (2160, 3840)
        );

        // The same, spelled as a number, so the two doors reach one room.
        assert_eq!(settings("short_edge: 2160\n"), settings("resolution: 4k\n"));
    }

    /// Naming both is refused rather than resolved by precedence: whichever
    /// lost would be a stated intention silently discarded (D-055's rule).
    #[test]
    fn resolution_and_short_edge_together_are_a_problem_not_a_precedence() {
        let found = problems("resolution: 4k\nshort_edge: 1080\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("resolution"), "{found:?}");
        assert!(found[0].contains("short_edge"), "{found:?}");

        // And the geometry falls back to the default rather than guessing.
        let (settings, _) = parse("resolution: 4k\nshort_edge: 1080\n").expect("parses");
        assert_eq!(settings.output_spec.short_edge(), 1080);
    }

    #[test]
    fn an_unknown_resolution_names_the_ones_that_exist() {
        let found = problems("resolution: 8k\n");
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("2160p"), "{found:?}");
    }

    #[test]
    fn a_blank_value_means_the_default() {
        let s = settings("output: \"\"\nmanifest: \"  \"\n");
        assert_eq!(s.output, PathBuf::from(DEFAULT_OUTPUT));
        assert_eq!(s.manifest, None);
    }

    /// The renderer never writes to `project.yaml` (D-013). Nothing in this
    /// module may open it for writing — a cheap standing check that survives
    /// someone adding a "save settings" helper here later.
    #[test]
    fn this_module_never_writes_the_manifest() {
        // Only the shipped half of the file. The test module below mentions
        // these names in order to forbid them, and a check that matched its
        // own needles would fail for the wrong reason — the same shape of bug
        // as the `odd.jpg` fixture in ffmpeg-findings.md §8b.
        let source = include_str!("settings.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("split always yields a first part");
        assert!(
            source.contains("pub fn load"),
            "the haystack must still be the real module"
        );

        for forbidden in ["fs::write", "File::create", "OpenOptions"] {
            assert!(
                !source.contains(forbidden),
                "D-013: project.yaml is an input; {forbidden} has no place here"
            );
        }
    }
}
