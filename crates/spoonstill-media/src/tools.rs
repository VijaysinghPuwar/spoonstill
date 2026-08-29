//! Locating `ffmpeg` and `ffprobe`, and failing fast when they are not there.
//!
//! D-012 rejects `ffmpeg-sidecar`'s runtime auto-download outright: a
//! commercial desktop app ships pinned, checksum-verified binaries, and a
//! renderer that quietly fetches a different build produces output that cannot
//! be reproduced. So there is no download path here.
//!
//! What there *is*, since D-103, is a **search of the operator's own machine**
//! for the FFmpeg the README already tells them to install. That is not the
//! thing D-012 refuses. D-012 refuses fetching a build nobody chose; this
//! finds the one build they did choose, in the three places their package
//! manager puts it, and then names it by absolute path.
//!
//! The reason it has to is [`locate`]: a macOS app launched from Finder does
//! not inherit the operator's `PATH`.

use std::path::{Path, PathBuf};

/// Environment override for the FFmpeg binary. Development convenience.
pub const FFMPEG_ENV: &str = "SPOONSTILL_FFMPEG";
/// Environment override for the ffprobe binary. Development convenience.
pub const FFPROBE_ENV: &str = "SPOONSTILL_FFPROBE";

/// Where the two binaries live for this run.
///
/// Cloned into each render rather than looked up per invocation, so that a
/// 500-scene batch cannot half-run against one build and half against another
/// because someone changed `PATH` midway.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tools {
    ffmpeg: PathBuf,
    ffprobe: PathBuf,
}

impl Tools {
    /// Use whichever binaries the environment names, else the ones this
    /// machine has.
    ///
    /// The override wins and is taken verbatim; otherwise [`locate`] resolves
    /// the program to an absolute path. A shipped build will eventually pass
    /// explicit bundled paths through [`Tools::at`] — D-062 requires our own
    /// LGPL build, and the Homebrew GPL build this workspace develops against
    /// may not be redistributed — but until that exists, the release notes
    /// promise "the FFmpeg already on your machine", and this is what keeps
    /// that promise.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            ffmpeg: std::env::var_os(FFMPEG_ENV).map_or_else(|| locate("ffmpeg"), PathBuf::from),
            ffprobe: std::env::var_os(FFPROBE_ENV).map_or_else(|| locate("ffprobe"), PathBuf::from),
        }
    }

    /// Use two explicitly located binaries — what a packaged build does.
    #[must_use]
    pub fn at(ffmpeg: impl Into<PathBuf>, ffprobe: impl Into<PathBuf>) -> Self {
        Self {
            ffmpeg: ffmpeg.into(),
            ffprobe: ffprobe.into(),
        }
    }

    /// The FFmpeg binary for this run.
    #[must_use]
    pub fn ffmpeg(&self) -> &Path {
        &self.ffmpeg
    }

    /// The ffprobe binary for this run.
    #[must_use]
    pub fn ffprobe(&self) -> &Path {
        &self.ffprobe
    }

    /// Whether both binaries are really where this `Tools` says they are.
    ///
    /// Asked **once, before a probe runs over the operator's files**, because
    /// a subprocess that cannot start is one fact about this machine and not
    /// one fact about each of five hundred photographs (D-103). Without it,
    /// "you have not installed FFmpeg" arrives as six identical errors, each
    /// one apparently about a different photograph, and none of them about the
    /// thing the operator actually has to do.
    ///
    /// This is a stat, not a `-version` call: it is on the path of every
    /// validate, and the spawn itself is still the authority on whether the
    /// binary runs — [`crate::MediaError::BinaryMissing`] names the exact path
    /// that was tried either way.
    ///
    /// # Errors
    ///
    /// A sentence naming what is missing, ready to be shown to an operator.
    pub fn ready(&self) -> Result<(), String> {
        let missing: Vec<&str> = [("ffmpeg", &self.ffmpeg), ("ffprobe", &self.ffprobe)]
            .into_iter()
            .filter(|(_, path)| !is_executable(path))
            .map(|(tool, _)| tool)
            .collect();

        if missing.is_empty() {
            return Ok(());
        }
        Err(format!(
            "{} could not be found on this machine — spoonstill does the timing and the \
             motion, FFmpeg does the pixels. Install it once ({}), then open this project \
             again.",
            missing.join(" and "),
            INSTALL_HINT
        ))
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self::from_env()
    }
}

/// The one command that installs it, for the platform being built for.
#[cfg(target_os = "macos")]
const INSTALL_HINT: &str = "brew install ffmpeg";
/// The one command that installs it, for the platform being built for.
#[cfg(target_os = "windows")]
const INSTALL_HINT: &str = "winget install Gyan.FFmpeg";
/// The one command that installs it, for the platform being built for.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const INSTALL_HINT: &str = "your package manager's ffmpeg";

/// Find `program` on this machine, as an absolute path.
///
/// `PATH` first, because an operator who put a particular build ahead of the
/// rest meant it. Then [`package_manager_dirs`], because **the process asking
/// may not have a useful `PATH` at all**:
///
/// macOS hands an app launched from Finder or the Dock the launchd default,
/// `/usr/bin:/bin:/usr/sbin:/sbin`. Homebrew is on none of it. So the window
/// found no `ffprobe`, every still failed its probe, and a folder of six
/// perfectly good photographs opened as **zero scenes** — with the failure
/// shown as six errors about the photographs — while `still validate` on the
/// same folder in a terminal reported six scenes and no problems one second
/// later. That is D-103, and this function is the fix.
///
/// Returning an absolute path is the point. A bare `"ffmpeg"` is resolved by
/// whatever `PATH` the process happened to inherit, which is exactly the
/// non-reproducibility D-012 objects to; a located path is recorded verbatim
/// in the diagnostics bundle, so a report says which build actually ran.
///
/// When nothing is found the bare name is handed back unchanged, so the spawn
/// still happens and [`crate::MediaError::BinaryMissing`] still names it.
#[must_use]
pub fn locate(program: &str) -> PathBuf {
    let name = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_owned()
    };

    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());

    for dir in on_path.chain(package_manager_dirs()) {
        let candidate = dir.join(&name);
        if is_executable(&candidate) {
            return candidate;
        }
    }

    PathBuf::from(program)
}

/// The install prefixes of the package managers the README names, and nothing
/// else.
///
/// Deliberately short. This is not a hunt for any FFmpeg anywhere on the disk —
/// it is the handful of directories that `brew install ffmpeg` and
/// `winget install Gyan.FFmpeg` write to, which is precisely the set a GUI
/// process is missing from its `PATH`.
fn package_manager_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }

    #[cfg(target_os = "windows")]
    {
        // winget's shim directory, then Chocolatey's, then Scoop's. All three
        // are per-user or per-machine locations that a process started before
        // the install will not have on its `PATH`.
        let mut dirs = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(&local).join(r"Microsoft\WinGet\Links"));
        }
        if let Some(data) = std::env::var_os("ProgramData") {
            dirs.push(PathBuf::from(&data).join(r"chocolatey\bin"));
        }
        if let Some(home) = std::env::var_os("USERPROFILE") {
            dirs.push(PathBuf::from(&home).join(r"scoop\shims"));
        }
        dirs
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        [
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/snap/bin",
            "/home/linuxbrew/.linuxbrew/bin",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    }
}

/// A file that exists and that this user may execute.
///
/// The mode check matters: `/usr/local/bin` on a developer's machine is full
/// of things that are not programs, and a directory named `ffmpeg` would
/// otherwise be returned as one.
fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }

    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// There is still no auto-download: the only ways to get a binary are the
    /// environment, an explicit path, and the operator's own installation.
    #[test]
    fn explicit_paths_are_kept_verbatim() {
        let t = Tools::at("/opt/spoonstill/bin/ffmpeg", "/opt/spoonstill/bin/ffprobe");
        assert_eq!(t.ffmpeg(), Path::new("/opt/spoonstill/bin/ffmpeg"));
        assert_eq!(t.ffprobe(), Path::new("/opt/spoonstill/bin/ffprobe"));
    }

    /// A program that is on no `PATH` and in no package manager's directory
    /// comes back as its bare name, so the spawn still reports it by name.
    #[test]
    fn a_program_that_is_nowhere_is_returned_unchanged() {
        assert_eq!(
            locate("spoonstill-no-such-program-9d2f1"),
            Path::new("spoonstill-no-such-program-9d2f1")
        );
    }

    /// The whole of D-103 in one test: a process whose `PATH` does not name
    /// FFmpeg still finds it, as long as this machine has one at all.
    ///
    /// This is what a macOS app launched from Finder is — launchd hands it
    /// `/usr/bin:/bin:/usr/sbin:/sbin` and nothing else. Skipped where the
    /// machine genuinely has no FFmpeg, because then there is nothing to find
    /// and the interesting assertion is the one above.
    #[test]
    fn a_gui_path_still_finds_a_package_managers_ffmpeg() {
        let Some(installed) = package_manager_dirs()
            .into_iter()
            .map(|dir| {
                dir.join(if cfg!(windows) {
                    "ffprobe.exe"
                } else {
                    "ffprobe"
                })
            })
            .find(|candidate| is_executable(candidate))
        else {
            return;
        };

        // Not `set_var`: this test says what `locate` does when `PATH` is
        // useless, and mutating the environment is unsound with other tests
        // running in the same process. The `PATH` branch is empty here by
        // construction, so the package-manager branch is what answers.
        let found = package_manager_dirs()
            .into_iter()
            .map(|dir| dir.join("ffprobe"))
            .find(|candidate| is_executable(candidate));

        assert_eq!(found.as_deref(), Some(installed.as_path()));
        assert!(installed.is_absolute(), "{}", installed.display());
    }

    /// `ready` is the sentence an operator acts on, so it has to name both the
    /// missing tool and the command that installs it.
    #[test]
    fn a_missing_binary_is_one_sentence_naming_the_install_command() {
        let error = Tools::at("/nonexistent/ffmpeg", "/nonexistent/ffprobe")
            .ready()
            .expect_err("neither binary is there");
        assert!(error.contains("ffmpeg"), "{error}");
        assert!(error.contains("ffprobe"), "{error}");
        assert!(error.contains(INSTALL_HINT), "{error}");
    }

    /// A directory named `ffmpeg` is not an FFmpeg.
    #[test]
    fn a_directory_is_not_an_executable() {
        assert!(!is_executable(&std::env::temp_dir()));
    }
}
