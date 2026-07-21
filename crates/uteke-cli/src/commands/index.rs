//! `uteke index` — walk a repo, chunk source files, store recallable memories.
//!
//! DB-per-repo model: the store is already scoped by the project's
//! `.uteke/uteke.toml` (resolved in config load). This command discovers the
//! project root and delegates the walk/chunk/prune to `uteke_core::index_tree`.

use std::io::Write;
use std::path::PathBuf;

use uteke_core::{IndexProgress, Uteke};

use crate::cli::Cli;

pub fn run(
    cli: &Cli,
    uteke: &mut Uteke,
    namespace: Option<&str>,
    path: &Option<String>,
    force: bool,
    dry_run: bool,
    status: bool,
) -> Result<(), String> {
    let ns = namespace.unwrap_or("default");

    // Status-only mode: report counts and return without walking the tree.
    if status {
        let (files, chunks) = uteke
            .code_index_status(ns)
            .map_err(|e| format!("index status failed: {e}"))?;
        if cli.json {
            let out = serde_json::json!({
                "namespace": ns,
                "files": files,
                "chunks": chunks,
            });
            println!("{}", serde_json::to_string_pretty(&out).unwrap());
        } else {
            println!("Code index (namespace {ns}): {files} file(s), {chunks} chunk(s)");
        }
        return Ok(());
    }

    // Resolve the root to index. Explicit path wins; otherwise project root;
    // otherwise cwd.
    let root: PathBuf = match path {
        Some(p) => PathBuf::from(p),
        None => crate::config::find_project_root()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "Cannot determine directory to index".to_string())?,
    };

    if !root.exists() {
        return Err(format!("Path does not exist: {}", root.display()));
    }

    // JSON mode: single summary, no live stream (keep output parseable).
    // Human mode: stream per-file progress — tree-sitter chunking is fast, but
    // embedding each chunk is the slow step, so per-file lines show real
    // motion instead of one long silent block.
    let summary = if cli.json {
        uteke
            .index_tree(ns, &root, force, dry_run)
            .map_err(|e| format!("index failed: {e}"))?
    } else {
        let stdout = std::io::stdout();
        uteke
            .index_tree_cb(ns, &root, force, dry_run, |ev| {
                let mut out = stdout.lock();
                match ev {
                    IndexProgress::Discovered { files } => {
                        if dry_run {
                            let _ = writeln!(out, "Scanning {files} source file(s) (dry run)…");
                        } else {
                            let _ = writeln!(
                                out,
                                "Discovered {files} source file(s). Chunking + embedding…"
                            );
                        }
                    }
                    IndexProgress::FileStarted { path, index, total } => {
                        // Carriage-return, no newline: overwritten by the
                        // FileIndexed/Skipped line so the stream stays tidy.
                        let _ = write!(out, "  [{index}/{total}] {path} …\r");
                        let _ = out.flush();
                    }
                    IndexProgress::FileIndexed { path, chunks } => {
                        let _ = writeln!(out, "  ✓ {path} — {chunks} chunk(s)        ");
                    }
                    IndexProgress::FileSkipped { path } => {
                        let _ = writeln!(out, "  · {path} — unchanged        ");
                    }
                    IndexProgress::Pruned { files } => {
                        if files > 0 {
                            let _ = writeln!(out, "  Pruned {files} deleted file(s)");
                        }
                    }
                }
            })
            .map_err(|e| format!("index failed: {e}"))?
    };

    if cli.json {
        let out = serde_json::json!({
            "namespace": ns,
            "root": root.display().to_string(),
            "indexed": summary.indexed,
            "skipped": summary.skipped,
            "chunks": summary.chunks,
            "pruned": summary.pruned,
            "dry_run": dry_run,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else if dry_run {
        println!(
            "Dry run: would index {} file(s) under {} (namespace: {ns})",
            summary.indexed,
            root.display()
        );
    } else {
        println!(
            "Indexed {} file(s), {} chunk(s); {} unchanged; {} pruned (namespace: {ns})",
            summary.indexed, summary.chunks, summary.skipped, summary.pruned
        );
    }

    Ok(())
}
