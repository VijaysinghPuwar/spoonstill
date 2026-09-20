//! The M1 exit gates that are not the motion matrix: the D-033 SAR regression,
//! hostile inputs, and clean cancellation.

mod common;

use std::time::Duration;

use spoonstill_core::{Aspect, OutputSpec};
use spoonstill_media::error::MediaError;
use spoonstill_media::scene::{Cancel, SceneRequest, render_scene};
use spoonstill_media::{SegmentProfile, assert_matches_profile, probe};

/// plan.md M1 exit gate 3, and the reason D-033 exists.
///
/// A 1999x1001 source through `scale` + `zoompan` with no trailing `setsar`
/// produces SAR 30007:30000 and DAR 30007:16875 instead of 16:9 — the
/// `SAR 12160:12159` class of bug `Automated-Video-Generator` records as BUG
/// W2-1. Per D-041, that segment would then concatenate with exit code 0 and no
/// warning.
///
/// The fixture is asserted to be genuinely odd by `common::odd_still` before it
/// is used, because an even one would make this test pass for the wrong reason
/// forever (`ffmpeg-findings.md` §8b).
#[test]
fn odd_dimensions_sar() {
    let tools = common::tools();
    let dir = common::out_dir("odd-dimensions-sar");

    // Every aspect, because the trap is about the source being odd, and the
    // resulting rounding differs per output aspect.
    for aspect in Aspect::ALL {
        let out = dir.join(format!("odd-{}.mp4", aspect.as_str().replace(':', "x")));
        let output = OutputSpec::new(aspect, 360, 30).unwrap();

        let mut request = SceneRequest::new(
            common::odd_still(),
            common::narration(1.0),
            out.clone(),
            output,
        );
        request.encode.preset = "veryfast".to_string();

        render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap_or_else(|e| panic!("odd source into {aspect}: {e}"));

        let probed = probe(&tools, &out, Duration::from_secs(60)).unwrap();
        let video = probed.video().unwrap();

        assert_eq!(
            video.sample_aspect_ratio.as_deref(),
            Some("1:1"),
            "a 1999x1001 source rendered to {aspect} produced SAR {:?}. \
             setsar=1 must be the last filter before format=yuv420p (D-033).",
            video.sample_aspect_ratio
        );
        assert_eq!(
            (video.width, video.height),
            (Some(output.width()), Some(output.height()))
        );

        // And the whole profile, because SAR is only the mismatch we know
        // about — the gate is uniformity, not one field (D-040).
        assert_matches_profile(&SegmentProfile::for_output(output), &probed)
            .unwrap_or_else(|m| panic!("odd source into {aspect}: {m:?}"));
    }
}

/// plan.md M1 exit gate 4. D-052: hostile input is the normal case.
///
/// The real proof that arguments are vectors rather than shell strings is a
/// filename that would be several words, a redirection and a substitution if it
/// ever reached a shell.
#[test]
fn hostile_paths_survive_the_process_boundary() {
    let tools = common::tools();
    let root = common::out_dir("hostile-paths");

    // What counts as a hostile *name* is platform-specific, and Win32 refuses
    // two of these before any of our code runs: `|` is a reserved character,
    // and a directory whose name ends in a space is `InvalidFilename` (error
    // 123). A test cannot assert that we survive a name the operating system
    // will not create — so the shapes they stand for, a shell metacharacter
    // and awkward surrounding whitespace, are covered by names that exist
    // everywhere, and the two POSIX-only ones still run where they are legal.
    // D-090; the macOS coverage is unchanged.
    let mut names = vec![
        "ünïcode spaced 名前",
        "it's a $(pwd) `name`",
        "semi;colon &and& ampersand",
        " leading space",
        "-leading-dash",
    ];
    if cfg!(unix) {
        names.push("semi;colon &and& pipe|");
        names.push("trailing space ");
    }

    for (index, name) in names.iter().enumerate() {
        // The hostile text is in the directory as well as the filenames.
        let dir = root.join(format!("{index} {name}"));
        let image = common::copy_to(&common::still(2000, 2000), &dir, &format!("{name}.jpg"));
        let audio = common::copy_to(&common::narration(0.5), &dir, &format!("{name}.wav"));
        let out = dir.join(format!("{name}.mp4"));

        let output = OutputSpec::new(Aspect::Square1x1, 360, 30).unwrap();
        let mut request = SceneRequest::new(image, audio, out.clone(), output);
        request.encode.preset = "veryfast".to_string();

        let rendered = render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap_or_else(|e| panic!("path {name:?}: {e}"));

        assert!(
            out.exists(),
            "path {name:?}: no segment at {}",
            out.display()
        );
        assert_eq!(rendered.frames, 15, "path {name:?}");

        // Nothing the filename could have done to a shell actually happened.
        assert!(
            !root.join("pwd").exists(),
            "a command substitution in a filename was executed"
        );
    }
}

/// plan.md M1 exit gate 5. D-045: cancellation is graceful, then forced, then
/// clean.
///
/// The requirement is precise: the destination is "absent, or present and
/// marked partial — never a valid-looking stub". spoonstill takes the first
/// branch, because nothing is ever written to the destination path until the
/// segment has passed the profile assertion (D-042).
#[test]
fn cancellation_leaves_no_valid_looking_stub() {
    let tools = common::tools();
    let dir = common::out_dir("cancellation");
    let out = dir.join("segment.mp4");

    // Long enough and large enough that the render is still running when the
    // cancellation lands.
    let output = OutputSpec::new(Aspect::Landscape16x9, 1080, 30).unwrap();
    let request = SceneRequest::new(
        common::still(4000, 3000),
        common::narration(20.0),
        out.clone(),
        output,
    );

    let cancel = Cancel::new();
    let trigger = cancel.clone();

    // Cancel once the encode has demonstrably started, rather than after a
    // fixed sleep: a timing-based test that cancels before FFmpeg has opened
    // its output file would pass without exercising anything.
    let mut seen_progress = false;
    let error = render_scene(
        &tools,
        &request,
        &cancel,
        &spoonstill_core::diagnostics::Noop,
        &mut |progress| {
            if !seen_progress && progress.frame.unwrap_or(0) > 0 {
                seen_progress = true;
                trigger.request();
            }
        },
    )
    .expect_err("a cancelled render must not report success");

    assert!(
        matches!(error, MediaError::Cancelled { .. }),
        "expected a cancellation, got: {error}"
    );
    assert!(
        seen_progress,
        "the render was cancelled before it started, so this test proved nothing"
    );

    assert!(
        !out.exists(),
        "a cancelled render left {} behind. Nothing may reach the destination \
         path until it has passed the profile assertion (D-042).",
        out.display()
    );

    // And no partial file is left littering the output directory either.
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        leftovers.is_empty(),
        "a cancelled render left files behind: {leftovers:?}"
    );
}

/// A truncated image must fail with a named cause, not a panic and not a
/// plausible-looking segment.
#[test]
fn a_truncated_image_is_refused_by_name() {
    let tools = common::tools();
    let dir = common::out_dir("truncated-image");

    let good = common::still(4000, 3000);
    let truncated = dir.join("truncated.jpg");
    let bytes = std::fs::read(&good).unwrap();
    std::fs::write(&truncated, &bytes[..4096]).unwrap();

    let out = dir.join("segment.mp4");
    let request = SceneRequest::new(
        truncated,
        common::narration(0.5),
        out.clone(),
        OutputSpec::new(Aspect::Square1x1, 360, 30).unwrap(),
    );

    let error = render_scene(
        &tools,
        &request,
        &Cancel::new(),
        &spoonstill_core::diagnostics::Noop,
        &mut |_| {},
    )
    .expect_err("a truncated image cannot render");
    assert!(
        error.to_string().contains("truncated.jpg"),
        "the error must name the file: {error}"
    );
    assert!(!out.exists(), "a failed render must leave no segment");
}

/// A zero-byte narration must be rejected with the file named, not turned into
/// a one-frame segment.
#[test]
fn empty_narration_is_refused_by_name() {
    let tools = common::tools();
    let dir = common::out_dir("empty-narration");

    let empty = dir.join("zero_byte.mp3");
    std::fs::write(&empty, b"").unwrap();

    let out = dir.join("segment.mp4");
    let request = SceneRequest::new(
        common::still(2000, 2000),
        empty,
        out.clone(),
        OutputSpec::new(Aspect::Square1x1, 360, 30).unwrap(),
    );

    let error = render_scene(
        &tools,
        &request,
        &Cancel::new(),
        &spoonstill_core::diagnostics::Noop,
        &mut |_| {},
    )
    .expect_err("an empty narration cannot drive a scene");
    assert!(
        error.to_string().contains("zero_byte.mp3"),
        "the error must name the file: {error}"
    );
    assert!(!out.exists());
}

/// D-035, end to end: the same scene identity renders byte-identically.
///
/// This is what makes the cache safe (D-043) and resume meaningful. An
/// unseeded `random.choice()` — `ffmpeg-ai`'s approach — would break both.
#[test]
fn the_same_scene_identity_renders_identically() {
    let tools = common::tools();
    let dir = common::out_dir("deterministic-render");

    let image = common::still(4000, 3000);
    let audio = common::narration(1.0);
    let output = OutputSpec::new(Aspect::Landscape16x9, 360, 30).unwrap();

    let mut bytes = Vec::new();
    for run in 0..2 {
        let out = dir.join(format!("run-{run}.mp4"));
        let mut request = SceneRequest::new(image.clone(), audio.clone(), out.clone(), output);
        request.project_id = "determinism".to_string();
        request.scene_index = 7;
        request.encode.preset = "veryfast".to_string();

        let rendered = render_scene(
            &tools,
            &request,
            &Cancel::new(),
            &spoonstill_core::diagnostics::Noop,
            &mut |_| {},
        )
        .unwrap();
        // The move is chosen from identity, so both runs must pick the same one.
        assert_eq!(
            rendered.motion.descriptor(),
            spoonstill_core::MotionSpec::seeded(
                "determinism",
                7,
                &format!(
                    "{:016x}",
                    spoonstill_core::hash::fnv1a(&std::fs::read(&image).unwrap())
                )
            )
            .descriptor()
        );
        bytes.push(std::fs::read(&out).unwrap());
    }

    assert_eq!(
        bytes[0].len(),
        bytes[1].len(),
        "two renders of the same scene identity differ in size"
    );
    assert!(
        bytes[0] == bytes[1],
        "two renders of the same scene identity are not byte-identical, so the \
         cache key of D-043 cannot be trusted"
    );
}

/// Where the two stand-in copies below announce themselves to each other.
///
/// Deliberately **outside** the directories under test: a rendezvous that
/// lived in `spoonstill-test-out/<test>/` would be wiped by the very defect
/// being measured, and the test would then fail for the wrong reason.
const RENDEZVOUS: &str = "SPOONSTILL_TEST_RENDEZVOUS";

/// Not a test: one of the two concurrent copies started by
/// `two_copies_of_one_test_keep_their_own_output`, selected by name.
///
/// It claims an output directory, writes its own pid into it, waits until the
/// other copy has claimed one too, and then asserts its own file is still its
/// own. Under the layout before D-180 both copies name one directory and the
/// second one's `remove_dir_all` takes the first one's marker, so the first
/// copy fails here. Run against the unfixed code and seen to do exactly that.
///
/// The overlap is a **fact, not a sleep**: neither copy proceeds until both
/// have arrived. A concurrency claim with no clock in it fails as a deadline
/// (D-149), which is what the twenty seconds below is.
#[test]
#[ignore = "spawned as a stand-in copy by two_copies_of_one_test_keep_their_own_output"]
fn stand_in_copy_writes_and_keeps_its_own_output() {
    let Some(rendezvous) = std::env::var_os(RENDEZVOUS).map(std::path::PathBuf::from) else {
        // Nothing to rendezvous with: this is the stand-in, not a test, and
        // it is only meaningful when its parent started it.
        println!("{RENDEZVOUS} is unset — this is a stand-in, started on its own.");
        return;
    };

    let dir = common::out_dir("concurrent-namespace");
    let mine = std::process::id().to_string();
    std::fs::write(dir.join("marker"), &mine).expect("write this copy's marker");
    std::fs::write(rendezvous.join(&mine), b"here").expect("announce this copy");

    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    loop {
        let arrived = std::fs::read_dir(&rendezvous)
            .map(|entries| entries.flatten().count())
            .unwrap_or(0);
        if arrived >= 2 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {arrived} of 2 copies arrived at {} — the other copy never \
             started, so this proved nothing",
            rendezvous.display()
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let marker = dir.join("marker");
    let found = std::fs::read_to_string(&marker).unwrap_or_else(|e| {
        panic!(
            "{} is gone while this copy is still running ({e}) — the other copy \
             emptied this copy's output directory",
            marker.display()
        )
    });
    assert_eq!(
        found,
        mine,
        "{} holds another copy's pid: two concurrent runs of one test shared \
         one output directory",
        marker.display()
    );
}

/// D-180. Two copies of one test, at once, in one checkout.
///
/// This author runs several models in parallel terminals against this tree, so
/// a shared output path is not a hypothetical: it is how a green product
/// reports a red gate. `crates/spoonstill-media/tests/common/mod.rs` used to
/// build `target/spoonstill-test-out/<test>` and wipe it on entry.
///
/// The stand-in is **this test binary**, re-run with `--exact` against the
/// ignored helper above — D-155's pattern, for D-155's reason: no shell, no
/// installed tool, arguments still a vector, and it exists wherever these
/// tests run.
#[test]
fn two_copies_of_one_test_keep_their_own_output() {
    let rendezvous = common::out_dir("concurrent-rendezvous");
    let exe = std::env::current_exe().expect("the test binary knows its own path");

    let mut copies = Vec::new();
    for _ in 0..2 {
        let copy = std::process::Command::new(&exe)
            .args([
                "--exact",
                "stand_in_copy_writes_and_keeps_its_own_output",
                "--ignored",
                "--nocapture",
            ])
            .env(RENDEZVOUS, &rendezvous)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("the stand-in copy starts");
        copies.push(copy);
    }
    // Their own pids are what each copy writes into its marker, so they are
    // what says the two surviving markers are *these* two runs and not a
    // leftover pair the sweep has not reached yet.
    let spawned: Vec<String> = copies.iter().map(|c| c.id().to_string()).collect();

    for (index, copy) in copies.into_iter().enumerate() {
        let finished = copy.wait_with_output().expect("the stand-in copy exits");
        assert!(
            finished.status.success(),
            "copy {index} failed:\n{}{}",
            String::from_utf8_lossy(&finished.stdout),
            String::from_utf8_lossy(&finished.stderr),
        );
        // A filter that matches nothing runs no tests and exits 0 (D-155), so
        // the exit status above is worth nothing on its own.
        assert!(
            String::from_utf8_lossy(&finished.stdout).contains("1 passed"),
            "copy {index} ran no test — the helper has been renamed:\n{}",
            String::from_utf8_lossy(&finished.stdout),
        );
    }

    // And each copy's output is still on disk afterwards, under two distinct
    // run directories of the one test name. A `TempDir` would pass everything
    // above and leave nothing to look at when a real test fails.
    let per_test = rendezvous
        .parent()
        .and_then(std::path::Path::parent)
        .expect("the run directory sits under the test's own directory")
        .join("concurrent-namespace");
    let mut found: Vec<String> = std::fs::read_dir(&per_test)
        .expect("the stand-ins' test directory exists")
        .flatten()
        .filter_map(|run| std::fs::read_to_string(run.path().join("marker")).ok())
        .filter(|pid| spawned.contains(pid))
        .collect();
    found.sort();
    found.dedup();
    assert_eq!(
        found.len(),
        2,
        "two copies ran as {spawned:?} and {} holds {} of their markers: \
         {found:?}",
        per_test.display(),
        found.len(),
    );
}

/// D-180's other half: the bound that stops `target/` growing without end.
///
/// Per-invocation directories trade a race for a disk unless something
/// collects them, and the fixed path this replaced at least held exactly one
/// generation. So: the newest few survive, the rest go, and a leftover from
/// the layout before D-180 goes with them rather than sitting there forever.
///
/// The two batches are separated by more than a second because the rule is
/// *by modification time*, and a filesystem whose timestamps are coarser than
/// the gap would order them arbitrarily — a test that ties is a test people
/// re-run (D-121).
#[test]
fn the_sweep_keeps_the_newest_runs_and_collects_the_rest() {
    let per_test = common::out_dir("sweep-collects").join("under-test");
    std::fs::create_dir_all(&per_test).unwrap();

    let older: Vec<_> = (0..3)
        .map(|n| per_test.join(format!("run-{n}-old")))
        .collect();
    for dir in &older {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("artifact"), b"x").unwrap();
    }
    // A file left directly in the test's directory is what the pre-D-180
    // layout wrote, and it is not a run directory.
    let leftover = per_test.join("segment.mp4");
    std::fs::write(&leftover, b"x").unwrap();

    std::thread::sleep(Duration::from_millis(1100));

    let newer: Vec<_> = (0..2)
        .map(|n| per_test.join(format!("run-{n}-new")))
        .collect();
    for dir in &newer {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("artifact"), b"x").unwrap();
    }

    common::sweep_old_runs_within(&per_test, 2, Duration::ZERO);

    for dir in &older {
        assert!(!dir.exists(), "{} survived the sweep", dir.display());
    }
    for dir in &newer {
        assert!(
            dir.join("artifact").exists(),
            "{} was collected, but it is one of the newest two",
            dir.display()
        );
    }
    assert!(
        !leftover.exists(),
        "a file from the layout before D-180 is never collected, so it sits \
         in {} forever",
        per_test.display()
    );
}

/// And the guard that matters more, because getting it wrong destroys the
/// evidence a parallel session is producing right now.
///
/// Whatever the keep count, a run touched inside the grace window is left
/// alone. Collection is delayed rather than prevented, which is the right way
/// round (D-178).
#[test]
fn the_sweep_leaves_a_recent_run_alone_however_many_there_are() {
    let per_test = common::out_dir("sweep-spares-recent").join("under-test");
    std::fs::create_dir_all(&per_test).unwrap();

    let runs: Vec<_> = (0..4)
        .map(|n| per_test.join(format!("run-{n}-now")))
        .collect();
    for dir in &runs {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("artifact"), b"x").unwrap();
    }
    let leftover = per_test.join("segment.mp4");
    std::fs::write(&leftover, b"x").unwrap();

    // Keep one — and still collect none of them, because all four are seconds
    // old. The **shipped** window is what is driven here, so a grace tightened
    // to nothing would fail this rather than passing it silently.
    common::sweep_old_runs_within(&per_test, 1, common::SWEEP_GRACE);

    // And the window's value is pinned to what it is for. The longest-running
    // test process in this crate is `caption_hostile_text` at 34 s and the
    // whole suite is 66 s; a window that did not clear those by an order of
    // magnitude would let a sweep take a live run's evidence.
    assert!(
        common::SWEEP_GRACE >= Duration::from_secs(5 * 60),
        "the grace window is {:?}, which is not clear of a running test",
        common::SWEEP_GRACE,
    );

    for dir in &runs {
        assert!(
            dir.join("artifact").exists(),
            "{} was collected while it could still belong to a running \
             session",
            dir.display()
        );
    }
    assert!(
        leftover.exists(),
        "a file written seconds ago was collected as though it were from an \
         abandoned run"
    );
}

/// D-181's middle check: a join already running notices a cancellation.
///
/// The imports live inside the gate rather than at the top of the file for
/// D-179's reason — unused imports on Windows are errors under `ci.yml`'s
/// `-D warnings`, and a test's own scaffolding is where the platform rule gets
/// forgotten (D-132, D-155).
#[cfg(unix)]
mod unix {
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use spoonstill_core::diagnostics::Noop;
    use spoonstill_core::{Aspect, OutputSpec};
    use spoonstill_media::concat::{Expectation, concat};
    use spoonstill_media::profile::SegmentProfile;
    use spoonstill_media::scene::Cancel;
    use spoonstill_media::tools::Tools;

    /// How long the test is willing to wait for a cancelled join to come back.
    ///
    /// `CONCAT_TIMEOUT` is an hour, so against the code before D-181 this test
    /// would not fail — it would **hang**, which is a failure nobody can read.
    /// The deadline is what turns it into a sentence. Generous against the two
    /// seconds `CANCEL_GRACE` spends asking politely before it forces.
    const PATIENCE: Duration = Duration::from_secs(30);

    /// A cancellation reaches a join that is already running.
    ///
    /// The three checks D-181 adds are at different points and only this one
    /// needs a process in flight, so it needs a join that will not finish on
    /// its own. **A FIFO with no data is that join** — the same instrument
    /// D-179 used to stall `ffprobe`.
    ///
    /// The rendezvous is exact rather than a sleep, and that is the whole
    /// reason this test is worth having: opening a FIFO for writing blocks
    /// until a reader opens it, so our `open` returning *is* FFmpeg having
    /// started to read. A `sleep` there would sometimes set the flag before
    /// the process existed, and the pre-spawn check would answer instead —
    /// the test would pass while proving nothing about the loop (D-116).
    #[test]
    fn a_running_join_notices_a_cancellation() {
        let dir = super::common::out_dir("join-cancelled-mid-flight");
        let stalled = dir.join("seg-aaaaaaaaaaaaaaaa.mp4");
        let made = Command::new("mkfifo")
            .arg(&stalled)
            .status()
            .expect("mkfifo is POSIX and present on every unix this targets");
        assert!(made.success(), "could not make a FIFO to stall the join on");

        let dest = dir.join("film.mp4");
        let segments = vec![stalled.clone()];
        let cancel = Cancel::new();

        let (done, joined) = mpsc::channel();
        let worker = {
            let (cancel, dest) = (cancel.clone(), dest.clone());
            std::thread::spawn(move || {
                let profile = SegmentProfile::for_output(
                    OutputSpec::new(Aspect::Landscape16x9, 360, 30).unwrap(),
                );
                let result = concat(
                    &Tools::from_env(),
                    &segments,
                    &dest,
                    &Expectation {
                        profile: &profile,
                        frames: 6,
                        segment_frames: &[6],
                        duration: 0.2,
                        fps: 30,
                    },
                    &cancel,
                    &Noop,
                );
                let _ = done.send(result.err().map(|e| e.to_string()));
            })
        };

        // Blocks until FFmpeg opens the FIFO to read. Held open for the rest
        // of the test, so FFmpeg stays blocked on a read rather than seeing
        // an end of file and exiting by itself.
        let writer = std::fs::OpenOptions::new()
            .write(true)
            .open(&stalled)
            .expect("FFmpeg opened the FIFO to read, so this open returns");

        let started = Instant::now();
        cancel.request();
        let outcome = joined.recv_timeout(PATIENCE).unwrap_or_else(|_| {
            panic!(
                "the join did not come back within {PATIENCE:?} of being cancelled — \
                 it is waiting out CONCAT_TIMEOUT, so the flag is not being read"
            )
        });
        let waited = started.elapsed();

        drop(writer);
        worker.join().expect("the join thread finished");

        let detail = outcome.expect("a cancelled join must not report success");
        assert!(
            detail.starts_with("cancelled"),
            "expected a cancellation, got: {detail}"
        );
        assert!(
            !dest.exists(),
            "{} was published for a run the operator stopped",
            dest.display()
        );
        let litter: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains(".partial-"))
            .collect();
        assert!(litter.is_empty(), "a stopped join left {litter:?} behind");

        // Stated as an upper bound only, which is the safe direction on a
        // shared runner (D-130): a loaded machine makes this less true, so it
        // is set against `CANCEL_GRACE`'s two seconds with room to spare
        // rather than against the 20 ms poll.
        assert!(
            waited < PATIENCE,
            "a cancelled join took {waited:?} to come back"
        );
    }
}

/// What this file does **not** check on Windows, said out loud (D-179's rule).
///
/// D-181's other two checks — before the join starts, and before the finished
/// film is renamed into place — are unit-tested in `concat.rs` on every
/// platform. Only the one that needs a join stuck in flight needs a FIFO.
#[test]
#[cfg(not(unix))]
fn the_mid_join_cancellation_check_is_unix_only_and_says_so() {
    eprintln!(
        "D-181's mid-join check needs a FIFO to stall the join on, and Windows has none. \
         The other two checks are covered by concat::tests::a_cancelled_run_does_not_publish_\
         the_film_it_just_finished and ::a_join_asked_for_after_a_cancellation_never_runs, \
         which run here."
    );
}
