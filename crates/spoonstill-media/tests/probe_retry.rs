//! D-179's wiring, against a real `ffprobe`.
//!
//! The rule itself — retry a timeout, and only a timeout — is stated in
//! `probe.rs` against a closure that fails on demand, because a policy driven
//! only through a working `ffprobe` is tested on the one input that cannot
//! exhibit the case it is about (D-116). What that cannot show is whether
//! [`probe`] is *wired* to it, which is what this file is for.
//!
//! **It lives here rather than beside the unit tests because it spawns a
//! process**, and `no_shell_strings.rs` allows that in exactly one file under
//! `src/`. Widening that guard to let a test through would trade a rule this
//! codebase relies on for one test's convenience; D-128 already settled that
//! the guard moves rather than widens.

/// Everything here needs a FIFO, which is a unix thing. The imports live
/// inside the gate as well as the test: left at the top of the file they are
/// unused on Windows, and `ci.yml` builds that leg with `-D warnings`, so the
/// tidier-looking arrangement fails a platform this machine cannot run
/// (D-132, and D-155's lesson that a test's own scaffolding is where the
/// platform rule gets forgotten).
#[cfg(unix)]
mod unix {
    use std::process::Command;
    use std::time::{Duration, Instant};

    use spoonstill_media::MediaError;
    use spoonstill_media::probe::probe;
    use spoonstill_media::tools::Tools;

    /// A FIFO with no writer is the one input measured to make `ffprobe` block
    /// rather than answer — it sits until it is killed. So both attempts run
    /// out of time, and the wall clock has to show two of them.
    ///
    /// Asserted as a **lower** bound, which is the safe direction on a shared
    /// runner (D-130): a loaded machine makes it more true, never less.
    /// Against the code before D-179 it reports the single attempt precisely —
    /// *"waited 404ms for a ceiling of 400ms"*.
    ///
    /// **No skip when `ffprobe` is missing.** A probe that cannot start fails
    /// as `BinaryMissing`, which this asserts against, so a machine without it
    /// gets a loud failure rather than a test that passes by finding nothing
    /// to check (D-125, D-154). Every other integration suite here renders
    /// real media, so a machine that can run them has FFmpeg.
    #[test]
    fn a_probe_that_runs_out_of_time_is_actually_tried_again() {
        let dir =
            std::env::temp_dir().join(format!("spoonstill-probe-retry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch directory");
        let fifo = dir.join("stalled.mp4");

        let made = Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo is POSIX and present on every unix this targets");
        assert!(made.success(), "could not make a FIFO to stall on");

        let tools = Tools::from_env();
        let ceiling = Duration::from_millis(400);
        let started = Instant::now();
        let error = probe(&tools, &fifo, ceiling).expect_err("a FIFO with no writer never answers");
        let waited = started.elapsed();

        assert!(
            matches!(error, MediaError::Timeout { .. }),
            "expected a timeout, got: {error}"
        );
        assert!(
            waited >= ceiling * 3 / 2,
            "waited {waited:?} for a ceiling of {ceiling:?} — only one attempt was made, \
             so the retry is not wired to the real probe"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// What this suite does **not** check on Windows, said out loud.
///
/// A file that simply compiles to nothing on a platform is a silent hole, and
/// this project has been bitten by exactly that (D-176's `HOME` redirect that
/// redirected nothing). The retry policy itself is unit-tested on every
/// platform; only the wiring check needs a FIFO.
#[test]
#[cfg(not(unix))]
fn the_wiring_check_is_unix_only_and_says_so() {
    eprintln!(
        "D-179's wiring check needs a FIFO to stall ffprobe on, and Windows has none. \
         The retry rule itself is covered by probe::tests::only_a_timeout_is_retried_and_only_once, \
         which runs here."
    );
}
