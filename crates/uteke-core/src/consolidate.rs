//! Consolidation: contradiction detection, duplicate finding, and merging.

use crate::error::Error;
use crate::memory::types::{ContradictionResult, DEFAULT_NAMESPACE};

impl crate::Uteke {
    /// Check for contradictions when storing a new memory.
    ///
    /// Read-only: searches the vector index for potential contradictions
    /// but does NOT mutate anything. The caller is responsible for
    /// deprecating the old memory after the new one is safely persisted.
    pub fn check_contradiction(
        &self,
        content: &str,
        embedding: &[f32],
        namespace: &str,
        threshold: f32,
    ) -> Result<ContradictionResult, Error> {
        let results = {
            let index = self
                .index
                .read()
                .map_err(|_| Error::lock("index read lock during contradiction check"))?;
            index.search(embedding, 5, 50)
        };

        for (id, distance) in &results {
            let similarity = 1.0 - distance;
            if similarity > threshold {
                if let Ok(Some(memory)) = self.store.get_by_id(id) {
                    if memory.namespace == namespace && !memory.deprecated {
                        tracing::info!(
                            "Contradiction detected (sim={:.3}): will deprecate '{}' → replace by '{}'",
                            similarity,
                            memory.content.chars().take(60).collect::<String>(),
                            content.chars().take(60).collect::<String>()
                        );
                        return Ok(ContradictionResult {
                            contradicted: true,
                            deprecated_id: Some(id.clone()),
                            similarity,
                        });
                    }
                }
            }
        }

        Ok(ContradictionResult {
            contradicted: false,
            deprecated_id: None,
            similarity: 0.0,
        })
    }

    /// Store a memory with contradiction detection and temporal metadata.
    ///
    /// Returns the ID of the new memory and any contradiction result.
    ///
    /// Reuses `remember()` for the actual insert — single code path for persistence.
    pub fn remember_with_contradiction(
        &self,
        content: &str,
        tags: &[&str],
        namespace: Option<&str>,
        memory_type: Option<&str>,
        check_contradiction: bool,
    ) -> Result<(String, ContradictionResult), Error> {
        crate::validate_input(content, tags)?;
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);

        // Embed first to check for contradictions before persisting
        let embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during remember_with_contradiction"))?
            .embed(content)?;

        // Check for contradictions (read-only)
        let contradiction = if check_contradiction {
            self.check_contradiction(content, &embedding, ns, 0.65)?
        } else {
            ContradictionResult {
                contradicted: false,
                deprecated_id: None,
                similarity: 0.0,
            }
        };

        // Use remember_precomputed to avoid double-embedding
        let id = self.remember_precomputed(
            content,
            tags,
            None,
            Some(ns),
            memory_type.unwrap_or("fact"),
            &embedding,
        )?;

        // Only deprecate the old memory AFTER the new one is safely persisted.
        // If deprecation fails, the new memory is still valid — the old one will
        // be deprecated on the next contradiction check. Non-fatal.
        if contradiction.contradicted {
            if let Some(ref deprecated_id) = contradiction.deprecated_id {
                if let Err(e) = self.store.deprecate(deprecated_id) {
                    tracing::warn!(
                        "Failed to deprecate {deprecated_id} after contradiction (new id={id}): {e}. \
                         New memory is safe. Old memory may be deprecated on next check."
                    );
                } else {
                    // Remove from vector index so it won't appear in future searches.
                    let mut idx = self.index.write().map_err(|_| {
                        Error::lock("index write lock during post-insert contradiction deprecation")
                    })?;
                    if idx.remove(deprecated_id) {
                        if let Err(e) = idx.save() {
                            tracing::warn!(
                                "Failed to persist vector index after deprecating id={deprecated_id}: {e}"
                            );
                        }
                    }
                }
            }
        }

        Ok((id, contradiction))
    }

    /// Find near-duplicate memory pairs (similarity > threshold).
    /// Uses the HNSW vector index for approximate search — O(n·k) instead of O(n²).
    pub fn find_duplicates(
        &self,
        namespace: Option<&str>,
        threshold: f32,
    ) -> Result<Vec<crate::memory::types::SimilarPair>, Error> {
        let memories = self.store.load_all(namespace)?;
        if memories.is_empty() {
            return Ok(Vec::new());
        }

        let index = self
            .index
            .read()
            .map_err(|_| Error::lock("index read lock during find_duplicates"))?;

        let mut seen = std::collections::HashSet::new();
        let mut pairs = Vec::new();

        for memory in &memories {
            if memory.embedding.is_empty() {
                continue;
            }

            // Search for top-2 nearest neighbors (first result may be self)
            let candidates = index.search(&memory.embedding, 5, 50);

            for (candidate_id, distance) in &candidates {
                if candidate_id == &memory.id {
                    continue;
                }
                let sim = 1.0 - distance;
                if sim <= threshold {
                    continue;
                }

                // Canonical pair ordering to avoid duplicates
                let (id_a, id_b) = if memory.id < *candidate_id {
                    (memory.id.clone(), candidate_id.clone())
                } else {
                    (candidate_id.clone(), memory.id.clone())
                };
                let pair_key = format!("{id_a}:{id_b}");
                if seen.contains(&pair_key) {
                    continue;
                }

                // Fetch candidate content for preview
                let content_b = self
                    .store
                    .get_by_id(candidate_id)
                    .ok()
                    .flatten()
                    .map(|m| m.content.chars().take(80).collect())
                    .unwrap_or_default();

                seen.insert(pair_key);
                pairs.push(crate::memory::types::SimilarPair {
                    id_a: id_a.clone(),
                    content_a: memory.content.chars().take(80).collect(),
                    id_b: id_b.clone(),
                    content_b,
                    similarity: sim,
                });
            }
        }

        pairs.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(pairs)
    }

    /// Consolidate near-duplicate memories (keeps newer, removes older).
    pub fn consolidate(
        &self,
        namespace: Option<&str>,
        threshold: f32,
        dry_run: bool,
    ) -> Result<crate::memory::types::ConsolidationResult, Error> {
        let pairs = self.find_duplicates(namespace, threshold)?;
        if pairs.is_empty() || dry_run {
            return Ok(crate::memory::types::ConsolidationResult {
                duplicates_found: pairs.len(),
                merged: 0,
                removed_ids: vec![],
                kept_ids: vec![],
            });
        }
        let mut removed_ids = Vec::new();
        let mut kept_ids = Vec::new();
        let mut already_removed = std::collections::HashSet::new();
        for pair in &pairs {
            if already_removed.contains(&pair.id_a) || already_removed.contains(&pair.id_b) {
                continue;
            }

            // Explicitly compare timestamps — delete the older memory, keep the newer.
            // Falls back to id_a if timestamps are equal.
            let (to_remove, to_keep) = {
                let mem_a = self.store.get_by_id(&pair.id_a);
                let mem_b = self.store.get_by_id(&pair.id_b);
                match (mem_a, mem_b) {
                    (Ok(Some(a)), Ok(Some(b))) => {
                        if b.created_at < a.created_at {
                            (&pair.id_b, &pair.id_a) // b is older → remove b
                        } else {
                            (&pair.id_a, &pair.id_b) // a is older or equal → remove a
                        }
                    }
                    _ => (&pair.id_a, &pair.id_b), // fallback: remove id_a
                }
            };

            self.store
                .delete(to_remove)
                .map_err(|e| Error::db("consolidate delete", e))?;
            // SQLite first (source of truth), then vector index.
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during consolidate"))?;
            if !index.remove(to_remove) {
                tracing::warn!(
                    "Vector index entry not found during consolidate for id={}",
                    to_remove
                );
            }
            if let Err(e) = index.save() {
                tracing::warn!(
                    "Failed to persist vector index after consolidate: {e}. \
                     Orphan entries will be cleaned up by verify/repair."
                );
            }
            removed_ids.push(to_remove.clone());
            kept_ids.push(to_keep.clone());
            already_removed.insert(to_remove.clone());
        }
        Ok(crate::memory::types::ConsolidationResult {
            duplicates_found: pairs.len(),
            merged: removed_ids.len(),
            removed_ids,
            kept_ids,
        })
    }
}

/// Compute cosine similarity between two vectors.
#[allow(dead_code)]
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::{ConsolidationResult, SimilarPair};

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 0.0, 0.0, 0.0, 0.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        assert!((cosine_similarity(&a, &b)).abs() < f32::EPSILON);
    }

    #[test]
    fn test_cosine_similarity_empty() {
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_different_lengths() {
        let a = vec![1.0, 2.0];
        let b = vec![1.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0, 0.0];
        let b = vec![1.0, 2.0, 3.0];
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![-1.0, 0.0, 0.0];
        // cosine_similarity clamps to [0, 1]
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_consolidation_result_serialization() {
        let result = ConsolidationResult {
            duplicates_found: 3,
            merged: 2,
            removed_ids: vec!["old1".to_string(), "old2".to_string()],
            kept_ids: vec!["new1".to_string(), "new2".to_string()],
        };
        let json = serde_json::to_string(&result).unwrap();
        let restored: ConsolidationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.duplicates_found, 3);
        assert_eq!(restored.merged, 2);
    }

    #[test]
    fn test_similar_pair_serialization() {
        let pair = SimilarPair {
            id_a: "a".to_string(),
            content_a: "hello world foo bar baz extra long content preview".to_string(),
            id_b: "b".to_string(),
            content_b: "hello world different content".to_string(),
            similarity: 0.95,
        };
        let json = serde_json::to_string(&pair).unwrap();
        let restored: SimilarPair = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.similarity, 0.95);
        assert!(restored.content_a.len() <= 80);
    }

    #[test]
    fn test_contradiction_result_serialization() {
        use crate::memory::types::ContradictionResult;
        let result = ContradictionResult {
            contradicted: true,
            deprecated_id: Some("old-id".to_string()),
            similarity: 0.85,
        };
        let json = serde_json::to_string(&result).unwrap();
        let restored: ContradictionResult = serde_json::from_str(&json).unwrap();
        assert!(restored.contradicted);
        assert_eq!(restored.similarity, 0.85);
    }
}
