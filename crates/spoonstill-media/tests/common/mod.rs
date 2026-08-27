//! Shared support for the integration tests.
//!
//! Test media is built here in Rust rather than by `scripts/gen-fixtures.sh`,
//! for two reasons. The matrix needs durations and sizes the M0 fixture set
//! does not contain; and D-071 puts Windows in scope from M1, where a bash
//! script is not a dependency the test suite may take.
//!
//! Everything is cached under `target/` and written with a temporary-then-
//! rename, so the parallel test threads inside one binary cannot race each
//! other into a half-written file.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use spoonstill_media::command::FfmpegCommand;
use spoonstill_media::tools::Tools;

/// Generous ceiling for a single test render.
pub const RENDER_TIMEOUT: Duration = Duration::from_secs(180);

/// The binaries under test.
pub fn tools() -> Tools {
    Tools::from_env()
}

/// Workspace root, from this crate's manifest directory.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root is two levels above the media manifest")
        .to_path_buf()
}

/// Where generated test media is cached between runs.
pub fn media_dir() -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("spoonstill-test-media");
    std::fs::create_dir_all(&dir).expect("create the test media directory");
    dir
}

/// A directory for one test's outputs, emptied on entry so a rerun cannot pass
/// on a file the previous run left behind.
pub fn out_dir(test: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target")
        .join("spoonstill-test-out")
        .join(test);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create the test output directory");
    dir
}

/// Run an FFmpeg command to completion, panicking with its stderr on failure.
fn run(command: FfmpegCommand) -> String {
    let finished = command
        .clone()
        .spawn()
        .expect("ffmpeg must be on PATH, or SPOONSTILL_FFMPEG must name it")
        .wait_until(RENDER_TIMEOUT)
        .expect("ffmpeg finished within the timeout");
    assert!(
        finished.status.success(),
        "ffmpeg failed\n  {}\n{}",
        finished.command,
        finished.stderr
    );
    finished.stderr
}

/// Every in-flight build gets its own temporary. See [`build_cached`].
static BUILD_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// Write a file through a temporary, so a concurrent reader never sees a
/// partial one and a crashed run leaves no half-built fixture.
///
/// The temporary is unique **per call**, not per process. Three tests in one
/// binary ask for `still(4000, 3000)` at once, and keying the temporary on the
/// process id alone gave all three threads the same path: three FFmpeg
/// processes writing one file, and then a rename of a file another thread
/// still had open. POSIX renames that quite happily, so it passed on macOS
/// forever; Windows refuses with a sharing violation, the `let _ =` swallowed
/// it, and the assert below fired on a fixture nothing had managed to build.
/// This is the exact class of Windows-only mistake D-071 predicted the CI job
/// would surface first — and the three concurrent tests are its regression
/// test, now that the job runs on every push.
fn build_cached(path: &Path, build: impl FnOnce(&Path)) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    let temporary = path.with_extension(format!(
        "building-{}-{}.{}",
        std::process::id(),
        BUILD_SEQUENCE.fetch_add(1, Ordering::Relaxed),
        path.extension().unwrap_or_default().to_string_lossy()
    ));
    build(&temporary);
    // Another thread may have won the race; either file is equally valid.
    let _ = std::fs::rename(&temporary, path);
    let _ = std::fs::remove_file(&temporary);
    assert!(path.exists(), "failed to build {}", path.display());
    path.to_path_buf()
}

/// A synthetic still of exactly `width` x `height`.
///
/// `testsrc2` gives sharp edges and gradients, which is what makes zoom
/// stepping (D-032) and black-edge bleed (D-034) visible rather than subtle.
pub fn still(width: u32, height: u32) -> PathBuf {
    let path = media_dir().join(format!("still-{width}x{height}.jpg"));
    build_cached(&path, |temporary| {
        let mut c = FfmpegCommand::new(tools().ffmpeg());
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .arg("-i")
            .arg(format!("testsrc2=size={width}x{height}:rate=1"))
            .args(["-frames:v", "1"])
            .arg(temporary);
        run(c);
    });
    path
}

/// The genuinely odd-dimension still that makes the D-033 test non-vacuous.
///
/// `ffmpeg-findings.md` §8b: FFmpeg rounds to even at three independent points,
/// each silently. So this asserts the result rather than trusting it — an even
/// fixture would make the SAR test pass for the wrong reason forever.
pub fn odd_still() -> PathBuf {
    let path = media_dir().join("still-1999x1001-odd.jpg");
    build_cached(&path, |temporary| {
        let mut c = FfmpegCommand::new(tools().ffmpeg());
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .arg("-i")
            .arg("testsrc2=size=2000x1002:rate=1")
            // exact=1, or crop aligns to chroma boundaries and rounds to even.
            .arg("-vf")
            .arg("crop=1999:1001:exact=1")
            // yuvj444p, or 4:2:0 cannot represent odd dimensions and rounds.
            .args(["-pix_fmt", "yuvj444p", "-frames:v", "1"])
            .arg(temporary);
        run(c);
    });

    let probe = spoonstill_media::probe(&tools(), &path, Duration::from_secs(30))
        .expect("the odd still probes");
    let video = probe.video().expect("the odd still has a video stream");
    assert_eq!(
        (video.width, video.height),
        (Some(1999), Some(1001)),
        "the odd-dimension fixture came out even. An even fixture makes the D-033 \
         SAR test vacuous — it would assert SAR 1:1 on a source that could never \
         have produced a bad SAR. See ffmpeg-findings.md §8b."
    );
    path
}

/// A narration track of exactly `seconds`, at the profile's sample rate.
pub fn narration(seconds: f64) -> PathBuf {
    let millis = (seconds * 1000.0).round() as u64;
    let path = media_dir().join(format!("narration-{millis}ms.wav"));
    build_cached(&path, |temporary| {
        let mut c = FfmpegCommand::new(tools().ffmpeg());
        c.args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .arg("-i")
            .arg(format!("sine=frequency=220:duration={seconds}"))
            .args(["-ac", "2", "-ar", "48000"])
            .arg(temporary);
        run(c);
    });
    path
}

/// Copy a file to a deliberately hostile name (D-052).
pub fn copy_to(source: &Path, dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("create the directory");
    let target = dir.join(name);
    std::fs::copy(source, &target).unwrap_or_else(|e| panic!("copy to {}: {e}", target.display()));
    target
}

/// Minimum luma across all four 4-pixel borders, for each requested frame.
///
/// A black edge — the D-034 failure — reads as 0 here. Verified non-vacuous by
/// `the_black_edge_probe_catches_a_real_letterbox` in `motion_matrix.rs`: a
/// deliberately letterboxed frame reports 0, a cover-fit one reports in the
/// thirties.
///
/// All four borders are checked in a single decode: each is cropped, scaled to
/// a common size with nearest-neighbour so no interpolation can blend a thin
/// black edge into its bright neighbour, and stacked into one image whose
/// minimum luma is therefore the minimum across every border.
pub fn border_luma_minima(path: &Path, width: u32, height: u32, frames: &[u32]) -> Vec<u32> {
    let selector = frames
        .iter()
        .map(|n| format!("eq(n\\,{n})"))
        .collect::<Vec<_>>()
        .join("+");
    let graph = format!(
        "[0:v]select='{selector}',split=4[a][b][c][d];\
         [a]crop={width}:4:0:0,scale=64:64:flags=neighbor[t];\
         [b]crop={width}:4:0:{bottom},scale=64:64:flags=neighbor[bo];\
         [c]crop=4:{height}:0:0,scale=64:64:flags=neighbor[l];\
         [d]crop=4:{height}:{right}:0,scale=64:64:flags=neighbor[r];\
         [t][bo][l][r]vstack=inputs=4,signalstats,\
         metadata=print:key=lavfi.signalstats.YMIN",
        bottom = height.saturating_sub(4),
        right = width.saturating_sub(4),
    );

    let mut c = FfmpegCommand::new(tools().ffmpeg());
    c.args(["-hide_banner", "-v", "info"])
        .input(path)
        .arg("-filter_complex")
        .arg(&graph)
        .args(["-fps_mode", "passthrough", "-f", "null", "-"]);
    let stderr = run(c);

    let minima: Vec<u32> = stderr
        .lines()
        .filter_map(|line| line.split("lavfi.signalstats.YMIN=").nth(1))
        .filter_map(|value| value.trim().parse().ok())
        .collect();

    assert_eq!(
        minima.len(),
        frames.len(),
        "expected one luma reading per requested frame {frames:?}, got {minima:?} \
         for {}. A missing reading means the frame was never decoded, which would \
         make this probe silently pass.",
        path.display()
    );
    minima
}
