//! `uteke edges <id>` — list auto-wired edges for a memory (#346).
//!
//! Without `--deep`, lists direct outgoing + incoming edges.
//! With `--deep N`, runs BFS across the `memory_edges` table and returns
//! reachable memory ids within N hops.

use crate::cli::Cli;
use uteke_core::Uteke;

/// Run `uteke edges`.
pub(crate) fn run(cli: &Cli, uteke: &Uteke, id: &str, deep: usize) -> Result<(), String> {
    if deep == 0 {
        let edges = uteke
            .edges_for(id)
            .map_err(|e| format!("Failed to list edges: {e}"))?;

        if cli.json {
            println!("{}", serde_json::to_string(&edges).unwrap());
            return Ok(());
        }

        if edges.total() == 0 {
            println!("No edges for memory {}.", &id[..8.min(id.len())]);
            return Ok(());
        }

        println!(
            "Edges for memory {} ({} total):",
            &id[..8.min(id.len())],
            edges.total()
        );
        if !edges.outgoing.is_empty() {
            println!("\n→ outgoing ({}):", edges.outgoing.len());
            for e in &edges.outgoing {
                println!(
                    "  {} → {}   [{}]",
                    short(&e.source_id),
                    short(&e.target_id),
                    e.edge_type
                );
            }
        }
        if !edges.incoming.is_empty() {
            println!("\n← incoming ({}):", edges.incoming.len());
            for e in &edges.incoming {
                println!(
                    "  {} ← {}   [{}]",
                    short(&e.target_id),
                    short(&e.source_id),
                    e.edge_type
                );
            }
        }
        return Ok(());
    }

    // BFS mode
    let reachable = uteke
        .related_via_edges(id, deep)
        .map_err(|e| format!("Failed BFS traversal: {e}"))?;

    if cli.json {
        let ids: Vec<&str> = reachable.iter().map(|m| m.id.as_str()).collect();
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "memory_id": id,
                "depth": deep,
                "reachable": ids,
            }))
            .unwrap()
        );
        return Ok(());
    }

    if reachable.is_empty() {
        println!(
            "No memories reachable within {deep} hop{} from {}.",
            if deep == 1 { "" } else { "s" },
            &id[..8.min(id.len())]
        );
        return Ok(());
    }
    println!(
        "Memories reachable within {deep} hop{} from {} ({}):",
        if deep == 1 { "" } else { "s" },
        &id[..8.min(id.len())],
        reachable.len()
    );
    for m in &reachable {
        let preview = preview_text(&m.content);
        println!("  {}  {}", short(&m.id), preview);
    }
    Ok(())
}

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn preview_text(content: &str) -> String {
    let s: String = content.chars().take(60).collect();
    if content.chars().count() > 60 {
        format!("{s}…")
    } else {
        s
    }
}
