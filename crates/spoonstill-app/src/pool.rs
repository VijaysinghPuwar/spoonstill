//! A bounded worker pool (D-044, D-045).
//!
//! D-044 is blunt about this: an unbounded fan-out over 500 scenes is a
//! defect, not an optimisation gone wrong. Five hundred concurrent FFmpeg
//! children would exhaust memory long before they exhausted the CPU — a 1080p
//! segment peaks near 780 MB (`ffmpeg-findings.md` §10b), so the 24 GB machine
//! this was measured on runs out at about thirty.
//!
//! So work is admitted through a fixed number of workers, and the number is a
//! product decision rather than an accident of how many items there are.
//!
//! ## What this pool guarantees
//!
//! - **Bounded.** At most `jobs` items are in flight, whatever the item count.
//! - **Order-preserving.** Results come back indexed by input position, so a
//!   film's segments concatenate in scene order no matter which worker
//!   finished first. Concurrency changes the *timing* of a render and nothing
//!   else — same inputs, same film, byte for byte (D-035).
//! - **Cancellable at the item boundary** (D-045): once cancellation is
//!   requested, no further item is admitted. Work already in flight stops
//!   through its own ladder — for a render, the FFmpeg quit/kill sequence in
//!   [`spoonstill_media::command`].
//!
//! ## Why threads rather than an async runtime
//!
//! The unit of work here is a child process that takes seconds. There is no
//! I/O concurrency problem to solve, so a runtime would add a dependency, a
//! colour to every function signature, and no throughput. `std::thread::scope`
//! lets a worker borrow the caller's data without an `Arc` around all of it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use spoonstill_media::scene::Cancel;

/// Most workers we will start without being asked to.
///
/// Four, and it is measured rather than chosen (`ffmpeg-findings.md` §10a).
/// Twelve 1080p scenes on this ten-core machine:
///
/// | `--jobs` | wall clock | speedup |
/// |---|---|---|
/// | 1 | 13.23 s | 1.00x |
/// | 2 | 8.56 s | 1.55x |
/// | 3 | 7.24 s | 1.83x |
/// | 4 | 7.17 s | 1.85x |
/// | 8 | 6.90 s | 1.92x |
///
/// The curve flattens at three, because x264 at `medium` already threads
/// internally — one worker was using 2.7 cores on its own, so extra workers
/// mostly compete with the threads of the ones already running. Memory does
/// **not** flatten: each concurrent segment peaks near 780 MB, so doubling
/// four to eight costs about 3 GB of resident memory to buy 4% of wall clock.
///
/// So the automatic default stops at four. `--jobs` is not capped — an
/// operator who knows their machine can ask for sixteen — and D-044's
/// RAM-derived capacity arrives at M3 with n=500 measurements behind it.
pub const MAX_AUTO_JOBS: usize = 4;

/// Roughly how much memory one concurrent 1080p segment needs, in bytes.
///
/// 780 MB, measured (`ffmpeg-findings.md` §10b). Not used to size the pool yet
/// — that is D-044's M3 deliverable — but recorded here as the number that
/// makes `--jobs 16` a decision about memory rather than about cores.
pub const SEGMENT_MEMORY_BYTES: u64 = 780 * 1024 * 1024;

/// How many workers to start when nobody said.
///
/// One per two cores, capped by [`MAX_AUTO_JOBS`], never zero. Half rather
/// than one per core because each worker is itself multi-threaded: at
/// `--jobs 1` the encoder was already using 2.7 of this machine's ten cores,
/// so one worker per core would oversubscribe by that factor and leave nothing
/// for the machine its operator is also using.
#[must_use]
pub fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .div_euclid(2)
        .clamp(1, MAX_AUTO_JOBS)
}

/// What a worker did, or why it did not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome<T> {
    /// The worker ran and produced this.
    Done(T),
    /// Cancellation was requested before this item was admitted (D-045).
    NotAdmitted,
}

impl<T> Outcome<T> {
    /// The value, if there is one.
    pub fn done(self) -> Option<T> {
        match self {
            Outcome::Done(value) => Some(value),
            Outcome::NotAdmitted => None,
        }
    }

    /// Whether this item was skipped because the run was cancelled.
    #[must_use]
    pub const fn was_skipped(&self) -> bool {
        matches!(self, Outcome::NotAdmitted)
    }
}

/// Run `work` over `items` with at most `jobs` in flight.
///
/// Results are returned in input order. `work` receives the item's index, so a
/// worker can name what it is doing without the caller threading an id through
/// the item type.
///
/// `jobs` is clamped to at least one and to at most the number of items —
/// starting five workers for three scenes is two threads that do nothing but
/// exist.
#[must_use]
pub fn run<T, R, F>(items: &[T], jobs: usize, cancel: &Cancel, work: F) -> Vec<Outcome<R>>
where
    T: Sync,
    R: Send,
    F: Fn(usize, &T) -> R + Sync,
{
    if items.is_empty() {
        return Vec::new();
    }

    let workers = jobs.clamp(1, items.len());
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<R>>> = Mutex::new((0..items.len()).map(|_| None).collect());

    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    // D-045's first rung: stop *admitting* work. Whatever is
                    // already running gets to finish or to be cancelled on its
                    // own terms; nothing new starts.
                    if cancel.is_requested() {
                        return;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(index) else { return };

                    let value = work(index, item);

                    // The lock is held only to place a finished value, never
                    // across the work itself — otherwise this would be an
                    // elaborate way of running one job at a time.
                    if let Ok(mut slot) = results.lock() {
                        slot[index] = Some(value);
                    }
                }
            });
        }
    });

    results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .into_iter()
        .map(|value| value.map_or(Outcome::NotAdmitted, Outcome::Done))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::time::Duration;

    /// The property the whole design rests on: what comes back is in input
    /// order, whatever order the workers finished in. Segments concatenate by
    /// this ordering, so getting it wrong would scramble a film rather than
    /// slow it down.
    #[test]
    fn results_come_back_in_input_order() {
        // Reverse-sleep, so completion order is the opposite of input order.
        let items: Vec<u64> = (0..8).collect();
        let outcomes = run(&items, 4, &Cancel::new(), |_, item| {
            std::thread::sleep(Duration::from_millis((8 - item) * 5));
            item * 10
        });

        let values: Vec<u64> = outcomes.into_iter().filter_map(Outcome::done).collect();
        assert_eq!(values, vec![0, 10, 20, 30, 40, 50, 60, 70]);
    }

    /// Bounded means bounded: the count in flight never exceeds `jobs`, no
    /// matter how many items there are. This is the D-044 defect, asserted.
    #[test]
    fn never_more_than_jobs_are_in_flight() {
        static IN_FLIGHT: AtomicU32 = AtomicU32::new(0);
        static PEAK: AtomicU32 = AtomicU32::new(0);
        IN_FLIGHT.store(0, Ordering::SeqCst);
        PEAK.store(0, Ordering::SeqCst);

        let items: Vec<u32> = (0..40).collect();
        let _ = run(&items, 3, &Cancel::new(), |_, _| {
            let now = IN_FLIGHT.fetch_add(1, Ordering::SeqCst) + 1;
            PEAK.fetch_max(now, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(2));
            IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
        });

        assert!(PEAK.load(Ordering::SeqCst) <= 3, "the pool is not bounded");
    }

    /// D-045: cancellation stops admission. Items that never started come back
    /// as `NotAdmitted` rather than as a silent gap or a fabricated result.
    #[test]
    fn cancellation_stops_admitting_work() {
        let cancel = Cancel::new();
        let items: Vec<u32> = (0..200).collect();

        let outcomes = run(&items, 2, &cancel, |index, item| {
            if index == 1 {
                cancel.request();
            }
            std::thread::sleep(Duration::from_millis(1));
            *item
        });

        let skipped = outcomes.iter().filter(|o| o.was_skipped()).count();
        assert!(skipped > 0, "cancellation admitted every item anyway");
        assert!(
            outcomes.len() == items.len(),
            "a cancelled run still accounts for every item"
        );
    }

    #[test]
    fn an_empty_batch_starts_no_workers() {
        let items: Vec<u32> = Vec::new();
        let outcomes = run(&items, 4, &Cancel::new(), |_, _| unreachable!());
        assert!(outcomes.is_empty());
    }

    /// Nonsense job counts are corrected rather than trusted: zero would mean
    /// a render that never starts, and forty workers for three scenes is
    /// thirty-seven threads that exist to do nothing.
    #[test]
    fn the_worker_count_is_clamped_to_something_sensible() {
        let items: Vec<u32> = (0..3).collect();
        for jobs in [0, 1, 40] {
            let outcomes = run(&items, jobs, &Cancel::new(), |_, item| *item);
            let values: Vec<u32> = outcomes.into_iter().filter_map(Outcome::done).collect();
            assert_eq!(values, vec![0, 1, 2], "jobs = {jobs}");
        }
    }

    #[test]
    fn the_default_is_at_least_one_and_never_runaway() {
        let jobs = default_jobs();
        assert!((1..=MAX_AUTO_JOBS).contains(&jobs), "{jobs}");
    }
}
