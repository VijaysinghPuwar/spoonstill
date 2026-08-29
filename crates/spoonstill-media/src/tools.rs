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
    let on_path = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|path| std::env::split_paths(&path).collect::<Vec<_>>());

    locate_in(on_path.chain(package_manager_dirs()), program)
        .unwrap_or_else(|| PathBuf::from(program))
}

/// The search itself, over whichever directories the caller offers.
///
/// Separate from [`locate`] so the interesting case is testable without
/// touching the process environment: **a caller that offers no `PATH` at all**
/// is exactly the GUI process D-103 is about, and `set_var` is not something
/// one test may do to every other test in the binary.
fn locate_in(dirs: impl IntoIterator<Item = PathBuf>, program: &str) -> Option<PathBuf> {
    let name = executable_name(program);
    dirs.into_iter()
        .map(|dir| dir.join(&name))
        .find(|candidate| is_executable(candidate))
}

/// What the file is actually called on this platform.
fn executable_name(program: &str) -> String {
    if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_owned()
    }
}

/// The install prefixes of the package managers the README names, and nothing
/// else.
///
/// Deliberately short. This is not a hunt for any FFmpeg anywhere on the disk —
/// it is the handful of directories that `brew install ffmpeg`,
/// `winget install Gyan.FFmpeg` and `pipx install edge-tts` write to, which is
/// precisely the set a GUI process is missing from its `PATH`.
///
/// The per-user half of the list is D-104's: `pipx` and `pip --user` are how
/// the README and the window's own Install button fetch `edge-tts`, and they
/// write under the home directory rather than under a system prefix. A GUI
/// process is missing those for the same launchd reason it is missing
/// Homebrew's, so the install succeeds and the tool stays unfindable.
fn package_manager_dirs() -> Vec<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut dirs: Vec<PathBuf> = ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .iter()
            .map(PathBuf::from)
            .collect();
        if let Some(home) = home_dir() {
            // pipx, then the versioned directory `python3 -m pip install
            // --user` writes to — `~/Library/Python/3.14/bin` today and a
            // different number after the next Python, which is why it is read
            // rather than spelled out.
            dirs.push(home.join(".local/bin"));
            dirs.extend(subdirectories(&home.join("Library/Python"), "bin"));
        }
        dirs
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
        if let Some(home) = home_dir() {
            dirs.push(home.join(r"scoop\shims"));
            // pipx's own shims, then `pip install --user`, whose directory
            // carries the Python version: `%APPDATA%\Python\Python314\Scripts`.
            dirs.push(home.join(r".local\bin"));
        }
        if let Some(roaming) = std::env::var_os("APPDATA") {
            dirs.extend(subdirectories(
                &PathBuf::from(&roaming).join("Python"),
                "Scripts",
            ));
        }
        dirs
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let mut dirs: Vec<PathBuf> = [
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/snap/bin",
            "/home/linuxbrew/.linuxbrew/bin",
        ]
        .iter()
        .map(PathBuf::from)
        .collect();
        if let Some(home) = home_dir() {
            dirs.push(home.join(".local/bin"));
        }
        dirs
    }
}

/// This user's home directory, from the environment and nothing else.
///
/// No `dirs` crate: `spoonstill-media` has three dependencies and this is one
/// variable. A GUI process launched by launchd or Explorer does lose `PATH`,
/// but it keeps `HOME` / `USERPROFILE` — that is what makes the per-user half
/// of [`package_manager_dirs`] reachable at all.
fn home_dir() -> Option<PathBuf> {
    let key = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(key)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// `parent/*/leaf`, for the one case where a package manager puts a version
/// number in the path.
///
/// Sorted, so the answer does not depend on the order the file system hands
/// entries back, and so the *newest* Python's directory is searched last
/// rather than arbitrarily — a machine with two of them has two `edge-tts`
/// shims and either runs.
fn subdirectories(parent: &Path, leaf: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path().join(leaf))
        .filter(|dir| dir.is_dir())
        .collect();
    found.sort();
    found
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

    /// The whole of D-103, deterministically: a search that is offered **no
    /// `PATH`** still finds a binary sitting in one of the directories a
    /// package manager writes to.
    ///
    /// That is precisely a macOS app launched from Finder, which launchd hands
    /// `/usr/bin:/bin:/usr/sbin:/sbin` and nothing else. The directory here is
    /// a temporary one rather than the machine's real Homebrew, so this proves
    /// the mechanism on a runner that has FFmpeg nowhere at all.
    #[test]
    fn a_process_with_no_useful_path_still_finds_an_installed_binary() {
        let dir = std::env::temp_dir().join(format!("spoonstill-d103-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        let planted = dir.join(executable_name("ffprobe"));
        // Contents are irrelevant and deliberately not a script: this file is
        // found, never run, and `no_shell_strings.rs` is right to object to a
        // shell path appearing anywhere in this crate's source.
        std::fs::write(&planted, "not a real program").expect("plant a binary");
        make_executable(&planted);

        let found = locate_in([dir.clone()], "ffprobe").expect("the planted binary is found");
        assert_eq!(found, planted);
        assert!(found.is_absolute(), "{}", found.display());

        // And a program that is not in that directory is not invented.
        assert_eq!(locate_in([dir.clone()], "ffmpeg"), None);

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Whatever this machine has, `locate` answers with something the spawn can
    /// use: an absolute path when FFmpeg is installed, the bare name when it is
    /// not — never a half-formed guess.
    #[test]
    fn locate_answers_with_an_absolute_path_or_the_bare_name() {
        let found = locate("ffprobe");
        assert!(
            found.is_absolute() || found == Path::new("ffprobe"),
            "{}",
            found.display()
        );
        if found.is_absolute() {
            assert!(is_executable(&found), "{}", found.display());
        }
    }

    /// Give a planted file the execute bit, where the platform has one.
    fn make_executable(path: &Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
                .expect("chmod +x");
        }
        #[cfg(not(unix))]
        {
            let _ = path;
        }
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

    /// D-104. `pipx install edge-tts` writes under the home directory, and a
    /// Finder-launched window is missing that for the same reason it is
    /// missing Homebrew's `bin`. If the per-user prefixes are not searched,
    /// the window's own Install button succeeds and the tool stays
    /// unfindable.
    #[test]
    fn the_per_user_install_prefixes_are_searched_too() {
        let Some(home) = home_dir() else {
            return; // A machine with no `HOME` is not one we can say this about.
        };
        let dirs = package_manager_dirs();

        #[cfg(windows)]
        let user_local = home.join(r".local\bin");
        #[cfg(not(windows))]
        let user_local = home.join(".local/bin");
        assert!(
            dirs.contains(&user_local),
            "pipx's directory is missing: {dirs:?}"
        );
        assert!(
            dirs.iter().all(|dir| dir.is_absolute()),
            "a relative candidate would be resolved against the working \
             directory, which is the bug this whole module exists for: {dirs:?}"
        );
    }

    /// The versioned directory `pip install --user` writes to is read from the
    /// disk rather than spelled out, because the number changes with Python.
    #[test]
    fn a_versioned_directory_is_found_by_reading_and_not_by_guessing() {
        let scratch = std::env::temp_dir().join(format!(
            "spoonstill-tools-{}-{}",
            std::process::id(),
            line!()
        ));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(scratch.join("3.14/bin")).expect("scratch");
        std::fs::create_dir_all(scratch.join("3.9/bin")).expect("scratch");
        // A version that installed nothing, and a stray file: neither is a
        // directory to search.
        std::fs::create_dir_all(scratch.join("3.13")).expect("scratch");
        std::fs::write(scratch.join("README"), b"").expect("scratch");

        let found = subdirectories(&scratch, "bin");
        assert_eq!(
            found,
            vec![scratch.join("3.14/bin"), scratch.join("3.9/bin")],
            "sorted, and only the ones that are really there"
        );
        assert!(
            subdirectories(&scratch.join("no-such-thing"), "bin").is_empty(),
            "a parent that does not exist is not an error"
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
