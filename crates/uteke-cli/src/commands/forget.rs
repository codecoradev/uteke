//! Forget command — delete memories (single, by tag, cold, all).

use std::io;

use crate::cli::Cli;
use crate::output;
use uteke_core::Uteke;

/// Flags for the forget command.
pub(crate) struct Flags {
    pub cold: bool,
    pub all: bool,
    pub confirm: bool,
    /// `wrong[: note]` also records a rejected-value tombstone (#1597).
    pub reason: Option<String>,
}

/// The y/N prompt of a single delete, skipped with `--confirm`. `true` = go on.
pub(crate) fn confirm_single(id: &str, confirmed: bool) -> bool {
    if confirmed {
        return true;
    }
    println!("About to delete memory: {id}");
    print!("Are you sure? [y/N] ");
    io::Write::flush(&mut io::stdout()).ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().eq_ignore_ascii_case("y")
}

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    id: &Option<String>,
    tag: &Option<String>,
    flags: &Flags,
) -> Result<(), String> {
    if let Some(id) = id {
        // The same resolution as `DELETE /forget` and MCP (#1439): a full id
        // or an unambiguous prefix, and a memory that is not stored is an
        // error rather than a silent "forgotten" (#762).
        let id = &uteke_core::resolve_memory_id(uteke, id).map_err(|e| e.message().to_string())?;
        // Single delete — confirm if no --confirm flag
        if !confirm_single(id, flags.confirm) {
            println!("Cancelled.");
            return Ok(());
        }
        tracing::info!("Forgetting memory: {id}");
        uteke
            .forget_with_reason(id.as_str(), flags.reason.as_deref())
            .map_err(|e| format!("Failed to delete memory: {e}"))?;
        let rejected = flags
            .reason
            .as_deref()
            .is_some_and(uteke_core::tombstone::is_wrong_reason);
        if cli.json {
            output::print_json(&serde_json::json!({"forgotten": id, "rejected": rejected}));
        } else if rejected {
            println!(
                "\u{2713} Memory forgotten: {id}\n  Rejected as wrong: this value cannot be stored again (`uteke tombstone list`)."
            );
        } else {
            println!("\u{2713} Memory forgotten: {id}");
        }
    } else if let Some(tag) = tag {
        // Bulk delete by tag
        tracing::info!("Bulk forgetting by tag: {tag}");
        let count = uteke.count_by_tag(tag.as_str(), ns).unwrap_or(0);
        if !flags.confirm && count > 0 {
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
    } else if flags.cold {
        // Bulk delete cold memories
        tracing::info!("Bulk forgetting cold memories");
        if !flags.confirm {
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
    } else if flags.all {
        // Delete all in namespace
        tracing::info!("Bulk forgetting all memories");
        if !flags.confirm {
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

/// `uteke tombstone list|remove` (#1597).
pub(crate) fn run_tombstone(
    cli: &Cli,
    uteke: &Uteke,
    command: &crate::cli::TombstoneCommands,
) -> Result<(), String> {
    use crate::cli::TombstoneCommands;
    match command {
        TombstoneCommands::List => {
            let list = uteke
                .tombstones()
                .map_err(|e| format!("Failed to list tombstones: {e}"))?;
            if cli.json {
                output::print_json(&list);
            } else if list.is_empty() {
                println!("No rejected values.");
            } else {
                println!("{} rejected value(s):", list.len());
                for t in &list {
                    println!(
                        "  {}  {}  {}\n      {}",
                        &t.hash[..12],
                        t.created_at.get(..10).unwrap_or(&t.created_at),
                        t.reason,
                        t.preview
                    );
                }
            }
        }
        TombstoneCommands::Remove { key } => {
            let removed = uteke
                .remove_tombstone(key)
                .map_err(|e| format!("Failed to remove tombstone: {e}"))?;
            if cli.json {
                output::print_json(&serde_json::json!({"removed": removed}));
            } else if removed == 0 {
                return Err(format!("No tombstone matches '{key}'"));
            } else {
                println!("\u{2713} Tombstone removed: the value can be stored again");
            }
        }
    }
    Ok(())
}
