//! Relationship graph traversal — follow edges stored in memory metadata.
//!
//! Relationships are stored in the `metadata` JSON field under a
//! `relationships` key, e.g.:
//! ```json
//! {"relationships": [{"type": "supersedes", "target": "uuid-xxx"}, ...]}
//! ```

use crate::error::Error;
use crate::memory::types::{Memory, SearchResult};
use std::collections::{HashMap, HashSet};

/// Known relationship types.
pub const REL_SUPERSEDES: &str = "supersedes";
pub const REL_CONTRADICTS: &str = "contradicts";
pub const REL_PART_OF: &str = "part_of";
pub const REL_REFERENCES: &str = "references";

/// All valid relationship type strings.
pub const VALID_REL_TYPES: &[&str] =
    &[REL_SUPERSEDES, REL_CONTRADICTS, REL_PART_OF, REL_REFERENCES];

/// A single relationship edge.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Relationship {
    #[serde(rename = "type")]
    pub rel_type: String,
    pub target: String,
}

/// Parse relationships from memory metadata.
fn parse_relationships(memory: &Memory) -> Vec<Relationship> {
    memory
        .metadata
        .get("relationships")
        .and_then(|v| serde_json::from_value::<Vec<Relationship>>(v.clone()).ok())
        .unwrap_or_default()
}

/// Build a relationship string for storing in metadata.
pub fn build_meta_relationship(rel_type: &str, target_id: &str) -> String {
    format!("rel:{rel_type}:{target_id}")
}

/// Check if a meta value is a relationship directive.
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
    /// Recall memories and follow relationship edges up to `depth` levels.
    ///
    /// Starts with the results of a normal recall, then traverses the
    /// `relationships` field in each memory's metadata to find related
    /// memories. Deduplicates by memory ID.
    pub fn recall_related(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        min_score: f32,
        depth: usize,
    ) -> Result<Vec<SearchResult>, Error> {
        // Initial recall
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

        // Collect all IDs to visit and track scores
        let mut visited: HashSet<String> = HashSet::new();
        let mut results: HashMap<String, (Memory, f32)> = HashMap::new();

        for sr in &initial {
            visited.insert(sr.memory.id.clone());
            results.insert(sr.memory.id.clone(), (sr.memory.clone(), sr.score));
        }

        // BFS traversal
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
                    // Missing targets not marked visited — allows alternate paths.
                }
            }

            tracing::debug!(
                "Relationship traversal level {}: found {} new memories",
                level + 1,
                next_frontier.len()
            );
            frontier = next_frontier;
        }

        // Convert to sorted results
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

    /// Get all memories related to a specific memory ID via relationship edges.
    /// Returns both outgoing and incoming relationships.
    pub fn get_related(&self, memory_id: &str) -> Result<Vec<Memory>, Error> {
        let mut related = Vec::new();
        let mut seen = HashSet::new();
        seen.insert(memory_id.to_string());

        // Outgoing: relationships in this memory's metadata
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

        // Incoming: scan memories that reference this one
        // This is O(n) on metadata — acceptable for now (see AGENT.md: metadata in JSON blob)
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
