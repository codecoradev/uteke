//! Forget command — delete memories (single, by tag, cold, all).

use std::io;

use crate::output;
use crate::Cli;
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    id: &Option<String>,
    tag: &Option<String>,
    cold: bool,
    all: bool,
    confirm: bool,
) -> Result<(), String> {
    if let Some(id) = id {
        // Single delete — confirm if no --confirm flag
        if !confirm {
            println!("About to delete memory: {id}");
            print!("Are you sure? [y/N] ");
            io::Write::flush(&mut io::stdout()).ok();
            let mut input = String::new();
            io::stdin().read_line(&mut input).ok();
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Cancelled.");
                return Ok(());
            }
        }
        tracing::info!("Forgetting memory: {id}");
        uteke
            .forget(id.as_str())
            .map_err(|e| format!("Failed to delete memory: {e}"))?;
        if cli.json {
            output::print_json(&serde_json::json!({"forgotten": id}));
        } else {
            println!("\u{2713} Memory forgotten: {id}");
        }
    } else if let Some(tag) = tag {
        // Bulk delete by tag
        tracing::info!("Bulk forgetting by tag: {tag}");
        let count = uteke.store().count_by_tag(tag.as_str(), ns).unwrap_or(0);
        if !confirm && count > 0 {
            println!("Found {count} memories with tag '{tag}'. Use --confirm to delete.");
            return Ok(());
        }
        let result = uteke
            .bulk_forget_by_tag(tag.as_str(), ns)
            .map_err(|e| format!("Failed: {e}"))?;
        if cli.json {
            output::print_json(&serde_json::json!({"deleted": result.deleted, "ids": result.ids}));
        } else {
            println!(
                "\u{2713} Deleted {} memories with tag '{}'",
                result.deleted, tag
            );
        }
    } else if cold {
        // Bulk delete cold memories
        tracing::info!("Bulk forgetting cold memories");
        if !confirm {
            println!("This will delete all cold memories (>30 days or never accessed).");
            println!("Use --confirm to proceed.");
            return Ok(());
        }
        let result = uteke
            .bulk_forget_cold(ns)
            .map_err(|e| format!("Failed: {e}"))?;
        if cli.json {
            output::print_json(&serde_json::json!({"deleted": result.deleted, "ids": result.ids}));
        } else {
            println!("\u{2713} Deleted {} cold memories", result.deleted);
        }
    } else if all {
        // Delete all in namespace
        tracing::info!("Bulk forgetting all memories");
        if !confirm {
            println!("\u{26a0}\u{fe0f}  This will delete ALL memories in the namespace.");
            println!("Use --confirm to proceed.");
            return Ok(());
        }
        let result = uteke
            .bulk_forget_all(ns)
            .map_err(|e| format!("Failed: {e}"))?;
        if cli.json {
            output::print_json(&serde_json::json!({"deleted": result.deleted, "ids": result.ids}));
        } else {
            println!("\u{2713} Deleted {} memories", result.deleted);
        }
    } else {
        return Err("Provide an ID, --tag, --cold, or --all".into());
    }
    Ok(())
}
