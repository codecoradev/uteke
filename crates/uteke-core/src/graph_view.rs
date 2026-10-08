//! Unified, bounded graph view for visualisation (`GET /graph`, MCP `uteke_graph`).
//!
//! Two stores can hold relations between memories:
//!
//! * `memory_edges` — written automatically on every `remember` (cosine
//!   links, tag/slug references) and by supersession/consolidation. This is
//!   where the data actually is.
//! * `graph_nodes` / `graph_edges` — explicit knowledge-graph entries added
//!   through `POST /graph/edge` and friends. Almost always empty.
//!
//! [`Uteke::graph_data`] only ever read the second one, so viewers (Corin)
//! got an empty graph on real stores. [`Uteke::graph_view`] merges both,
//! keyed by **memory id** (what a viewer can open), restricted to live
//! memories, optionally limited to one memory's neighbourhood, and bounded.

use crate::Error;
use crate::Uteke;
use crate::graph::{GraphNode, GraphStats};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

/// Default number of edges returned when the caller passes no limit.
pub const GRAPH_VIEW_DEFAULT_LIMIT: usize = 500;
/// Hard upper bound for `limit`.
pub const GRAPH_VIEW_MAX_LIMIT: usize = 5000;

/// Derived `referenced_by` backlinks mirror a forward edge one-to-one; they
/// add clutter to a picture, so the view leaves them out (`GET /edges?id=`
/// still lists them).
const DERIVED_BACKLINK: &str = "referenced_by";

/// One edge in a [`GraphView`].
///
/// `source`/`target` repeat `source_id`/`target_id` because viewers read the
/// short names, while the long names are what the rest of the API uses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GraphViewEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub source: String,
    pub target: String,
    pub relation: String,
    pub weight: f64,
    pub created_at: String,
}

impl GraphViewEdge {
    fn new(id: String, source: &str, target: &str, relation: &str, weight: f64, at: &str) -> Self {
        Self {
            id,
            source_id: source.to_string(),
            target_id: target.to_string(),
            source: source.to_string(),
            target: target.to_string(),
            relation: relation.to_string(),
            weight,
            created_at: at.to_string(),
        }
    }
}

/// Nodes + edges + counts of what is in the (possibly truncated) view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphView {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphViewEdge>,
    pub stats: GraphStats,
    /// `true` when more edges matched than `limit` allowed.
    pub truncated: bool,
}

/// First 60 characters of a memory, plus its short id (matches the label the
/// explicit graph uses, #1187).
fn memory_label(content: &str, id: &str) -> String {
    let preview: String = content.chars().take(60).collect();
    let short: String = id.chars().take(8).collect();
    format!("{preview} — {short}")
}

impl Uteke {
    /// Build the unified graph view.
    ///
    /// * `namespace` — only memories of this namespace (both endpoints).
    /// * `node_id` — only edges touching this memory id (its neighbourhood).
    /// * `limit` — maximum number of edges (clamped to `1..=GRAPH_VIEW_MAX_LIMIT`).
    ///
    /// Only live (non-deprecated) memories appear, so edges that point at
    /// deprecated memories never reach a viewer.
    pub fn graph_view(
        &self,
        namespace: Option<&str>,
        node_id: Option<&str>,
        limit: usize,
    ) -> Result<GraphView, Error> {
        let limit = limit.clamp(1, GRAPH_VIEW_MAX_LIMIT);
        // One extra row tells us whether the view was truncated.
        let fetch = (limit + 1) as i64;
        let conn = &self.store.conn;

        // --- memory_edges between live memories ------------------------------
        // Fixed SQL: NULL parameters switch the optional filters off.
        let mut edges: Vec<GraphViewEdge> = Vec::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT e.id, e.source_id, e.target_id, e.edge_type, e.created_at \
                     FROM memory_edges e \
                     JOIN memories s ON s.id = e.source_id AND s.deprecated = 0 \
                     JOIN memories t ON t.id = e.target_id AND t.deprecated = 0 \
                     WHERE e.edge_type <> ?1 \
                       AND (?2 IS NULL OR (s.namespace = ?2 AND t.namespace = ?2)) \
                       AND (?3 IS NULL OR e.source_id = ?3 OR e.target_id = ?3) \
                     ORDER BY e.created_at DESC, e.id DESC LIMIT ?4",
                )
                .map_err(|e| Error::db("prepare graph_view memory edges", e))?;
            let rows = stmt
                .query_map(params![DERIVED_BACKLINK, namespace, node_id, fetch], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                    ))
                })
                .map_err(|e| Error::db("query graph_view memory edges", e))?;
            for row in rows {
                let (id, s, t, ty, at) =
                    row.map_err(|e| Error::db("read graph_view memory edge", e))?;
                edges.push(GraphViewEdge::new(
                    format!("me:{id}"),
                    &s,
                    &t,
                    &ty,
                    1.0,
                    &at,
                ));
            }
        }

        // --- explicit graph, re-keyed by memory id ---------------------------
        let explicit = self.graph_data(namespace)?;
        let node_key: HashMap<&str, &str> = explicit
            .nodes
            .iter()
            .map(|n| {
                (
                    n.id.as_str(),
                    n.memory_id.as_deref().unwrap_or(n.id.as_str()),
                )
            })
            .collect();
        for e in &explicit.edges {
            let (Some(&s), Some(&t)) = (
                node_key.get(e.source_id.as_str()),
                node_key.get(e.target_id.as_str()),
            ) else {
                continue;
            };
            if let Some(node) = node_id {
                if s != node && t != node {
                    continue;
                }
            }
            edges.push(GraphViewEdge::new(
                format!("ge:{}", e.id),
                s,
                t,
                &e.relation,
                e.weight,
                &e.created_at,
            ));
        }

        // Newest first across both sources, then bound.
        edges.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.id.cmp(&a.id))
        });
        let truncated = edges.len() > limit;
        edges.truncate(limit);

        // --- nodes: every endpoint of a returned edge ------------------------
        let mut ids: BTreeSet<&str> = BTreeSet::new();
        for e in &edges {
            ids.insert(e.source_id.as_str());
            ids.insert(e.target_id.as_str());
        }
        let explicit_by_key: HashMap<&str, &GraphNode> = explicit
            .nodes
            .iter()
            .map(|n| (n.memory_id.as_deref().unwrap_or(n.id.as_str()), n))
            .collect();

        let id_list = serde_json::to_string(&ids.iter().collect::<Vec<_>>())
            .map_err(|e| Error::Validation(format!("graph_view id list: {e}")))?;
        let mut memory_nodes: HashMap<String, GraphNode> = HashMap::new();
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, content, created_at FROM memories \
                     WHERE id IN (SELECT value FROM json_each(?1))",
                )
                .map_err(|e| Error::db("prepare graph_view nodes", e))?;
            let rows = stmt
                .query_map(params![id_list], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                    ))
                })
                .map_err(|e| Error::db("query graph_view nodes", e))?;
            for row in rows {
                let (id, content, created_at) =
                    row.map_err(|e| Error::db("read graph_view node", e))?;
                memory_nodes.insert(
                    id.clone(),
                    GraphNode {
                        label: memory_label(&content, &id),
                        entity_type: Some("memory".to_string()),
                        properties: serde_json::Value::Null,
                        memory_id: Some(id.clone()),
                        created_at,
                        id,
                    },
                );
            }
        }

        let mut nodes: Vec<GraphNode> = Vec::new();
        for id in &ids {
            if let Some(n) = memory_nodes.remove(*id) {
                nodes.push(n);
            } else if let Some(n) = explicit_by_key.get(id) {
                // Entity node of the explicit graph (no memory behind it).
                let mut n = (*n).clone();
                n.id = (*id).to_string();
                nodes.push(n);
            }
        }
        // Edges whose endpoint is neither a live memory nor a known entity
        // node cannot be drawn; drop them rather than ship dangling ids.
        let known: BTreeSet<&str> = nodes.iter().map(|n| n.id.as_str()).collect();
        edges.retain(|e| {
            known.contains(e.source_id.as_str()) && known.contains(e.target_id.as_str())
        });

        let mut relation_types: Vec<String> = edges.iter().map(|e| e.relation.clone()).collect();
        relation_types.sort();
        relation_types.dedup();
        let stats = GraphStats {
            node_count: nodes.len(),
            edge_count: edges.len(),
            relation_types,
        };

        Ok(GraphView {
            nodes,
            edges,
            stats,
            truncated,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GraphStore;

    fn open() -> Uteke {
        Uteke::open(":memory:").unwrap()
    }

    /// Embedder-free seeding (CI has no ONNX runtime). Vectors are opaque here.
    fn mem(u: &Uteke, text: &str, ns: &str) -> String {
        u.remember_precomputed(
            text,
            &[],
            None,
            Some(ns),
            "fact",
            "text",
            &vec![0.1_f32; 768],
        )
        .unwrap()
    }

    fn rels(v: &GraphView, rel: &str) -> Vec<(String, String)> {
        v.edges
            .iter()
            .filter(|e| e.relation == rel)
            .map(|e| (e.source_id.clone(), e.target_id.clone()))
            .collect()
    }

    #[test]
    fn memory_edges_appear_keyed_by_memory_id() {
        let u = open();
        let a = mem(&u, "alpha decision about caching", "gv");
        let b = mem(&u, "beta follow up", "gv");
        assert!(u.link_memories(&a, &b, "related").unwrap());

        let v = u.graph_view(Some("gv"), None, 100).unwrap();
        assert!(rels(&v, "related").contains(&(a.clone(), b.clone())));
        let ids: Vec<&str> = v.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&a.as_str()) && ids.contains(&b.as_str()));
        let node_a = v.nodes.iter().find(|n| n.id == a).unwrap();
        assert!(node_a.label.starts_with("alpha decision about caching"));
        assert_eq!(node_a.entity_type.as_deref(), Some("memory"));
        // the short names viewers read mirror the long ones
        let e = v.edges.iter().find(|e| e.relation == "related").unwrap();
        assert_eq!((&e.source, &e.target), (&e.source_id, &e.target_id));
        assert_eq!(v.stats.edge_count, v.edges.len());
        assert_eq!(v.stats.node_count, v.nodes.len());
    }

    #[test]
    fn derived_backlinks_deprecated_and_other_namespaces_are_left_out() {
        let u = open();
        let a = mem(&u, "live a", "gv1");
        let b = mem(&u, "live b", "gv1");
        let dead = mem(&u, "will be deprecated", "gv1");
        let other = mem(&u, "other namespace", "gv2");
        u.link_memories(&a, &b, "related").unwrap(); // also writes a referenced_by backlink
        u.link_memories(&a, &dead, "related").unwrap();
        u.link_memories(&a, &other, "related").unwrap();
        u.soft_forget(&dead, "test").unwrap();

        let v = u.graph_view(Some("gv1"), None, 100).unwrap();
        assert!(
            rels(&v, "referenced_by").is_empty(),
            "backlinks are omitted"
        );
        assert!(
            !v.edges
                .iter()
                .any(|e| e.target_id == dead || e.source_id == dead)
        );
        assert!(!v.nodes.iter().any(|n| n.id == dead || n.id == other));
        assert!(rels(&v, "related").contains(&(a.clone(), b.clone())));

        // Without a namespace the cross-namespace edge is visible, deprecated still not.
        let all = u.graph_view(None, None, 100).unwrap();
        assert!(rels(&all, "related").contains(&(a.clone(), other.clone())));
        assert!(!all.nodes.iter().any(|n| n.id == dead));
    }

    #[test]
    fn node_id_returns_only_that_neighbourhood() {
        let u = open();
        let a = mem(&u, "hub", "gv");
        let b = mem(&u, "spoke one", "gv");
        let c = mem(&u, "spoke two", "gv");
        let d = mem(&u, "unrelated x", "gv");
        let e = mem(&u, "unrelated y", "gv");
        u.link_memories(&a, &b, "related").unwrap();
        u.link_memories(&c, &a, "related").unwrap();
        u.link_memories(&d, &e, "related").unwrap();

        let v = u.graph_view(Some("gv"), Some(&a), 100).unwrap();
        let rel = rels(&v, "related");
        assert!(rel.contains(&(a.clone(), b.clone())) && rel.contains(&(c.clone(), a.clone())));
        assert!(!rel.contains(&(d.clone(), e.clone())));
        assert!(v.edges.iter().all(|x| x.source_id == a || x.target_id == a));
    }

    #[test]
    fn limit_bounds_edges_and_reports_truncation() {
        let u = open();
        let hub = mem(&u, "hub", "gv");
        for i in 0..6 {
            let m = mem(&u, &format!("spoke {i}"), "gv");
            u.link_memories(&hub, &m, "related").unwrap();
        }
        let few = u.graph_view(Some("gv"), None, 3).unwrap();
        assert_eq!(few.edges.len(), 3);
        assert!(few.truncated);
        // every returned edge has both endpoints in `nodes`
        let ids: std::collections::HashSet<&str> =
            few.nodes.iter().map(|n| n.id.as_str()).collect();
        assert!(
            few.edges
                .iter()
                .all(|e| ids.contains(e.source_id.as_str()) && ids.contains(e.target_id.as_str()))
        );
        let all = u.graph_view(Some("gv"), None, 1000).unwrap();
        assert!(!all.truncated);
        assert!(all.edges.len() >= 6);
        // limit 0 is clamped, not an error
        assert!(u.graph_view(None, None, 0).unwrap().edges.len() <= 1);
    }

    #[test]
    fn explicit_graph_edges_are_rekeyed_to_memory_ids() {
        let u = open();
        let a = mem(&u, "explicit a", "gv");
        let b = mem(&u, "explicit b", "gv");
        GraphStore::new(u.graph_store())
            .add_edge_for_memories(&a, &b, "contradicts", 0.9)
            .unwrap();

        let v = u.graph_view(Some("gv"), None, 100).unwrap();
        assert!(
            rels(&v, "contradicts").contains(&(a.clone(), b.clone())),
            "explicit edge must use memory ids, got {:?}",
            v.edges
        );
        let e = v
            .edges
            .iter()
            .find(|e| e.relation == "contradicts")
            .unwrap();
        assert!((e.weight - 0.9).abs() < 1e-9);
        // one node per memory even though both stores know it
        assert_eq!(v.nodes.iter().filter(|n| n.id == a).count(), 1);
    }

    /// Corin (`src-tauri/src/uteke_client.rs`) parses /graph into these
    /// shapes. Before #1366 edges carried only `source_id`/`target_id`, so a
    /// non-empty graph failed to deserialize there.
    #[test]
    fn response_deserializes_into_the_shape_corin_reads() {
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct CorinNode {
            id: String,
            label: String,
            entity_type: Option<String>,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct CorinEdge {
            source: String,
            target: String,
            relation: String,
            weight: f32,
        }
        #[derive(Deserialize)]
        #[allow(dead_code)]
        struct CorinGraph {
            nodes: Vec<CorinNode>,
            edges: Vec<CorinEdge>,
            stats: GraphStats,
        }

        let u = open();
        let a = mem(&u, "corin a", "gv");
        let b = mem(&u, "corin b", "gv");
        u.link_memories(&a, &b, "related").unwrap();
        let json = serde_json::to_value(u.graph_view(Some("gv"), None, 50).unwrap()).unwrap();
        let parsed: CorinGraph = serde_json::from_value(json).expect("Corin shape");
        assert!(parsed.edges.iter().any(|e| e.source == a && e.target == b));
    }

    #[test]
    fn add_edge_with_raw_memory_ids_violates_fk_but_the_helper_works() {
        // #1365: this is the exact call MCP uteke_graph_add_edge and dream made.
        let u = open();
        let a = mem(&u, "fk a", "gv");
        let b = mem(&u, "fk b", "gv");
        let gs = GraphStore::new(u.graph_store());
        assert!(
            gs.add_edge(&a, &b, "related", 1.0).is_err(),
            "raw memory ids must fail the graph_nodes FK"
        );
        gs.add_edge_for_memories(&a, &b, "related", 1.0).unwrap();
        assert!(
            gs.remove_edge_between(&a, &b).unwrap(),
            "removable by memory ids"
        );
        assert!(
            !gs.remove_edge_between(&a, &b).unwrap(),
            "second removal is a no-op"
        );
        assert!(!gs.remove_edge_between("nope", &b).unwrap());
    }
}
