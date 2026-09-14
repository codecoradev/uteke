//! Room command handlers — list, stats, recall, delete.

use crate::cli::{Cli, RoomCommands};
use crate::config::Config;
use crate::output;
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    command: &RoomCommands,
    config: &Config,
) -> Result<(), String> {
    match command {
        RoomCommands::Create { room_id, title } => {
            let ns_str = ns.unwrap_or("default");
            uteke
                .create_room(room_id, title.as_deref(), ns_str)
                .map_err(|e| format!("Failed to create room: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"created": room_id, "namespace": ns_str})
                );
            } else {
                println!("✓ Room '{room_id}' created");
            }
            Ok(())
        }
        RoomCommands::Rename { old_id, new_id } => {
            let room = uteke
                .rename_room(old_id, new_id)
                .map_err(|e| format!("Failed to rename room: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"renamed": old_id, "to": new_id, "room": room})
                );
            } else {
                println!("✓ Room '{old_id}' renamed to '{new_id}'");
            }
            Ok(())
        }
        RoomCommands::Update {
            room_id,
            title,
            description,
        } => {
            let room = uteke
                .update_room(room_id, title.as_deref(), description.as_deref())
                .map_err(|e| format!("Failed to update room: {e}"))?;
            match room {
                Some(room) => {
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&room).unwrap());
                    } else {
                        println!("✓ Room '{room_id}' updated");
                    }
                }
                None => return Err(format!("Room not found: {room_id}")),
            }
            Ok(())
        }
        RoomCommands::MoveMemory {
            memory_id,
            from_room,
            to_room,
        } => {
            let moved = uteke
                .move_memory_to_room(memory_id, from_room, to_room)
                .map_err(|e| format!("Failed to move memory: {e}"))?;
            if moved == 0 {
                return Err(format!(
                    "Memory '{memory_id}' has no link in room '{from_room}'"
                ));
            }
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "moved": memory_id,
                        "from_room": from_room,
                        "to_room": to_room
                    })
                );
            } else {
                println!("✓ Memory '{memory_id}' moved from '{from_room}' to '{to_room}'");
            }
            Ok(())
        }
        RoomCommands::List { namespace } => {
            // Rooms are cross-namespace collaboration spaces (#392).
            // Only filter when --namespace is explicitly passed on the
            // room subcommand, not from the global --namespace flag.
            let filter_ns = namespace.as_deref();
            let rooms = uteke
                .list_rooms(filter_ns)
                .map_err(|e| format!("Failed to list rooms: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&rooms).unwrap());
            } else if rooms.is_empty() {
                println!("No rooms found.");
            } else {
                println!("Found {} room(s):\n", rooms.len());
                for room in &rooms {
                    let title = room.title.as_deref().unwrap_or("(untitled)");
                    println!("  {}  {}", room.id, title);
                    println!(
                        "    namespace: {}  created: {}",
                        room.namespace,
                        room.created_at.get(..19).unwrap_or(&room.created_at)
                    );
                }
            }
            Ok(())
        }

        RoomCommands::Stats { room_id } => {
            let stats = uteke
                .room_stats(room_id)
                .map_err(|e| format!("Failed to get room stats: {e}"))?
                .ok_or_else(|| format!("Room not found: {room_id}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats).unwrap());
            } else {
                println!("Room: {}", stats.room_id);
                if let Some(title) = &stats.title {
                    println!("  Title: {title}");
                }
                println!("  Memories: {}", stats.memory_count);
                println!("  Participants: {}", stats.participant_count);
                if !stats.participants.is_empty() {
                    println!("    {}", stats.participants.join(", "));
                }
                println!(
                    "  Created: {}",
                    stats.created_at.get(..19).unwrap_or(&stats.created_at)
                );
                if let Some(last) = &stats.last_activity {
                    println!("  Last activity: {}", last.get(..19).unwrap_or(last));
                }
            }
            Ok(())
        }

        RoomCommands::Recall {
            room_id,
            query,
            query_flag,
            author,
            limit,
            min,
        } => {
            // Positional query takes precedence over --query flag.
            let effective_query = query.as_ref().or(query_flag.as_ref());
            if let Some(q) = effective_query {
                // Semantic recall — rank by relevance
                let min_score = min.unwrap_or(config.recall.min_score as f32);
                let results = uteke
                    .recall_room_semantic(room_id, q, *limit, author.as_deref(), min_score)
                    .map_err(|e| format!("Failed to recall room: {e}"))?;

                if cli.json {
                    output::print_json(&results);
                } else if results.is_empty() {
                    println!("No matching memories found in room {room_id}.");
                    if min_score > 0.0 {
                        println!("(min_score threshold: {:.2})", min_score);
                    }
                } else {
                    output::print_room_semantic_human(room_id, &results);
                }
            } else {
                // Chronological recall — original behavior
                let memories = uteke
                    .recall_room(room_id, author.as_deref(), *limit)
                    .map_err(|e| format!("Failed to recall room: {e}"))?;

                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&memories).unwrap());
                } else if memories.is_empty() {
                    println!("No memories found in room {room_id}.");
                } else {
                    println!(
                        "Found {} memory/memories in room {}:\n",
                        memories.len(),
                        room_id
                    );
                    for (i, m) in memories.iter().enumerate() {
                        let preview = if m.content.len() > 80 {
                            format!("{}...", uteke_core::safe_truncate(&m.content, 77))
                        } else {
                            m.content.clone()
                        };
                        let tags = if m.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", m.tags.join(", "))
                        };
                        println!(
                            "  {}. {} (ns: {}){}\n     ID: {}",
                            i + 1,
                            preview,
                            m.namespace,
                            tags,
                            &m.id[..8],
                        );
                    }
                }
            }
            Ok(())
        }

        RoomCommands::Delete { room_id, confirm } => {
            if !confirm {
                eprintln!("This will delete room {room_id} and all memory links.");
                eprintln!("Memories themselves are NOT deleted. Use --confirm to proceed.");
                return Err("Operation not confirmed".to_string());
            }

            let unlinked = uteke
                .delete_room(room_id)
                .map_err(|e| format!("Failed to delete room: {e}"))?;

            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "deleted": room_id,
                        "unlinked_memories": unlinked,
                    })
                );
            } else {
                println!(
                    "Room {room_id} deleted. {unlinked} memory link(s) removed; memories are preserved in their namespaces."
                );
            }
            Ok(())
        }

        RoomCommands::Summary { room_id } => {
            let summary = uteke
                .room_summary(room_id)
                .map_err(|e| format!("Failed to summarize room: {e}"))?
                .ok_or_else(|| format!("Room not found: {room_id}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&summary).unwrap());
            } else {
                println!("Room: {}", summary.room_id);
                if let Some(title) = &summary.title {
                    println!("  Title: {title}");
                }
                println!("  Memories: {}", summary.total_memories);
                println!("  Participants: {}", summary.participants.join(", "));
                println!(
                    "  Time: {} → {}",
                    summary.time_range.earliest, summary.time_range.latest
                );

                if !summary.pinned_highlights.is_empty() {
                    println!("\n📌 Pinned:");
                    for p in &summary.pinned_highlights {
                        println!("  • {}", p);
                    }
                }

                if !summary.clusters.is_empty() {
                    println!("\nTopics:");
                    for (i, cluster) in summary.clusters.iter().enumerate() {
                        println!(
                            "  {}. {} ({} memories, score: {:.2})",
                            i + 1,
                            cluster.topic,
                            cluster.memory_count,
                            cluster.score
                        );
                        for preview in &cluster.top_memories {
                            println!("     • {}", preview);
                        }
                        println!(
                            "     tags: {}  participants: {}",
                            cluster.tags.join(", "),
                            cluster.participants.join(", ")
                        );
                    }
                }

                if !summary.top_tags.is_empty() {
                    println!("\nTop Tags:");
                    for tag in &summary.top_tags {
                        println!("  • {} ({})", tag.name, tag.count);
                    }
                }

                if !summary.recent_decisions.is_empty() {
                    println!("\nRecent Decisions:");
                    for d in &summary.recent_decisions {
                        println!("  • {}", d);
                    }
                }
            }
            Ok(())
        }

        RoomCommands::Document { room_id } => {
            let doc = uteke
                .room_summary_document(room_id)
                .map_err(|e| format!("Failed to generate room summary document: {e}"))?
                .ok_or_else(|| format!("Room not found: {room_id}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&doc).unwrap());
            } else {
                output::print_room_document_human(&doc);
            }
            Ok(())
        }
        RoomCommands::AddDocument { room_id, doc_slug } => {
            uteke
                .room_add_document(room_id, doc_slug)
                .map_err(|e| format!("Failed to link document: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "room_id": room_id,
                        "doc_slug": doc_slug,
                        "linked": true
                    })
                );
            } else {
                println!("Linked document '{doc_slug}' to room '{room_id}'.");
            }
            Ok(())
        }
        RoomCommands::RemoveDocument { room_id, doc_slug } => {
            uteke
                .room_remove_document(room_id, doc_slug)
                .map_err(|e| format!("Failed to unlink document: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "room_id": room_id,
                        "doc_slug": doc_slug,
                        "linked": false
                    })
                );
            } else {
                println!("Unlinked document '{doc_slug}' from room '{room_id}'.");
            }
            Ok(())
        }
        RoomCommands::ListDocuments { room_id } => {
            let docs = uteke
                .room_list_documents(room_id)
                .map_err(|e| format!("Failed to list documents: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "room_id": room_id,
                        "documents": docs
                    })
                );
            } else if docs.is_empty() {
                println!("No documents linked to room '{room_id}'.");
            } else {
                println!("Documents linked to room '{room_id}':");
                for slug in &docs {
                    println!("  • {slug}");
                }
            }
            Ok(())
        }
        RoomCommands::ListRooms { doc_slug } => {
            let rooms = uteke
                .document_list_rooms(doc_slug)
                .map_err(|e| format!("Failed to list rooms: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "doc_slug": doc_slug,
                        "rooms": rooms
                    })
                );
            } else if rooms.is_empty() {
                println!("No rooms reference document '{doc_slug}'.");
            } else {
                println!("Rooms referencing document '{doc_slug}':");
                for room_id in &rooms {
                    println!("  • {room_id}");
                }
            }
            Ok(())
        }
        RoomCommands::Consolidate {
            room_id,
            apply,
            max_calls,
        } => {
            let dry = uteke_core::consolidation_api::plan_room(uteke, room_id)
                .map_err(|e| format!("Failed to plan consolidation: {e}"))?;

            if !apply {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&dry).unwrap());
                } else {
                    println!("Consolidation plan for room '{room_id}' (dry-run, no LLM calls):");
                    println!("  Memories: {}", dry.plan.total_memories);
                    println!("  Batches:  {}", dry.plan.batches.len());
                    println!(
                        "  Estimated LLM calls: {}",
                        dry.plan.batches.len().min(*max_calls)
                    );
                    if dry.plan.batches.len() > *max_calls {
                        println!(
                            "  ⚠ {} batches exceed --max-calls {max_calls}; run with higher --max-calls or in parts.",
                            dry.plan.batches.len() - max_calls
                        );
                    }
                    println!("  Pass --apply to execute.");
                }
                return Ok(());
            }

            // Apply: requires extraction LLM setup (shared with import --extract).
            let ext = &config.extraction;
            if ext.api_key.is_empty() {
                return Err(
                    "Consolidation --apply needs LLM config: set [extraction] in uteke.toml \
                     or UTEKE_EXTRACTION_API_KEY (same setup as `import --extract`)."
                        .to_string(),
                );
            }
            let result =
                uteke_core::consolidation_api::consolidate_room(uteke, room_id, ext, *max_calls)
                    .map_err(|e| format!("Consolidation failed: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            } else {
                println!("Consolidation of room '{room_id}' complete:");
                println!("  Batches processed: {}", result.batches_processed);
                println!("  Records written:   {}", result.records_written);
                println!("  Sources deprecated: {}", result.sources_deprecated);
                if result.budget_skipped > 0 {
                    println!("  Budget-skipped:    {}", result.budget_skipped);
                }
                if result.rejected_by_policy > 0 {
                    println!("  Rejected (policy): {}", result.rejected_by_policy);
                }
                if !result.batch_errors.is_empty() {
                    println!("  ⚠ Batch errors (isolated, run continued):");
                    for e in &result.batch_errors {
                        println!("    - {e}");
                    }
                }
            }
            Ok(())
        }
    }
}
