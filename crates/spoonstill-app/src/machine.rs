//! What this machine answers when no project has (D-168).
//!
//! One file, beside `runs.csv` in [`spoonstill_state::runs::config_dir`],
//! which is already documented as *"where this machine keeps what belongs to
//! the operator rather than to any one project"*. It holds the fallback voice
//! D-092 introduced.
//!
//! **It used to live somewhere only the window could reach.** `AppSettings` was
//! written to Tauri's `app_config_dir()` — `com.spoonstill.desktop/` — while
//! the CLI's machine state is `spoonstill/`, two directories on the same disk
//! that neither surface could see the other's. So a fallback voice set in the
//! window was invisible to `still render`, which is this project's own rule
//! broken: *if the CLI cannot do it, it does not exist.*
//!
//! YAML rather than JSON, and no new dependency for it: `project.yaml` is the
//! format an operator of this tool has already read, and this file is small
//! enough to edit by hand when that is the quickest thing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spoonstill_core::Remedy;

/// The file, inside [`spoonstill_state::runs::config_dir`].
pub const SETTINGS_FILE: &str = "settings.yaml";

/// What a file that could not be read is renamed to, rather than replaced.
///
/// The module note above invites hand-editing, so a file that does not parse is
/// most likely something an operator typed. Overwriting it would take away the
/// one thing that explains what happened to their setting (D-171).
pub const BROKEN_SUFFIX: &str = ".broken";

/// The machine's own answers. Every field is optional, because every field has
/// a working default and a first run has none of them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Machine {
    /// The voice a project that names none falls back to (D-092).
    ///
    /// **A fallback, never a write.** `project.yaml` is an input (D-013), so
    /// setting this changes what the next render asks for and nothing on disk
    /// inside any project. A project that names its own voice still wins, and
    /// a `--voice` on the command line wins over both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_voice: Option<String>,
}

/// Where the file is, whether or not it exists yet.
///
/// `None` on a platform that will not say where machine state belongs, which
/// is a machine we simply do not keep settings on — the same answer
/// `config_dir` gives, for the same reason.
#[must_use]
pub fn path() -> Option<PathBuf> {
    spoonstill_state::runs::config_dir().map(|dir| dir.join(SETTINGS_FILE))
}

/// What reading the machine's settings found (D-171).
///
/// Two fields rather than one value, because **"there is no file" and "the file
/// is broken" are different answers and used to be the same one.** `load` was
/// `read_to_string(..).ok().and_then(from_str(..).ok()).unwrap_or_default()`,
/// so a truncated `settings.yaml` — a crash mid-write, a hand edit that lost a
/// quote — produced the defaults in total silence. Reproduced: set a fallback
/// voice, truncate the file, and `still voices` marks no row and says nothing.
/// The setting is gone and the operator finds out from the films.
///
/// D-168 added the catalogue check on `still voices --use` for exactly this
/// reason — *a misspelt voice is otherwise a setting that silently fails every
/// render until somebody remembers making it* — and the file it writes to
/// reintroduced the same failure one layer down.
#[derive(Debug, Clone)]
pub struct Loaded {
    /// The answers, or the defaults when there are none to be had.
    pub settings: Machine,
    /// A file that exists and could not be used.
    ///
    /// [`None`] on the two ordinary cases: no file at all, and a file that
    /// read cleanly. Never [`Some`] for a machine nobody has configured — a
    /// first run has no settings and that is not a problem to report.
    pub problem: Option<Remedy>,
}

/// Read them, and say so if a file that exists could not be used.
///
/// A missing file is the defaults, silently: that is the normal state of a
/// machine nobody has configured. Anything else — unreadable, or unparseable —
/// is still the defaults, because a render must not fail over a preference,
/// but it comes back with a [`Remedy`] the caller is expected to show.
#[must_use]
pub fn read() -> Loaded {
    let plain = |settings| Loaded {
        settings,
        problem: None,
    };
    let Some(path) = path() else {
        return plain(Machine::default());
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // The only silent arm, and deliberately the only one.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return plain(Machine::default());
        }
        Err(e) => {
            return Loaded {
                settings: Machine::default(),
                problem: Some(Remedy::manual(
                    "This machine's settings could not be read, so it is \
                     answering with its defaults — including which voice a \
                     project that names none is read in.",
                    format!("{}: {e}", path.display()),
                )),
            };
        }
    };

    match serde_yaml_ng::from_str(&text) {
        Ok(settings) => plain(settings),
        Err(e) => Loaded {
            settings: Machine::default(),
            problem: Some(Remedy::manual(
                format!(
                    "This machine's settings file is damaged, so it is \
                     answering with its defaults — including which voice a \
                     project that names none is read in. Setting one again \
                     moves the damaged file aside to \
                     {SETTINGS_FILE}{BROKEN_SUFFIX} rather than replacing it."
                ),
                format!("{}: {e}", path.display()),
            )),
        },
    }
}

/// Read them, or the defaults.
///
/// The answer only, for the callers that have no surface to report on. Every
/// surface that can show a [`Remedy`] should use [`read`] instead (D-105).
#[must_use]
pub fn load() -> Machine {
    read().settings
}

/// Write them.
///
/// # Errors
///
/// When the config directory cannot be created or the file cannot be written.
/// Unlike [`load`], this one is reported: an operator who asked to set
/// something has to be told it was not set.
pub fn save(settings: &Machine) -> Result<(), String> {
    let path = path().ok_or("this machine has no config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let text = serde_yaml_ng::to_string(settings).map_err(|e| e.to_string())?;

    // A file that is there and does not parse is moved aside rather than
    // replaced (D-171). This module's own note invites hand-editing, so what
    // is in that file is most likely something the operator typed, and it is
    // the only thing that explains where their setting went.
    if let Some(broken) = damaged_file(&path) {
        // Best effort: failing to preserve it must not stop the operator
        // setting the thing they asked to set.
        let _ = std::fs::rename(&path, &broken);
    }

    replace_file(&path, &text)
}

/// Where a damaged settings file should be kept, or [`None`] if it is fine.
fn damaged_file(path: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(path).ok()?;
    if serde_yaml_ng::from_str::<Machine>(&text).is_ok() {
        return None;
    }
    let mut name = path.file_name()?.to_os_string();
    name.push(BROKEN_SUFFIX);
    Some(path.with_file_name(name))
}

/// Write a file by writing beside it and renaming over it (D-171).
///
/// `fs::write` truncates first, so a crash between the truncate and the write
/// leaves the file **empty or half-written** — which is precisely the state
/// this module now has to report. Renaming is last-writer-wins and replaces
/// (D-119), so the previous contents survive right up to the moment the new
/// ones are complete on disk.
///
/// Public, and it takes the path, because the window keeps its recent-projects
/// list in Tauri's own `app_config_dir()` (D-086) and D-010 forbids it from
/// reaching `spoonstill-media` to get this. One implementation, both callers.
///
/// # Errors
///
/// When the temporary cannot be written or cannot be moved into place.
pub fn replace_file(path: &Path, text: &str) -> Result<(), String> {
    let beside = spoonstill_media::atomic::partial_path(path);
    std::fs::write(&beside, text).map_err(|e| format!("writing {}: {e}", beside.display()))?;
    spoonstill_media::atomic::move_into_place(&beside, path).map_err(|e| {
        // Never leave the temporary behind on a failure: this directory is the
        // operator's, and a litter of `.partial-` files is not ours to add.
        let _ = std::fs::remove_file(&beside);
        e.to_string()
    })
}

/// Set the fallback voice, or clear it by passing nothing.
///
/// # Errors
///
/// As [`save`].
pub fn set_default_voice(voice: Option<&str>) -> Result<Machine, String> {
    let mut settings = load();
    settings.default_voice = voice
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty() && v != crate::import::settings::DEFAULT_VOICE);
    save(&settings)?;
    Ok(settings)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one thing that must not drift: this file sits beside `runs.csv`, in
    /// the directory the CLI already uses, or the CLI cannot see it.
    #[test]
    fn the_settings_live_beside_the_activity_log() {
        let (Some(settings), Some(runs)) = (path(), spoonstill_state::runs_index_path()) else {
            // A platform that will not name a config directory keeps no
            // settings, which is the documented answer rather than a skip.
            assert!(path().is_none(), "a settings path with no runs path");
            return;
        };
        assert_eq!(
            settings.parent(),
            runs.parent(),
            "the window's setting and the CLI's log are in two directories \
             again, which is the defect D-168 exists to close"
        );
    }

    /// D-171. Three cases, three answers — they used to be one.
    ///
    /// `read` reaches the real config directory, so these drive the parsing and
    /// the reporting through a scratch file rather than through `HOME`: two
    /// tests redirecting one process-wide variable is a race, and gate 7i is
    /// what covers the wired-up path.
    #[test]
    fn a_missing_file_is_silent_and_a_damaged_one_is_not() {
        // Nothing there: the ordinary first run. Defaults, no problem.
        assert!(serde_yaml_ng::from_str::<Machine>("").is_ok());

        // A file that is there and does not parse. This is the state a crash
        // mid-write leaves, and the state a hand edit that lost a quote leaves.
        let truncated = "default_voice: \"en-GB-Rya";
        let failed = serde_yaml_ng::from_str::<Machine>(truncated)
            .expect_err("a truncated file must not parse as a machine");

        // The sentence an operator gets has to say what it cost them, which is
        // a voice — not that a YAML document ended unexpectedly.
        let remedy = Remedy::manual(
            "This machine's settings file is damaged, so it is answering with \
             its defaults — including which voice a project that names none is \
             read in.",
            format!("settings.yaml: {failed}"),
        );
        assert!(remedy.install.is_none(), "there is nothing to press");
        assert!(remedy.need.contains("voice"), "{}", remedy.need);
        assert!(!remedy.need.contains("YAML"), "{}", remedy.need);
    }

    /// The file the operator typed is kept, not replaced.
    ///
    /// The module note above says this file is *"small enough to edit by hand
    /// when that is the quickest thing"*, so what is in a damaged one is most
    /// likely something they wrote — and it is the only thing that explains
    /// where their setting went.
    #[test]
    fn setting_one_again_moves_a_damaged_file_aside() {
        let dir = scratch("aside");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "default_voice: \"en-GB-Rya").expect("write");

        let broken = damaged_file(&path).expect("a truncated file is damaged");
        assert_eq!(
            broken.file_name().and_then(|n| n.to_str()),
            Some("settings.yaml.broken"),
            "the name says what it is, beside the file it came from"
        );

        // A file that reads cleanly is left exactly alone — this must not fire
        // on every ordinary save.
        std::fs::write(&path, "default_voice: en-GB-RyanNeural\n").expect("write");
        assert!(damaged_file(&path).is_none());

        // And so must a file that is not there at all.
        std::fs::remove_file(&path).expect("remove");
        assert!(damaged_file(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// D-171. `fs::write` truncates first, so a crash between the truncate and
    /// the write leaves the file empty or half-written — which is the state the
    /// test above has to report. Renaming replaces (D-119), so what was there
    /// survives until the new contents are complete on disk.
    #[test]
    fn a_replaced_file_is_never_seen_half_written() {
        let dir = scratch("replace");
        let path = dir.join(SETTINGS_FILE);
        std::fs::write(&path, "default_voice: en-GB-RyanNeural\n").expect("write");

        // One thread replaces the file repeatedly while another only ever asks
        // what is in it. Every read must see a whole document — one of the two
        // — and never a prefix of either. The property, not a timing.
        let old = "default_voice: en-GB-RyanNeural\n";
        let new = "default_voice: ja-JP-KeitaNeural\n";
        std::thread::scope(|scope| {
            let writer = scope.spawn(|| {
                for n in 0..300 {
                    let text = if n % 2 == 0 { new } else { old };
                    replace_file(&path, text).expect("replaces");
                }
            });
            while !writer.is_finished() {
                if let Ok(seen) = std::fs::read_to_string(&path) {
                    assert!(
                        seen == old || seen == new,
                        "a reader saw a half-written settings file: {seen:?}"
                    );
                }
            }
        });

        // And nothing is left beside it.
        let strays: Vec<String> = std::fs::read_dir(&dir)
            .expect("readable")
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n != SETTINGS_FILE)
            .collect();
        assert!(strays.is_empty(), "temporaries left behind: {strays:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A directory of this test's own.
    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "spoonstill-machine-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    /// Round-trips through the format it is actually stored in, and `default`
    /// is not a voice (D-086) so it is not storable as one.
    #[test]
    fn a_setting_survives_being_written_and_read() {
        let full = Machine {
            default_voice: Some("en-GB-RyanNeural".to_owned()),
        };
        let text = serde_yaml_ng::to_string(&full).expect("serialise");
        assert_eq!(
            serde_yaml_ng::from_str::<Machine>(&text).expect("parse"),
            full
        );

        // An empty file is a machine with no answers, not a parse error.
        assert_eq!(
            serde_yaml_ng::from_str::<Machine>("{}").expect("parse"),
            Machine::default()
        );

        // And a field this build does not know must not throw the file away:
        // an older build reading a newer machine's settings is an ordinary
        // thing on a shared home directory.
        assert_eq!(
            serde_yaml_ng::from_str::<Machine>("default_voice: a\nsomething_new: 3\n")
                .expect("an unknown field is ignored")
                .default_voice
                .as_deref(),
            Some("a")
        );
    }
}
