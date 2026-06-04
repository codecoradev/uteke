//! Uteke memory benchmark — measures recall, search, list, and consolidate
//! performance at different scales (100, 1K, 5K memories).
//!
//! Usage: `cargo run --bin memory-bench`
//!
//! Requires the ONNX embedding model to be available (handled by ort's
//! download-binaries feature).

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Instant;
use uteke_core::Uteke;

const SIZES: &[usize] = &[100, 1_000, 5_000, 10_000];
const RECALL_QUERIES: &[&str] = &[
    "rust programming language",
    "machine learning model training",
    "database performance optimization",
    "web server deployment",
    "error handling patterns",
];

fn bench_dir() -> PathBuf {
    let dir = std::env::temp_dir().join("uteke-bench");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn generate_memories(count: usize) -> Vec<(String, Vec<String>)> {
    let categories = [
        "rust",
        "python",
        "database",
        "web",
        "devops",
        "ml",
        "security",
        "networking",
    ];
    let topics = [
        "programming patterns",
        "best practices",
        "common mistakes",
        "optimization tips",
        "debugging techniques",
        "testing strategies",
        "deployment guides",
        "configuration",
        "performance tuning",
        "error handling",
        "security hardening",
        "monitoring setup",
    ];

    (0..count)
        .map(|i| {
            let cat = categories[i % categories.len()];
            let topic = topics[(i / categories.len()) % topics.len()];
            let content = format!(
                "Memory #{i}: This is about {} and {}. The quick brown fox jumps over the lazy dog. \
                 Specific detail: benchmark iteration {} with unique identifier.",
                cat, topic, i
            );
            let tags = vec![
                cat.to_string(),
                topic.to_string(),
                format!("bench-{i}"),
            ];
            (content, tags)
        })
        .collect()
}

fn main() {
    println!("Uteke Memory Benchmark");
    println!("========================");
    println!();

    for &size in SIZES {
        println!("--- {} memories ---", size);
        let dir = bench_dir();
        let db_path = dir.join("bench.db");

        // Open
        let open_start = Instant::now();
        let uteke = match Uteke::open(&db_path) {
            Ok(u) => u,
            Err(e) => {
                eprintln!("Failed to open: {e}");
                continue;
            }
        };
        let open_ms = open_start.elapsed().as_millis();
        println!("  open:         {:>6} ms", open_ms);

        // Insert
        let memories = generate_memories(size);
        let insert_start = Instant::now();
        for (content, tags) in &memories {
            let tags_ref: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let _ = uteke.remember(content, &tags_ref, None, Some("bench"));
        }
        let insert_ms = insert_start.elapsed().as_millis();
        let insert_per = if insert_ms > 0 {
            size as f64 / insert_ms as f64 * 1000.0
        } else {
            f64::INFINITY
        };
        println!(
            "  insert ({}):   {:>6} ms ({:.0}/s)",
            size, insert_ms, insert_per
        );

        // Recall — Uteke::recall(query, limit, tags_filter, namespace)
        let recall_start = Instant::now();
        for query in RECALL_QUERIES {
            let _ = black_box(uteke.recall(query, 5, None, Some("bench")));
        }
        let recall_ms = recall_start.elapsed().as_millis();
        let recall_per = recall_ms as f64 / RECALL_QUERIES.len() as f64;
        println!(
            "  recall ({}q):  {:>6} ms (avg {:.1}ms/q)",
            RECALL_QUERIES.len(),
            recall_ms,
            recall_per
        );

        // Search — Uteke::search(query, limit, tags_filter, namespace)
        let search_start = Instant::now();
        for query in RECALL_QUERIES {
            let _ = black_box(uteke.search(query, 5, None, Some("bench")));
        }
        let search_ms = search_start.elapsed().as_millis();
        let search_per = search_ms as f64 / RECALL_QUERIES.len() as f64;
        println!(
            "  search ({}q):  {:>6} ms (avg {:.1}ms/q)",
            RECALL_QUERIES.len(),
            search_ms,
            search_per
        );

        // List — Uteke::list(tag, limit, offset, namespace)
        let list_start = Instant::now();
        let _ = black_box(uteke.list(None, size, 0, Some("bench")));
        let list_ms = list_start.elapsed().as_millis();
        println!("  list:         {:>6} ms", list_ms);

        // Consolidate — Uteke::consolidate(namespace, threshold, dry_run)
        let consolidate_start = Instant::now();
        let _ = black_box(uteke.consolidate(Some("bench"), 0.90, false));
        let consolidate_ms = consolidate_start.elapsed().as_millis();
        println!("  consolidate:  {:>6} ms", consolidate_ms);

        // Forget (delete all in namespace)
        let forget_start = Instant::now();
        let _ = black_box(uteke.bulk_forget_all(Some("bench")));
        let forget_ms = forget_start.elapsed().as_millis();
        println!("  forget_all:  {:>6} ms", forget_ms);

        // Cleanup
        let _ = std::fs::remove_dir_all(&dir);
        println!();
    }

    println!("Done.");
}
