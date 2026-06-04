//! Command handler implementations for all CLI subcommands.

use std::io::{self, Read};

use crate::output;
use crate::resolve_namespace;
use crate::AgingCommands;
use crate::Cli;
use crate::Commands;
use crate::Config;
use crate::NamespaceCommands;
use crate::SupportedShell;
use crate::TagCommands;
use uteke_core::Uteke;

/// Check if uteke server is reachable.
pub(crate) fn is_server_running(url: &str) -> bool {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_millis(100))
        .build()
        .map(|c| c.get(format!("{url}/health")).send().is_ok())
        .unwrap_or(false)
}

/// Route CLI commands through the HTTP server for <50ms latency.
pub(crate) fn run_via_server(cli: &Cli, server_url: &str) -> Result<(), String> {
    let client = reqwest::blocking::Client::new();
    let ns = cli.namespace.as_deref().unwrap_or("default");

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
        } => {
            let mut body = serde_json::json!({
                "content": content,
                "tags": tags,
                "namespace": ns
            });
            if !r#type.is_empty() {
                body["type"] = serde_json::json!(r#type);
            }
            if *detect_contradiction {
                body["detect_contradiction"] = serde_json::json!(true);
            }
            let resp = client
                .post(format!("{server_url}/remember"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let data: serde_json::Value = resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!("{data}");
            } else {
                println!("\u{2713} Memory stored\n  ID: {}", data["id"]);
            }
        }
        Commands::Recall {
            query, limit, tags, ..
        } => {
            let body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/recall"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let results: Vec<uteke_core::SearchResult> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_recall_human(&results);
            }
        }
        Commands::Search {
            query, limit, tags, ..
        } => {
            let body = serde_json::json!({
                "query": query,
                "limit": limit,
                "tags": tags,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/search"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let results: Vec<uteke_core::SearchResult> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_search_human(&results);
            }
        }
        Commands::List {
            tag, limit, offset, ..
        } => {
            let body = serde_json::json!({
                "tag": tag,
                "limit": limit,
                "offset": offset,
                "namespace": ns
            });
            let resp = client
                .post(format!("{server_url}/list"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let memories: Vec<uteke_core::Memory> =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&memories);
            } else {
                output::print_list_human(&memories);
            }
        }
        Commands::Stats => {
            let body = serde_json::json!({ "namespace": ns });
            let resp = client
                .post(format!("{server_url}/stats"))
                .json(&body)
                .send()
                .map_err(|e| format!("Server error: {e}"))?;
            let stats: uteke_core::StoreStats =
                resp.json().map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&stats);
            } else {
                output::print_stats_human(&stats);
            }
        }
        Commands::Forget {
            id,
            tag,
            cold: _,
            all: _,
            confirm: _,
        } => {
            if let Some(id) = id {
                let resp = client
                    .delete(format!(
                        "{server_url}/forget?id={}",
                        urlencoding::encode(id)
                    ))
                    .send()
                    .map_err(|e| format!("Server error: {e}"))?;
                let data: serde_json::Value =
                    resp.json().map_err(|e| format!("Parse error: {e}"))?;
                if cli.json {
                    println!("{data}");
                } else {
                    println!("\u{2713} Memory forgotten: {id}");
                }
            } else if let Some(tag) = tag {
                let resp = client
                    .delete(format!(
                        "{server_url}/forget?tag={}&namespace={}",
                        urlencoding::encode(tag),
                        urlencoding::encode(ns)
                    ))
                    .send()
                    .map_err(|e| format!("Server error: {e}"))?;
                let data: serde_json::Value =
                    resp.json().map_err(|e| format!("Parse error: {e}"))?;
                if cli.json {
                    println!("{data}");
                } else {
                    println!(
                        "\u{2713} Deleted {} memories with tag '{}'",
                        data["deleted"], tag
                    );
                }
            } else {
                return Err("Provide an ID, --tag, --cold, or --all".into());
            }
        }
        // Commands not supported via server fall through to local
        _ => {
            return Err("unsupported".into());
        }
    }
    Ok(())
}

/// Dispatch all CLI subcommands to their handler implementations.
pub(crate) fn run_command(cli: &Cli, uteke: &Uteke, config: &Config) -> Result<(), String> {
    // Resolve effective namespace once: CLI > env > config > "default"
    let resolved_ns = resolve_namespace(cli, config);
    let ns: Option<&str> = Some(resolved_ns.as_str());

    match &cli.command {
        Commands::Remember {
            content,
            tags,
            r#type,
            detect_contradiction,
        } => {
            tracing::info!(
                "Remembering: {content} (type: {type}, contradiction: {detect_contradiction})"
            );
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();

            if *detect_contradiction {
                let (id, contradiction) = uteke
                    .remember_with_contradiction(
                        content,
                        &tag_refs,
                        ns,
                        Some(r#type.as_str()),
                        true,
                    )
                    .map_err(|e| format!("Failed to store memory: {e}"))?;
                tracing::info!("Memory stored with ID: {id}");
                if cli.json {
                    let obj = serde_json::json!({
                        "id": id,
                        "contradiction": {
                            "detected": contradiction.contradicted,
                            "deprecated_id": contradiction.deprecated_id,
                            "similarity": contradiction.similarity
                        }
                    });
                    println!("{}", obj);
                } else {
                    output::print_remember_human(&id);
                    if contradiction.contradicted {
                        if let Some(dep_id) = &contradiction.deprecated_id {
                            println!(
                                "  \u{26a0} Contradiction detected (sim: {:.3}): deprecated {}",
                                contradiction.similarity,
                                &dep_id[..8]
                            );
                        }
                    }
                }
            } else {
                let id = uteke
                    .remember(content, &tag_refs, None, ns)
                    .map_err(|e| format!("Failed to store memory: {e}"))?;
                tracing::info!("Memory stored with ID: {id}");
                if cli.json {
                    let obj = serde_json::json!({"id": id});
                    println!("{}", obj);
                } else {
                    output::print_remember_human(&id);
                }
            }
            Ok(())
        }
        Commands::Recall { query, limit, tags } => {
            tracing::info!("Recalling: {query} (limit: {limit})");
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let tags_filter = if tag_refs.is_empty() {
                None
            } else {
                Some(tag_refs.as_slice())
            };
            let results = uteke
                .recall(query, *limit, tags_filter, ns)
                .map_err(|e| format!("Failed to recall: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_recall_human(&results);
            }
            Ok(())
        }
        Commands::Search { query, limit, tags } => {
            tracing::info!("Searching: {query} (limit: {limit})");
            let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
            let tags_filter = if tag_refs.is_empty() {
                None
            } else {
                Some(tag_refs.as_slice())
            };
            let results = uteke
                .search(query, *limit, tags_filter, ns)
                .map_err(|e| format!("Failed to search: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_search_human(&results);
            }
            Ok(())
        }
        Commands::List { tag, limit, offset } => {
            tracing::info!(
                "Listing memories (tag: {:?}, limit: {limit}, offset: {offset})",
                tag
            );
            let results = uteke
                .list(tag.as_deref(), *limit, *offset, ns)
                .map_err(|e| format!("Failed to list: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_list_human(&results);
            }
            Ok(())
        }
        Commands::Get { id } => {
            tracing::info!("Getting memory: {id}");
            let memory = uteke
                .get(id)
                .map_err(|e| format!("Failed to get memory: {e}"))?;
            if cli.json {
                output::print_json(&memory);
            } else {
                output::print_get_human(&memory);
            }
            Ok(())
        }
        Commands::Forget {
            id,
            tag,
            cold,
            all,
            confirm,
        } => {
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
                    println!("✓ Memory forgotten: {id}");
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
                    output::print_json(
                        &serde_json::json!({"deleted": result.deleted, "ids": result.ids}),
                    );
                } else {
                    println!("✓ Deleted {} memories with tag '{}'", result.deleted, tag);
                }
            } else if *cold {
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
                    output::print_json(
                        &serde_json::json!({"deleted": result.deleted, "ids": result.ids}),
                    );
                } else {
                    println!("✓ Deleted {} cold memories", result.deleted);
                }
            } else if *all {
                // Delete all in namespace
                tracing::info!("Bulk forgetting all memories");
                if !confirm {
                    println!("⚠️  This will delete ALL memories in the namespace.");
                    println!("Use --confirm to proceed.");
                    return Ok(());
                }
                let result = uteke
                    .bulk_forget_all(ns)
                    .map_err(|e| format!("Failed: {e}"))?;
                if cli.json {
                    output::print_json(
                        &serde_json::json!({"deleted": result.deleted, "ids": result.ids}),
                    );
                } else {
                    println!("✓ Deleted {} memories", result.deleted);
                }
            } else {
                return Err("Provide an ID, --tag, --cold, or --all".into());
            }
            Ok(())
        }
        Commands::Stats => {
            tracing::info!("Getting stats");
            let stats = uteke
                .stats(ns)
                .map_err(|e| format!("Failed to get stats: {e}"))?;
            if cli.json {
                output::print_json(&stats);
            } else {
                output::print_stats_human(&stats);
            }
            Ok(())
        }
        Commands::Doctor => {
            tracing::info!("Running doctor");
            let report = uteke.doctor().map_err(|e| format!("Doctor failed: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                output::print_doctor_human(&report);
            }
            Ok(())
        }
        Commands::Verify => {
            tracing::info!("Running verify");
            let report = uteke.verify().map_err(|e| format!("Verify failed: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                output::print_verify_human(&report);
            }
            Ok(())
        }
        Commands::Repair => {
            tracing::info!("Running repair");
            let report = uteke.repair().map_err(|e| format!("Repair failed: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                output::print_repair_human(&report);
            }
            Ok(())
        }
        Commands::Tags { command } => {
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
                        println!("✓ Tag '{old}' renamed to '{new}' ({count} memories updated)");
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
                                println!(
                                    "Tag '{}' is used by {} memory(ies).",
                                    info.name, info.count
                                );
                                println!("Use --confirm to proceed with deletion.");
                                return Err(
                                    "Tag deletion not confirmed. Use --confirm flag.".to_string()
                                );
                            }
                            None => {
                                return Err(format!("Tag '{}' not found.", tag));
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
                        println!("✓ Tag '{tag}' deleted ({count} memories updated)");
                    }
                }
            }
            Ok(())
        }
        Commands::Prune { ttl, dry_run } => {
            tracing::info!("Pruning with TTL={ttl}d (dry_run={dry_run})");
            let result = uteke
                .prune(*ttl, ns, *dry_run)
                .map_err(|e| format!("Failed to prune: {e}"))?;
            if cli.json {
                output::print_json(&result);
            } else {
                if result.deprecated_ids.is_empty() && result.pruned == 0 {
                    println!("No deprecated memories to prune.");
                } else if *dry_run {
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
            }
            Ok(())
        }
        Commands::Consolidate { threshold, dry_run } => {
            tracing::info!("Consolidating (threshold: {threshold}, dry_run: {dry_run})");
            if *dry_run {
                let pairs = uteke
                    .find_duplicates(ns, *threshold)
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
                    .consolidate(ns, *threshold, false)
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
        Commands::Export { output } => {
            tracing::info!("Exporting memories to {output}");
            let jsonl = uteke
                .export(ns)
                .map_err(|e| format!("Failed to export: {e}"))?;

            if output == "-" {
                println!("{jsonl}");
            } else {
                std::fs::write(output, &jsonl)
                    .map_err(|e| format!("Failed to write export file: {e}"))?;
                let count = jsonl.lines().filter(|l| !l.trim().is_empty()).count();
                if cli.json {
                    output::print_json(&serde_json::json!({"exported": count}));
                } else {
                    println!("✓ Exported {count} memories");
                }
            }
            Ok(())
        }
        Commands::Import { input } => {
            tracing::info!("Importing memories from {input}");
            let jsonl = if input == "-" {
                let mut buf = String::new();
                io::stdin()
                    .read_to_string(&mut buf)
                    .map_err(|e| format!("Failed to read stdin: {e}"))?;
                buf
            } else {
                std::fs::read_to_string(input)
                    .map_err(|e| format!("Failed to read import file: {e}"))?
            };

            let result = uteke
                .import(&jsonl, ns)
                .map_err(|e| format!("Failed to import: {e}"))?;

            if cli.json {
                output::print_json(&result);
            } else {
                println!(
                    "✓ Imported {} memories ({} skipped)",
                    result.imported, result.skipped
                );
            }
            Ok(())
        }
        Commands::Completions { .. } => {
            // Already handled in main()
            Ok(())
        }
        Commands::Init { agent } => crate::init::run_init(agent, cli.json),
        Commands::Namespace { command } => match command {
            NamespaceCommands::List => {
                tracing::info!("Listing namespaces");
                let namespaces = uteke
                    .list_namespaces()
                    .map_err(|e| format!("Failed to list namespaces: {e}"))?;
                if cli.json {
                    output::print_json(&namespaces);
                } else {
                    if namespaces.is_empty() {
                        println!("No namespaces found.");
                    } else {
                        println!("Namespaces ({} total):\n", namespaces.len());
                        for ns_name in &namespaces {
                            let count = uteke.store().count(Some(ns_name.as_str())).unwrap_or(0);
                            println!("  {} ({} memories)", ns_name, count);
                        }
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
                    println!("✓ Default namespace set to '{name}'");
                }
                Ok(())
            }
        },
        Commands::Aging { command } => match command {
            AgingCommands::Status => {
                tracing::info!("Aging status");
                let status = uteke
                    .aging_status(ns)
                    .map_err(|e| format!("Failed to get aging status: {e}"))?;
                if cli.json {
                    output::print_json(&status);
                } else {
                    output::print_aging_status_human(&status);
                }
                Ok(())
            }
            AgingCommands::Preview {
                older_than_days,
                max_access_count,
            } => {
                tracing::info!(
                    "Aging preview (older_than: {}d, max_access: {})",
                    older_than_days,
                    max_access_count
                );
                let memories = uteke
                    .aging_preview(*older_than_days, *max_access_count, ns)
                    .map_err(|e| format!("Failed to preview aged memories: {e}"))?;
                if cli.json {
                    output::print_json(&memories);
                } else {
                    output::print_aging_preview_human(&memories);
                }
                Ok(())
            }
            AgingCommands::Cleanup {
                older_than_days,
                max_access_count,
                yes,
            } => {
                // Preview first
                let preview = uteke
                    .aging_preview(*older_than_days, *max_access_count, ns)
                    .map_err(|e| format!("Failed to preview aged memories: {e}"))?;

                if preview.is_empty() {
                    if cli.json {
                        output::print_json(&uteke_core::CleanupResult { deleted: 0 });
                    } else {
                        println!("No aged memories to clean up.");
                    }
                    return Ok(());
                }

                if !yes {
                    if !cli.json {
                        output::print_aging_preview_human(&preview);
                        println!();
                        println!(
                            "⚠ About to delete {} memory(ies). Use --yes to confirm.",
                            preview.len()
                        );
                    }
                    return Err("Cleanup not confirmed. Use --yes flag to proceed.".to_string());
                }

                let count = preview.len();
                let result = uteke
                    .aging_cleanup(*older_than_days, *max_access_count, ns)
                    .map_err(|e| format!("Failed to cleanup aged memories: {e}"))?;
                if cli.json {
                    output::print_json(&result);
                } else {
                    println!("✓ Cleaned up {} aged memory(ies)", result.deleted);
                }
                let _ = count;
                Ok(())
            }
        },
        Commands::Hook { shell } => {
            let script = match shell {
                SupportedShell::Bash => include_str!("../../../scripts/shell/uteke-hook.bash"),
                SupportedShell::Zsh => include_str!("../../../scripts/shell/uteke-hook.zsh"),
                SupportedShell::Fish => include_str!("../../../scripts/shell/uteke-hook.fish"),
            };
            print!("{script}");
            Ok(())
        }
        Commands::VerifyChecksums {
            checksums_file,
            binary,
        } => {
            let checksums = std::fs::read_to_string(&checksums_file)
                .map_err(|e| format!("Failed to read checksums file: {e}"))?;

            let binary_filename = std::path::Path::new(&binary)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("binary");

            let expected_line = checksums.lines().find(|l| l.contains(binary_filename));

            match expected_line {
                Some(line) => {
                    let expected_hash = line.split_whitespace().next().unwrap_or("");
                    let output = std::process::Command::new("sha256sum")
                        .arg(&binary)
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
                    return Err(
                        format!("Binary not found in checksums file: {}", binary_filename).into(),
                    );
                }
            }
            Ok(())
        }
    }
}
