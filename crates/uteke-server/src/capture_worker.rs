//! Scheduled extraction of the capture queue (#1466).
//!
//! Every `[capture] interval_minutes` the server extracts up to
//! `budget_per_run` queued sessions. The store lock is taken per session, not
//! per run, so a run of LLM calls does not hold every request back for its
//! whole length. A run the extractor stopped (rate limit, provider down) or
//! that failed doubles the wait before the next one
//! (`uteke_core::capture::next_run_delay`); a run that went through resets it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tracing::{info, warn};
use uteke_core::Uteke;
use uteke_core::capture::{CaptureSettings, ExtractionRunReport, FactSource, next_run_delay};
use uteke_core::extraction::ExtractionConfig;

/// One scheduled run: the merged report, and whether the schedule should
/// back off.
pub(crate) struct Cycle {
    pub report: ExtractionRunReport,
    pub failed: bool,
}

/// Extract up to `settings.budget_per_run` sessions, one store lock per
/// session. Stops early when the queue is empty, the extractor stops the
/// run, or the store fails.
pub(crate) fn run_cycle(
    uteke: &Mutex<Uteke>,
    facts: &dyn FactSource,
    settings: &CaptureSettings,
) -> Cycle {
    let mut cycle = Cycle {
        report: ExtractionRunReport::default(),
        failed: false,
    };
    for _ in 0..settings.budget_per_run {
        let run = match uteke.lock() {
            Ok(u) => u.run_capture_extraction(facts, 1, settings),
            Err(_) => {
                warn!("Capture extraction: store lock poisoned, skipping the run");
                cycle.failed = true;
                break;
            }
        };
        let r = match run {
            Ok(r) => r,
            Err(e) => {
                warn!("Capture extraction failed: {e}");
                cycle.failed = true;
                break;
            }
        };
        let worked = r.sessions_done + r.sessions_failed > 0;
        let total = &mut cycle.report;
        total.sessions_done += r.sessions_done;
        total.sessions_failed += r.sessions_failed;
        total.facts_stored += r.facts_stored;
        total.facts_refused += r.facts_refused;
        total.reclaimed += r.reclaimed;
        total.pruned += r.pruned;
        total.sessions_pending = r.sessions_pending;
        if r.stopped.is_some() {
            total.stopped = r.stopped;
            cycle.failed = true;
            break;
        }
        if !worked {
            break;
        }
    }
    cycle
}

/// The schedule: sleep, run, adjust the wait. Returns when `shutdown` is set.
pub(crate) fn run_forever(
    uteke: &Mutex<Uteke>,
    extraction: &ExtractionConfig,
    settings: CaptureSettings,
    shutdown: &AtomicBool,
) {
    // Minimum one minute to prevent a busy loop.
    let interval = Duration::from_secs(settings.interval_minutes.max(1) * 60);
    let mut wait = interval;
    loop {
        // Sleep in one-second steps so a shutdown is seen promptly.
        let mut slept = Duration::ZERO;
        while slept < wait {
            if shutdown.load(Ordering::SeqCst) {
                return;
            }
            std::thread::sleep(Duration::from_secs(1));
            slept += Duration::from_secs(1);
        }
        let failed = match uteke_core::capture::fact_source(extraction) {
            Ok(facts) => {
                let cycle = run_cycle(uteke, facts.as_ref(), &settings);
                log_cycle(&cycle);
                cycle.failed
            }
            Err(e) => {
                warn!("Capture extraction: cannot initialize the extractor: {e}");
                true
            }
        };
        wait = next_run_delay(interval, wait, failed);
        if failed {
            warn!(
                "Capture extraction: next run in {} min (backing off)",
                wait.as_secs() / 60
            );
        }
    }
}

fn log_cycle(cycle: &Cycle) {
    let r = &cycle.report;
    if r.sessions_done + r.sessions_failed > 0 || r.reclaimed > 0 || r.pruned > 0 {
        info!(
            "Capture extraction: {} session(s), {} facts stored, {} refused, {} failed, {} requeued, {} pruned; {} pending",
            r.sessions_done,
            r.facts_stored,
            r.facts_refused,
            r.sessions_failed,
            r.reclaimed,
            r.pruned,
            r.sessions_pending
        );
    }
    if let Some(why) = &r.stopped {
        warn!("Capture extraction stopped early, the extractor failed: {why}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use uteke_core::Error;
    use uteke_core::capture::Role;
    use uteke_core::extraction::ExtractedFact;

    fn store_with(sessions: usize) -> Mutex<Uteke> {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        for i in 0..sessions {
            u.capture(
                &format!("s{i}"),
                Role::User,
                &format!("I prefer tabs over spaces in project {i}"),
                None,
                None,
            )
            .expect("capture");
        }
        Mutex::new(u)
    }

    fn settings(budget: usize) -> CaptureSettings {
        CaptureSettings {
            enabled: true,
            budget_per_run: budget,
            ..Default::default()
        }
    }

    /// Counts calls; fails from call `fail_from` on.
    struct Counting {
        calls: Cell<usize>,
        fail_from: usize,
    }

    impl FactSource for Counting {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, Error> {
            let n = self.calls.get();
            self.calls.set(n + 1);
            if n >= self.fail_from {
                return Err(Error::Validation("429 Too Many Requests".into()));
            }
            Ok(vec![])
        }
    }

    #[test]
    fn a_cycle_extracts_at_most_its_budget() {
        let uteke = store_with(5);
        let facts = Counting {
            calls: Cell::new(0),
            fail_from: usize::MAX,
        };
        let cycle = run_cycle(&uteke, &facts, &settings(3));
        assert!(!cycle.failed);
        assert_eq!(facts.calls.get(), 3);
        assert_eq!(cycle.report.sessions_done, 3);
        assert_eq!(cycle.report.sessions_pending, 2);
    }

    #[test]
    fn a_cycle_ends_when_the_queue_is_empty() {
        let uteke = store_with(2);
        let facts = Counting {
            calls: Cell::new(0),
            fail_from: usize::MAX,
        };
        let cycle = run_cycle(&uteke, &facts, &settings(10));
        assert!(!cycle.failed);
        assert_eq!(cycle.report.sessions_done, 2);
        assert_eq!(cycle.report.sessions_pending, 0);
    }

    #[test]
    fn an_extractor_error_stops_the_cycle_and_backs_off() {
        let uteke = store_with(4);
        let facts = Counting {
            calls: Cell::new(0),
            fail_from: 1,
        };
        let cycle = run_cycle(&uteke, &facts, &settings(10));
        assert!(cycle.failed, "the schedule backs off");
        assert_eq!(facts.calls.get(), 2, "no call after the failing one");
        assert_eq!(cycle.report.sessions_done, 1);
        assert!(cycle.report.stopped.is_some());
        let status = uteke.lock().unwrap().capture_status().unwrap();
        assert_eq!(
            status.extracting, 0,
            "the failed session went back to the queue"
        );
        assert_eq!(status.sessions_pending, 3);
    }

    #[test]
    fn a_zero_budget_runs_nothing() {
        let uteke = store_with(1);
        let facts = Counting {
            calls: Cell::new(0),
            fail_from: usize::MAX,
        };
        let cycle = run_cycle(&uteke, &facts, &settings(0));
        assert!(!cycle.failed);
        assert_eq!(facts.calls.get(), 0);
    }
}
