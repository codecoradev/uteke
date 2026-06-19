//! `uteke orphans` — find disconnected memories (#351).

use crate::cli::Cli;
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    threshold: Option<f64>,
    limit: usize,
) -> Result<(), String> {
    let threshold = threshold.unwrap_or(uteke_core::DEFAULT_ORPHAN_THRESHOLD);
    let orphans = uteke
        .find_orphans(ns, threshold, limit)
        .map_err(|e| format!("Failed to find orphans: {e}"))?;

    if cli.json {
        println!("{}", serde_json::to_string(&orphans).unwrap());
        return Ok(());
    }

    if orphans.is_empty() {
        println!("No orphan memories found (threshold {threshold}).");
        return Ok(());
    }

    println!(
        "{} orphan memor{} (threshold {threshold}):",
        orphans.len(),
        if orphans.len() == 1 { "y" } else { "ies" }
    );
    for o in &orphans {
        let preview: String = o.memory.content.chars().take(50).collect();
        let ellipsis = if o.memory.content.chars().count() > 50 {
            "…"
        } else {
            ""
        };
        println!(
            "  {}  score={:.2}  type={}  importance={:.2}  {preview}{ellipsis}",
            &o.memory.id[..8.min(o.memory.id.len())],
            o.orphan_score,
            o.memory.memory_type,
            o.memory.importance,
        );
    }
    Ok(())
}
