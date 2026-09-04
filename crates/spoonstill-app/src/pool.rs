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

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

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

/// One stage's results: one entry per input item, in input order.
///
/// A name rather than the type written out, because [`pipeline`] returns two of
/// them and the pair is unreadable inline.
pub type Staged<T, E> = Vec<Outcome<Result<T, E>>>;

/// Run two dependent stages over the same items, overlapping them (D-146).
///
/// Stage one runs `first_jobs` at a time; stage two runs `second_jobs` at a
/// time and starts on an item **as soon as that item's stage one finished** —
/// not when the whole of stage one has. That is the difference this function
/// exists for: a film's narrations are network-bound and its segments are
/// CPU-bound, and running them as two barriers left a quarter of every cold
/// render with the CPU idle (`findings.md` F-12).
///
/// ## What it guarantees
///
/// - **Both stages are bounded**, independently. At most `first_jobs` +
///   `second_jobs` items are in flight.
/// - **Both result vectors are in input order**, exactly as [`run`]'s is. The
///   overlap changes when work happens and nothing about what comes back.
/// - **Stage two only ever sees a stage one that succeeded.** An item whose
///   first stage failed is `NotAdmitted` in the second — its failure is in the
///   first vector, which the caller reports.
/// - **Cancellation stops admission in both** (D-045).
/// - **A stage-one failure stops admitting stage two.** Without it, one failed
///   narration at scene 1 would encode all 499 other segments before the run
///   was allowed to fail — work the old two-barrier shape never did, because
///   it failed at the barrier. What is *reported* is unchanged, since the
///   caller checks the first vector before the second.
///
/// The value from stage one is moved through the queue and handed back in the
/// first vector, so nothing is cloned and the caller still owns it afterwards.
#[must_use]
pub fn pipeline<T, A, B, E1, E2, F, G>(
    items: &[T],
    first_jobs: usize,
    second_jobs: usize,
    cancel: &Cancel,
    first: F,
    second: G,
) -> (Staged<A, E1>, Staged<B, E2>)
where
    T: Sync,
    A: Send,
    B: Send,
    E1: Send,
    E2: Send,
    F: Fn(usize, &T) -> Result<A, E1> + Sync,
    G: Fn(usize, &T, &A) -> Result<B, E2> + Sync,
{
    if items.is_empty() {
        return (Vec::new(), Vec::new());
    }

    let producers = first_jobs.clamp(1, items.len());
    let consumers = second_jobs.clamp(1, items.len());

    let next = AtomicUsize::new(0);
    let first_failed = AtomicBool::new(false);
    let firsts: Mutex<Vec<Option<Result<A, E1>>>> =
        Mutex::new((0..items.len()).map(|_| None).collect());
    let seconds: Mutex<Vec<Option<Result<B, E2>>>> =
        Mutex::new((0..items.len()).map(|_| None).collect());

    // The hand-off. `live` is how many producers are still running, and it is
    // the *only* thing that tells a consumer no more work is coming — a queue
    // that is empty right now says nothing about that.
    let handoff: Mutex<Handoff<A>> = Mutex::new(Handoff {
        ready: std::collections::VecDeque::new(),
        live: producers,
    });
    let arrived = Condvar::new();

    std::thread::scope(|scope| {
        for _ in 0..producers {
            scope.spawn(|| {
                loop {
                    // D-045's first rung, same as `run`: stop admitting.
                    if cancel.is_requested() {
                        break;
                    }
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    let Some(item) = items.get(index) else { break };

                    match first(index, item) {
                        Ok(value) => {
                            if let Ok(mut handoff) = handoff.lock() {
                                handoff.ready.push_back((index, value));
                            }
                            arrived.notify_one();
                        }
                        Err(error) => {
                            first_failed.store(true, Ordering::SeqCst);
                            if let Ok(mut slot) = firsts.lock() {
                                slot[index] = Some(Err(error));
                            }
                        }
                    }
                }

                // Last one out tells every waiting consumer that nothing more
                // is coming. Under the lock, so a consumer cannot check `live`
                // and then miss the notification.
                if let Ok(mut handoff) = handoff.lock() {
                    handoff.live -= 1;
                }
                arrived.notify_all();
            });
        }

        for _ in 0..consumers {
            scope.spawn(|| {
                loop {
                    let Some((index, value)) = take(&handoff, &arrived) else {
                        return;
                    };

                    // Admitted or not, stage one's value goes back to the
                    // caller: it is the narration this run resolved, and the
                    // caller counts and reports it whatever happened next.
                    let skip = cancel.is_requested() || first_failed.load(Ordering::SeqCst);
                    if !skip {
                        let result = second(index, &items[index], &value);
                        if let Ok(mut slot) = seconds.lock() {
                            slot[index] = Some(result);
                        }
                    }
                    if let Ok(mut slot) = firsts.lock() {
                        slot[index] = Some(Ok(value));
                    }
                }
            });
        }
    });

    (into_outcomes(firsts), into_outcomes(seconds))
}

/// What stage one has finished and stage two has not started.
struct Handoff<A> {
    ready: std::collections::VecDeque<(usize, A)>,
    /// Producers still running. Zero means the queue will never refill.
    live: usize,
}

/// Wait for the next finished item, or `None` when there will not be one.
fn take<A>(handoff: &Mutex<Handoff<A>>, arrived: &Condvar) -> Option<(usize, A)> {
    let mut guard = handoff.lock().ok()?;
    loop {
        if let Some(pair) = guard.ready.pop_front() {
            return Some(pair);
        }
        if guard.live == 0 {
            return None;
        }
        guard = arrived.wait(guard).ok()?;
    }
}

fn into_outcomes<R>(slots: Mutex<Vec<Option<R>>>) -> Vec<Outcome<R>> {
    slots
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

    /// The property `pipeline` exists for (D-146): stage two starts on an item
    /// while stage one is still working on others. Asserted as an *ordering of
    /// events*, not as a wall-clock number, because a shared runner is slow for
    /// reasons that are not defects.
    #[test]
    fn the_second_stage_starts_before_the_first_stage_finishes() {
        static SECOND_STARTED: AtomicU32 = AtomicU32::new(0);
        static FIRST_FINISHED: AtomicU32 = AtomicU32::new(0);
        SECOND_STARTED.store(0, Ordering::SeqCst);
        FIRST_FINISHED.store(0, Ordering::SeqCst);
        // Set when a second-stage item runs while first-stage items remain.
        static OVERLAPPED: AtomicBool = AtomicBool::new(false);
        OVERLAPPED.store(false, Ordering::SeqCst);

        let items: Vec<u32> = (0..8).collect();
        let (firsts, seconds) = pipeline(
            &items,
            2,
            2,
            &Cancel::new(),
            |_, item: &u32| -> Result<u32, ()> {
                std::thread::sleep(Duration::from_millis(10));
                FIRST_FINISHED.fetch_add(1, Ordering::SeqCst);
                Ok(*item)
            },
            |_, _item: &u32, value: &u32| -> Result<u32, ()> {
                SECOND_STARTED.fetch_add(1, Ordering::SeqCst);
                if FIRST_FINISHED.load(Ordering::SeqCst) < 8 {
                    OVERLAPPED.store(true, Ordering::SeqCst);
                }
                std::thread::sleep(Duration::from_millis(10));
                Ok(value * 10)
            },
        );

        assert!(
            OVERLAPPED.load(Ordering::SeqCst),
            "every second-stage item waited for the whole first stage — this is \
             the two-barrier shape pipelining replaces"
        );
        let a: Vec<u32> = firsts
            .into_iter()
            .filter_map(Outcome::done)
            .map(Result::unwrap)
            .collect();
        let b: Vec<u32> = seconds
            .into_iter()
            .filter_map(Outcome::done)
            .map(Result::unwrap)
            .collect();
        assert_eq!(
            a,
            (0..8).collect::<Vec<u32>>(),
            "first stage, in input order"
        );
        assert_eq!(
            b,
            (0..8).map(|i| i * 10).collect::<Vec<u32>>(),
            "second stage, in input order"
        );
    }

    /// Order is the whole reason segments concatenate into a film rather than
    /// a shuffle, so it is asserted against completion order that is the
    /// reverse of input order in **both** stages.
    #[test]
    fn both_stages_come_back_in_input_order() {
        let items: Vec<u64> = (0..10).collect();
        let (firsts, seconds) = pipeline(
            &items,
            4,
            4,
            &Cancel::new(),
            |_, item: &u64| -> Result<u64, ()> {
                std::thread::sleep(Duration::from_millis((10 - item) * 3));
                Ok(*item)
            },
            |_, _, value: &u64| -> Result<u64, ()> {
                std::thread::sleep(Duration::from_millis(*value * 3));
                Ok(value + 100)
            },
        );

        let a: Vec<u64> = firsts
            .into_iter()
            .filter_map(Outcome::done)
            .map(Result::unwrap)
            .collect();
        let b: Vec<u64> = seconds
            .into_iter()
            .filter_map(Outcome::done)
            .map(Result::unwrap)
            .collect();
        assert_eq!(a, (0..10).collect::<Vec<u64>>());
        assert_eq!(b, (0..10).map(|i| i + 100).collect::<Vec<u64>>());
    }

    /// Two pools, two bounds. The whole point of separate numbers is that the
    /// network-bound stage can be wider than the memory-bound one (D-044).
    #[test]
    fn each_stage_is_bounded_by_its_own_job_count() {
        static FIRST_NOW: AtomicU32 = AtomicU32::new(0);
        static FIRST_PEAK: AtomicU32 = AtomicU32::new(0);
        static SECOND_NOW: AtomicU32 = AtomicU32::new(0);
        static SECOND_PEAK: AtomicU32 = AtomicU32::new(0);
        for counter in [&FIRST_NOW, &FIRST_PEAK, &SECOND_NOW, &SECOND_PEAK] {
            counter.store(0, Ordering::SeqCst);
        }

        let items: Vec<u32> = (0..40).collect();
        let _ = pipeline(
            &items,
            5,
            2,
            &Cancel::new(),
            |_, _: &u32| -> Result<u32, ()> {
                let now = FIRST_NOW.fetch_add(1, Ordering::SeqCst) + 1;
                FIRST_PEAK.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
                FIRST_NOW.fetch_sub(1, Ordering::SeqCst);
                Ok(0)
            },
            |_, _, _: &u32| -> Result<u32, ()> {
                let now = SECOND_NOW.fetch_add(1, Ordering::SeqCst) + 1;
                SECOND_PEAK.fetch_max(now, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(2));
                SECOND_NOW.fetch_sub(1, Ordering::SeqCst);
                Ok(0)
            },
        );

        assert!(
            FIRST_PEAK.load(Ordering::SeqCst) <= 5,
            "first stage unbounded"
        );
        assert!(
            SECOND_PEAK.load(Ordering::SeqCst) <= 2,
            "second stage unbounded"
        );
    }

    /// An item whose first stage failed never reaches the second, and its
    /// failure is where the caller looks for it.
    #[test]
    fn a_failed_first_stage_never_reaches_the_second() {
        let items: Vec<u32> = (0..6).collect();
        let (firsts, seconds) = pipeline(
            &items,
            1,
            1,
            &Cancel::new(),
            |index, item: &u32| -> Result<u32, String> {
                if index == 3 {
                    return Err("no narration".to_owned());
                }
                Ok(*item)
            },
            |index, _, value: &u32| -> Result<u32, String> {
                assert_ne!(index, 3, "the failed item was handed to stage two");
                Ok(*value)
            },
        );

        assert!(matches!(&firsts[3], Outcome::Done(Err(e)) if e == "no narration"));
        assert!(
            seconds[3].was_skipped(),
            "a scene with no narration has no segment"
        );
    }

    /// One failed narration must not cost a full render before the run is
    /// allowed to fail. Serial on both stages so the failure at index 0 is
    /// known before anything else could start.
    #[test]
    fn a_first_stage_failure_stops_admitting_second_stage_work() {
        static SECOND_RAN: AtomicU32 = AtomicU32::new(0);
        SECOND_RAN.store(0, Ordering::SeqCst);

        let items: Vec<u32> = (0..50).collect();
        let (firsts, _) = pipeline(
            &items,
            1,
            1,
            &Cancel::new(),
            |index, item: &u32| -> Result<u32, String> {
                if index == 0 {
                    return Err("the voice service is gone".to_owned());
                }
                Ok(*item)
            },
            |_, _, value: &u32| -> Result<u32, String> {
                SECOND_RAN.fetch_add(1, Ordering::SeqCst);
                Ok(*value)
            },
        );

        assert!(matches!(&firsts[0], Outcome::Done(Err(_))));
        assert!(
            SECOND_RAN.load(Ordering::SeqCst) < 50,
            "every segment rendered anyway, after the run was already doomed"
        );
    }

    /// D-045 in both stages, and — the reason this test exists at all — no
    /// deadlock: a consumer waiting on the queue has to be woken by the last
    /// producer leaving, whether it left because it ran out of work or because
    /// the run was cancelled.
    #[test]
    fn cancellation_stops_both_stages_without_hanging() {
        let cancel = Cancel::new();
        let items: Vec<u32> = (0..200).collect();

        let (firsts, seconds) = pipeline(
            &items,
            2,
            2,
            &cancel,
            |index, item: &u32| -> Result<u32, ()> {
                if index == 1 {
                    cancel.request();
                }
                std::thread::sleep(Duration::from_millis(1));
                Ok(*item)
            },
            |_, _, value: &u32| -> Result<u32, ()> { Ok(*value) },
        );

        assert_eq!(firsts.len(), items.len(), "every item is accounted for");
        assert_eq!(seconds.len(), items.len());
        assert!(
            firsts.iter().any(Outcome::was_skipped),
            "nothing was skipped"
        );
    }

    #[test]
    fn an_empty_pipeline_starts_no_workers() {
        let items: Vec<u32> = Vec::new();
        let (a, b) = pipeline(
            &items,
            4,
            4,
            &Cancel::new(),
            |_, _: &u32| -> Result<u32, ()> { unreachable!() },
            |_, _, _: &u32| -> Result<u32, ()> { unreachable!() },
        );
        assert!(a.is_empty() && b.is_empty());
    }

    /// Nonsense counts on either side are corrected, and more consumers than
    /// there is work must still terminate rather than wait forever.
    #[test]
    fn pipeline_job_counts_are_clamped_and_always_terminate() {
        let items: Vec<u32> = (0..3).collect();
        for (f, g) in [(0, 0), (1, 40), (40, 1), (40, 40)] {
            let (a, b) = pipeline(
                &items,
                f,
                g,
                &Cancel::new(),
                |_, item: &u32| -> Result<u32, ()> { Ok(*item) },
                |_, _, value: &u32| -> Result<u32, ()> { Ok(value * 2) },
            );
            let a: Vec<u32> = a
                .into_iter()
                .filter_map(Outcome::done)
                .map(Result::unwrap)
                .collect();
            let b: Vec<u32> = b
                .into_iter()
                .filter_map(Outcome::done)
                .map(Result::unwrap)
                .collect();
            assert_eq!(a, vec![0, 1, 2], "first stage, jobs = {f}/{g}");
            assert_eq!(b, vec![0, 2, 4], "second stage, jobs = {f}/{g}");
        }
    }
}
