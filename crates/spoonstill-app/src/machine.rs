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

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The file, inside [`spoonstill_state::runs::config_dir`].
pub const SETTINGS_FILE: &str = "settings.yaml";

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

/// Read them, or the defaults.
///
/// Every failure here means "there are none yet", which is the normal state of
/// a machine nobody has configured and is never an error in front of anyone —
/// the same bargain `recent_projects` makes.
#[must_use]
pub fn load() -> Machine {
    path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_yaml_ng::from_str(&text).ok())
        .unwrap_or_default()
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
    std::fs::write(&path, text).map_err(|e| format!("writing {}: {e}", path.display()))
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
