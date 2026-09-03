//! How many segment workers this machine can afford (D-144).
//!
//! D-076 sized the render pool from the core count and capped it at four. That
//! table was measured entirely at 1080p, and it stayed the whole answer when
//! D-143 made 4K reachable from `still render` — so `--resolution 4k` kept
//! four workers while quietly asking for 2.3x the memory each.
//!
//! The failure that produced this module was not a slow render. It was a
//! Windows machine that **froze on start**: four 4K workers committed more
//! than the machine had, the page file took the difference, and nothing —
//! including the log line that would have said so — got written. Memory is the
//! one resource where being wrong costs the whole machine rather than some
//! wall clock, which is why the automatic answer is now derived from it.
//!
//! D-044 required exactly this and deferred it to M3. A frozen machine is what
//! moved it forward.

use spoonstill_core::OutputSpec;

/// What one worker costs before a single pixel of canvas — the FFmpeg process,
/// its demuxers, the decoded still, and x264's fixed structures.
///
/// The intercept of the fit in [`segment_memory_bytes`], rounded up.
pub const SEGMENT_BASE_BYTES: u64 = 192 * 1024 * 1024;

/// What one worker costs per pixel of the *prescale* canvas (D-032).
///
/// The prescale canvas, not the output frame, because that is what the filter
/// graph holds: at 1080p it is 5760x3240 and at 4K it is 11520x6480, which is
/// the 4x that makes 4K expensive. 36 bytes per pixel is about 24 frames of
/// `yuv420p` (1.5 bytes/px) held across the zoompan pipeline and x264's
/// lookahead — offered as a sanity check on the magnitude, not as a derivation.
/// The number is measured.
pub const SEGMENT_BYTES_PER_PRESCALE_PIXEL: u64 = 36;

/// Share of total RAM one render may plan to occupy, as a percentage.
///
/// The rest is the operating system, the window if the render was started from
/// it, and whatever else the operator is doing on the machine they are also
/// using. 70% rather than something tighter because a render that is refused
/// capacity it actually had is a regression against D-076's measured curve.
pub const BUDGET_PERCENT: u64 = 70;

/// Roughly what one concurrent segment worker needs at this geometry, in bytes.
///
/// Measured on macOS arm64 2026-09-03, one scene per resolution, peak RSS of
/// the FFmpeg child (`ffmpeg-findings.md` §12):
///
/// | output | prescale | measured | this model | headroom |
/// |---|---|---|---|---|
/// | 1280x720 | 3840x2160 | 369 MB | 477 MB | +29% |
/// | 1920x1080 | 5760x3240 | 768 MB | 833 MB | +8% |
/// | 2560x1440 | 7680x4320 | 1219 MB | 1331 MB | +9% |
/// | 3840x2160 | 11520x6480 | 2630 MB | 2755 MB | +5% |
///
/// The model is deliberately **above** every measurement. Over-estimating
/// costs a worker; under-estimating costs the machine.
///
/// The base was 128 MB in the first draft, which put 1080p **0.7 MB** above
/// its measurement — arithmetically above and practically equal, so a re-run
/// that measured 769 MB would have made a liar of the sentence above. The
/// headroom column exists so that a future measurement changes a number here
/// rather than quietly inverting the rule.
///
/// The 768 MB at 1080p is the same number `ffmpeg-findings.md` §10b recorded
/// as 780 MB by a different method, which is the reason to trust the other
/// three rows.
#[must_use]
pub fn segment_memory_bytes(output: OutputSpec) -> u64 {
    let pixels = u64::from(output.prescale_width()) * u64::from(output.prescale_height());
    SEGMENT_BASE_BYTES + pixels * SEGMENT_BYTES_PER_PRESCALE_PIXEL
}

/// This machine's total physical memory, if it can be read.
///
/// Total rather than *available*: available fluctuates second to second, so
/// sizing a pool from it makes two runs of one project pick different worker
/// counts for reasons the operator cannot see. Total is stable and is a number
/// they know about their own machine.
///
/// `None` when the platform will not say, in which case the caller keeps
/// D-076's core-derived answer — this narrows a pool, it never widens one.
#[must_use]
pub fn total_memory_bytes() -> Option<u64> {
    let mut system = sysinfo::System::new();
    system.refresh_memory();
    match system.total_memory() {
        0 => None,
        bytes => Some(bytes),
    }
}

/// The environment variable that overrides the budget, in whole megabytes.
///
/// Two uses, and the second is why it exists. An operator who has closed
/// everything can hand a render more than [`BUDGET_PERCENT`] of the machine;
/// and a test can state what machine it is pretending to be, which is the only
/// way to check this module's behaviour on a machine other than the one
/// running the test. A gate that can only assert what this laptop happens to
/// have is a gate that passes for the wrong reason.
pub const BUDGET_ENV: &str = "SPOONSTILL_MEMORY_BUDGET_MB";

/// How much memory a render may plan to use.
///
/// [`BUDGET_ENV`] first, then [`BUDGET_PERCENT`] of physical memory. An
/// unparseable or zero value in the environment is ignored rather than
/// refused: this is a tuning knob on a path whose failure mode is a frozen
/// machine, and falling back to the measured default is always safe.
#[must_use]
pub fn budget_bytes() -> Option<u64> {
    if let Some(megabytes) = std::env::var(BUDGET_ENV)
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .filter(|megabytes| *megabytes > 0)
    {
        return Some(megabytes * 1024 * 1024);
    }
    total_memory_bytes().map(|total| total / 100 * BUDGET_PERCENT)
}

/// How the worker count for this run was arrived at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    /// Workers this run will actually start.
    pub jobs: usize,
    /// What D-076's core-count rule alone would have said.
    pub by_cores: usize,
    /// What the memory budget alone would have said, if memory could be read.
    pub by_memory: Option<usize>,
    /// Estimated cost of one worker at this geometry.
    pub per_worker: u64,
    /// The budget that cost was measured against.
    pub budget: Option<u64>,
}

impl Capacity {
    /// Whether the memory budget, not the core count, decided this.
    #[must_use]
    pub fn limited_by_memory(&self) -> bool {
        self.by_memory.is_some_and(|memory| memory < self.by_cores)
    }
}

/// Decide the automatic worker count for a render at this geometry.
///
/// The lower of D-076's core-derived answer and what the memory budget affords,
/// never below one. One worker may exceed the budget on a small machine at a
/// large geometry — that is reported by [`pressure`] rather than refused,
/// because refusing to render at all is worse than rendering slowly, and the
/// operator may have closed everything else.
#[must_use]
pub fn plan(output: OutputSpec) -> Capacity {
    plan_within(output, budget_bytes(), crate::pool::default_jobs())
}

/// [`plan`], with the machine stated rather than measured.
///
/// The decision, as a pure function. `plan` is this with two readings of the
/// real machine substituted in — which is the only reason the rule can be
/// tested at a size other than whatever the test runner happens to have.
/// D-116's trap is live here: on a 24 GB laptop every resolution affords four
/// workers, so a test that called `plan` would assert the right property on an
/// input that cannot exhibit the defect, and pass forever.
#[must_use]
pub fn plan_within(output: OutputSpec, budget: Option<u64>, by_cores: usize) -> Capacity {
    let per_worker = segment_memory_bytes(output);
    let by_memory = budget.map(|budget| usize::try_from(budget / per_worker).unwrap_or(usize::MAX));
    let jobs = by_memory
        .map_or(by_cores, |memory| by_cores.min(memory))
        .max(1);
    Capacity {
        jobs,
        by_cores,
        by_memory,
        per_worker,
        budget,
    }
}

/// A run that is planning to use more memory than this machine should give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pressure {
    /// Workers this run will start.
    pub jobs: usize,
    /// What they are estimated to need together.
    pub needed: u64,
    /// What the machine affords.
    pub budget: u64,
    /// The largest worker count that fits, which may be zero.
    pub fits: usize,
}

/// Whether `jobs` workers at this geometry overcommit the machine.
///
/// `None` when it fits, or when the platform would not say how much memory it
/// has — a warning nobody can act on is noise, and this one is only worth
/// printing when it names a smaller number to pass.
#[must_use]
pub fn pressure(output: OutputSpec, jobs: usize) -> Option<Pressure> {
    pressure_within(output, budget_bytes(), jobs)
}

/// [`pressure`], with the budget stated rather than measured.
#[must_use]
pub fn pressure_within(output: OutputSpec, budget: Option<u64>, jobs: usize) -> Option<Pressure> {
    let budget = budget?;
    let per_worker = segment_memory_bytes(output);
    let needed = per_worker.saturating_mul(jobs as u64);
    if needed <= budget {
        return None;
    }
    Some(Pressure {
        jobs,
        needed,
        budget,
        fits: usize::try_from(budget / per_worker).unwrap_or(usize::MAX),
    })
}

/// Bytes as an operator reads them: `2.6 GB`.
#[must_use]
pub fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spoonstill_core::{Aspect, Resolution};

    fn spec(resolution: Resolution) -> OutputSpec {
        OutputSpec::new(Aspect::Landscape16x9, resolution.short_edge(), 30).expect("a legal size")
    }

    /// The whole point: the model must not sit *below* what was measured, or a
    /// pool sized from it overcommits exactly the way D-144 was written to
    /// stop. Measured peak RSS of one FFmpeg child, macOS arm64 2026-09-03.
    #[test]
    fn the_model_is_never_under_what_was_measured() {
        let measured_mb = [
            (Resolution::Hd720, 369_u64),
            (Resolution::Fhd1080, 768),
            (Resolution::Qhd1440, 1219),
            (Resolution::Uhd2160, 2630),
        ];
        for (resolution, measured) in measured_mb {
            let modelled = segment_memory_bytes(spec(resolution)) / (1024 * 1024);
            assert!(
                modelled >= measured,
                "{resolution:?}: model says {modelled} MB, measurement was {measured} MB — \
                 under-estimating memory is the failure this module exists to prevent"
            );
            // Above by a margin that survives a re-measurement. The first
            // draft cleared 1080p by 0.7 MB, which is arithmetically above and
            // practically equal — the CI runner or a warm cache would have
            // inverted it. 2% is not much; it is enough to be a decision.
            assert!(
                modelled >= measured + measured / 50,
                "{resolution:?}: model {modelled} MB clears the measured {measured} MB \
                 by less than 2%, which is noise rather than headroom"
            );
            // And not so far over that it strands capacity a machine has.
            assert!(
                modelled <= measured * 3 / 2,
                "{resolution:?}: model says {modelled} MB against {measured} MB measured, \
                 which would refuse workers this machine can afford"
            );
        }
    }

    /// The defect, stated as arithmetic: 4K costs materially more per worker
    /// than 1080p, and nothing before D-144 knew it.
    #[test]
    fn four_k_costs_more_per_worker_than_1080p() {
        let hd = segment_memory_bytes(spec(Resolution::Fhd1080));
        let uhd = segment_memory_bytes(spec(Resolution::Uhd2160));
        assert!(
            uhd > hd * 3,
            "4K was measured at 2630 MB against 768 MB — a model that does not \
             reproduce that gap cannot size a pool"
        );
    }

    /// Cost follows the prescale canvas, so the two vertical aspects cost what
    /// the landscape one does at the same short edge — the *frame* is rotated,
    /// the pixel count is not.
    #[test]
    fn an_aspect_is_not_cheaper_than_its_rotation() {
        let landscape =
            segment_memory_bytes(OutputSpec::new(Aspect::Landscape16x9, 1080, 30).expect("16:9"));
        let portrait =
            segment_memory_bytes(OutputSpec::new(Aspect::Portrait9x16, 1080, 30).expect("9:16"));
        assert_eq!(
            landscape, portrait,
            "a Short is the same pixels as the landscape film it was cropped from"
        );
    }

    /// Eight gigabytes, which is what the machine in the report had to be
    /// near. 70% of it is the budget these tests reason about.
    const EIGHT_GB_BUDGET: Option<u64> = Some(5_600 * 1024 * 1024);

    /// **The defect.** Four workers at 4K on an 8 GB machine is what froze a
    /// Windows laptop hard enough to need the power button. The rule must pick
    /// fewer, and the same machine at 1080p must be untouched.
    #[test]
    fn a_small_machine_gets_fewer_workers_at_4k_and_the_same_at_1080p() {
        let uhd = plan_within(spec(Resolution::Uhd2160), EIGHT_GB_BUDGET, 4);
        assert_eq!(
            uhd.jobs, 2,
            "4K on an 8 GB machine: four workers is the freeze this exists to stop"
        );
        assert!(uhd.limited_by_memory());

        let hd = plan_within(spec(Resolution::Fhd1080), EIGHT_GB_BUDGET, 4);
        assert_eq!(
            hd.jobs, 4,
            "1080p on the same machine fits, and must not be slowed down by this rule"
        );
        assert!(!hd.limited_by_memory());
    }

    /// D-144's own rule: this narrows a pool, it never widens one. A machine
    /// with plenty of memory still gets D-076's core-derived answer.
    #[test]
    fn the_memory_cap_never_exceeds_the_core_derived_answer() {
        for budget in [EIGHT_GB_BUDGET, Some(u64::from(u32::MAX)), None] {
            for resolution in Resolution::ALL {
                let plan = plan_within(spec(resolution), budget, 4);
                assert!(
                    plan.jobs <= plan.by_cores,
                    "{resolution:?} at {budget:?}: {} workers is more than the core rule's {}",
                    plan.jobs,
                    plan.by_cores
                );
                assert!(plan.jobs >= 1, "{resolution:?}: a render must be possible");
            }
        }
    }

    /// A bigger frame never gets more workers than a smaller one — asserted at
    /// a budget where the sizes actually differ, or it proves nothing.
    #[test]
    fn a_bigger_frame_never_gets_more_workers() {
        let mut previous = usize::MAX;
        let mut seen = std::collections::BTreeSet::new();
        for resolution in Resolution::ALL {
            let jobs = plan_within(spec(resolution), EIGHT_GB_BUDGET, 4).jobs;
            assert!(
                jobs <= previous,
                "{resolution:?} got {jobs} workers, more than the smaller size's {previous}"
            );
            previous = jobs;
            seen.insert(jobs);
        }
        assert!(
            seen.len() > 1,
            "every size got the same worker count, so this test would pass \
             against a rule that ignores geometry entirely (D-116)"
        );
    }

    /// A machine too small for even one worker still renders. Refusing outright
    /// is worse than rendering slowly, and the operator may have closed
    /// everything else.
    #[test]
    fn a_machine_too_small_for_one_worker_still_renders() {
        let tiny = plan_within(spec(Resolution::Uhd2160), Some(512 * 1024 * 1024), 4);
        assert_eq!(tiny.jobs, 1, "a render must always be possible");
    }

    /// The automatic count is inside the budget by construction, so it must
    /// never be the thing that triggers the warning — otherwise every 4K
    /// render on a small machine warns about a number nobody chose.
    #[test]
    fn the_automatic_count_does_not_warn_about_itself() {
        for budget in [EIGHT_GB_BUDGET, Some(24 * 1024 * 1024 * 1024)] {
            for resolution in Resolution::ALL {
                let output = spec(resolution);
                let plan = plan_within(output, budget, 4);
                // The one exception is the floor: when even a single worker
                // exceeds the budget we render anyway and say so.
                if plan.jobs > 1 {
                    assert!(
                        pressure_within(output, budget, plan.jobs).is_none(),
                        "{resolution:?}: the count this module chose warns about itself"
                    );
                }
            }
        }
    }

    /// An operator who names a number is obeyed and warned, not overruled
    /// (D-076: `--jobs` is not capped in either direction).
    #[test]
    fn an_extravagant_request_is_reported_with_a_number_to_use_instead() {
        let output = spec(Resolution::Uhd2160);
        let Some(pressure) = pressure(output, 512) else {
            // Only reachable on a machine that will not report its memory.
            assert!(
                budget_bytes().is_none(),
                "512 4K workers fit in this budget?"
            );
            return;
        };
        assert_eq!(pressure.jobs, 512);
        assert!(pressure.needed > pressure.budget);
        assert!(
            pressure.fits < 512,
            "a warning that suggests the number that caused it is not a warning"
        );
    }

    /// Bytes are worded once, in Rust, so both surfaces say the same thing.
    #[test]
    fn memory_is_worded_the_way_an_operator_reads_it() {
        assert_eq!(gigabytes(2_630 * 1024 * 1024), "2.6 GB");
        assert_eq!(gigabytes(0), "0.0 GB");
    }
}
