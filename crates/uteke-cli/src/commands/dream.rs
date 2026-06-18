//! `uteke dream` — coordinated maintenance pipeline (#353).

use crate::cli::Cli;
use uteke_core::{DreamPhase, Uteke};

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    phases: &[String],
    skip: &[String],
    dry_run: bool,
    quiet: bool,
) -> Result<(), String> {
    // Parse --phases into DreamPhase values. Empty = all.
    let mut selected: Vec<DreamPhase> = phases
        .iter()
        .filter_map(|s| DreamPhase::from_str_opt(s))
        .collect();
    if selected.is_empty() {
        selected = DreamPhase::all_in_order().to_vec();
    }
    // Apply --skip filter.
    let skip_set: std::collections::HashSet<String> = skip.iter().cloned().collect();
    selected.retain(|p| !skip_set.contains(p.as_str()));

    let report = uteke
        .dream(ns, dry_run, &selected)
        .map_err(|e| format!("Dream cycle failed: {e}"))?;

    if cli.json {
        println!("{}", serde_json::to_string(&report).unwrap());
        return Ok(());
    }

    if quiet {
        // Only print warnings/errors.
        for r in &report.phases {
            if r.status != uteke_core::PhaseStatus::Ok {
                println!("{}", r.summary);
            }
        }
    } else {
        let mode = if dry_run { " (dry-run)" } else { "" };
        println!("═══ uteke dream{mode} ═══");
        for r in &report.phases {
            println!("\nPhase: {}", r.phase);
            println!("  {}", r.summary);
        }
        println!("\n───────────────────────────────────────────────────");
        println!(
            "Total: {} changes | {} warnings | {} errors | {} ms",
            report.total_changes, report.total_warnings, report.total_errors, report.duration_ms
        );
    }

    // Exit non-zero on errors so cron / monitoring can detect failures.
    if report.total_errors > 0 {
        return Err(format!(
            "Dream cycle completed with {} error(s)",
            report.total_errors
        ));
    }
    Ok(())
}
