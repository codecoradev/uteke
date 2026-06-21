//! `uteke edges <id>` — list auto-wired edges for a memory (#346).
//!
//! Without `--deep`, lists direct outgoing + incoming edges.
//! With `--deep N`, runs BFS across the `memory_edges` table and returns
//! reachable memory ids within N hops.

use crate::cli::Cli;
use uteke_core::Uteke;

/// Run `uteke edges`.
pub(crate) fn run(
    cli: &Cli,
    uteke: &Uteke,
    id: &str,
    deep: usize,
    direction: &str,
) -> Result<(), String> {
    let dir = direction.to_ascii_lowercase();
    let show_out = dir == "both" || dir == "outgoing";
    let show_in = dir == "both" || dir == "incoming";
    if !show_out && !show_in {
        return Err(format!(
            "invalid --direction '{direction}': expected incoming, outgoing, or both"
        ));
    }

    if deep == 0 {
        let edges = uteke
            .edges_for(id)
            .map_err(|e| format!("Failed to list edges: {e}"))?;

        if cli.json {
            println!("{}", serde_json::to_string(&edges).unwrap());
            return Ok(());
        }

        let visible = (if show_out { edges.outgoing.len() } else { 0 })
            + (if show_in { edges.incoming.len() } else { 0 });
        if visible == 0 {
            println!("No edges for memory {}.", &id[..8.min(id.len())]);
            return Ok(());
        }

        println!(
            "Edges for memory {} ({} total):",
            &id[..8.min(id.len())],
            visible
        );
        if show_out && !edges.outgoing.is_empty() {
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
        if show_in && !edges.incoming.is_empty() {
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

/// Run `uteke rebuild-backlinks` (#350).
///
/// Scans all forward edges and ensures a `referenced_by` inverse edge exists
/// for each. Idempotent. Reports how many new backlinks were created.
pub(crate) fn run_rebuild_backlinks(cli: &Cli, uteke: &Uteke, quiet: bool) -> Result<(), String> {
    let before = uteke
        .count_edges()
        .map_err(|e| format!("count edges: {e}"))?;
    let created = uteke
        .rebuild_backlinks()
        .map_err(|e| format!("rebuild backlinks: {e}"))?;
    let after = uteke
        .count_edges()
        .map_err(|e| format!("count edges: {e}"))?;

    if cli.json {
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({
                "created": created,
                "edges_before": before,
                "edges_after": after,
            }))
            .unwrap()
        );
        return Ok(());
    }

    if quiet {
        println!("{created}");
        return Ok(());
    }

    if created == 0 {
        println!("No new backlinks needed — all forward edges already had inverses.");
    } else {
        println!(
            "Created {created} new `referenced_by` backlink edge{} (edges: {before} → {after}).",
            if created == 1 { "" } else { "s" }
        );
    }
    Ok(())
}
