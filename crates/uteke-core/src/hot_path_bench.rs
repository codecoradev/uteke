//! Micro-benchmarks of the Rust and SQLite hot paths, without the embedder (#1522).
//!
//! The end-to-end numbers (`benchmarks/internal`) are dominated by the ~20 ms
//! embedding call, which hides a change of a few microseconds. These measure
//! the parts a code change can actually move, with the deterministic keyword
//! embedder, so a candidate optimisation (cached prepared statements, a
//! pre-filter in the secret scan, ...) can be judged on numbers before and
//! after. Ignored by default; run in release mode:
//!
//! ```text
//! cargo test -p uteke-core --release --lib hot_path_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! Each line is `name: median_us (min..max over N runs)`, medians of several
//! passes so a busy machine does not decide the result.

use crate::test_support::open_keyword;
use std::time::Instant;

fn median_us(mut v: Vec<f64>) -> (f64, f64, f64) {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[v.len() / 2], v[0], v[v.len() - 1])
}

fn words(i: usize) -> String {
    const W: [&str; 12] = [
        "system", "server", "database", "client", "agent", "module", "service", "pipeline",
        "router", "worker", "cache", "index",
    ];
    format!(
        "The {} {} handles {} requests for {} number {}",
        W[i % 12],
        W[(i / 12) % 12],
        W[(i / 144) % 12],
        W[(i * 7) % 12],
        i
    )
}

fn report(name: &str, per_pass_us: Vec<f64>) {
    let n = per_pass_us.len();
    let (m, lo, hi) = median_us(per_pass_us);
    println!("{name}: {m:.1} us (min {lo:.1} .. max {hi:.1}, {n} passes)");
}

#[test]
#[ignore = "micro-benchmark: run with --release --ignored --nocapture"]
fn hot_paths() {
    let u = open_keyword();

    // remember (write tail without a real embedder), per call.
    let mut passes = Vec::new();
    let mut next = 0usize;
    for _ in 0..5 {
        let t = Instant::now();
        for _ in 0..400 {
            u.remember(&words(next), &[], None, None).unwrap();
            next += 1;
        }
        passes.push(t.elapsed().as_secs_f64() * 1e6 / 400.0);
    }
    report("remember (per call)", passes);

    // Grow the store so recall has something to scan.
    while next < 4000 {
        u.remember(&words(next), &[], None, None).unwrap();
        next += 1;
    }

    // recall, per call, over a 4000-memory store.
    for (label, run) in [
        ("recall vector", crate::RecallStrategy::Vector),
        ("recall hybrid", crate::RecallStrategy::Hybrid),
        ("recall fusion", crate::RecallStrategy::Fusion),
    ] {
        let mut passes = Vec::new();
        for p in 0..5 {
            let t = Instant::now();
            for i in 0..200 {
                // A different query each time so the recall cache does not answer.
                u.recall_hybrid(&words(p * 1000 + i * 3 + 1), 5, None, None, run, 0.0)
                    .unwrap();
            }
            passes.push(t.elapsed().as_secs_f64() * 1e6 / 200.0);
        }
        report(&format!("{label} (per call, 4000 memories)"), passes);
    }

    // FTS5 alone (no vector), to see how the hybrid cost splits.
    let mut passes = Vec::new();
    for p in 0..5 {
        let t = Instant::now();
        for i in 0..200 {
            u.recall_hybrid(
                &words(p * 1000 + i * 3 + 1),
                5,
                None,
                None,
                crate::RecallStrategy::Fts5,
                0.0,
            )
            .unwrap();
        }
        passes.push(t.elapsed().as_secs_f64() * 1e6 / 200.0);
    }
    report("recall fts5 (per call, 4000 memories)", passes);

    // get_by_id, per call.
    let ids: Vec<String> = u
        .list(None, 200, 0, None)
        .unwrap()
        .into_iter()
        .map(|m| m.id)
        .collect();
    let mut passes = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        for _ in 0..5 {
            for id in &ids {
                u.get_by_id(id).unwrap();
            }
        }
        passes.push(t.elapsed().as_secs_f64() * 1e6 / (5.0 * ids.len() as f64));
    }
    report("get_by_id (per call)", passes);

    // The secret scan on a 4 KiB text with no secret in it (the common case).
    let text: String = (0..60).map(words).collect::<Vec<_>>().join(". ");
    let mut passes = Vec::new();
    for _ in 0..5 {
        let t = Instant::now();
        for _ in 0..200 {
            let _ = crate::privacy::redact(&text, &[]);
        }
        passes.push(t.elapsed().as_secs_f64() * 1e6 / 200.0);
    }
    report(
        &format!("privacy::redact ({} bytes, no secret)", text.len()),
        passes,
    );
}

/// The same operations on a database file, where every commit is an fsync
/// (`:memory:` hides that). This is what the server pays.
#[test]
#[ignore = "micro-benchmark: run with --release --ignored --nocapture"]
fn hot_paths_on_disk() {
    let dir = tempfile::tempdir().unwrap();
    let u = crate::test_support::open_keyword_at(dir.path().join("uteke.db"));

    let mut passes = Vec::new();
    let mut next = 0usize;
    for _ in 0..5 {
        let t = Instant::now();
        for _ in 0..100 {
            u.remember(&words(next), &[], None, None).unwrap();
            next += 1;
        }
        passes.push(t.elapsed().as_secs_f64() * 1e6 / 100.0);
    }
    report("on disk: remember (per call)", passes);

    while next < 2000 {
        u.remember(&words(next), &[], None, None).unwrap();
        next += 1;
    }
    for (label, run) in [
        ("on disk: recall vector", crate::RecallStrategy::Vector),
        ("on disk: recall fusion", crate::RecallStrategy::Fusion),
    ] {
        let mut passes = Vec::new();
        for p in 0..5 {
            let t = Instant::now();
            for i in 0..100 {
                u.recall_hybrid(&words(p * 1000 + i * 3 + 1), 5, None, None, run, 0.0)
                    .unwrap();
            }
            passes.push(t.elapsed().as_secs_f64() * 1e6 / 100.0);
        }
        report(&format!("{label} (per call, 2000 memories)"), passes);
    }
}
