//! Room command handlers — list, stats, recall, delete.

use crate::cli::{Cli, RoomCommands};
use uteke_core::Uteke;

pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    command: &RoomCommands,
) -> Result<(), String> {
    match command {
        RoomCommands::List { namespace } => {
            let filter_ns = namespace.as_deref().or(ns);
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
            author,
            limit,
        } => {
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
                        format!("{}...", &m.content[..77])
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
            Ok(())
        }

        RoomCommands::Delete { room_id, confirm } => {
            if !confirm {
                eprintln!("This will delete room {room_id} and all memory links.");
                eprintln!("Memories themselves are NOT deleted. Use --confirm to proceed.");
                return Err("Operation not confirmed".to_string());
            }

            uteke
                .delete_room(room_id)
                .map_err(|e| format!("Failed to delete room: {e}"))?;

            if cli.json {
                println!("{}", serde_json::json!({"deleted": room_id}));
            } else {
                println!("Room {room_id} deleted. Memories are preserved in their namespaces.");
            }
            Ok(())
        }
    }
}
