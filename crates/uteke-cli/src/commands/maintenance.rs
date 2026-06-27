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
            ))
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

    let extractor =
        crate::extract::Extractor::new(&api_key, &model, &base_url, &cfg.endpoint_path, max_facts)
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
