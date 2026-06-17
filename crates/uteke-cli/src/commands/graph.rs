//! Graph command handlers.

use crate::cli::GraphCommands;
use crate::Cli;
use uteke_core::GraphStore;

/// Run graph command.
pub fn run(cli: &Cli, uteke: &uteke_core::Uteke, command: &GraphCommands) -> Result<(), String> {
    let store = uteke.graph_store();
    let graph = GraphStore::new(store);

    match command {
        GraphCommands::Nodes { entity_type } => {
            let nodes = graph
                .all_nodes()
                .map_err(|e| format!("Failed to list nodes: {e}"))?;
            let filtered: Vec<_> = if let Some(et) = entity_type {
                nodes
                    .into_iter()
                    .filter(|n| n.entity_type.as_deref() == Some(et.as_str()))
                    .collect()
            } else {
                nodes
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
            } else if filtered.is_empty() {
                println!("No graph nodes found.");
            } else {
                println!("Graph Nodes ({}):", filtered.len());
                for node in &filtered {
                    let etype = node.entity_type.as_deref().unwrap_or("—");
                    println!("  • {} [{}] ({})", node.label, etype, node.id);
                }
            }
        }

        GraphCommands::Edges { relation } => {
            let edges = graph
                .all_edges()
                .map_err(|e| format!("Failed to list edges: {e}"))?;
            let filtered: Vec<_> = if let Some(rel) = relation {
                edges
                    .into_iter()
                    .filter(|e| e.relation.eq_ignore_ascii_case(rel))
                    .collect()
            } else {
                edges
            };

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&filtered).unwrap());
            } else if filtered.is_empty() {
                println!("No graph edges found.");
            } else {
                println!("Graph Edges ({}):", filtered.len());
                for edge in &filtered {
                    println!(
                        "  • {} -[{}]-> {} (w={:.1})",
                        edge.source_id, edge.relation, edge.target_id, edge.weight
                    );
                }
            }
        }

        GraphCommands::Neighbors { label, depth } => {
            let node = graph
                .find_node(label)
                .map_err(|e| format!("Failed to find node: {e}"))?
                .ok_or_else(|| format!("Node '{label}' not found"))?;

            let edges = graph
                .neighbors(&node.id, *depth)
                .map_err(|e| format!("Failed to get neighbors: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&edges).unwrap());
            } else {
                println!("Neighbors of '{}' (depth {}):", label, depth);
                if edges.is_empty() {
                    println!("  (none)");
                } else {
                    for edge in &edges {
                        let target = graph
                            .get_node(&edge.target_id)
                            .map_err(|e| format!("Failed to get node: {e}"))?;
                        let target_label = target
                            .map(|n| n.label)
                            .unwrap_or_else(|| edge.target_id.clone());
                        println!(
                            "  -[{}]-> {} (w={:.1})",
                            edge.relation, target_label, edge.weight
                        );
                    }
                }
            }
        }

        GraphCommands::Path {
            source,
            target,
            max_depth,
        } => {
            let path = graph
                .find_path(source, target, *max_depth)
                .map_err(|e| format!("Failed to find path: {e}"))?;

            match path {
                Some(path) => {
                    if cli.json {
                        println!("{}", serde_json::to_string_pretty(&path).unwrap());
                    } else {
                        let labels: Vec<&str> =
                            path.nodes.iter().map(|n| n.label.as_str()).collect();
                        let rels: Vec<&str> =
                            path.edges.iter().map(|e| e.relation.as_str()).collect();
                        println!("Path: {}", labels.join(" → "));
                        if !rels.is_empty() {
                            println!("Relations: {}", rels.join(", "));
                        }
                        println!("Total weight: {:.1}", path.total_weight);
                    }
                }
                None => {
                    if cli.json {
                        println!("null");
                    } else {
                        println!(
                            "No path found between '{}' and '{}' (max depth {})",
                            source, target, max_depth
                        );
                    }
                }
            }
        }

        GraphCommands::Query { relation } => {
            let triples = graph
                .query_relation(relation)
                .map_err(|e| format!("Failed to query relation: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&triples).unwrap());
            } else if triples.is_empty() {
                println!("No edges with relation '{}'", relation);
            } else {
                println!("Relation '{}' ({}):", relation, triples.len());
                for t in &triples {
                    println!(
                        "  • {} -[{}]-> {} (w={:.1})",
                        t.source.label, t.edge.relation, t.target.label, t.edge.weight
                    );
                }
            }
        }

        GraphCommands::Stats => {
            let stats = graph
                .stats()
                .map_err(|e| format!("Failed to get stats: {e}"))?;

            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats).unwrap());
            } else {
                println!("Graph Statistics:");
                println!("  Nodes: {}", stats.node_count);
                println!("  Edges: {}", stats.edge_count);
                if !stats.relation_types.is_empty() {
                    println!("  Relations: {}", stats.relation_types.join(", "));
                }
            }
        }
    }

    Ok(())
}
