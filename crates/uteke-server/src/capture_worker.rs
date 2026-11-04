//! Scheduled extraction of the capture queue (#1466).
//!
//! Every `[capture] interval_minutes` the server extracts up to
//! `budget_per_run` queued sessions. The store lock is held only to claim a
//! batch and to store its facts, never across the extractor call (#1663), so a
//! slow or hung provider does not hold recall and writes back. A run the extractor stopped (rate limit, provider down) or
//! that failed doubles the wait before the next one
//! (`uteke_core::capture::next_run_delay`); a run that went through resets it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tracing::{info, warn};
use uteke_core::Uteke;
use uteke_core::capture::{
    CaptureSettings, ExtractionRunReport, FactSource, extract_batch, next_run_delay,
};
use uteke_core::extraction::ExtractionConfig;

/// One scheduled run: the merged report, and whether the schedule should
/// back off.
pub(crate) struct Cycle {
    pub report: ExtractionRunReport,
    pub failed: bool,
}

/// Run `f` on the store, or `None` (and a warning) when the lock is poisoned.
fn with_store<T>(uteke: &Mutex<Uteke>, f: impl FnOnce(&Uteke) -> T) -> Option<T> {
    match uteke.lock() {
        Ok(u) => Some(f(&u)),
        Err(_) => {
            warn!("Capture extraction: store lock poisoned, skipping the run");
            None
        }
    }
}

/// Extract up to `settings.budget_per_run` sessions. The store lock is held
/// to claim a batch and to store its facts, never across the extractor call
/// (#1663): that is a blocking request of up to two minutes per chunk, and a
/// held lock would stall every recall and write behind it. Between the claim
/// and the store the batch is leased, so a crash there hands it back as the
/// same batch. Housekeeping runs once per cycle. Stops early when the queue
/// is empty, the extractor stops the run, the store fails, or `shutdown` is
/// set (checked between sessions).
pub(crate) fn run_cycle(
    uteke: &Mutex<Uteke>,
    facts: &dyn FactSource,
    settings: &CaptureSettings,
    shutdown: &AtomicBool,
) -> Cycle {
    let mut cycle = Cycle {
        report: ExtractionRunReport::default(),
        failed: false,
    };
    let report = &mut cycle.report;
    let sessions = with_store(uteke, |u| {
        u.capture_housekeeping(settings, report)?;
        u.capture_pending_sessions(settings.budget_per_run)
    });
    let sessions = match sessions {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            warn!("Capture extraction failed: {e}");
            cycle.failed = true;
            return cycle;
        }
        None => {
            cycle.failed = true;
            return cycle;
        }
    };
    for session in sessions {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        let claimed = with_store(uteke, |u| u.claim_capture_batch(&session));
        let batch = match claimed {
            Some(Ok(Some(b))) => b,
            // Another run took the session first.
            Some(Ok(None)) => continue,
            Some(Err(e)) => {
                warn!("Capture extraction failed: {e}");
                cycle.failed = true;
                break;
            }
            None => {
                cycle.failed = true;
                break;
            }
        };
        // No lock here.
        let extracted = extract_batch(facts, &batch, settings);
        let report = &mut cycle.report;
        match with_store(uteke, |u| {
            u.finish_capture_batch(&batch, extracted, settings, report)
        }) {
            Some(Ok(Ok(()))) => {}
            Some(Ok(Err(stop))) => {
                cycle.report.stopped = Some(stop);
                cycle.failed = true;
                break;
            }
            Some(Err(e)) => {
                warn!("Capture extraction failed: {e}");
                cycle.failed = true;
                break;
            }
            // The batch stays leased and is handed back when the lease expires.
            None => {
                cycle.failed = true;
                break;
            }
        }
    }
    if let Some(Ok(n)) = with_store(uteke, |u| u.sessions_pending()) {
        cycle.report.sessions_pending = n;
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
                let cycle = run_cycle(uteke, facts.as_ref(), &settings, shutdown);
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

    static NO_STOP: AtomicBool = AtomicBool::new(false);

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
        let cycle = run_cycle(&uteke, &facts, &settings(3), &NO_STOP);
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
        let cycle = run_cycle(&uteke, &facts, &settings(10), &NO_STOP);
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
        let cycle = run_cycle(&uteke, &facts, &settings(10), &NO_STOP);
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
        let cycle = run_cycle(&uteke, &facts, &settings(0), &NO_STOP);
        assert!(!cycle.failed);
        assert_eq!(facts.calls.get(), 0);
    }

    /// Sleeps in the extractor call, as a slow provider does.
    struct Slow {
        started: std::sync::atomic::AtomicBool,
        sleep: Duration,
    }

    impl FactSource for Slow {
        fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, Error> {
            self.started.store(true, Ordering::SeqCst);
            std::thread::sleep(self.sleep);
            Ok(vec![ExtractedFact::flat("the cache is redis")])
        }
    }

    #[test]
    fn the_store_lock_is_free_while_the_extractor_runs() {
        let uteke = store_with(1);
        uteke
            .lock()
            .unwrap()
            .remember("the db is postgres", &[], None, None)
            .unwrap();
        let slow = Slow {
            started: AtomicBool::new(false),
            sleep: Duration::from_millis(1500),
        };
        std::thread::scope(|scope| {
            let cycle = scope.spawn(|| run_cycle(&uteke, &slow, &settings(5), &NO_STOP));
            while !slow.started.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(5));
            }
            // The extractor is mid-call: a request needing the lock goes through.
            let t = std::time::Instant::now();
            let hits = uteke
                .lock()
                .unwrap()
                .search("postgres", 5, None, None)
                .unwrap();
            assert_eq!(hits.len(), 1);
            assert!(
                t.elapsed() < Duration::from_millis(800),
                "waited {:?} for the lock",
                t.elapsed()
            );
            assert!(!cycle.is_finished(), "the extraction was still running");
            let cycle = cycle.join().unwrap();
            assert_eq!(
                (cycle.report.sessions_done, cycle.report.facts_stored),
                (1, 1)
            );
        });
    }

    #[test]
    fn a_failure_between_claim_and_store_leaves_the_batch_recoverable() {
        let uteke = store_with(1);
        // the extractor call fails: the batch goes back to the queue
        let facts = Counting {
            calls: Cell::new(0),
            fail_from: 0,
        };
        let cycle = run_cycle(&uteke, &facts, &settings(5), &NO_STOP);
        assert!(cycle.failed);
        assert_eq!(uteke.lock().unwrap().capture_status().unwrap().pending, 1);

        // the thread dies after the claim (a panic in the extractor): the
        // lock is not poisoned, and once the lease expires the batch is
        // extracted again
        struct Panics;
        impl FactSource for Panics {
            fn extract(&self, _text: &str) -> Result<Vec<ExtractedFact>, Error> {
                panic!("extractor died");
            }
        }
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            run_cycle(&uteke, &Panics, &settings(5), &NO_STOP)
        }));
        assert!(caught.is_err());
        assert!(!uteke.is_poisoned(), "no lock was held across the panic");
        assert_eq!(
            uteke.lock().unwrap().capture_status().unwrap().extracting,
            1
        );
        let ok = Counting {
            calls: Cell::new(0),
            fail_from: usize::MAX,
        };
        let expiring = CaptureSettings {
            lease_minutes: 0,
            ..settings(5)
        };
        let cycle = run_cycle(&uteke, &ok, &expiring, &NO_STOP);
        assert_eq!((cycle.report.reclaimed, cycle.report.sessions_done), (1, 1));
    }

    #[test]
    fn a_shutdown_stops_the_cycle_between_sessions() {
        let uteke = store_with(3);
        let facts = Counting {
            calls: Cell::new(0),
            fail_from: usize::MAX,
        };
        let stop = AtomicBool::new(true);
        let cycle = run_cycle(&uteke, &facts, &settings(5), &stop);
        assert_eq!(facts.calls.get(), 0);
        assert!(!cycle.failed);
        assert_eq!(cycle.report.sessions_pending, 3);
    }

    #[test]
    fn a_session_the_provider_refuses_is_tried_once_per_cycle() {
        let uteke = store_with(2);
        struct Refuses(Cell<usize>);
        impl FactSource for Refuses {
            fn extract(&self, text: &str) -> Result<Vec<ExtractedFact>, Error> {
                self.0.set(self.0.get() + 1);
                if text.contains("project 0") {
                    return Err(Error::Generic(
                        "Extraction endpoint returned HTTP 413 Payload Too Large".into(),
                    ));
                }
                Ok(vec![])
            }
        }
        let facts = Refuses(Cell::new(0));
        let cycle = run_cycle(&uteke, &facts, &settings(10), &NO_STOP);
        assert!(!cycle.failed);
        assert_eq!(facts.0.get(), 2, "one attempt each, not a retry loop");
        assert_eq!(cycle.report.sessions_done, 1);
    }
}
