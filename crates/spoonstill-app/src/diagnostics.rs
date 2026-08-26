//! Exporting a diagnostics bundle (see `spoonstill-state::logs`).
//!
//! The operator-facing story is one sentence: *when something fails on your
//! machine, run one command and send us the file it produces.* Everything here
//! exists to make that sentence true — in particular the environment block,
//! because "it fails for me" and "it works for us" are usually a difference in
//! FFmpeg build, OS version, or CPU architecture, and none of those are visible
//! in a log line.

use std::path::{Path, PathBuf};

use spoonstill_media::{Tools, command::FfmpegCommand};
use spoonstill_state::{BundleReport, EnvironmentLine, write_bundle};

/// How long to wait for a version probe before giving up on it.
const VERSION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Collect the environment facts a remote diagnosis actually turns on.
#[must_use]
pub fn environment() -> Vec<EnvironmentLine> {
    let mut lines = vec![
        line("spoonstill", env!("CARGO_PKG_VERSION")),
        line("os", std::env::consts::OS),
        line("arch", std::env::consts::ARCH),
        line("family", std::env::consts::FAMILY),
    ];

    let tools = Tools::from_env();
    lines.push(line("ffmpeg path", tools.ffmpeg().display().to_string()));
    lines.push(line("ffmpeg", first_line(&version_of(tools.ffmpeg()))));
    lines.push(line("ffprobe path", tools.ffprobe().display().to_string()));
    lines.push(line("ffprobe", first_line(&version_of(tools.ffprobe()))));

    // The build configuration, not just the version: a missing libx264 or a
    // GPL-vs-LGPL difference (D-062) explains a whole class of report.
    lines.push(line("ffmpeg build", configuration_of(tools.ffmpeg())));

    lines.push(line(
        "working directory",
        std::env::current_dir().map_or_else(|_| "<unknown>".into(), |p| p.display().to_string()),
    ));
    lines
}

fn line(key: &str, value: impl Into<String>) -> EnvironmentLine {
    EnvironmentLine {
        key: key.to_string(),
        value: value.into(),
    }
}

/// Ask a binary for its version, tolerating its absence.
///
/// A missing FFmpeg is itself the most useful line the bundle can carry, so
/// this reports the failure rather than omitting the field.
fn version_of(program: &Path) -> String {
    let mut command = FfmpegCommand::new(program);
    command.args(["-hide_banner", "-version"]);
    match command.spawn().and_then(|c| c.wait_until(VERSION_TIMEOUT)) {
        Ok(finished) => String::from_utf8_lossy(&finished.stdout).into_owned(),
        Err(error) => format!("<could not be run: {error}>"),
    }
}

fn first_line(text: &str) -> String {
    text.lines()
        .next()
        .unwrap_or("<no output>")
        .trim()
        .to_string()
}

/// The `configuration:` line from `ffmpeg -version`, condensed.
fn configuration_of(program: &Path) -> String {
    let text = version_of(program);
    let Some(configuration) = text
        .lines()
        .find(|l| l.trim_start().starts_with("configuration:"))
    else {
        return "<not reported>".to_string();
    };

    // The full configuration line is hundreds of characters of flags. Keep the
    // ones that change behaviour we care about, and say how many were dropped
    // rather than quietly truncating.
    let flags: Vec<&str> = configuration.split_whitespace().skip(1).collect();
    let interesting: Vec<&str> = flags
        .iter()
        .copied()
        .filter(|f| {
            f.contains("gpl")
                || f.contains("libx264")
                || f.contains("videotoolbox")
                || f.contains("nvenc")
                || f.contains("nonfree")
                || f.contains("version")
        })
        .collect();
    format!(
        "{} ({} other flags not shown)",
        interesting.join(" "),
        flags.len().saturating_sub(interesting.len())
    )
}

/// Write a diagnostics bundle for the project rooted at `project_root`.
///
/// # Errors
///
/// Any filesystem error writing `destination`.
pub fn export(project_root: &Path, destination: &Path) -> std::io::Result<BundleReport> {
    write_bundle(project_root, destination, &environment())
}

/// The default name for an exported bundle.
///
/// Names itself so an operator who exports twice does not overwrite the first
/// one, and so a file sitting in a downloads folder still says what it is.
#[must_use]
pub fn default_bundle_name(stamp: &str) -> PathBuf {
    PathBuf::from(format!("spoonstill-diagnostics-{stamp}.txt"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The facts that separate "works for us" from "fails for them".
    #[test]
    fn the_environment_names_the_build_under_test() {
        let env = environment();
        let keys: Vec<&str> = env.iter().map(|l| l.key.as_str()).collect();
        for expected in [
            "spoonstill",
            "os",
            "arch",
            "ffmpeg",
            "ffmpeg path",
            "ffmpeg build",
        ] {
            assert!(keys.contains(&expected), "{expected} missing from {keys:?}");
        }
    }

    /// A missing FFmpeg is the most useful line a bundle can carry, so it must
    /// be reported rather than omitted.
    #[test]
    fn a_missing_binary_is_reported_not_skipped() {
        let text = version_of(Path::new("/nonexistent/spoonstill/ffmpeg"));
        assert!(text.contains("could not be run"), "{text}");
        assert!(text.contains("/nonexistent/spoonstill/ffmpeg"), "{text}");
    }

    /// The configuration line must say how much it left out, rather than
    /// truncating silently.
    #[test]
    fn the_build_configuration_admits_what_it_omitted() {
        let tools = Tools::from_env();
        let text = configuration_of(tools.ffmpeg());
        assert!(
            text.contains("not shown") || text.contains("not reported"),
            "{text}"
        );
    }

    #[test]
    fn a_bundle_name_carries_its_stamp() {
        let name = default_bundle_name("20260826-123456");
        assert_eq!(
            name,
            PathBuf::from("spoonstill-diagnostics-20260826-123456.txt")
        );
    }

    /// End to end: a project with no logs still exports a file that explains
    /// itself, so "run this and send me the file" never produces nothing.
    #[test]
    fn exporting_an_untouched_project_still_produces_a_usable_file() {
        let dir = std::env::temp_dir().join(format!("spoonstill-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let destination = dir.join("bundle.txt");

        let report = export(&dir, &destination).unwrap();
        assert_eq!(report.records, 0);

        let text = std::fs::read_to_string(&destination).unwrap();
        assert!(text.contains("ENVIRONMENT"));
        assert!(text.contains("no log files found"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
