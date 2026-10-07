//! How fast this machine asks a voice service for speech (D-195).
//!
//! Measured in the author's own `runs.csv` on 2026-10-03: a 431-scene project
//! sent **335 lines in three and a half minutes**, eight at a time — about a
//! hundred a minute — and then every connection to `speech.platform.bing.com`
//! was refused (`Connect call failed ('150.171.27.10', 443)`, and the same from
//! a second address). Eleven renders over the next fifty-four minutes failed
//! at the same scene within forty seconds of starting. A machine that can
//! reach nothing does not usually get refused by two of one company's
//! addresses and nobody else's, so the reading is a throttle, and the cure is
//! not to provoke it.
//!
//! Three rules, process-wide because the throttle is per machine, not per
//! render and not per `Edge` value:
//!
//! - at most [`Pace::in_flight`] requests at once, however wide the audio pool;
//! - request starts at least [`Pace::gap`] apart;
//! - once the service **refuses a connection**, every request waits out a
//!   cool-down that doubles while refusals continue and resets on the first
//!   line spoken — so a throttled machine stops hammering the door instead of
//!   eight workers retrying at it twice a second.
//!
//! The threshold the service applies is **not known**; the default is about
//! half the rate that tripped it. At 4K a segment takes far longer to encode
//! than a line takes to speak, so under D-146's overlap the pacing costs a
//! render no wall time at all; at 1080p with no captions it can.
//!
//! **And a throttle is waited out, not reported (D-199).** The pace above did
//! not prevent it: on 2026-10-06 the author's Windows machine, on v0.1.19,
//! spoke about twenty lines a minute — well under the fifty allowed — and was
//! still cut off after ~390 lines, the same count as the 335–396 of the two
//! earlier refusals at five times the rate. So the limit is a **count**, not a
//! rate, and no pace slow enough to be usable avoids it. What the service did
//! this time was let connections time out (`ConnectionTimeoutError`) rather
//! than refuse them, and D-195 knew only refusals, so each line used its three
//! attempts in seconds and eight scenes failed the render. A refusal streak is
//! now waited through for up to [`Pace::patience`] while the segment pool
//! keeps encoding everything already spoken — or [`Pace::patience_cold`] when
//! this process has not spoken a line yet, because a machine whose firewall
//! drops the service from the first request should not wait half an hour to
//! be told so.
//!
//! The arithmetic is [`Schedule`], which takes the clock as an argument so it
//! is tested without sleeping. [`Gate`] is the blocking shell around it.

use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use spoonstill_media::command::Cancel;

/// How a provider spaces its requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// Requests allowed at once.
    pub in_flight: usize,
    /// The least time between two request starts.
    pub gap: Duration,
    /// The first cool-down after a refused connection; doubled for each one
    /// after, up to [`Pace::max_cooldown`].
    pub cooldown: Duration,
    /// The longest a single cool-down grows.
    pub max_cooldown: Duration,
    /// How long one streak of refusals is waited through, once this process
    /// has spoken a line, before a line gives up (D-199).
    pub patience: Duration,
    /// The same, before this process has spoken anything: a service that has
    /// never answered is more likely a blocked network than a throttle.
    pub patience_cold: Duration,
}

impl Pace {
    /// What the Edge service is asked at: two at a time, starts 1.2 s apart —
    /// at most fifty a minute against the hundred that was refused.
    #[must_use]
    pub const fn service() -> Self {
        Pace {
            in_flight: 2,
            gap: Duration::from_millis(1200),
            cooldown: Duration::from_secs(15),
            max_cooldown: Duration::from_secs(120),
            patience: Duration::from_secs(60 * 60),
            patience_cold: Duration::from_secs(3 * 60),
        }
    }

    /// No pacing at all — what a test against a stand-in tool asks for.
    #[must_use]
    pub const fn unpaced() -> Self {
        Pace {
            in_flight: usize::MAX,
            gap: Duration::ZERO,
            cooldown: Duration::ZERO,
            max_cooldown: Duration::ZERO,
            patience: Duration::ZERO,
            patience_cold: Duration::ZERO,
        }
    }

    fn is_unpaced(self) -> bool {
        self.in_flight == usize::MAX && self.gap.is_zero() && self.cooldown.is_zero()
    }
}

/// The pure state of the gate. Every method takes `now`.
#[derive(Debug, Default)]
pub struct Schedule {
    in_flight: usize,
    next_start: Option<Instant>,
    cool_until: Option<Instant>,
    /// Consecutive refusals since the last line spoken.
    refusals: u32,
    /// When the current streak of refusals began; `None` while answering.
    streak_since: Option<Instant>,
    /// Streaks begun in this process, so a watcher can tell a new one.
    episodes: u64,
    /// Whether this process has spoken a line at all.
    spoken_any: bool,
    /// What the service said the last time it turned this machine away, so
    /// the log can say it while a render waits rather than only when a line
    /// finally gives up (D-016).
    said: String,
}

impl Schedule {
    /// Take a slot now, or say how long until one could be free.
    ///
    /// `Ok(())` means the request may start. `Err(wait)` means ask again after
    /// at most `wait`; a full house answers with a short poll, because a slot
    /// frees when another request ends, not at a time this can predict.
    pub fn try_start(&mut self, pace: Pace, now: Instant) -> Result<(), Duration> {
        let earliest = [self.next_start, self.cool_until]
            .into_iter()
            .flatten()
            .max();
        if let Some(earliest) = earliest
            && earliest > now
        {
            return Err(earliest - now);
        }
        if self.in_flight >= pace.in_flight {
            return Err(Duration::from_millis(100));
        }
        self.in_flight += 1;
        self.next_start = Some(now + pace.gap);
        Ok(())
    }

    /// A request that started has ended.
    pub fn finish(&mut self) {
        self.in_flight = self.in_flight.saturating_sub(1);
    }

    /// The service refused a connection: everyone waits, longer each time.
    pub fn refused(&mut self, pace: Pace, now: Instant) -> Duration {
        let wait = pace
            .cooldown
            .checked_mul(2u32.saturating_pow(self.refusals))
            .unwrap_or(pace.max_cooldown)
            .min(pace.max_cooldown);
        self.refusals = self.refusals.saturating_add(1);
        if self.streak_since.is_none() {
            self.streak_since = Some(now);
            self.episodes += 1;
        }
        let until = now + wait;
        self.cool_until = Some(self.cool_until.map_or(until, |current| current.max(until)));
        wait
    }

    /// A line was spoken: the service is answering again.
    pub fn spoke(&mut self) {
        self.refusals = 0;
        self.streak_since = None;
        self.spoken_any = true;
    }

    /// Whether a line turned away now should keep waiting rather than give up:
    /// the current streak is younger than the patience that applies (D-199).
    pub fn keep_waiting(&self, pace: Pace, now: Instant) -> bool {
        let patience = if self.spoken_any {
            pace.patience
        } else {
            pace.patience_cold
        };
        self.streak_since
            .is_some_and(|since| now.saturating_duration_since(since) < patience)
    }

    /// The current streak, if the service is turning this machine away.
    pub fn throttle(&self, now: Instant) -> Option<Throttle> {
        self.streak_since.map(|since| Throttle {
            episode: self.episodes,
            for_how_long: now.saturating_duration_since(since),
            said: self.said.clone(),
        })
    }
}

/// A streak of refusals in progress, as a watcher sees it (D-199).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Throttle {
    /// Which streak this is in the life of the process; a new number is a new
    /// episode worth telling the operator about.
    pub episode: u64,
    /// How long the service has been turning this machine away.
    pub for_how_long: Duration,
    /// What it said the last time, verbatim.
    pub said: String,
}

/// The process-wide gate every Edge request goes through.
pub struct Gate {
    schedule: Mutex<Schedule>,
    changed: Condvar,
}

/// One request's slot; ending it frees the slot, however the request ended.
pub struct Slot<'a> {
    gate: &'a Gate,
    pace: Pace,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        if self.pace.is_unpaced() {
            return;
        }
        if let Ok(mut schedule) = self.gate.schedule.lock() {
            schedule.finish();
        }
        self.gate.changed.notify_all();
    }
}

impl Gate {
    /// The one gate for this process.
    pub fn shared() -> &'static Gate {
        static GATE: OnceLock<Gate> = OnceLock::new();
        GATE.get_or_init(|| Gate {
            schedule: Mutex::new(Schedule::default()),
            changed: Condvar::new(),
        })
    }

    /// Wait for a slot. `None` when the run was cancelled while waiting.
    pub fn enter(&self, pace: Pace, cancel: &Cancel) -> Option<Slot<'_>> {
        if pace.is_unpaced() {
            return Some(Slot { gate: self, pace });
        }
        let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancel.is_requested() {
                return None;
            }
            match schedule.try_start(pace, Instant::now()) {
                Ok(()) => return Some(Slot { gate: self, pace }),
                // Never sleep longer than a cancellation poll, so Stop is
                // obeyed during a two-minute cool-down (D-186).
                Err(wait) => {
                    schedule = self
                        .changed
                        .wait_timeout(schedule, wait.min(Duration::from_millis(100)))
                        .map_or_else(|e| e.into_inner().0, |(guard, _)| guard);
                }
            }
        }
    }

    /// Record a refused connection and what the service said; returns the
    /// cool-down it started.
    pub fn refused(&self, pace: Pace, said: &str) -> Duration {
        if pace.is_unpaced() {
            return Duration::ZERO;
        }
        let mut schedule = self.schedule.lock().unwrap_or_else(|e| e.into_inner());
        said.clone_into(&mut schedule.said);
        let wait = schedule.refused(pace, Instant::now());
        drop(schedule);
        self.changed.notify_all();
        wait
    }

    /// Whether a line the service just turned away should keep waiting.
    pub fn keep_waiting(&self, pace: Pace) -> bool {
        if pace.is_unpaced() {
            return false;
        }
        self.schedule
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keep_waiting(pace, Instant::now())
    }

    /// The streak in progress, if any: what a render polls to tell the
    /// operator it is waiting rather than stuck.
    pub fn throttle(&self) -> Option<Throttle> {
        self.schedule
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .throttle(Instant::now())
    }

    /// Record a line spoken.
    pub fn spoke(&self, pace: Pace) {
        if pace.is_unpaced() {
            return;
        }
        self.schedule
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .spoke();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PACE: Pace = Pace {
        in_flight: 2,
        gap: Duration::from_millis(1200),
        cooldown: Duration::from_secs(15),
        max_cooldown: Duration::from_secs(120),
        patience: Duration::from_secs(60 * 60),
        patience_cold: Duration::from_secs(3 * 60),
    };

    #[test]
    fn starts_are_spaced_by_the_gap() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        assert_eq!(s.try_start(PACE, t0), Ok(()));
        assert_eq!(
            s.try_start(PACE, t0 + Duration::from_millis(200)),
            Err(Duration::from_millis(1000))
        );
        assert_eq!(s.try_start(PACE, t0 + Duration::from_millis(1200)), Ok(()));
    }

    #[test]
    fn no_more_than_in_flight_run_at_once() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        let mut t = t0;
        assert_eq!(s.try_start(PACE, t), Ok(()));
        t += PACE.gap;
        assert_eq!(s.try_start(PACE, t), Ok(()));
        t += PACE.gap;
        assert!(
            s.try_start(PACE, t).is_err(),
            "a third would be over the cap"
        );
        s.finish();
        assert_eq!(s.try_start(PACE, t), Ok(()));
    }

    /// The defect in the log, as arithmetic: a hundred lines in a minute is
    /// what was refused, and no sequence of starts gets past fifty.
    #[test]
    fn a_minute_holds_at_most_fifty_starts() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        let mut started = 0;
        let mut t = t0;
        while t < t0 + Duration::from_secs(60) {
            if s.try_start(PACE, t).is_ok() {
                started += 1;
                s.finish();
            }
            t += Duration::from_millis(10);
        }
        assert!(started <= 50, "{started} starts in one minute");
        assert!(
            started >= 49,
            "pacing should not be stricter than stated: {started}"
        );
    }

    #[test]
    fn a_refusal_holds_everyone_and_doubles_until_a_line_is_spoken() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        assert_eq!(s.refused(PACE, t0), Duration::from_secs(15));
        assert_eq!(
            s.try_start(PACE, t0 + Duration::from_secs(5)),
            Err(Duration::from_secs(10))
        );
        assert_eq!(s.refused(PACE, t0), Duration::from_secs(30));
        assert_eq!(s.refused(PACE, t0), Duration::from_secs(60));
        assert_eq!(s.refused(PACE, t0), Duration::from_secs(120));
        assert_eq!(s.refused(PACE, t0), Duration::from_secs(120), "capped");
        s.spoke();
        assert_eq!(s.refused(PACE, t0), Duration::from_secs(15), "reset");
    }

    /// D-199: the throttle in the author's log lasted minutes, so a line keeps
    /// waiting through it, for an hour once this process has spoken and
    /// three minutes when it never has.
    #[test]
    fn a_streak_is_waited_through_for_the_patience_and_no_longer() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        assert!(!s.keep_waiting(PACE, t0), "no streak, nothing to wait for");

        // Never spoken: the short patience applies.
        s.refused(PACE, t0);
        assert!(s.keep_waiting(PACE, t0 + Duration::from_secs(170)));
        assert!(!s.keep_waiting(PACE, t0 + Duration::from_secs(181)));

        // Once a line has been spoken in this process, the long one does.
        s.spoke();
        assert!(s.throttle(t0).is_none(), "speaking ends the streak");
        let t1 = t0 + Duration::from_secs(600);
        s.refused(PACE, t1);
        s.refused(PACE, t1 + Duration::from_secs(60));
        assert!(
            s.keep_waiting(PACE, t1 + Duration::from_secs(59 * 60)),
            "the streak is timed from its first refusal, not its latest"
        );
        assert!(!s.keep_waiting(PACE, t1 + Duration::from_secs(61 * 60)));
    }

    #[test]
    fn each_streak_is_a_new_episode_and_repeated_refusals_are_not() {
        let t0 = Instant::now();
        let mut s = Schedule::default();
        s.refused(PACE, t0);
        s.refused(PACE, t0);
        assert_eq!(s.throttle(t0).map(|t| t.episode), Some(1));
        s.spoke();
        s.refused(PACE, t0 + Duration::from_secs(5));
        let throttle = s.throttle(t0 + Duration::from_secs(65)).expect("a streak");
        assert_eq!(throttle.episode, 2);
        assert!(
            throttle.said.is_empty(),
            "the schedule alone is told nothing"
        );
        assert_eq!(throttle.for_how_long, Duration::from_secs(60));
    }

    #[test]
    fn an_unpaced_gate_never_waits() {
        let gate = Gate {
            schedule: Mutex::new(Schedule::default()),
            changed: Condvar::new(),
        };
        gate.refused(Pace::unpaced(), "429");
        assert!(!gate.keep_waiting(Pace::unpaced()));
        assert!(gate.throttle().is_none());
    }

    #[test]
    fn a_cancelled_wait_returns_promptly() {
        let gate = Gate {
            schedule: Mutex::new(Schedule::default()),
            changed: Condvar::new(),
        };
        gate.refused(PACE, "429");
        let cancel = Cancel::new();
        cancel.request();
        let started = Instant::now();
        assert!(gate.enter(PACE, &cancel).is_none());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
