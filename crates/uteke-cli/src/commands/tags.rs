//! Tags subcommands — list, rename, delete.

use crate::cli::Cli;
use crate::cli::TagCommands;
use crate::output;
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    command: &TagCommands,
) -> Result<(), String> {
    match command {
        TagCommands::List { by_count } => {
            tracing::info!("Listing tags (by_count: {by_count})");
            let mut tags = uteke
                .tags_with_counts(ns)
                .map_err(|e| format!("Failed to list tags: {e}"))?;
            if *by_count {
                tags.sort_by_key(|b| std::cmp::Reverse(b.count));
            } else {
                tags.sort_by(|a, b| a.name.cmp(&b.name));
            }
            if cli.json {
                output::print_json(&tags);
            } else {
                output::print_tags_human(&tags, *by_count);
            }
        }
        TagCommands::Rename { old, new } => {
            tracing::info!("Renaming tag: {old} -> {new}");
            let count = uteke
                .rename_tag(old, new, ns)
                .map_err(|e| format!("Failed to rename tag: {e}"))?;
            if cli.json {
                output::print_json(
                    &serde_json::json!({"renamed": count, "tag": old, "new_tag": new}),
                );
            } else {
                println!("\u{2713} Tag '{old}' renamed to '{new}' ({count} memories updated)");
            }
        }
        TagCommands::Delete { tag, confirm } => {
            if !confirm {
                // Check if tag exists and show count
                let tags = uteke
                    .tags_with_counts(ns)
                    .map_err(|e| format!("Failed to list tags: {e}"))?;
                let info = tags.iter().find(|t| t.name == *tag);
                match info {
                    Some(info) => {
                        println!("Tag '{}' is used by {} memory(ies).", info.name, info.count);
                        println!("Use --confirm to proceed with deletion.");
                        return Err("Tag deletion not confirmed. Use --confirm flag.".to_string());
                    }
                    None => {
                        return Err(format!("Tag '{tag}' not found."));
                    }
                }
            }
            tracing::info!("Deleting tag: {tag}");
            let count = uteke
                .delete_tag(tag, ns)
                .map_err(|e| format!("Failed to delete tag: {e}"))?;
            if cli.json {
                output::print_json(&serde_json::json!({"deleted": count, "tag": tag}));
            } else {
                println!("\u{2713} Tag '{tag}' deleted ({count} memories updated)");
            }
        }
    }
    Ok(())
}
