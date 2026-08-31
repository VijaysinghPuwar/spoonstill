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
use std::time::Duration;

use spoonstill_core::Remedy;

use crate::MediaError;
use crate::command::FfmpegCommand;

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
    /// A [`Remedy`]: one plain sentence for the operator, the technical half
    /// kept separate, and the fact that this application can install FFmpeg
    /// itself — which is what turns the report into a button (D-105).
    pub fn ready(&self) -> Result<(), Remedy> {
        let missing: Vec<(&str, &PathBuf)> = [("ffmpeg", &self.ffmpeg), ("ffprobe", &self.ffprobe)]
            .into_iter()
            .filter(|(_, path)| !is_executable(path))
            .collect();

        if missing.is_empty() {
            return Ok(());
        }

        // Both halves ship in one package everywhere, so "ffmpeg and ffprobe
        // are missing" is one fact with two names attached and reads to an
        // operator as two problems. The sentence names the product; the detail
        // names the files.
        let tried = missing
            .iter()
            .map(|(tool, path)| format!("{tool}: tried {}", path.display()))
            .collect::<Vec<_>>()
            .join("; ");

        Err(Remedy::installable(
            format!(
                "Spoonstill needs FFmpeg to turn your photos into video, and it is not \
                 installed yet. Press Install and it will be fetched with {INSTALL_WITH}.",
            ),
            FFMPEG_TOOL,
            tried,
        ))
    }
}

impl Default for Tools {
    fn default() -> Self {
        Self::from_env()
    }
}

/// The id a control surface hands back to [`install`]. One word, because it
/// crosses a JSON boundary into the window (D-105).
pub const FFMPEG_TOOL: &str = "ffmpeg";

/// The package manager named in the plain sentence, so the operator knows what
/// the button is about to run before they press it.
#[cfg(target_os = "macos")]
const INSTALL_WITH: &str = "Homebrew";
/// The package manager named in the plain sentence.
#[cfg(target_os = "windows")]
const INSTALL_WITH: &str = "winget";
/// The package manager named in the plain sentence.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const INSTALL_WITH: &str = "your package manager";

/// The package managers to try, in the order that works most often.
///
/// The same table, and the same reasoning, as `spoonstill_tts::edge`'s: the
/// first candidate that both exists and succeeds wins, and every one of them
/// is the platform's own package manager. Nothing is downloaded from us —
/// D-012 refuses a build nobody chose, and this is the operator pressing a
/// button that says install (D-105).
#[cfg(target_os = "macos")]
const INSTALLERS: &[(&str, &[&str])] = &[
    ("brew", &["install", "ffmpeg"]),
    ("port", &["install", "ffmpeg"]),
];

/// Windows has no Homebrew. winget ships with the OS; the other two are there
/// only if the operator put them there, which is why they come after it.
#[cfg(target_os = "windows")]
const INSTALLERS: &[(&str, &[&str])] = &[
    (
        "winget",
        &[
            "install",
            "--id",
            "Gyan.FFmpeg",
            "-e",
            "--accept-source-agreements",
            "--accept-package-agreements",
        ],
    ),
    ("choco", &["install", "ffmpeg", "-y"]),
    ("scoop", &["install", "ffmpeg"]),
];

/// Neither platform this project targets (D-071), but the module still builds.
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const INSTALLERS: &[(&str, &[&str])] = &[
    ("apt-get", &["install", "-y", "ffmpeg"]),
    ("dnf", &["install", "-y", "ffmpeg"]),
];

/// How long a package manager gets. FFmpeg is a large build with many
/// dependencies and a cold Homebrew can genuinely take minutes; the ceiling is
/// here so a wedged download eventually returns the button to the operator
/// rather than leaving "Installing…" on screen forever.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(900);

/// How long the post-install re-check gets. A stat and a spawn.
const VERIFY_TIMEOUT: Duration = Duration::from_secs(20);

/// Fetch FFmpeg through whichever package manager this machine has.
///
/// This is the second half of D-105, and it exists because the first half was
/// only ever true of speech: `edge-tts` had an Install button since D-092,
/// while FFmpeg — which every single render needs — offered the operator the
/// string `brew install ffmpeg` and no way to run it. The screen that reported
/// the more serious of the two problems was the one that could do less about
/// it.
///
/// Located, not named, for D-104's reason: a package manager is exactly as
/// invisible to a Finder-launched window as the tool it installs, so an
/// Install button reached by bare name reports "brew is not on this machine"
/// on a machine that has it.
///
/// # Errors
///
/// Why an installer did not run, in the operator's terms.
///
/// The four ways `spawn().and_then(wait_until)` can fail mean four different
/// things, and only one of them is "install this first" (D-123). Collapsing
/// them into that one is a wrong diagnosis, not a vague one.
fn describe_failure(program: &str, error: &MediaError) -> String {
    match error {
        MediaError::BinaryMissing { .. } => format!("`{program}` is not on this machine"),
        MediaError::Timeout { waited, .. } => format!(
            "`{program}` was still running after {} minutes and was stopped — \
             it is installed, and the install did not finish in time",
            waited.as_secs() / 60
        ),
        MediaError::Cancelled { .. } => format!("`{program}` was cancelled"),
        MediaError::Spawn { source, .. } => format!("`{program}` could not be started: {source}"),
        other => format!("`{program}` failed: {other}"),
    }
}

/// A [`Remedy`] naming every candidate that was tried and what each said. Not
/// installable — pressing the button again would do the same thing, and the
/// operator now needs the detail rather than another press.
pub fn install() -> Result<String, Remedy> {
    // Already there. Fifteen minutes of a package manager re-resolving a
    // dependency tree is not what a button press asked for.
    if Tools::from_env().ready().is_ok() {
        return Ok("FFmpeg is already installed".to_owned());
    }

    let mut tried: Vec<String> = Vec::new();

    for (program, args) in INSTALLERS {
        let mut command = FfmpegCommand::new(locate(program));
        command.args(*args);
        let shown = command.display();

        match command.spawn().and_then(|c| c.wait_until(INSTALL_TIMEOUT)) {
            Ok(finished) if finished.status.success() => {
                // Present is not the same as runnable, and the paths this
                // process started with are stale by construction: the binaries
                // did not exist when `from_env` looked. Locate them again, and
                // only then claim success (D-104).
                return match verify_installed() {
                    Ok(()) => Ok(shown),
                    Err(detail) => Err(Remedy::manual(
                        "The install finished, but FFmpeg still cannot be run. It may have \
                         landed somewhere this application does not look — restarting \
                         spoonstill is the usual fix.",
                        format!("`{shown}` succeeded; {detail}"),
                    )),
                };
            }
            Ok(finished) => tried.push(format!(
                "`{shown}` exited {}: {}",
                finished.status.code().unwrap_or(-1),
                last_line(&finished.stderr)
            )),
            // Why it did not run, rather than one guess for every cause
            // (D-123). `brew install ffmpeg` on a slow connection genuinely
            // outlives the 15-minute ceiling, and telling that operator their
            // package manager "is not on this machine" sends them to install
            // the one they already have.
            Err(error) => tried.push(describe_failure(program, &error)),
        }
    }

    Err(Remedy::manual(
        format!(
            "Spoonstill could not install FFmpeg for you — none of the package managers it \
             knows about worked on this machine. Installing {INSTALL_WITH} first, then \
             pressing Install again, is the usual fix."
        ),
        tried.join("; "),
    ))
}

/// Whether a freshly installed FFmpeg actually runs, located from scratch.
fn verify_installed() -> Result<(), String> {
    for program in ["ffmpeg", "ffprobe"] {
        let path = locate(program);
        let mut command = FfmpegCommand::new(&path);
        command.arg("-version");
        match command.spawn().and_then(|c| c.wait_until(VERIFY_TIMEOUT)) {
            Ok(finished) if finished.status.success() => {}
            Ok(finished) => {
                return Err(format!(
                    "`{} -version` exited {}",
                    path.display(),
                    finished.status.code().unwrap_or(-1)
                ));
            }
            Err(_) => return Err(format!("{} is still not runnable", path.display())),
        }
    }
    Ok(())
}

/// The last thing a failing tool said, which is the part that names the cause.
///
/// A package manager prints pages; an error box holds a sentence. Same rule,
/// and the same function, as `spoonstill_tts::edge`'s.
fn last_line(stderr: &str) -> String {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no output")
        .to_owned()
}

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
            let winget = PathBuf::from(&local).join(r"Microsoft\WinGet");
            // The shim directory first, where a package that publishes one puts
            // a name that survives an upgrade.
            dirs.push(winget.join("Links"));
            // Then the package's own unpacked build, because `Gyan.FFmpeg` —
            // the id `INSTALL_HINT` tells the operator to run — publishes **no**
            // shim: it lands under `WinGet\Packages\Gyan.FFmpeg_<source>\
            // ffmpeg-<version>-full_build\bin` and leaves `Links` empty, so the
            // operator who ran exactly the command the error named still had no
            // FFmpeg this search could see (D-142).
            dirs.extend(winget_ffmpeg_bins(&winget.join("Packages")));
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

/// Every `bin` directory belonging to a winget-installed `Gyan.FFmpeg`.
///
/// The layout is `Packages\Gyan.FFmpeg_<source>\<build>\bin`: two levels, the
/// first filtered to the one package this project names so this stays the set
/// of directories a *named* install writes to and not a hunt across the disk.
///
/// Newest build last for the same reason [`subdirectories`] sorts: a machine
/// that upgraded but kept the old version has two, and searching them in a
/// defined order beats an arbitrary one.
#[cfg(target_os = "windows")]
fn winget_ffmpeg_bins(packages: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(packages) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = entries
        .flatten()
        .filter(|package| {
            package
                .file_name()
                .to_string_lossy()
                .starts_with("Gyan.FFmpeg")
        })
        .flat_map(|package| subdirectories(&package.path(), "bin"))
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

    /// D-142: `winget install Gyan.FFmpeg` — the exact command the missing-tool
    /// remedy prints — unpacks to `Packages\Gyan.FFmpeg_<source>\<build>\bin`
    /// and publishes no shim, so the operator who ran it still had a FFmpeg this
    /// search could not see. The planted tree is that layout, plus one package
    /// that must be ignored.
    #[test]
    #[cfg(target_os = "windows")]
    fn a_winget_package_build_of_ffmpeg_is_found_without_a_shim() {
        let root = std::env::temp_dir().join(format!("spoonstill-d142-{}", std::process::id()));
        let packages = root.join("Packages");
        let bin = packages
            .join("Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe")
            .join("ffmpeg-9.0.1-full_build")
            .join("bin");
        std::fs::create_dir_all(&bin).expect("planted build tree");
        let planted = bin.join(executable_name("ffmpeg"));
        std::fs::write(&planted, "not a real program").expect("plant ffmpeg.exe");

        // A neighbour that is not this package is left out of the answer.
        std::fs::create_dir_all(packages.join("Some.Other_x").join("v1").join("bin"))
            .expect("planted decoy");

        let bins = winget_ffmpeg_bins(&packages);
        assert_eq!(bins, vec![bin.clone()], "only the Gyan.FFmpeg build");
        assert_eq!(
            locate_in(bins, "ffmpeg"),
            Some(planted),
            "and locate finds ffmpeg.exe inside it"
        );

        std::fs::remove_dir_all(&root).ok();
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

    /// D-105. `ready` is what an operator acts on, so it is a [`Remedy`] and
    /// not a sentence: the plain half names the product and no paths, the
    /// technical half names both files that were tried, and it is installable
    /// — which is the whole difference between a report and a button.
    #[test]
    fn a_missing_binary_is_a_remedy_with_a_button_on_it() {
        let remedy = Tools::at("/nonexistent/ffmpeg", "/nonexistent/ffprobe")
            .ready()
            .expect_err("neither binary is there");

        assert_eq!(
            remedy.install.as_deref(),
            Some(FFMPEG_TOOL),
            "the window cannot draw a button without this"
        );
        assert!(remedy.need.contains("FFmpeg"), "{}", remedy.need);
        assert!(
            !remedy.need.contains('/'),
            "a path is detail, not a sentence: {}",
            remedy.need
        );
        // Both halves named, because a bundle that says only "ffmpeg" cannot
        // tell a broken install from a half-installed one.
        assert!(
            remedy.detail.contains("/nonexistent/ffmpeg"),
            "{}",
            remedy.detail
        );
        assert!(
            remedy.detail.contains("/nonexistent/ffprobe"),
            "{}",
            remedy.detail
        );
    }

    /// One of the two present and the other missing is still one problem, and
    /// the detail names which one it was.
    #[test]
    fn one_half_missing_still_names_which_half() {
        let real = locate("ffprobe");
        if !is_executable(&real) {
            return; // Nothing to be half-installed from.
        }
        let remedy = Tools::at("/nonexistent/ffmpeg", &real)
            .ready()
            .expect_err("ffmpeg is not there");
        assert!(remedy.detail.contains("ffmpeg: tried"), "{}", remedy.detail);
        assert!(
            !remedy.detail.contains("ffprobe: tried"),
            "the one that is present is not a problem: {}",
            remedy.detail
        );
    }

    /// The installers are argument vectors of literals, and every one of them
    /// is a package manager rather than a URL. D-012 refuses fetching a build
    /// nobody chose; this table is only ever the platform's own.
    #[test]
    fn every_installer_is_a_package_manager_and_not_a_download() {
        assert!(
            !INSTALLERS.is_empty(),
            "a platform with no installer has no button"
        );
        for (program, args) in INSTALLERS {
            assert!(
                !program.contains(' '),
                "`{program}` is a command line, not a program"
            );
            for arg in *args {
                assert!(
                    !arg.starts_with("http"),
                    "`{program}` reaches for a URL, which is what D-012 refuses: {arg}"
                );
            }
        }
    }

    /// Install is not attempted on a machine that already has it — the guard
    /// is what keeps a mis-press from costing fifteen minutes of Homebrew.
    #[test]
    fn install_is_a_no_op_when_ffmpeg_is_already_there() {
        if Tools::from_env().ready().is_err() {
            return; // This machine has none, and the real path needs a network.
        }
        let said = install().expect("already installed is a success");
        assert!(said.contains("already"), "{said}");
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

    /// D-123. Four ways an installer can fail to run, and only one of them
    /// means "install this first".
    ///
    /// The realistic one is the timeout: `brew install ffmpeg` on a slow
    /// connection genuinely outlives the 15-minute ceiling, and the operator
    /// used to be told their package manager was not on the machine — sending
    /// them to install the one they had just watched run for a quarter of an
    /// hour.
    #[test]
    fn an_installer_that_fails_says_why_rather_than_guessing() {
        let missing = MediaError::BinaryMissing {
            tool: "brew",
            tried: PathBuf::from("brew"),
            source: std::io::Error::from(std::io::ErrorKind::NotFound),
        };
        assert_eq!(
            describe_failure("brew", &missing),
            "`brew` is not on this machine"
        );

        let timed_out = MediaError::Timeout {
            command: "brew install ffmpeg".to_owned(),
            waited: Duration::from_secs(900),
        };
        let said = describe_failure("brew", &timed_out);
        assert!(said.contains("15 minutes"), "{said}");
        assert!(
            said.contains("it is installed"),
            "a timeout must not read as an absence: {said}"
        );
        assert!(
            !said.contains("is not on this machine"),
            "the wrong diagnosis survived: {said}"
        );

        let refused = MediaError::Spawn {
            command: "brew install ffmpeg".to_owned(),
            source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
        };
        let said = describe_failure("brew", &refused);
        assert!(said.contains("could not be started"), "{said}");
        assert!(!said.contains("is not on this machine"), "{said}");

        let stopped = MediaError::Cancelled {
            command: "brew install ffmpeg".to_owned(),
        };
        assert!(describe_failure("brew", &stopped).contains("cancelled"));
    }
}
