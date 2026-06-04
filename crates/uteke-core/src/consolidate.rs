//! Consolidation: contradiction detection, duplicate finding, and merging.

use crate::error::Error;
use crate::memory::types::{ContradictionResult, Memory, DEFAULT_NAMESPACE};

impl crate::Uteke {
    /// Check for contradictions when storing a new memory.
    ///
    /// Compares new embedding against existing memories in the same namespace.
    /// If similarity > threshold (0.65), marks the old memory as deprecated.
    pub fn check_contradiction(
        &self,
        content: &str,
        embedding: &[f32],
        namespace: &str,
        threshold: f32,
    ) -> Result<ContradictionResult, Error> {
        // Search the vector index for potential contradictions.
        let results = {
            let index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during contradiction check"))?;
            index.search(embedding, 5, 50)
        };
        // Lock released before we potentially re-acquire for mutation below.

        for (id, distance) in &results {
            let similarity = 1.0 - distance;
            if similarity > threshold {
                if let Ok(Some(memory)) = self.store.get_by_id(id) {
                    if memory.namespace == namespace && !memory.deprecated {
                        self.store.deprecate(id)?;
                        // Also remove deprecated memory from vector index so
                        // it doesn't appear in future similarity searches.
                        // Re-acquire lock for mutation (safe: search lock was dropped above).
                        let mut idx = self.index.lock().map_err(|_| {
                            Error::lock("index lock during contradiction deprecation")
                        })?;
                        if idx.remove(id) {
                            if let Err(e) = idx.save() {
                                tracing::warn!(
                                    "Failed to persist vector index after deprecating id={id}: {e}"
                                );
                            }
                        }
                        tracing::info!(
                            "Contradiction detected (sim={:.3}): deprecating '{}' → replaced by '{}'",
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
    pub fn remember_with_contradiction(
        &self,
        content: &str,
        tags: &[&str],
        namespace: Option<&str>,
        memory_type: Option<&str>,
        check_contradiction: bool,
    ) -> Result<(String, ContradictionResult), Error> {
        crate::validate_input(content, tags)?;
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during remember_with_contradiction"))?
            .embed(content)?;

        // Check for contradictions before inserting
        let contradiction = if check_contradiction {
            // Release embedder lock first, then check
            self.check_contradiction(content, &embedding, ns, 0.65)?
        } else {
            ContradictionResult {
                contradicted: false,
                deprecated_id: None,
                similarity: 0.0,
            }
        };

        let memory = Memory {
            id: id.clone(),
            content: content.to_string(),
            embedding: embedding.clone(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            namespace: ns.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: Some(now),
            valid_until: None,
            memory_type: memory_type.unwrap_or("fact").to_string(),
        };

        // Write-ahead: vector index first (can be rolled back), then SQLite.
        // Note: contradiction check above may have deprecated a memory and
        // removed it from the vector index — that's done before this point.
        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during remember_with_contradiction"))?;
            index.insert(&id, &embedding);
            if let Err(e) = index.save() {
                tracing::warn!("Failed to persist vector index after insert: {e}");
            }
        }

        // Then insert into SQLite (source of truth for reads)
        if let Err(e) = self.store.insert(&memory) {
            // Rollback: remove from vector index
            let mut index = self.index.lock().map_err(|_| {
                Error::lock("index lock during remember_with_contradiction rollback")
            })?;
            if index.remove(&id) {
                if let Err(e) = index.save() {
                    tracing::warn!("Failed to persist vector index during rollback: {e}");
                }
            }
            return Err(e);
        }

        Ok((id, contradiction))
    }

    /// Find near-duplicate memory pairs (similarity > threshold).
    pub fn find_duplicates(
        &self,
        namespace: Option<&str>,
        threshold: f32,
    ) -> Result<Vec<crate::memory::types::SimilarPair>, Error> {
        let memories = self.store.load_all(namespace)?;
        let mut pairs = Vec::new();
        for i in 0..memories.len() {
            for j in (i + 1)..memories.len() {
                let sim = cosine_similarity(&memories[i].embedding, &memories[j].embedding);
                if sim > threshold {
                    pairs.push(crate::memory::types::SimilarPair {
                        id_a: memories[i].id.clone(),
                        content_a: memories[i].content.chars().take(80).collect(),
                        id_b: memories[j].id.clone(),
                        content_b: memories[j].content.chars().take(80).collect(),
                        similarity: sim,
                    });
                }
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
            self.store
                .delete(&pair.id_a)
                .map_err(|e| Error::db("consolidate delete", e))?;
            // SQLite first (source of truth), then vector index.
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::lock("index lock during consolidate"))?;
            if !index.remove(&pair.id_a) {
                tracing::warn!(
                    "Vector index entry not found during consolidate for id={}",
                    pair.id_a
                );
            }
            if let Err(e) = index.save() {
                tracing::warn!(
                    "Failed to persist vector index after consolidate: {e}. \
                     Orphan entries will be cleaned up by verify/repair."
                );
            }
            removed_ids.push(pair.id_a.clone());
            kept_ids.push(pair.id_b.clone());
            already_removed.insert(pair.id_a.clone());
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
