//! Graph handlers: graph, edges, timeline, provenance and contradictions.

use super::*;

pub(super) fn get_graph(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let ns = parse_query_namespace(path);
    let query = path.split('?').nth(1).unwrap_or("");
    // `node_id` = memory id whose neighbourhood to return (viewers);
    // `limit` bounds the number of edges (default 500, max 5000).
    let node_id = parse_query_param(query, "node_id").filter(|s| !s.is_empty());
    let limit = parse_query_param(query, "limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(uteke_core::graph_view::GRAPH_VIEW_DEFAULT_LIMIT);
    match uteke.graph_view(ns.as_deref(), node_id.as_deref(), limit) {
        Ok(data) => ctx.ok_response_for(req, &data),
        Err(e) => {
            error!("Graph data error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_graph_edge(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    match read_body::<GraphEdgeRequest>(req.as_reader()) {
        Ok(req_data) => {
            // Reject self-loops
            if req_data.source == req_data.target {
                return ctx.error_response_for(
                    req,
                    400,
                    "Self-loop edges are not allowed (source == target)",
                );
            }

            let conn = uteke.graph_store();
            let gs = uteke_core::graph::GraphStore::new(conn);

            // Resolve an input ID to its graph node without creating
            // anything: memory-linked nodes first, then explicit node IDs
            // (clients may pass IDs from GET /graph) (#1180).
            let resolve_node = |id: &str| -> Result<Option<String>, uteke_core::Error> {
                if let Some(nid) = gs.node_id_for_memory(id)? {
                    return Ok(Some(nid));
                }
                gs.get_node(id).map(|n| n.map(|node| node.id))
            };

            // Validate both sides exist as memories or graph nodes —
            // memory IDs are ensured into nodes below (#1180, relaxes the
            // memory-only #542 check that made every valid call 500).
            for (role, id) in [("Source", &req_data.source), ("Target", &req_data.target)] {
                if matches!(uteke.get_by_id(id), Ok(Some(_))) {
                    continue;
                }
                match resolve_node(id) {
                    Ok(Some(_)) => {}
                    Ok(None) => {
                        return ctx.error_response_for(
                            req,
                            404,
                            format!("{role} memory not found: {id}"),
                        );
                    }
                    Err(e) => {
                        error!("Graph resolve error: {e}");
                        return ctx.error_response_for(req, 500, "Internal server error");
                    }
                }
            }

            let relation = req_data.edge_type.as_deref().unwrap_or("related");
            let weight = req_data.weight.unwrap_or(1.0);

            // Map memory IDs → graph node IDs before insertion (#1180).
            // `graph_edges.source_id/target_id` carry FKs to
            // `graph_nodes(id)`, so inserting raw memory IDs violates the
            // FK and always 500s. Validation above guarantees each ID is
            // a memory (node gets ensured) or an existing node.
            let ensure = |id: &str| -> Result<String, uteke_core::Error> {
                match resolve_node(id)? {
                    Some(nid) => Ok(nid),
                    None => gs.ensure_node_for_memory(id),
                }
            };
            let (src_node, tgt_node) = match (ensure(&req_data.source), ensure(&req_data.target)) {
                (Ok(s), Ok(t)) => (s, t),
                (Err(e), _) | (_, Err(e)) => {
                    error!("Graph ensure_node error: {e}");
                    return ctx.error_response_for(req, 500, "Internal server error");
                }
            };

            match gs.add_edge(&src_node, &tgt_node, relation, weight) {
                Ok(()) => ctx.ok_response_for(
                    req,
                    &serde_json::json!({
                        "ok": true,
                        "source_node": src_node,
                        "target_node": tgt_node,
                    }),
                ),
                Err(e) => {
                    error!("Graph add_edge error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn delete_graph_edge(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    let source = parse_query_param(query, "source");
    let target = parse_query_param(query, "target");

    match (&source, &target) {
        (Some(src), Some(tgt)) => {
            let conn = uteke.graph_store();
            let gs = uteke_core::graph::GraphStore::new(conn);
            // Accept either memory IDs or graph node IDs (#1180) —
            // resolve both sides to node IDs before deleting.
            let resolve = |id: &str| -> Result<Option<String>, uteke_core::Error> {
                if let Some(nid) = gs.node_id_for_memory(id)? {
                    return Ok(Some(nid));
                }
                gs.get_node(id).map(|n| n.map(|node| node.id))
            };
            let (src_node, tgt_node) = match (resolve(src), resolve(tgt)) {
                (Ok(Some(s)), Ok(Some(t))) => (s, t),
                (Ok(_), Ok(_)) => {
                    return ctx.error_response_for(
                        req,
                        404,
                        format!("Edge not found: {src} -> {tgt}"),
                    );
                }
                (Err(e), _) | (_, Err(e)) => {
                    error!("Graph node resolve error: {e}");
                    return ctx.error_response_for(req, 500, "Internal server error");
                }
            };
            match gs.remove_edge(&src_node, &tgt_node) {
                Ok(true) => ctx.ok_response_for(req, &serde_json::json!({"ok": true})),
                Ok(false) => {
                    ctx.error_response_for(req, 404, format!("Edge not found: {src} -> {tgt}"))
                }
                Err(e) => {
                    error!("Graph remove_edge error: {e}");
                    ctx.error_response_for(req, 500, "Internal server error")
                }
            }
        }
        _ => ctx.error_response_for(
            req,
            400,
            "Provide both ?source=...&target=... query parameters",
        ),
    }
}

pub(super) fn get_provenance(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    let id = match parse_query_param(query, "id") {
        Some(id) => id,
        None => return ctx.error_response_for(req, 400, "Missing 'id' query parameter"),
    };
    match uteke.provenance(&id) {
        Ok(Some(report)) => ctx.ok_response_for(req, &report),
        Ok(None) => ctx.error_response_for(req, 404, format!("Memory not found: {id}")),
        Err(e) => {
            error!("Provenance error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn get_contradictions(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    let ns = parse_query_namespace(path);
    let limit = parse_query_param(query, "limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(MAX_LIST_LIMIT);
    match uteke.contradiction_resolutions(ns.as_deref(), limit) {
        Ok(resolutions) => ctx.ok_response_for(req, &resolutions),
        Err(e) => {
            error!("Contradiction resolutions error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn post_contradictions_undo(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    #[derive(Deserialize)]
    struct ContradictionUndoRequest {
        /// ID of the retired (superseded) memory to restore.
        id: String,
    }
    match read_body::<ContradictionUndoRequest>(req.as_reader()) {
        Ok(req_data) => match uteke.undo_supersession(&req_data.id) {
            Ok(Some(winner)) => ctx.ok_response_for(
                req,
                &serde_json::json!({
                    "undone": true,
                    "restored": req_data.id,
                    "was_superseded_by": winner,
                }),
            ),
            Ok(None) => ctx.error_response_for(
                req,
                404,
                format!("No supersession found for memory: {}", req_data.id),
            ),
            Err(e) => {
                error!("Contradiction undo error: {e}");
                ctx.error_response_for(req, 500, "Internal server error")
            }
        },
        Err(e) => ctx.error_response_for(req, 400, e),
    }
}

pub(super) fn get_timeline(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    let id = match parse_query_param(query, "id") {
        Some(id) => id,
        None => return ctx.error_response_for(req, 400, "Missing 'id' query parameter"),
    };
    let limit = parse_query_param(query, "limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50);
    match uteke.timeline(&id, limit) {
        Ok(events) => ctx.ok_response_for(req, &events),
        Err(e) => {
            error!("Timeline error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}

pub(super) fn get_edges(c: &mut Call) -> Response<Cursor<Vec<u8>>> {
    let uteke = c.uteke;
    let ctx = c.ctx;
    let req = &mut *c.req;
    let path = c.path;
    let query = path.split('?').nth(1).unwrap_or("");
    let id = match parse_query_param(query, "id") {
        Some(id) => id,
        None => return ctx.error_response_for(req, 400, "Missing 'id' query parameter"),
    };
    match uteke.edges_for(&id) {
        Ok(edges) => ctx.ok_response_for(req, &edges),
        Err(e) => {
            error!("Edges error: {e}");
            ctx.error_response_for(req, 500, "Internal server error")
        }
    }
}
