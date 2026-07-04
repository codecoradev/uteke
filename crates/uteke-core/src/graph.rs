//! Knowledge graph module — relationship traversal + SQLite graph storage.
//!
//! Two systems coexist:
//! 1. **Relationship edges** — stored in memory metadata JSON (v0.1.0, #246)
//! 2. **Graph storage** — graph_nodes + graph_edges tables (v0.2.0, #317)
//!
//! The metadata-based system is for lightweight per-memory relationships
//! (supersedes, contradicts, etc). The table-based system is for rich
//! entity graphs with multi-hop traversal.

use crate::error::Error;
use crate::memory::types::{Memory, SearchResult};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};

// ── Known relationship types (metadata-based) ──────────────────────────

pub const REL_SUPERSEDES: &str = "supersedes";
pub const REL_CONTRADICTS: &str = "contradicts";
pub const REL_PART_OF: &str = "part_of";
pub const REL_REFERENCES: &str = "references";

pub const VALID_REL_TYPES: &[&str] =
    &[REL_SUPERSEDES, REL_CONTRADICTS, REL_PART_OF, REL_REFERENCES];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Relationship {
    #[serde(rename = "type")]
    pub rel_type: String,
    pub target: String,
}

fn parse_relationships(memory: &Memory) -> Vec<Relationship> {
    memory
        .metadata
        .get("relationships")
        .and_then(|v| serde_json::from_value::<Vec<Relationship>>(v.clone()).ok())
        .unwrap_or_default()
}

pub fn build_meta_relationship(rel_type: &str, target_id: &str) -> String {
    format!("rel:{rel_type}:{target_id}")
}

pub fn is_relationship_meta(value: &str) -> Option<(&str, &str)> {
    let rest = value.strip_prefix("rel:")?;
    let (rel_type, target) = rest.split_once(':')?;
    if VALID_REL_TYPES.contains(&rel_type) {
        Some((rel_type, target))
    } else {
        None
    }
}

impl crate::Uteke {
    pub fn recall_related(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
        depth: usize,
    ) -> Result<Vec<SearchResult>, Error> {
        let initial = self.recall_hybrid(
            query,
            limit,
            tags_filter,
            namespace,
            crate::memory::types::RecallStrategy::Hybrid,
            min_score,
        )?;

        if depth == 0 || initial.is_empty() {
            return Ok(initial);
        }

        let mut visited: HashSet<String> = HashSet::new();
        let mut results: HashMap<String, (Memory, f32)> = HashMap::new();

        for sr in &initial {
            visited.insert(sr.memory.id.clone());
            results.insert(sr.memory.id.clone(), (sr.memory.clone(), sr.score));
        }

        let mut frontier: Vec<String> = initial.iter().map(|sr| sr.memory.id.clone()).collect();

        for level in 0..depth {
            if frontier.is_empty() {
                break;
            }
            let mut next_frontier: Vec<String> = Vec::new();
            for memory_id in &frontier {
                let memory = match self.get_by_id(memory_id)? {
                    Some(m) => m,
                    None => continue,
                };
                let rels = parse_relationships(&memory);
                for rel in rels {
                    if visited.contains(&rel.target) {
                        continue;
                    }
                    if let Some(target_memory) = self.get_by_id(&rel.target)? {
                        visited.insert(rel.target.clone());
                        let decayed_score = (results[memory_id].1 * 0.8).max(0.1);
                        results.insert(rel.target.clone(), (target_memory.clone(), decayed_score));
                        next_frontier.push(rel.target.clone());
                    }
                }
            }
            tracing::debug!(
                "Relationship traversal level {}: found {} new memories",
                level + 1,
                next_frontier.len()
            );
            frontier = next_frontier;
        }

        let mut all_results: Vec<SearchResult> = results
            .into_iter()
            .map(|(_, (memory, score))| SearchResult { memory, score })
            .collect();
        all_results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_results.truncate(limit);
        Ok(all_results)
    }

    pub fn get_related(&self, memory_id: &str) -> Result<Vec<Memory>, Error> {
        // Prefer edge table (v8, #346) — indexed SQL, O(log n) per hop.
        // Union with legacy JSON metadata scan so we never lose relations
        // that exist in metadata but not (yet) in the edge table (e.g. a
        // store that predates v8 auto-wiring, or refs we couldn't resolve).
        let mut related = self.related_via_edges(memory_id, 1)?;
        let mut seen: HashSet<String> = related.iter().map(|m| m.id.clone()).collect();
        seen.insert(memory_id.to_string());

        if let Some(memory) = self.get_by_id(memory_id)? {
            for rel in parse_relationships(&memory) {
                if !seen.contains(&rel.target) {
                    if let Some(target) = self.get_by_id(&rel.target)? {
                        seen.insert(rel.target.clone());
                        related.push(target);
                    }
                }
            }
        }

        let all_memories = self.store.load_all(None)?;
        for m in all_memories {
            if seen.contains(&m.id) {
                continue;
            }
            for rel in parse_relationships(&m) {
                if rel.target == memory_id {
                    seen.insert(m.id.clone());
                    related.push(m);
                    break;
                }
            }
        }
        Ok(related)
    }
}

// ── SQLite graph storage (#317) ─────────────────────────────────────

/// A graph node representing an entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub label: String,
    pub entity_type: Option<String>,
    pub properties: serde_json::Value,
    pub memory_id: Option<String>,
    pub created_at: String,
}

/// A directed edge between two nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    pub id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation: String,
    pub weight: f64,
    pub created_at: String,
}

/// A path between two nodes via BFS.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphPath {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    pub total_weight: f64,
}

/// (source, edge, target) triple for relation queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphTriple {
    pub source: GraphNode,
    pub edge: GraphEdge,
    pub target: GraphNode,
}

/// Graph statistics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub node_count: usize,
    pub edge_count: usize,
    pub relation_types: Vec<String>,
}

/// Graph store backed by SQLite. Borrows a connection from the main Store.
pub struct GraphStore<'a> {
    conn: &'a Connection,
}

impl<'a> GraphStore<'a> {
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    /// Get or create a node by label. Returns node ID.
    /// Idempotent — if a node with the same label exists, returns its ID.
    pub fn upsert_node(
        &self,
        label: &str,
        entity_type: Option<&str>,
        memory_id: Option<&str>,
    ) -> Result<String, Error> {
        let existing: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM graph_nodes WHERE label = ?1 COLLATE NOCASE",
                params![label],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::db("graph node lookup", e))?;

        if let Some(id) = existing {
            // Only set memory_id if not already set (don't overwrite existing link).
            if let Some(mid) = memory_id {
                self.conn
                    .execute(
                        "UPDATE graph_nodes SET memory_id = ?1 WHERE id = ?2 AND memory_id IS NULL",
                        params![mid, id],
                    )
                    .map_err(|e| Error::db("update graph node", e))?;
            }
            return Ok(id);
        }

        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO graph_nodes (id, label, entity_type, properties_json, memory_id, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, label, entity_type, "{}", memory_id, now],
            )
            .map_err(|e| Error::db("insert graph node", e))?;
        Ok(id)
    }

    /// Create an edge between two nodes. Ignores if edge already exists (INSERT OR IGNORE).
    pub fn add_edge(
        &self,
        source_id: &str,
        target_id: &str,
        relation: &str,
        weight: f64,
    ) -> Result<(), Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let affected = self
            .conn
            .execute(
                "INSERT OR IGNORE INTO graph_edges (id, source_id, target_id, relation, weight, created_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![id, source_id, target_id, relation, weight, now],
            )
            .map_err(|e| Error::db("insert graph edge", e))?;
        if affected == 0 {
            tracing::debug!(
                "Edge already exists: {} -[{}]-> {} (new weight {weight} ignored)",
                source_id,
                relation,
                target_id
            );
        }
        Ok(())
    }

    /// Remove an edge between two nodes by source and target IDs.
    /// Returns true if an edge was removed, false if none existed.
    pub fn remove_edge(&self, source_id: &str, target_id: &str) -> Result<bool, Error> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM graph_edges WHERE source_id = ?1 AND target_id = ?2",
                params![source_id, target_id],
            )
            .map_err(|e| Error::db("remove graph edge", e))?;
        Ok(affected > 0)
    }

    /// Find a node by label (case-insensitive).
    pub fn find_node(&self, label: &str) -> Result<Option<GraphNode>, Error> {
        self.conn
            .query_row(
                "SELECT id, label, entity_type, properties_json, memory_id, created_at \
                 FROM graph_nodes WHERE label = ?1 COLLATE NOCASE",
                params![label],
                row_to_node,
            )
            .optional()
            .map_err(|e| Error::db("find graph node", e))
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: &str) -> Result<Option<GraphNode>, Error> {
        self.conn
            .query_row(
                "SELECT id, label, entity_type, properties_json, memory_id, created_at \
                 FROM graph_nodes WHERE id = ?1",
                params![id],
                row_to_node,
            )
            .optional()
            .map_err(|e| Error::db("get graph node", e))
    }

    /// Get outgoing edges from a node, up to `max_depth` hops via BFS.
    pub fn neighbors(&self, node_id: &str, max_depth: usize) -> Result<Vec<GraphEdge>, Error> {
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(node_id.to_string());
        let mut result: Vec<GraphEdge> = Vec::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        queue.push_back((node_id.to_string(), 0));

        while let Some((nid, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let edges = self.outgoing_edges(&nid)?;
            for edge in edges {
                result.push(edge.clone());
                if visited.insert(edge.target_id.clone()) {
                    queue.push_back((edge.target_id, depth + 1));
                }
            }
        }
        Ok(result)
    }

    /// Find shortest path between two nodes using BFS.
    pub fn find_path(
        &self,
        source_label: &str,
        target_label: &str,
        max_depth: usize,
    ) -> Result<Option<GraphPath>, Error> {
        let source = match self.find_node(source_label)? {
            Some(n) => n,
            None => return Ok(None),
        };
        let target = match self.find_node(target_label)? {
            Some(n) => n,
            None => return Ok(None),
        };

        if source.id == target.id {
            return Ok(Some(GraphPath {
                nodes: vec![source],
                edges: vec![],
                total_weight: 0.0,
            }));
        }

        // BFS with parent tracking
        let mut visited: HashSet<String> = HashSet::new();
        visited.insert(source.id.clone());
        let mut parent: HashMap<String, (String, GraphEdge)> = HashMap::new();
        let mut queue: VecDeque<(String, usize)> = VecDeque::new();
        queue.push_back((source.id.clone(), 0));

        while let Some((nid, depth)) = queue.pop_front() {
            if depth >= max_depth {
                continue;
            }
            let edges = self.outgoing_edges(&nid)?;
            for edge in edges {
                if visited.insert(edge.target_id.clone()) {
                    parent.insert(edge.target_id.clone(), (nid.clone(), edge.clone()));
                    if edge.target_id == target.id {
                        // Reconstruct path
                        return Ok(Some(self.reconstruct_path(&source, &target, &parent)?));
                    }
                    queue.push_back((edge.target_id, depth + 1));
                }
            }
        }
        Ok(None)
    }

    /// Query all (source, edge, target) triples matching a relation type.
    pub fn query_relation(&self, relation: &str) -> Result<Vec<GraphTriple>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, s.label, s.entity_type, s.properties_json, s.memory_id, s.created_at, \
                        e.id, e.source_id, e.target_id, e.relation, e.weight, e.created_at, \
                        t.id, t.label, t.entity_type, t.properties_json, t.memory_id, t.created_at \
                 FROM graph_edges e \
                 JOIN graph_nodes s ON e.source_id = s.id \
                 JOIN graph_nodes t ON e.target_id = t.id \
                 WHERE e.relation = ?1 COLLATE NOCASE",
            )
            .map_err(|e| Error::db("prepare relation query", e))?;

        let rows = stmt
            .query_map(params![relation], |row| {
                Ok(GraphTriple {
                    source: row_to_node_at(row, 0)?,
                    edge: row_to_edge_at(row, 6)?,
                    target: row_to_node_at(row, 12)?,
                })
            })
            .map_err(|e| Error::db("query relation", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("collect relation rows", e))
    }

    /// Get all nodes.
    pub fn all_nodes(&self) -> Result<Vec<GraphNode>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, label, entity_type, properties_json, memory_id, created_at FROM graph_nodes ORDER BY label",
            )
            .map_err(|e| Error::db("prepare all nodes", e))?;

        let rows = stmt
            .query_map([], row_to_node)
            .map_err(|e| Error::db("query all nodes", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("collect nodes", e))
    }

    /// Get all edges.
    pub fn all_edges(&self) -> Result<Vec<GraphEdge>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source_id, target_id, relation, weight, created_at FROM graph_edges ORDER BY created_at",
            )
            .map_err(|e| Error::db("prepare all edges", e))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(GraphEdge {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    target_id: row.get(2)?,
                    relation: row.get(3)?,
                    weight: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| Error::db("query all edges", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("collect edges", e))
    }

    /// Get graph statistics.
    pub fn stats(&self) -> Result<GraphStats, Error> {
        let node_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM graph_nodes", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|e| Error::db("count nodes", e))? as usize;

        let edge_count: usize = self
            .conn
            .query_row("SELECT COUNT(*) FROM graph_edges", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(|e| Error::db("count edges", e))? as usize;

        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT relation FROM graph_edges ORDER BY relation")
            .map_err(|e| Error::db("prepare relation types", e))?;

        let relation_types: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| Error::db("query relation types", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("collect relation types", e))?;

        Ok(GraphStats {
            node_count,
            edge_count,
            relation_types,
        })
    }

    // --- Internal helpers ---

    fn outgoing_edges(&self, node_id: &str) -> Result<Vec<GraphEdge>, Error> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, source_id, target_id, relation, weight, created_at \
                 FROM graph_edges WHERE source_id = ?1",
            )
            .map_err(|e| Error::db("prepare outgoing edges", e))?;

        let rows = stmt
            .query_map(params![node_id], |row| {
                Ok(GraphEdge {
                    id: row.get(0)?,
                    source_id: row.get(1)?,
                    target_id: row.get(2)?,
                    relation: row.get(3)?,
                    weight: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })
            .map_err(|e| Error::db("query outgoing edges", e))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("collect outgoing edges", e))
    }

    fn reconstruct_path(
        &self,
        source: &GraphNode,
        target: &GraphNode,
        parent: &HashMap<String, (String, GraphEdge)>,
    ) -> Result<GraphPath, Error> {
        let mut nodes = vec![target.clone()];
        let mut edges: Vec<GraphEdge> = Vec::new();
        let mut total_weight = 0.0;

        let mut current = target.id.clone();
        while current != source.id {
            let (parent_id, edge) = match parent.get(&current) {
                Some(p) => p.clone(),
                None => break,
            };
            edges.push(edge.clone());
            total_weight += edge.weight;
            if let Some(node) = self.get_node(&parent_id)? {
                nodes.push(node);
            }
            current = parent_id;
        }

        nodes.reverse();
        edges.reverse();

        Ok(GraphPath {
            nodes,
            edges,
            total_weight,
        })
    }
}

// --- Row mappers ---

fn row_to_node(row: &rusqlite::Row) -> rusqlite::Result<GraphNode> {
    let props_str: String = row.get(3)?;
    let properties = serde_json::from_str(&props_str).unwrap_or(serde_json::json!({}));
    Ok(GraphNode {
        id: row.get(0)?,
        label: row.get(1)?,
        entity_type: row.get(2)?,
        properties,
        memory_id: row.get(4)?,
        created_at: row.get(5)?,
    })
}

fn row_to_node_at(row: &rusqlite::Row, offset: usize) -> rusqlite::Result<GraphNode> {
    let props_str: String = row.get(offset + 3)?;
    let properties = serde_json::from_str(&props_str).unwrap_or(serde_json::json!({}));
    Ok(GraphNode {
        id: row.get(offset)?,
        label: row.get(offset + 1)?,
        entity_type: row.get(offset + 2)?,
        properties,
        memory_id: row.get(offset + 4)?,
        created_at: row.get(offset + 5)?,
    })
}

fn row_to_edge_at(row: &rusqlite::Row, offset: usize) -> rusqlite::Result<GraphEdge> {
    Ok(GraphEdge {
        id: row.get(offset)?,
        source_id: row.get(offset + 1)?,
        target_id: row.get(offset + 2)?,
        relation: row.get(offset + 3)?,
        weight: row.get(offset + 4)?,
        created_at: row.get(offset + 5)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        // Don't enable foreign_keys in test — graph_nodes references memories(id)
        // which doesn't exist in this minimal test setup.
        conn.execute_batch(
            r#"
            CREATE TABLE graph_nodes (
                id TEXT PRIMARY KEY,
                label TEXT NOT NULL COLLATE NOCASE,
                entity_type TEXT,
                properties_json TEXT DEFAULT '{}',
                memory_id TEXT,
                created_at TEXT NOT NULL
            );
            CREATE TABLE graph_edges (
                id TEXT PRIMARY KEY,
                source_id TEXT NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
                target_id TEXT NOT NULL REFERENCES graph_nodes(id) ON DELETE CASCADE,
                relation TEXT NOT NULL COLLATE NOCASE,
                weight REAL NOT NULL DEFAULT 1.0,
                created_at TEXT NOT NULL,
                UNIQUE(source_id, target_id, relation)
            );
            CREATE INDEX idx_graph_nodes_label ON graph_nodes(label);
            CREATE INDEX idx_graph_edges_source ON graph_edges(source_id);
            CREATE INDEX idx_graph_edges_target ON graph_edges(target_id);
            "#,
        )
        .unwrap();
        conn
    }

    #[test]
    fn test_upsert_node_creates_new() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let id = g.upsert_node("Alice", Some("person"), None).unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn test_upsert_node_idempotent() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let id1 = g.upsert_node("Alice", None, None).unwrap();
        let id2 = g.upsert_node("Alice", None, None).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_upsert_node_case_insensitive() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let id1 = g.upsert_node("Alice", None, None).unwrap();
        let id2 = g.upsert_node("alice", None, None).unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_add_edge() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        let edges = g.all_edges().unwrap();
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].relation, "knows");
    }

    #[test]
    fn test_add_edge_idempotent() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap(); // INSERT OR IGNORE
        let edges = g.all_edges().unwrap();
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn test_find_node() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        g.upsert_node("Alice", Some("person"), None).unwrap();
        let node = g.find_node("alice").unwrap();
        assert!(node.is_some());
        assert_eq!(node.unwrap().label, "Alice");
    }

    #[test]
    fn test_find_node_not_found() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let node = g.find_node("Nobody").unwrap();
        assert!(node.is_none());
    }

    #[test]
    fn test_neighbors_direct() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        let carol = g.upsert_node("Carol", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        g.add_edge(&alice, &carol, "manages", 1.0).unwrap();
        let neighbors = g.neighbors(&alice, 1).unwrap();
        assert_eq!(neighbors.len(), 2);
    }

    #[test]
    fn test_neighbors_multi_hop() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        let carol = g.upsert_node("Carol", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        g.add_edge(&bob, &carol, "knows", 1.0).unwrap();
        let neighbors = g.neighbors(&alice, 2).unwrap();
        assert_eq!(neighbors.len(), 2); // alice→bob, bob→carol
    }

    #[test]
    fn test_find_path_direct() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        let path = g.find_path("Alice", "Bob", 5).unwrap();
        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.nodes.len(), 2);
        assert_eq!(path.edges.len(), 1);
    }

    #[test]
    fn test_find_path_multi_hop() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        let carol = g.upsert_node("Carol", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        g.add_edge(&bob, &carol, "knows", 1.0).unwrap();
        let path = g.find_path("Alice", "Carol", 5).unwrap();
        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.nodes.len(), 3);
        assert_eq!(path.edges.len(), 2);
    }

    #[test]
    fn test_find_path_not_found() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        g.upsert_node("Alice", None, None).unwrap();
        g.upsert_node("Bob", None, None).unwrap();
        let path = g.find_path("Alice", "Bob", 5).unwrap();
        assert!(path.is_none()); // no edge
    }

    #[test]
    fn test_find_path_same_node() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        g.upsert_node("Alice", None, None).unwrap();
        let path = g.find_path("Alice", "Alice", 5).unwrap();
        assert!(path.is_some());
        assert_eq!(path.unwrap().nodes.len(), 1);
    }

    #[test]
    fn test_query_relation() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        let project = g.upsert_node("ProjectX", Some("project"), None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        g.add_edge(&alice, &project, "owns", 1.0).unwrap();
        g.add_edge(&bob, &project, "contributes", 1.0).unwrap();

        let triples = g.query_relation("owns").unwrap();
        assert_eq!(triples.len(), 1);
        assert_eq!(triples[0].source.label, "Alice");
        assert_eq!(triples[0].target.label, "ProjectX");
    }

    #[test]
    fn test_remove_edge() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        assert_eq!(g.all_edges().unwrap().len(), 1);

        let removed = g.remove_edge(&alice, &bob).unwrap();
        assert!(removed);
        assert_eq!(g.all_edges().unwrap().len(), 0);
    }

    #[test]
    fn test_remove_edge_nonexistent() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        let removed = g.remove_edge(&alice, &bob).unwrap();
        assert!(!removed);
    }

    #[test]
    fn test_stats() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        let alice = g.upsert_node("Alice", None, None).unwrap();
        let bob = g.upsert_node("Bob", None, None).unwrap();
        let project = g.upsert_node("ProjectX", None, None).unwrap();
        g.add_edge(&alice, &bob, "knows", 1.0).unwrap();
        g.add_edge(&alice, &project, "owns", 1.0).unwrap();

        let stats = g.stats().unwrap();
        assert_eq!(stats.node_count, 3);
        assert_eq!(stats.edge_count, 2);
        assert_eq!(stats.relation_types.len(), 2);
    }

    #[test]
    fn test_all_nodes() {
        let conn = setup();
        let g = GraphStore::new(&conn);
        g.upsert_node("Charlie", None, None).unwrap();
        g.upsert_node("Alice", None, None).unwrap();
        g.upsert_node("Bob", None, None).unwrap();
        let nodes = g.all_nodes().unwrap();
        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].label, "Alice"); // sorted
    }
}
