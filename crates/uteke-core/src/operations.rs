//! Core memory operations: remember, recall, search, forget, list, get, tags.

use crate::error::Error;
use crate::memory::types::{
    BulkDeleteResult, Memory, MemoryTier, SearchResult, TagInfo, DEFAULT_NAMESPACE,
};
use crate::memory::vector::euclidean_to_cosine;

impl crate::Uteke {
    /// Store a new memory.
    ///
    /// Returns the UUID of the created memory.
    pub fn remember(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
    ) -> Result<String, Error> {
        self.remember_typed(content, tags, metadata, namespace, "fact")
    }

    /// Store a new memory with explicit type.
    ///
    /// Returns the UUID of the created memory.
    pub fn remember_typed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
    ) -> Result<String, Error> {
        crate::validate_input(content, tags)?;
        let embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during remember"))?
            .embed(content)?;
        self.remember_precomputed(content, tags, metadata, namespace, memory_type, &embedding)
    }

    /// Store a new memory with a pre-computed embedding.
    ///
    /// Use when the embedding has already been computed (e.g., contradiction check).
    /// Returns the UUID of the created memory.
    pub(crate) fn remember_precomputed(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
        embedding: &[f32],
    ) -> Result<String, Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();

        let memory = Memory {
            id: id.clone(),
            content: content.to_string(),
            embedding: embedding.to_vec(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            metadata: metadata.unwrap_or(serde_json::Value::Null),
            created_at: now,
            updated_at: now,
            namespace: namespace.unwrap_or(DEFAULT_NAMESPACE).to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: Some(now),
            valid_until: None,
            memory_type: memory_type.to_string(),
        };

        // Acquire index lock BEFORE any writes so lock failures are detected early.
        // If SQLite commit fails after index insert, the orphan index entry is harmless
        // and will be cleaned up by verify/repair.
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during remember"))?;

        self.store.insert(&memory)?;

        index.insert(&id, embedding);
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after remember for id={id}: {e}. \
                 Index entry can be rebuilt via `uteke repair`."
            );
        }

        Ok(id)
    }

    /// Recall memories relevant to a query using vector similarity.
    ///
    /// Optionally filter by tags and namespace.
    pub fn recall(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
    ) -> Result<Vec<SearchResult>, Error> {
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let query_embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::lock("embedder lock during recall"))?
            .embed(query)?;

        // Search usearch index
        let index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during recall"))?;
        // Cap k to index size
        let index_len = index.len();
        let k = (limit * 3).min(index_len).max(1);
        let candidates = index.search(&query_embedding, k, limit * 4);

        // Fetch full memories and apply tag filter
        let mut results = Vec::new();
        for (memory_id, distance) in candidates {
            if results.len() >= limit {
                break;
            }

            let memory = match self.store.get_by_id(&memory_id)? {
                Some(m) => m,
                None => continue,
            };

            // Apply namespace filter
            if memory.namespace != ns {
                continue;
            }

            // Apply tag filter
            if let Some(filter_tags) = tags_filter {
                let has_tag = filter_tags
                    .iter()
                    .any(|ft| memory.tags.iter().any(|t| t == ft));
                if !has_tag {
                    continue;
                }
            }

            let score = euclidean_to_cosine(distance);

            // Boost hot memories (configurable boost)
            let tier = MemoryTier::from_last_accessed(
                memory.last_accessed,
                self.tier_config.hot_days,
                self.tier_config.warm_days,
            );
            let boosted_score = match tier {
                MemoryTier::Hot => (score + self.tier_config.hot_boost as f32).min(1.0),
                _ => score,
            };

            results.push(SearchResult {
                memory,
                score: boosted_score,
            });
        }

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Touch access for returned results
        for r in &results {
            self.store.touch_access(&r.memory.id).ok();
        }

        Ok(results)
    }

    /// Search memories by content text (LIKE-based for v2).
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
    ) -> Result<Vec<SearchResult>, Error> {
        let memories = self.store.search_content(query, namespace, limit)?;

        let results = memories
            .into_iter()
            .filter(|memory| {
                if let Some(filter_tags) = tags_filter {
                    filter_tags
                        .iter()
                        .any(|ft| memory.tags.iter().any(|t| t == ft))
                } else {
                    true
                }
            })
            .map(|memory| SearchResult {
                memory,
                score: 1.0, // Text search doesn't have meaningful scores
            })
            .collect();

        Ok(results)
    }

    /// Delete a memory by ID. Incremental — no index rebuild.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        // SQLite first (source of truth), then vector index.
        // If vector index remove fails, the orphan is harmless —
        // verify/repair can clean it up later.
        self.store.delete(id)?;

        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during forget"))?;
        if !index.remove(id) {
            tracing::warn!("Vector index entry not found during forget for id={id}");
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after forget for id={id}: {e}. \
                 Orphan entry will be cleaned up by verify/repair."
            );
        }

        Ok(())
    }

    /// Bulk delete memories by tag. Also removes from index.
    pub fn bulk_forget_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<BulkDeleteResult, Error> {
        // SQLite first (source of truth), then vector index.
        let ids = self.store.bulk_delete_by_tag(tag, namespace)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during bulk_forget_by_tag"))?;
        for id in &ids {
            if !index.remove(id) {
                tracing::warn!(
                    "Vector index entry not found during bulk_forget_by_tag for id={id}"
                );
            }
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after bulk_forget_by_tag: {e}. \
                 Orphan entries will be cleaned up by verify/repair."
            );
        }
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all cold memories. Also removes from index.
    pub fn bulk_forget_cold(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        // SQLite first (source of truth), then vector index.
        let ids = self
            .store
            .bulk_delete_cold(namespace, self.tier_config.warm_days)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during bulk_forget_cold"))?;
        for id in &ids {
            if !index.remove(id) {
                tracing::warn!("Vector index entry not found during bulk_forget_cold for id={id}");
            }
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after bulk_forget_cold: {e}. \
                 Orphan entries will be cleaned up by verify/repair."
            );
        }
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all memories in a namespace. Also removes from index.
    pub fn bulk_forget_all(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        // SQLite first (source of truth), then vector index.
        let ids = self.store.bulk_delete_all(namespace)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::lock("index lock during bulk_forget_all"))?;
        for id in &ids {
            if !index.remove(id) {
                tracing::warn!("Vector index entry not found during bulk_forget_all for id={id}");
            }
        }
        if let Err(e) = index.save() {
            tracing::warn!(
                "Failed to persist vector index after bulk_forget_all: {e}. \
                 Orphan entries will be cleaned up by verify/repair."
            );
        }
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// List memories with optional tag filter and pagination.
    pub fn list(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        self.store.list(tag, namespace, limit, offset)
    }

    /// Get a single memory by ID.
    pub fn get(&self, id: &str) -> Result<Memory, Error> {
        let memory = self
            .store
            .get_by_id(id)?
            .ok_or_else(|| Error::db_msg(format!("Memory not found: {id}")))?;
        self.store.touch_access(id).ok();
        Ok(memory)
    }

    /// List all namespaces.
    pub fn list_namespaces(&self) -> Result<Vec<String>, Error> {
        self.store.list_namespaces()
    }

    /// List all tags with their usage counts.
    pub fn tags_with_counts(&self, namespace: Option<&str>) -> Result<Vec<TagInfo>, Error> {
        self.store.tags_with_counts(namespace)
    }

    /// Rename a tag across all memories in a namespace.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        self.store.rename_tag(old, new, namespace)
    }

    /// Delete a tag from all memories in a namespace.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.delete_tag(tag, namespace)
    }

    /// Count memories by tag in a namespace.
    pub fn count_by_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count_by_tag(tag, namespace)
    }

    /// Count total memories, optionally filtered by namespace.
    pub fn count(&self, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count(namespace)
    }

    /// Get a memory by ID (without touching access count — used for internal lookups).
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        self.store.get_by_id(id)
    }
}
