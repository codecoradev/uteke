//! Maintenance commands — doctor, verify, repair, prune, consolidate, export, import, checksums.

use std::io::Read;

use crate::cli::Cli;
use crate::output;
use uteke_core::Uteke;

pub(crate) fn run_doctor(cli: &Cli, uteke: &Uteke) -> Result<(), String> {
    tracing::info!("Running doctor");
    let report = uteke.doctor().map_err(|e| format!("Doctor failed: {e}"))?;
    if cli.json {
        output::print_json(&report);
    } else {
        output::print_doctor_human(&report);
    }
    Ok(())
}

pub(crate) fn run_verify(cli: &Cli, uteke: &Uteke) -> Result<(), String> {
    tracing::info!("Running verify");
    let report = uteke.verify().map_err(|e| format!("Verify failed: {e}"))?;
    if cli.json {
        output::print_json(&report);
    } else {
        output::print_verify_human(&report);
    }
    Ok(())
}

pub(crate) fn run_repair(cli: &Cli, uteke: &Uteke) -> Result<(), String> {
    tracing::info!("Running repair");
    let report = uteke.repair().map_err(|e| format!("Repair failed: {e}"))?;
    if cli.json {
        output::print_json(&report);
    } else {
        output::print_repair_human(&report);
    }
    Ok(())
}

pub(crate) fn run_prune(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    ttl: u32,
    dry_run: bool,
) -> Result<(), String> {
    tracing::info!("Pruning with TTL={ttl}d (dry_run={dry_run})");
    let result = uteke
        .prune(ttl, ns, dry_run)
        .map_err(|e| format!("Failed to prune: {e}"))?;
    if cli.json {
        output::print_json(&result);
    } else if result.deprecated_ids.is_empty() && result.pruned == 0 {
        println!("No deprecated memories to prune.");
    } else if dry_run {
        println!(
            "Dry run — {} deprecated memories would be pruned (TTL: {ttl}d):",
            result.deprecated
        );
        for id in &result.deprecated_ids {
            println!("  {}", id);
        }
    } else {
        println!(
            "\u{2713} Pruned {} deprecated memories (TTL: {ttl}d)",
            result.pruned
        );
    }
    Ok(())
}

pub(crate) fn run_consolidate(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    threshold: f32,
    dry_run: bool,
) -> Result<(), String> {
    tracing::info!("Consolidating (threshold: {threshold}, dry_run: {dry_run})");
    if dry_run {
        let pairs = uteke
            .find_duplicates(ns, threshold)
            .map_err(|e| format!("Failed to find duplicates: {e}"))?;
        if cli.json {
            output::print_json(&pairs);
        } else if pairs.is_empty() {
            println!("No duplicate pairs found (threshold: {threshold}).");
        } else {
            println!("Found {} potential duplicate(s):\n", pairs.len());
            for (i, p) in pairs.iter().enumerate() {
                println!("  {}. sim={:.3}", i + 1, p.similarity);
                println!("     A: {}", p.content_a);
                println!("     B: {}", p.content_b);
            }
        }
    } else {
        let result = uteke
            .consolidate(ns, threshold, false)
            .map_err(|e| format!("Failed to consolidate: {e}"))?;
        if cli.json {
            output::print_json(&result);
        } else {
            println!("\u{2713} Consolidation complete:");
            println!("  Duplicates found: {}", result.duplicates_found);
            println!("  Merged: {}", result.merged);
            if !result.removed_ids.is_empty() {
                println!("  Removed:");
                for id in &result.removed_ids {
                    println!("    {}", id);
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn run_export(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    output: &str,
) -> Result<(), String> {
    tracing::info!("Exporting memories to {output}");
    let jsonl = uteke
        .export(ns)
        .map_err(|e| format!("Failed to export: {e}"))?;

    if output == "-" {
        println!("{jsonl}");
    } else {
        std::fs::write(output, &jsonl).map_err(|e| format!("Failed to write export file: {e}"))?;
        let count = jsonl.lines().filter(|l| !l.trim().is_empty()).count();
        if cli.json {
            output::print_json(&serde_json::json!({"exported": count}));
        } else {
            println!("\u{2713} Exported {count} memories");
        }
    }
    Ok(())
}

/// CLI-level overrides for LLM extraction during import, plus a borrow of the
/// resolved `[extraction]` config.
///
/// Populated from `--extract*` flags. When `enabled` is false the importer
/// behaves exactly as before (no network, offline-first).
pub(crate) struct ExtractOpts<'a> {
    pub enabled: bool,
    pub model: Option<String>,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub max_facts: Option<usize>,
    pub cfg: &'a crate::config::ExtractionConfig,
}

pub(crate) fn run_import(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    input: &str,
    tags: &[String],
    format: &str,
    extract_opts: ExtractOpts<'_>,
) -> Result<(), String> {
    tracing::info!("Importing memories from {input} (format: {format})");

    let content = if input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read stdin: {e}"))?;
        buf
    } else {
        std::fs::read_to_string(input).map_err(|e| format!("Failed to read file: {e}"))?
    };

    // LLM extraction path (opt-in). Distill the raw input into atomic facts,
    // then store each fact as its own memory. Bypasses format detection because
    // the model handles arbitrary noisy text (transcripts, dumps, notes).
    if extract_opts.enabled {
        let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
        let result = import_with_extraction(uteke, &content, &tag_refs, ns, &extract_opts)?;
        if cli.json {
            output::print_json(&result);
        } else {
            println!(
                "\u{2713} Extracted and imported {} facts ({} skipped)",
                result.imported, result.skipped
            );
        }
        return Ok(());
    }

    let detected_format = if format == "auto" {
        detect_format(input, &content)
    } else {
        format.to_string()
    };

    let result = match detected_format.as_str() {
        "jsonl" => uteke
            .import(&content, ns)
            .map_err(|e| format!("Failed to import JSONL: {e}"))?,
        "markdown" | "text" => {
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            import_text(uteke, &content, &tag_refs, ns)?
        }
        other => {
            return Err(format!(
                "Unknown import format: '{other}'. Use: jsonl, markdown, text"
            ));
        }
    };

    if cli.json {
        output::print_json(&result);
    } else {
        println!(
            "\u{2713} Imported {} memories ({} skipped) as {detected_format}",
            result.imported, result.skipped
        );
    }
    Ok(())
}

/// Resolve extraction settings (CLI flag > env-merged config > built-in
/// default), call the LLM to distill facts, and store each fact as a memory.
///
/// API-key resolution falls back to the embedding/OpenAI key so users who
/// already configured an OpenAI-compatible setup for embeddings don't have to
/// duplicate the credential.
fn import_with_extraction(
    uteke: &Uteke,
    content: &str,
    tags: &[&str],
    ns: Option<&str>,
    opts: &ExtractOpts<'_>,
) -> Result<uteke_core::ImportResult, String> {
    use uteke_core::ImportResult;

    let cfg = opts.cfg;
    let model = opts
        .model
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.model.clone());
    let base_url = opts
        .base_url
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.base_url.clone());
    let api_key = opts
        .api_key
        .clone()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.api_key.clone());
    let max_facts = opts.max_facts.unwrap_or(cfg.max_facts);

    let ext_config = uteke_core::extraction::ExtractionConfig {
        model,
        api_key,
        base_url,
        endpoint_path: cfg.endpoint_path.clone(),
        max_facts,
    };

    let extractor = uteke_core::extraction::Extractor::new(&ext_config)
        .map_err(|e| format!("Failed to initialize extractor: {e}"))?;

    let facts = extractor
        .extract(content)
        .map_err(|e| format!("Extraction failed: {e}"))?;

    if facts.is_empty() {
        tracing::warn!("Extraction produced no facts from input");
        return Ok(ImportResult {
            imported: 0,
            skipped: 0,
        });
    }

    let mut imported = 0usize;
    let mut skipped = 0usize;
    for fact in &facts {
        match uteke.remember(fact, tags, None, ns) {
            Ok(_) => imported += 1,
            Err(e) => {
                tracing::warn!("Failed to store extracted fact: {e}");
                skipped += 1;
            }
        }
    }

    Ok(ImportResult { imported, skipped })
}

/// Detect import format from file extension and content.
fn detect_format(filename: &str, content: &str) -> String {
    // Check file extension
    let ext = filename.rsplit('.').next().unwrap_or("");
    match ext {
        "jsonl" => "jsonl".to_string(),
        "md" | "markdown" => "markdown".to_string(),
        "txt" => "text".to_string(),
        "csv" => "text".to_string(), // treat CSV as text for now
        _ => {
            // Auto-detect from content
            let first_line = content.lines().next().unwrap_or("");
            if first_line.starts_with('{') {
                "jsonl".to_string()
            } else {
                "text".to_string()
            }
        }
    }
}

/// Import text/markdown content as memories.
/// Splits by double newline (paragraphs) or markdown headings.
fn import_text(
    uteke: &Uteke,
    content: &str,
    tags: &[&str],
    ns: Option<&str>,
) -> Result<uteke_core::ImportResult, String> {
    use uteke_core::ImportResult;

    let chunks: Vec<String> = if content.contains("\n# ") || content.contains("\n## ") {
        // Markdown with headings — split by headings
        split_markdown(content)
    } else {
        // Plain text — split by double newline (paragraphs)
        content
            .split("\n\n")
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty() && s.len() > 10) // skip very short chunks
            .collect()
    };

    let mut imported = 0usize;
    let mut skipped = 0usize;

    for chunk in &chunks {
        match uteke.remember(chunk, tags, None, ns) {
            Ok(_) => imported += 1,
            Err(e) => {
                tracing::warn!("Failed to import chunk: {e}");
                skipped += 1;
            }
        }
    }

    Ok(ImportResult { imported, skipped })
}

/// Split markdown by headings. Each heading + body becomes a chunk.
fn split_markdown(content: &str) -> Vec<String> {
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();

    for line in content.lines() {
        let is_heading =
            line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ");
        if is_heading {
            // New section — save previous
            if !current.trim().is_empty() {
                chunks.push(current.trim().to_string());
            }
            current = line.to_string();
            current.push('\n'); // separator after heading
        } else {
            if !current.is_empty() {
                current.push('\n');
            }
            current.push_str(line);
        }
    }
    if !current.trim().is_empty() {
        chunks.push(current.trim().to_string());
    }

    // Filter very short chunks
    chunks.into_iter().filter(|c| c.len() > 10).collect()
}

/// Batch import command — processes all files in a directory.
///
/// Two strategies:
/// - **Document** (.md): full content → auto-chunk → embed. No LLM.
/// - **MemoryExtract** (.txt/.jsonl, or .md with --extract): LLM extraction → atomic facts → embed.
///
/// Default: sequential. Parallel extraction via --extract-parallel N.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_import_batch(
    cli: &Cli,
    uteke: &Uteke,
    dir: &str,
    ns: Option<&str>,
    tags: &[String],
    extract_opts: ExtractOpts<'_>,
    force_strategy: Option<ImportStrategy>,
    recursive: bool,
    dry_run: bool,
    max_size: usize,
) -> Result<(), String> {
    let dir_path = match std::path::Path::new(dir).canonicalize() {
        Ok(p) => p,
        Err(e) => return Err(format!("Cannot resolve path '{}': {}", dir, e)),
    };
    let start = std::time::Instant::now();

    // Discover files
    let files = discover_files(&dir_path, max_size, recursive)?;
    if files.is_empty() {
        println!("No importable files found in '{}'.", dir);
        return Ok(());
    }

    // Classify files by strategy
    let mut doc_files = Vec::new();
    let mut memory_files = Vec::new();

    for path in &files {
        let strategy = determine_strategy(path, force_strategy.clone(), extract_opts.enabled);
        match strategy {
            ImportStrategy::Document => doc_files.push(path.clone()),
            ImportStrategy::MemoryExtract => memory_files.push(path.clone()),
        }
    }

    if dry_run {
        if cli.json {
            output::print_json(&serde_json::json!({
                "dir": dir,
                "total_files": files.len(),
                "doc_files": doc_files.len(),
                "memory_files": memory_files.len(),
                "files": files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
                "strategies": files.iter().map(|f| {
                    let s = determine_strategy(f, force_strategy.clone(), extract_opts.enabled);
                    (f.display().to_string(), format!("{:?}", s))
                }).collect::<std::collections::HashMap<_, _>>()
            }));
        } else {
            println!(
                "Dry run — would import {} files from '{}':",
                files.len(),
                dir
            );
            println!("  Documents (no LLM):  {}", doc_files.len());
            println!("  Memory extract (LLM): {}", memory_files.len());
            println!();
            for f in &files {
                let s = determine_strategy(f, force_strategy.clone(), extract_opts.enabled);
                println!("  [{:?}] {}", s, f.display());
            }
        }
        return Ok(());
    }

    let mut result = BatchResult {
        files: files.len(),
        total_items: 0,
        imported: 0,
        skipped_files: 0,
        skipped_facts: 0,
        errors: 0,
        doc_files: doc_files.len(),
        memory_files: memory_files.len(),
        elapsed_ms: 0,
    };

    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

    // ── Phase 1: Document imports (no LLM, sequential) ──
    for path in &doc_files {
        let slug = slug_from_path(&dir_path, path);
        let title = title_from_slug(&slug);
        match import_single_document(uteke, path, &slug, &title, &tag_refs, ns) {
            Ok(count) => {
                result.total_items += count;
                result.imported += 1;
                tracing::info!("Imported document '{}' — {} chunks", slug, count);
                if !cli.json {
                    println!("  ✓ [doc] {} — {} chunks", path.display(), count);
                }
            }
            Err(e) => {
                result.errors += 1;
                tracing::error!("Failed to import document '{}': {}", path.display(), e);
                if !cli.json {
                    println!("  ✗ [doc] {} — {}", path.display(), e);
                }
            }
        }
    }

    // ── Phase 2: Memory extraction imports (sequential for now; parallel is task #8-9) ──
    if !memory_files.is_empty() && !extract_opts.enabled {
        if !cli.json {
            println!();
            eprintln!(
                "Warning: {} file(s) require --extract for LLM fact extraction.",
                memory_files.len()
            );
            println!("Run with --extract to process them, or use --as-doc to import as documents.");
        }
        result.skipped_files = memory_files.len();
    } else if !memory_files.is_empty() {
        for (i, path) in memory_files.iter().enumerate() {
            // Bail after 5 consecutive errors with zero success
            if result.errors > 5 && result.imported == 0 {
                eprintln!("\nStopping: too many consecutive errors. Check your extraction config.");
                break;
            }

            let content = match std::fs::read_to_string(path) {
                Ok(c) => c,
                Err(e) => {
                    result.errors += 1;
                    if !cli.json {
                        println!("  ✗ [read] {} — {}", path.display(), e);
                    }
                    continue;
                }
            };

            let progress_suffix = if memory_files.len() > 1 {
                format!(" [{}/{}]", i + 1, memory_files.len())
            } else {
                String::new()
            };

            match import_with_extraction(uteke, &content, &tag_refs, ns, &extract_opts) {
                Ok(import_result) => {
                    result.total_items += import_result.imported;
                    result.imported += 1;
                    result.skipped_facts += import_result.skipped;
                    if !cli.json {
                        println!(
                            "  ✓ [memory]{} {} — {} facts ({} skipped)",
                            progress_suffix,
                            path.display(),
                            import_result.imported,
                            import_result.skipped
                        );
                    }
                }
                Err(e) => {
                    result.errors += 1;
                    if !cli.json {
                        println!("  ✗ [memory]{} {} — {}", progress_suffix, path.display(), e);
                    }
                }
            }
        }
    }

    result.elapsed_ms = start.elapsed().as_millis() as u64;

    // ── Summary ──
    if cli.json {
        output::print_json(&result);
    } else {
        println!();
        if result.errors > 0 {
            println!(
                "⚠ Batch import complete with {} error(s) in {}ms",
                result.errors, result.elapsed_ms
            );
        } else {
            println!("✓ Batch import complete in {}ms", result.elapsed_ms);
        }
        println!("  Files processed: {}/{}", result.imported, result.files);
        println!("  Total items: {}", result.total_items);
        let total_skipped = result.skipped_files + result.skipped_facts;
        if total_skipped > 0 {
            println!(
                "  Skipped: {} files, {} facts",
                result.skipped_files, result.skipped_facts
            );
        }
        if result.errors > 0 {
            println!("  Errors: {}", result.errors);
        }
    }

    Ok(())
}

/// Import a single file as a document (full content, auto-chunk, embed).
fn import_single_document(
    uteke: &Uteke,
    path: &std::path::Path,
    slug: &str,
    title: &str,
    tags: &[&str],
    ns: Option<&str>,
) -> Result<usize, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))?;

    if content.trim().is_empty() {
        return Ok(0);
    }

    let _doc_id = uteke
        .doc_upsert_with_parent(slug, title, &content, tags, ns, None)
        .map_err(|e| format!("Document upsert failed: {e}"))?;

    // Count approximate chunks (content / ~1500 chars per chunk)
    let chunk_count = (content.len() / 1500).max(1);
    Ok(chunk_count)
}

pub(crate) fn run_verify_checksums(
    cli: &Cli,
    checksums_file: &str,
    binary: &str,
) -> Result<(), String> {
    let checksums = std::fs::read_to_string(checksums_file)
        .map_err(|e| format!("Failed to read checksums file: {e}"))?;

    let binary_filename = std::path::Path::new(binary)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("binary");

    let expected_line = checksums.lines().find(|l| l.contains(binary_filename));

    match expected_line {
        Some(line) => {
            let expected_hash = line.split_whitespace().next().unwrap_or("");
            let output = std::process::Command::new("sha256sum")
                .arg(binary)
                .output()
                .map_err(|e| format!("Failed to run sha256sum: {e}"))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let actual_hash = stdout.split_whitespace().next().unwrap_or("");

            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "binary": binary_filename,
                        "expected": expected_hash,
                        "actual": actual_hash,
                        "match": expected_hash == actual_hash
                    })
                );
            } else if expected_hash == actual_hash {
                println!("OK Checksum verified for {}", binary_filename);
            } else {
                eprintln!("FAIL Checksum mismatch for {}", binary_filename);
                eprintln!("  Expected: {}", expected_hash);
                eprintln!("  Actual:   {}", actual_hash);
                return Err("Checksum verification failed".into());
            }
        }
        None => {
            return Err(format!(
                "Binary not found in checksums file: {}",
                binary_filename
            ));
        }
    }
    Ok(())
}

// ── Batch Import ──────────────────────────────────────────────────────────

/// Import strategy for each file in a batch.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ImportStrategy {
    /// Store full document content → auto-chunk → embed (no LLM).
    Document,
    /// Extract atomic facts via LLM → embed each fact.
    MemoryExtract,
}

/// Aggregate batch result.
#[derive(serde::Serialize)]
struct BatchResult {
    files: usize,
    total_items: usize,
    imported: usize,
    skipped_files: usize,
    skipped_facts: usize,
    errors: usize,
    doc_files: usize,
    memory_files: usize,
    elapsed_ms: u64,
}

/// Discover importable files from a directory (recursive).
///
/// Skips hidden files/directories, binary files, and files > max_size.
/// Returns sorted list for deterministic processing.
fn discover_files(
    dir: &std::path::Path,
    max_size: usize,
    recursive: bool,
) -> Result<Vec<std::path::PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()));
    }

    let supported_exts = ["md", "markdown", "txt", "jsonl"];
    let mut files = Vec::new();

    fn walk(
        path: &std::path::Path,
        supported_exts: &[&str],
        max_size: usize,
        recursive: bool,
        files: &mut Vec<std::path::PathBuf>,
    ) -> Result<(), String> {
        let entries = std::fs::read_dir(path)
            .map_err(|e| format!("Cannot read directory '{}': {}", path.display(), e))?;

        let mut entries: Vec<_> = entries
            .filter_map(|e| match e {
                Ok(entry) => Some(entry),
                Err(err) => {
                    tracing::warn!(dir = %path.display(), "Failed to read directory entry: {}", err);
                    None
                }
            })
            .collect();
        entries.sort_by_key(|e| e.file_name());

        for entry in entries {
            let path = entry.path();
            let file_name = entry.file_name();
            let name_str = file_name.to_string_lossy();

            // Skip hidden files/directories
            if name_str.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                if recursive {
                    walk(&path, supported_exts, max_size, recursive, files)?;
                }
                continue;
            }

            // Check extension
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !supported_exts.contains(&ext) {
                continue;
            }

            // Check file size
            match std::fs::metadata(&path) {
                Ok(metadata) if metadata.len() as usize > max_size => {
                    tracing::warn!(
                        "Skipping large file: {} ({} bytes)",
                        path.display(),
                        metadata.len()
                    );
                    continue;
                }
                Err(e) => {
                    tracing::warn!("Cannot stat file '{}': {}", path.display(), e);
                    continue;
                }
                _ => {}
            }

            files.push(path);
        }
        Ok(())
    }

    walk(dir, &supported_exts, max_size, recursive, &mut files)?;
    Ok(files)
}

/// Determine import strategy for a file based on its extension and flags.
///
/// - .md/.markdown files → Document strategy (full content, auto-chunk)
/// - .txt/.jsonl files → MemoryExtract strategy (LLM extraction)
/// - Overridden by --as-doc / --as-memory flags
fn determine_strategy(
    path: &std::path::Path,
    force_strategy: Option<ImportStrategy>,
    extract_enabled: bool,
) -> ImportStrategy {
    if let Some(s) = force_strategy {
        return s;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    match ext {
        "md" | "markdown" => {
            // .md goes to document (no LLM) unless --extract is explicitly set
            if extract_enabled {
                ImportStrategy::MemoryExtract
            } else {
                ImportStrategy::Document
            }
        }
        _ => ImportStrategy::MemoryExtract,
    }
}

/// Generate a slug from a file path (relative to base dir).
///
/// "skills/system-audit/SKILL.md" → "skills-system-audit-skill"
fn slug_from_path(base: &std::path::Path, file: &std::path::Path) -> String {
    let relative = file.strip_prefix(base).unwrap_or(file);
    let slug = relative.to_string_lossy().replace(['/', '\\'], "-");
    let slug = slug
        .trim_end_matches(".md")
        .trim_end_matches(".markdown")
        .trim_end_matches(".txt")
        .trim_end_matches(".jsonl")
        .trim_end_matches('-')
        .to_lowercase();
    slug
}

/// Generate a title from slug (human-readable).
fn title_from_slug(slug: &str) -> String {
    slug.replace('-', " ")
        .split_whitespace()
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_discover_files_finds_md_txt_jsonl() {
        let dir = tempfile::tempdir().unwrap();
        // Create test files
        std::fs::write(dir.path().join("readme.md"), "# Hello").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "Some notes").unwrap();
        std::fs::write(dir.path().join("data.jsonl"), "{\"key\":\"val\"}\n").unwrap();
        // Unsupported extension — should be skipped
        std::fs::write(dir.path().join("image.png"), b"\x89PNG").unwrap();
        // Hidden file — should be skipped
        std::fs::write(dir.path().join(".hidden.md"), "hidden").unwrap();

        let files = discover_files(dir.path(), 1_000_000, false).unwrap();
        assert_eq!(files.len(), 3);

        let names: Vec<_> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"readme.md"));
        assert!(names.contains(&"notes.txt"));
        assert!(names.contains(&"data.jsonl"));
    }

    #[test]
    fn test_discover_files_recursive() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.path().join("top.md"), "# Top").unwrap();
        std::fs::write(sub.join("nested.md"), "# Nested").unwrap();

        // Non-recursive: only top-level
        let files = discover_files(dir.path(), 1_000_000, false).unwrap();
        assert_eq!(files.len(), 1);

        // Recursive: includes subdir
        let files = discover_files(dir.path(), 1_000_000, true).unwrap();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_discover_files_max_size() {
        let dir = tempfile::tempdir().unwrap();
        let mut big = std::fs::File::create(dir.path().join("big.md")).unwrap();
        big.write_all(&vec![0u8; 2000]).unwrap();
        std::fs::write(dir.path().join("small.md"), "# Small").unwrap();

        let files = discover_files(dir.path(), 1000, false).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].file_name().unwrap().to_str().unwrap() == "small.md");
    }

    #[test]
    fn test_discover_files_not_directory() {
        let file = std::path::PathBuf::from("/tmp/nonexistent_dir_uteke_test");
        let result = discover_files(&file, 1_000_000, false);
        assert!(result.is_err());
    }

    #[test]
    fn test_determine_strategy_default() {
        // .md without extract → Document
        let md = std::path::PathBuf::from("test.md");
        assert_eq!(
            determine_strategy(&md, None, false),
            ImportStrategy::Document
        );

        // .txt → MemoryExtract (needs LLM)
        let txt = std::path::PathBuf::from("test.txt");
        assert_eq!(
            determine_strategy(&txt, None, false),
            ImportStrategy::MemoryExtract
        );

        // .jsonl → MemoryExtract
        let jsonl = std::path::PathBuf::from("test.jsonl");
        assert_eq!(
            determine_strategy(&jsonl, None, false),
            ImportStrategy::MemoryExtract
        );
    }

    #[test]
    fn test_determine_strategy_force() {
        let md = std::path::PathBuf::from("test.md");
        // Force as_doc
        assert_eq!(
            determine_strategy(&md, Some(ImportStrategy::Document), false),
            ImportStrategy::Document
        );
        // Force as_memory
        assert_eq!(
            determine_strategy(&md, Some(ImportStrategy::MemoryExtract), false),
            ImportStrategy::MemoryExtract
        );
    }

    #[test]
    fn test_determine_strategy_extract_flag() {
        // .txt with extract enabled should still be MemoryExtract
        let txt = std::path::PathBuf::from("test.txt");
        assert_eq!(
            determine_strategy(&txt, None, true),
            ImportStrategy::MemoryExtract
        );
    }

    #[test]
    fn test_slug_from_path() {
        let dir = std::path::Path::new("/data/docs");
        let file = std::path::PathBuf::from("/data/docs/subfolder/my-file.md");
        let slug = slug_from_path(dir, &file);
        // slug_from_path replaces / with - for flat slug
        assert_eq!(slug, "subfolder-my-file");
    }

    #[test]
    fn test_slug_from_path_txt() {
        let dir = std::path::Path::new("/data/docs");
        let file = std::path::PathBuf::from("/data/docs/notes.txt");
        let slug = slug_from_path(dir, &file);
        assert_eq!(slug, "notes");
    }

    #[test]
    fn test_title_from_slug() {
        assert_eq!(title_from_slug("my-file"), "My File");
        assert_eq!(title_from_slug("subfolder-my-file"), "Subfolder My File");
        assert_eq!(title_from_slug("hello-world"), "Hello World");
        assert_eq!(title_from_slug(""), "");
    }
}
