//! Namespace subcommands — list, stats, switch, move, rename, delete, migrate.

use crate::Config;
use crate::cli::Cli;
use crate::cli::NamespaceCommands;
use crate::output;
use std::path::{Path, PathBuf};
use uteke_core::Uteke;
use uteke_core::namespace_migrate::{NamespaceMigrationMap, NamespaceMigrationReport};

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    config: &Config,
    command: &NamespaceCommands,
) -> Result<(), String> {
    match command {
        NamespaceCommands::List => {
            tracing::info!("Listing namespaces");
            let namespaces = uteke
                .list_namespaces()
                .map_err(|e| format!("Failed to list namespaces: {e}"))?;
            if cli.json {
                output::print_json(&namespaces);
            } else if namespaces.is_empty() {
                println!("No namespaces found.");
            } else {
                println!("Namespaces ({} total):\n", namespaces.len());
                for ns_name in &namespaces {
                    let count = uteke.count(Some(ns_name.as_str())).unwrap_or(0);
                    println!("  {} ({} memories)", ns_name, count);
                }
            }
            Ok(())
        }
        NamespaceCommands::Stats { name } => {
            tracing::info!("Namespace stats: {name}");
            let stats = uteke
                .stats(Some(name.as_str()))
                .map_err(|e| format!("Failed to get namespace stats: {e}"))?;
            if cli.json {
                output::print_json(&stats);
            } else {
                println!("Namespace: {name}");
                output::print_stats_human(&stats);
            }
            Ok(())
        }
        NamespaceCommands::Switch { name } => {
            tracing::info!("Switching default namespace to: {name}");
            Config::set_default_namespace(name)
                .map_err(|e| format!("Failed to switch namespace: {e}"))?;
            if cli.json {
                output::print_json(&serde_json::json!({"default_namespace": name}));
            } else {
                println!("\u{2713} Default namespace set to '{name}'");
            }
            Ok(())
        }
        NamespaceCommands::Move { id, namespace } => {
            tracing::info!("Moving memory {id} to namespace '{namespace}'");
            let moved = uteke
                .move_memory(id, namespace)
                .map_err(|e| format!("Failed to move memory: {e}"))?;
            if !moved {
                return Err(format!("Memory not found: {id}"));
            }
            if cli.json {
                output::print_json(&serde_json::json!({
                    "moved": true,
                    "id": id,
                    "namespace": namespace,
                }));
            } else {
                println!("\u{2713} Moved memory {id} to namespace '{namespace}'");
            }
            Ok(())
        }
        NamespaceCommands::Rename { from, to } => {
            tracing::info!("Renaming namespace '{from}' to '{to}'");
            let result = uteke
                .rename_namespace(from, to)
                .map_err(|e| format!("Failed to rename namespace: {e}"))?;
            if cli.json {
                output::print_json(&result);
            } else {
                let kind = if result.target_existed {
                    "merged into existing"
                } else {
                    "renamed to"
                };
                println!(
                    "\u{2713} Namespace '{from}' {kind} '{to}' — {} memories moved",
                    result.moved
                );
            }
            Ok(())
        }
        NamespaceCommands::Delete {
            name,
            strategy,
            target,
            confirm,
        } => {
            if !confirm {
                return Err(
                    "Refusing to delete a namespace without --confirm (this affects its memories)"
                        .to_string(),
                );
            }
            tracing::info!("Deleting namespace '{name}' (strategy={strategy})");
            let result = uteke
                .delete_namespace(name, strategy, target.as_deref())
                .map_err(|e| format!("Failed to delete namespace: {e}"))?;
            if cli.json {
                output::print_json(&result);
            } else {
                match result.strategy.as_str() {
                    "merge" => println!(
                        "\u{2713} Moved {} memories from '{}' to '{}' — namespace removed",
                        result.affected,
                        result.name,
                        result.target.as_deref().unwrap_or("?")
                    ),
                    "deprecate" => println!(
                        "\u{2713} Soft-deleted {} memories in '{}' (restorable via promote; the name stays visible as deprecated-only)",
                        result.affected, result.name
                    ),
                    _ => println!("\u{2713} Namespace '{}' deleted", result.name),
                }
            }
            Ok(())
        }
        NamespaceCommands::Migrate {
            map,
            apply,
            allow_drop,
        } => migrate(cli, uteke, config, map, *apply, *allow_drop),
    }
}

fn migrate(
    cli: &Cli,
    uteke: &Uteke,
    config: &Config,
    map_path: &Path,
    apply: bool,
    allow_drop: bool,
) -> Result<(), String> {
    let text = std::fs::read_to_string(map_path)
        .map_err(|e| format!("Failed to read map {}: {e}", map_path.display()))?;
    let map = NamespaceMigrationMap::parse(&text).map_err(|e| e.to_string())?;
    let report = if apply {
        let export = export_path(&crate::resolve_store_path(cli, config));
        uteke.apply_namespace_migration(&map, &export, allow_drop)
    } else {
        uteke.plan_namespace_migration(&map)
    }
    .map_err(|e| format!("Namespace migrate failed: {e}"))?;
    if cli.json {
        output::print_json(&report);
    } else {
        print_migration(&report, allow_drop);
    }
    Ok(())
}

/// `<store dir>/namespace-migrate-<UTC timestamp>.ndjson`, next to the database.
fn export_path(store: &str) -> PathBuf {
    let store = Path::new(store);
    let dir = if store.extension().is_some() && !store.is_dir() {
        store.parent().unwrap_or(Path::new("."))
    } else {
        store
    };
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
    dir.join(format!("namespace-migrate-{stamp}.ndjson"))
}

fn print_migration(report: &NamespaceMigrationReport, allow_drop: bool) {
    let verb = if report.applied {
        "Migrated"
    } else {
        "Dry run"
    };
    println!("{verb}: {} mapped namespace(s)\n", report.namespaces.len());
    for e in &report.namespaces {
        if !e.found {
            println!(
                "  {}: no live memories (unknown or already migrated), skipped",
                e.namespace
            );
            continue;
        }
        let t = &e.target;
        let mut parts = Vec::new();
        if t.drop {
            parts.push("drop (soft-delete)".to_string());
        }
        if let Some(r) = &t.room {
            let new = if e.room_created { ", new room" } else { "" };
            parts.push(format!("room {r}{new}"));
        }
        if let Some(x) = &t.tag {
            parts.push(format!("tag {x}"));
        }
        if let Some(x) = &t.author {
            parts.push(format!("author {x}"));
        }
        if let Some(x) = &t.source {
            parts.push(format!("source {x}"));
        }
        let dest = if t.drop { "" } else { " -> default" };
        println!(
            "  {}: {} memories{dest}, {}",
            e.namespace,
            e.memories,
            parts.join(", ")
        );
        if e.author_conflicts > 0 {
            println!("    {} keep their own author", e.author_conflicts);
        }
        if e.source_conflicts > 0 {
            println!("    {} keep their own source", e.source_conflicts);
        }
        if e.soft_deleted > 0 {
            println!(
                "    {} soft-deleted memories stay in the namespace",
                e.soft_deleted
            );
        }
        if t.drop && !allow_drop && !report.applied {
            println!("    needs --allow-drop");
        }
    }
    for u in &report.unmapped {
        println!(
            "  {}: {} memories, not in the map, left alone",
            u.namespace, u.memories
        );
    }
    match (&report.export_path, report.applied) {
        (Some(p), _) => {
            println!(
                "\nExport written first: {p} (restore it with `uteke import --full` into a fresh store)"
            )
        }
        (None, true) => println!("\nNothing to change."),
        (None, false) => {
            println!("\nNothing was written. Run with --apply (on a copy first) to migrate.")
        }
    }
}
