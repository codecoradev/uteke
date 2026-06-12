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

pub(crate) fn run_import(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    input: &str,
) -> Result<(), String> {
    tracing::info!("Importing memories from {input}");
    let jsonl = if input == "-" {
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("Failed to read stdin: {e}"))?;
        buf
    } else {
        std::fs::read_to_string(input).map_err(|e| format!("Failed to read import file: {e}"))?
    };

    let result = uteke
        .import(&jsonl, ns)
        .map_err(|e| format!("Failed to import: {e}"))?;

    if cli.json {
        output::print_json(&result);
    } else {
        println!(
            "\u{2713} Imported {} memories ({} skipped)",
            result.imported, result.skipped
        );
    }
    Ok(())
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
