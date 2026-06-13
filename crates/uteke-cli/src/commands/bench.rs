//! Performance benchmark command — generates synthetic memories and measures
//! insert throughput, recall latency, and storage footprint at multiple scales.

use std::path::{Path, PathBuf};
use std::time::Instant;
use uteke_core::Uteke;

/// Single-scale benchmark result.
#[derive(serde::Serialize)]
pub struct BenchResult {
    pub count: usize,
    pub insert_ops_per_sec: f64,
    pub insert_total_ms: f64,
    pub recall_avg_ms: f64,
    pub recall_p95_ms: f64,
    pub db_size_kb: f64,
    pub index_size_kb: f64,
}

// --- Deterministic synthetic content generator (no external rand dep) ---

const SUBJECTS: &[&str] = &[
    "system", "server", "database", "client", "agent", "module", "service", "pipeline", "router",
    "worker",
];
const VERBS: &[&str] = &[
    "processes",
    "stores",
    "manages",
    "handles",
    "routes",
    "validates",
    "transforms",
    "caches",
    "indexes",
    "schedules",
];
const OBJECTS: &[&str] = &[
    "requests",
    "data",
    "events",
    "messages",
    "records",
    "transactions",
    "sessions",
    "connections",
    "payloads",
    "queries",
];
const ADJECTIVES: &[&str] = &[
    "large",
    "complex",
    "distributed",
    "concurrent",
    "async",
    "cached",
    "indexed",
    "batched",
    "streaming",
    "realtime",
];
const ADVERBS: &[&str] = &[
    "efficiently",
    "reliably",
    "asynchronously",
    "transparently",
    "consistently",
    "securely",
    "dynamically",
    "autonomously",
];

/// Simple xorshift PRNG — deterministic, no dependencies.
struct Rng {
    state: u64,
}

impl Rng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x9E3779B97F4A7C15 } else { seed },
        }
    }

    /// xorshift64*: fast, decent distribution for non-crypto use.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn pick<'a, T>(&mut self, slice: &'a [T]) -> &'a T {
        let idx = (self.next_u64() as usize) % slice.len();
        &slice[idx]
    }
}

/// Generate a synthetic memory sentence.
fn gen_content(rng: &mut Rng) -> String {
    let adj1 = rng.pick(ADJECTIVES);
    let subj = rng.pick(SUBJECTS);
    let verb = rng.pick(VERBS);
    let adj2 = rng.pick(ADJECTIVES);
    let obj = rng.pick(OBJECTS);
    let adv = rng.pick(ADVERBS);
    format!("The {adj1} {subj} {verb} {adj2} {obj} {adv}")
}

/// Generate a synthetic recall query (shorter, question-like).
fn gen_query(rng: &mut Rng) -> String {
    let subj = rng.pick(SUBJECTS);
    let verb = rng.pick(VERBS);
    let obj = rng.pick(OBJECTS);
    format!("What does the {subj} {verb} for {obj}?")
}

const TAG_POOL: &[&str] = &["test", "bench", "synthetic", "data"];

/// Create a unique temp directory for the benchmark store.
fn make_temp_dir(label: &str) -> Result<PathBuf, String> {
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("uteke-bench-{label}-{pid}-{ts}"));
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;
    Ok(dir)
}

/// File size in KB (1 KB = 1024 bytes).
fn file_size_kb(path: &Path) -> f64 {
    std::fs::metadata(path)
        .map(|m| m.len() as f64 / 1024.0)
        .unwrap_or(0.0)
}

/// Percentile from a sorted slice of values. `pct` is 0.0–1.0.
fn percentile(sorted: &[f64], pct: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64) * pct).ceil() as usize;
    let idx = idx.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

/// Run the benchmark across multiple memory counts.
///
/// Each count:
/// 1. Creates a fresh temp store
/// 2. Inserts `count` synthetic memories (measures throughput)
/// 3. Runs 50 recall queries (measures latency + p95)
/// 4. Measures DB + index file sizes
/// 5. Cleans up
pub fn run_bench(json_output: bool, counts: Vec<usize>) -> Result<(), String> {
    // Cap to prevent accidental runaway (max 100K memories per count).
    const MAX_COUNT: usize = 100_000;
    let counts: Vec<usize> = counts.into_iter().map(|c| c.min(MAX_COUNT)).collect();

    let mut results: Vec<BenchResult> = Vec::with_capacity(counts.len());

    for &count in &counts {
        let result = bench_single_count(count)?;
        results.push(result);
    }

    if json_output {
        // Print JSON array of all results
        let json =
            serde_json::to_string_pretty(&results).map_err(|e| format!("JSON encode: {e}"))?;
        println!("{json}");
    } else {
        print_human(&results);
    }

    Ok(())
}

/// Benchmark a single memory count.
fn bench_single_count(count: usize) -> Result<BenchResult, String> {
    let dir = make_temp_dir(&format!("n{count}"))?;

    // Ensure cleanup even on error (RAII guard).
    let _cleanup = TempDirGuard(dir.clone());

    // Scope the store so it's dropped (flushed) before measuring file sizes.
    let result = {
        let uteke = Uteke::open(&dir).map_err(|e| format!("Failed to open store: {e}"))?;

        let mut rng = Rng::new(count as u64);

        // --- Insert benchmark ---
        let insert_start = Instant::now();
        for _ in 0..count {
            let content = gen_content(&mut rng);
            let tag_idx = (rng.next_u64() as usize) % TAG_POOL.len();
            let tag_refs: [&str; 1] = [TAG_POOL[tag_idx]];
            uteke
                .remember(&content, &tag_refs, None, Some("bench"))
                .map_err(|e| format!("Insert failed: {e}"))?;
        }
        let insert_elapsed = insert_start.elapsed();
        let insert_total_ms = insert_elapsed.as_secs_f64() * 1000.0;
        let insert_ops_per_sec = if insert_total_ms > 0.0 {
            (count as f64) / (insert_total_ms / 1000.0)
        } else {
            0.0
        };

        // --- Recall benchmark (50 queries) ---
        let num_queries = 50usize;
        let mut latencies: Vec<f64> = Vec::with_capacity(num_queries);
        for _ in 0..num_queries {
            let query = gen_query(&mut rng);
            let q_start = Instant::now();
            uteke
                .recall(&query, 5, None, Some("bench"), 0.0)
                .map_err(|e| format!("Recall failed: {e}"))?;
            latencies.push(q_start.elapsed().as_secs_f64() * 1000.0);
        }
        latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let recall_avg_ms = latencies.iter().sum::<f64>() / (latencies.len() as f64);
        let recall_p95_ms = percentile(&latencies, 0.95);

        // Flush index to disk before measuring file sizes
        let _ = uteke.shutdown();

        let db_path = dir.join("uteke.db");
        let index_path = dir.join("uteke_index.usearch");
        let keys_path = dir.join("uteke_index.keys");

        let db_size_kb = file_size_kb(&db_path);
        let index_size_kb = file_size_kb(&index_path) + file_size_kb(&keys_path);

        BenchResult {
            count,
            insert_ops_per_sec,
            insert_total_ms,
            recall_avg_ms,
            recall_p95_ms,
            db_size_kb,
            index_size_kb,
        }
    };

    // Guard drops here and removes the temp dir.
    drop(_cleanup);
    Ok(result)
}

/// RAII guard that removes the temp dir on drop. Holds an owned PathBuf
/// so the guard uniquely owns its path and cannot accidentally remove a
/// directory that was reused elsewhere.
struct TempDirGuard(std::path::PathBuf);

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Print human-readable benchmark table.
fn print_human(results: &[BenchResult]) {
    println!("uteke bench");
    println!("─────────────────────────────────────────────────────");

    for (i, r) in results.iter().enumerate() {
        if i > 0 {
            println!();
        }
        println!("  Count: {}", r.count);
        println!(
            "  Insert: {:.0} ops/sec ({:.0}ms total)",
            r.insert_ops_per_sec, r.insert_total_ms
        );
        println!(
            "  Recall avg: {:.1}ms, p95: {:.1}ms",
            r.recall_avg_ms, r.recall_p95_ms
        );
        println!(
            "  DB size: {:.0} KB, Index: {:.0} KB",
            r.db_size_kb, r.index_size_kb
        );
    }
}
