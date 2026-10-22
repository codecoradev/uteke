//! Lifecycle CLI commands (#935): cycle, promote, status.

use crate::cli::{Cli, LifecycleCommands};
use crate::output;
use uteke_core::Uteke;

/// Run lifecycle subcommands.
pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    command: &LifecycleCommands,
) -> Result<(), String> {
    match command {
        LifecycleCommands::Cycle { namespace } => {
            let target_ns = namespace.as_deref().or(ns);
            let result = uteke
                .lifecycle_cycle(target_ns)
                .map_err(|e| format!("{e}"))?;

            if cli.json {
                output::print_json(&serde_json::json!({
                    "total_active": result.total_active,
                    "candidates": result.candidates,
                    "cap": result.cap,
                    "deprecated": result.deprecated,
                    "deprecated_ids": result.deprecated_ids,
                    "pruned": result.pruned,
                    "pruned_ids": result.pruned_ids,
                }));
            } else {
                println!("Lifecycle cycle complete:");
                println!("  Active memories:      {}", result.total_active);
                println!("  Aged candidates:      {}", result.candidates);
                println!(
                    "  Cap applied:          {} (clamped to min/max)",
                    result.cap
                );
                println!("  Deprecated this run:  {}", result.deprecated);
                if !result.deprecated_ids.is_empty() {
                    println!("    IDs: {}", result.deprecated_ids.join(", "));
                }
                println!("  Pruned (hard delete): {}", result.pruned);
                if !result.pruned_ids.is_empty() {
                    println!("    IDs: {}", result.pruned_ids.join(", "));
                }
            }

            Ok(())
        }

        LifecycleCommands::Promote { id } => {
            let restored = uteke.promote(id).map_err(|e| format!("{e}"))?;
            if cli.json {
                output::print_json(&serde_json::json!({
                    "promoted": restored,
                    "id": id,
                }));
            } else if restored {
                println!("Memory {id} promoted back to active status.");
            } else {
                println!("Memory {id} was not deprecated (no action taken).");
            }
            Ok(())
        }

        LifecycleCommands::Status { namespace } => {
            let target_ns = namespace.as_deref().or(ns);
            let active = uteke.count_active(target_ns).map_err(|e| format!("{e}"))?;
            let deprecated = uteke
                .count_deprecated(target_ns)
                .map_err(|e| format!("{e}"))?;
            let cfg = uteke.get_lifecycle_config();

            if cli.json {
                output::print_json(&serde_json::json!({
                    "namespace": target_ns.unwrap_or("all"),
                    "active": active,
                    "deprecated": deprecated,
                    "config": {
                        "soft_delete_only": cfg.soft_delete_only,
                        "auto_aging_enabled": cfg.auto_aging_enabled,
                        "auto_aging_interval_hours": cfg.auto_aging_interval_hours,
                        "min_age_days": cfg.min_age_days,
                        "max_access_count": cfg.max_access_count,
                        "max_deprecate_percent": cfg.max_deprecate_percent,
                        "deprecated_ttl_days": cfg.deprecated_ttl_days,
                        "auto_prune_enabled": cfg.auto_prune_enabled,
                    }
                }));
            } else {
                println!(
                    "Lifecycle Status (namespace: {})",
                    target_ns.unwrap_or("all")
                );
                println!("  Active memories:     {active}");
                println!("  Deprecated memories: {deprecated}");
                println!();
                println!("Configuration:");
                println!("  soft_delete_only:          {}", cfg.soft_delete_only);
                println!("  auto_aging_enabled:        {}", cfg.auto_aging_enabled);
                println!(
                    "  auto_aging_interval_hours: {}",
                    cfg.auto_aging_interval_hours
                );
                println!("  min_age_days:              {}", cfg.min_age_days);
                println!("  max_access_count:          {}", cfg.max_access_count);
                println!(
                    "  max_deprecate_percent:     {}%",
                    cfg.max_deprecate_percent
                );
                println!("  deprecated_ttl_days:       {}", cfg.deprecated_ttl_days);
                println!("  auto_prune_enabled:        {}", cfg.auto_prune_enabled);
            }

            Ok(())
        }
    }
}
