//! Uteke Core — persistent memory library for AI agents.
//!
//! # Example
//! ```ignore
//! use uteke_core::Uteke;
//!
//! let uteke = Uteke::open("~/.uteke/db.sqlite")?;
//! let id = uteke.remember("important context", &["tag1"], None)?;
//! let results = uteke.recall("query", 5, None)?;
//! ```

mod embed;
pub mod memory;

pub use memory::types::{
    AgingStatus, BulkDeleteResult, CleanupResult, ConsolidationResult, ContradictionResult,
    ExportEntry, ImportResult, Memory, MemoryTier, MemoryType, PruneResult, SearchResult,
    SimilarPair, StoreStats, TagInfo, DEFAULT_NAMESPACE,
};

use embed::EmbeddingEngine;
use memory::store::Store;
use memory::vector::euclidean_to_cosine;
use memory::VectorIndex;

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Resolve uteke data directory.
///
/// Uses `UTEKE_HOME` environment variable when set, otherwise falls back to
/// `~/.uteke`. This allows Docker containers and custom deployments to
/// override the default storage location.
///
/// ```text
/// UTEKE_HOME=/data   → /data
/// (not set)           → ~/.uteke
/// ```
pub fn uteke_home() -> PathBuf {
    if let Ok(home) = std::env::var("UTEKE_HOME") {
        PathBuf::from(home)
    } else {
        dirs::home_dir()
            .expect("Cannot determine home directory. Set UTEKE_HOME or HOME.")
            .join(".uteke")
    }
}

/// Uteke — AI agent memory engine.
///
/// Combines SQLite persistence, HNSW vector search, and ONNX embedding
/// into a single cohesive memory system.
pub struct Uteke {
    store: Store,
    index: Mutex<VectorIndex>,
    embedder: Mutex<EmbeddingEngine>,
}

impl Uteke {
    /// Open or create a Uteke memory store.
    ///
    /// `path` can be a directory path (will create `uteke.db` inside)
    /// or a direct path to a `.sqlite` file.
    /// Use `:memory:` for an in-memory database (testing).
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        let embedder = EmbeddingEngine::new()?;
        Self::finish_open(store, embedder)
    }

    /// Open with a custom embedder (for testing).
    pub fn open_with_embedder(
        path: impl AsRef<Path>,
        embedder: EmbeddingEngine,
    ) -> Result<Self, Error> {
        let (_db_str, store) = Self::open_store(path)?;
        Self::finish_open(store, embedder)
    }

    fn open_store(path: impl AsRef<Path>) -> Result<(String, Store), Error> {
        let db_path = path.as_ref();
        let db_str = resolve_db_path(db_path)?;
        let store = Store::open(&db_str)?;
        Ok((db_str, store))
    }

    fn finish_open(store: Store, embedder: EmbeddingEngine) -> Result<Self, Error> {
        // Determine index path: same directory as SQLite DB
        let index_path = store.path().map(|p| {
            let dir = p.parent().unwrap_or(Path::new("."));
            dir.join("uteke_index.usearch")
        });

        let mut index = match &index_path {
            Some(path) => VectorIndex::load_or_create(path, EmbeddingEngine::dims())?,
            None => VectorIndex::new(EmbeddingEngine::dims()),
        };

        // If index is empty but SQLite has memories, build from SQLite (migration)
        if index.is_empty() {
            let all_memories = store.load_all(None)?;
            if !all_memories.is_empty() {
                let items: Vec<(String, Vec<f32>)> = all_memories
                    .into_iter()
                    .map(|m| (m.id, m.embedding))
                    .collect();
                index.build(&items);
                index.save().ok(); // Persist after migration build
            }
        }

        Ok(Self {
            store,
            index: Mutex::new(index),
            embedder: Mutex::new(embedder),
        })
    }

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
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::Database("Failed to acquire embedder lock".into()))?
            .embed(content)?;

        let memory = Memory {
            id: id.clone(),
            content: content.to_string(),
            embedding: embedding.clone(),
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
            memory_type: "fact".to_string(),
        };

        self.store.insert(&memory)?;

        // Add to usearch index and persist
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        index.insert(&id, &embedding);
        index.save().ok(); // Persist after insert

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
            .map_err(|_| Error::Database("Failed to acquire embedder lock".into()))?
            .embed(query)?;

        // Search usearch index
        let index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
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

            // Boost hot memories (+0.1 bonus)
            let tier = MemoryTier::from_last_accessed(memory.last_accessed);
            let boosted_score = match tier {
                MemoryTier::Hot => (score + 0.1).min(1.0),
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
        self.store.delete(id)?;

        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        index.remove(id);
        index.save().ok(); // Persist after delete

        Ok(())
    }

    /// Bulk delete memories by tag. Also removes from index.
    pub fn bulk_forget_by_tag(
        &self,
        tag: &str,
        namespace: Option<&str>,
    ) -> Result<BulkDeleteResult, Error> {
        let ids = self.store.bulk_delete_by_tag(tag, namespace)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        for id in &ids {
            index.remove(id);
        }
        index.save().ok();
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all cold memories. Also removes from index.
    pub fn bulk_forget_cold(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        let ids = self.store.bulk_delete_cold(namespace)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        for id in &ids {
            index.remove(id);
        }
        index.save().ok();
        Ok(BulkDeleteResult {
            deleted: ids.len(),
            ids,
        })
    }

    /// Bulk delete all memories in a namespace. Also removes from index.
    pub fn bulk_forget_all(&self, namespace: Option<&str>) -> Result<BulkDeleteResult, Error> {
        let ids = self.store.bulk_delete_all(namespace)?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        for id in &ids {
            index.remove(id);
        }
        index.save().ok();
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
            .ok_or_else(|| Error::Database(format!("Memory not found: {id}")))?;
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

    /// Check system health: DB, index, model, consistency.
    pub fn doctor(&self) -> Result<DoctorReport, Error> {
        let mut checks = Vec::new();

        // 1. SQLite DB
        let db_count = self.store.count(None)?;
        let db_path = self.store.path();
        let db_size = db_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);
        checks.push(DoctorCheck {
            name: "SQLite DB".to_string(),
            status: DoctorStatus::Ok,
            detail: format!("{} memories, {}", db_count, format_bytes(db_size)),
        });

        // 2. usearch index
        let index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        let index_count = index.len();
        checks.push(DoctorCheck {
            name: "usearch index".to_string(),
            status: DoctorStatus::Ok,
            detail: format!("{} vectors", index_count),
        });

        // 3. Index consistency
        if db_count == index_count {
            checks.push(DoctorCheck {
                name: "Index consistency".to_string(),
                status: DoctorStatus::Ok,
                detail: format!("DB={} Index={}", db_count, index_count),
            });
        } else {
            checks.push(DoctorCheck {
                name: "Index consistency".to_string(),
                status: DoctorStatus::Error,
                detail: format!(
                    "MISMATCH: DB={} Index={} — run `uteke repair`",
                    db_count, index_count
                ),
            });
        }

        // 4. Embedding model
        let model_dir = uteke_home().join("models").join("embeddinggemma-q4");
        let model_file = model_dir.join("onnx").join("model_q4.onnx");
        let tokenizer_file = model_dir.join("tokenizer.json");
        let model_exists = model_file.exists() && tokenizer_file.exists();
        checks.push(DoctorCheck {
            name: "Embedding model".to_string(),
            status: if model_exists {
                DoctorStatus::Ok
            } else {
                DoctorStatus::Error
            },
            detail: if model_exists {
                "embeddinggemma-q4".to_string()
            } else {
                "Model files not found — will download on first use".to_string()
            },
        });

        Ok(DoctorReport { checks })
    }

    /// Verify DB and index consistency. Returns mismatch count.
    pub fn verify(&self) -> Result<VerifyReport, Error> {
        let db_count = self.store.count(None)?;
        let index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        let index_count = index.len();

        let consistent = db_count == index_count;
        Ok(VerifyReport {
            db_count,
            index_count,
            consistent,
        })
    }

    /// Repair: rebuild usearch index from SQLite.
    pub fn repair(&self) -> Result<RepairReport, Error> {
        let before_db = self.store.count(None)?;
        let before_index = {
            let index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            index.len()
        };

        // Load all from SQLite and rebuild index
        let all_memories = self.store.load_all(None)?;
        let items: Vec<(String, Vec<f32>)> = all_memories
            .iter()
            .map(|m| (m.id.clone(), m.embedding.clone()))
            .collect();

        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            index.build(&items);
            index.save().ok();
        }

        Ok(RepairReport {
            db_count: before_db,
            index_before: before_index,
            index_after: items.len(),
        })
    }

    /// Get statistics about the memory store.
    /// Access the underlying store (read-only reference).
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn stats(&self, namespace: Option<&str>) -> Result<StoreStats, Error> {
        let total_memories = self.store.count(namespace)?;
        let unique_tags = self.store.unique_tags(namespace)?.len();
        let (hot, warm, cold) = self.store.tier_counts(namespace)?;

        let db_size_bytes = self
            .store
            .path()
            .and_then(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .unwrap_or(0);

        Ok(StoreStats {
            total_memories,
            unique_tags,
            db_size_bytes,
            hot,
            warm,
            cold,
        })
    }

    /// Get aging status — breakdown of memories by access tier.
    pub fn aging_status(&self, namespace: Option<&str>) -> Result<AgingStatus, Error> {
        let total = self.store.count(namespace)?;
        let (hot, warm, cold) = self.store.tier_counts(namespace)?;
        let never_accessed = self.store.count_never_accessed(namespace)?;

        Ok(AgingStatus {
            total,
            hot,
            warm,
            cold,
            never_accessed,
        })
    }

    /// Preview aged memories eligible for cleanup (dry-run).
    pub fn aging_preview(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<Vec<Memory>, Error> {
        self.store
            .find_aged(older_than_days, max_access_count, namespace)
    }

    /// Cleanup aged memories — deletes from SQLite AND removes from vector index.
    pub fn aging_cleanup(
        &self,
        older_than_days: u32,
        max_access_count: u32,
        namespace: Option<&str>,
    ) -> Result<CleanupResult, Error> {
        // Find aged memories first to get IDs for vector index removal
        let aged = self
            .store
            .find_aged(older_than_days, max_access_count, namespace)?;
        let ids: Vec<String> = aged.into_iter().map(|m| m.id).collect();

        if ids.is_empty() {
            return Ok(CleanupResult { deleted: 0 });
        }

        // Delete from SQLite
        let deleted = self
            .store
            .cleanup_aged(older_than_days, max_access_count, namespace)?;

        // Remove from vector index
        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            for id in &ids {
                index.remove(id);
            }
            index.save().ok();
        }

        Ok(CleanupResult { deleted })
    }

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
        let index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;

        let results = index.search(embedding, 5, 50);

        for (id, distance) in &results {
            let similarity = 1.0 - distance;
            if similarity > threshold {
                if let Ok(Some(memory)) = self.store.get_by_id(id) {
                    if memory.namespace == namespace && !memory.deprecated {
                        self.store.deprecate(id)?;
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
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now();
        let ns = namespace.unwrap_or(DEFAULT_NAMESPACE);
        let embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::Database("Failed to acquire embedder lock".into()))?
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

        self.store.insert(&memory)?;

        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        index.insert(&id, &embedding);
        index.save().ok();

        Ok((id, contradiction))
    }

    /// Prune deprecated memories older than TTL days.
    ///
    /// Deletes from both SQLite and vector index.
    pub fn prune(
        &self,
        ttl_days: u32,
        namespace: Option<&str>,
        dry_run: bool,
    ) -> Result<PruneResult, Error> {
        let deprecated = self.store.find_deprecated_for_prune(ttl_days, namespace)?;
        let ids: Vec<String> = deprecated.iter().map(|m| m.id.clone()).collect();
        let count = ids.len();

        if dry_run || count == 0 {
            return Ok(PruneResult {
                pruned: 0,
                ids: vec![],
                deprecated: count,
                deprecated_ids: ids,
            });
        }

        // Delete from SQLite
        let pruned = self.store.prune_ttl(ttl_days, namespace)?;

        // Remove from vector index
        {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            for id in &ids {
                index.remove(id);
            }
            index.save().ok();
        }

        Ok(PruneResult {
            pruned,
            ids: ids.clone(),
            deprecated: count,
            deprecated_ids: ids,
        })
    }

    /// Find near-duplicate memory pairs (similarity > threshold).
    pub fn find_duplicates(
        &self,
        namespace: Option<&str>,
        threshold: f32,
    ) -> Result<Vec<SimilarPair>, Error> {
        let memories = self.store.load_all(namespace)?;
        let mut pairs = Vec::new();
        for i in 0..memories.len() {
            for j in (i + 1)..memories.len() {
                let sim = cosine_similarity(&memories[i].embedding, &memories[j].embedding);
                if sim > threshold {
                    pairs.push(SimilarPair {
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
    ) -> Result<ConsolidationResult, Error> {
        let pairs = self.find_duplicates(namespace, threshold)?;
        if pairs.is_empty() || dry_run {
            return Ok(ConsolidationResult {
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
                .map_err(|e| Error::Database(e.to_string()))?;
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            index.remove(&pair.id_a);
            index.save().ok();
            removed_ids.push(pair.id_a.clone());
            kept_ids.push(pair.id_b.clone());
            already_removed.insert(pair.id_a.clone());
        }
        Ok(ConsolidationResult {
            duplicates_found: pairs.len(),
            merged: removed_ids.len(),
            removed_ids,
            kept_ids,
        })
    }

    /// Export all memories to JSONL format (one JSON object per line).
    ///
    /// Embeddings are NOT exported — they will be re-computed on import.
    /// This keeps export files small and portable.
    pub fn export(&self, namespace: Option<&str>) -> Result<String, Error> {
        let memories = self.store.load_all(namespace)?;
        let entries: Vec<ExportEntry> = memories
            .into_iter()
            .map(|m| ExportEntry {
                content: m.content,
                tags: m.tags,
                metadata: m.metadata,
                created_at: m.created_at,
            })
            .collect();

        let mut lines = Vec::with_capacity(entries.len());
        for entry in &entries {
            let line = serde_json::to_string(entry).map_err(|e| Error::Database(e.to_string()))?;
            lines.push(line);
        }

        Ok(lines.join("\n"))
    }

    /// Graceful shutdown — save dirty index to disk.
    pub fn shutdown(&self) -> Result<(), Error> {
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("lock".into()))?;
        if index.is_dirty() {
            index.save()?;
        }
        Ok(())
    }

    /// Import memories from JSONL format.
    ///
    /// Each line should be a valid JSON object with `content`, `tags`, `metadata`, `created_at`.
    /// Embeddings are re-computed during import.
    pub fn import(&self, jsonl: &str, namespace: Option<&str>) -> Result<ImportResult, Error> {
        let mut imported = 0;
        let mut skipped = 0;

        for line in jsonl.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let entry: ExportEntry = match serde_json::from_str(line) {
                Ok(e) => e,
                Err(_) => {
                    skipped += 1;
                    continue;
                }
            };

            if entry.content.is_empty() {
                skipped += 1;
                continue;
            }

            // Re-embed the content
            let embedding = self
                .embedder
                .lock()
                .map_err(|_| Error::Database("Failed to acquire embedder lock".into()))?
                .embed(&entry.content)?;

            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now();

            let memory = Memory {
                id: id.clone(),
                content: entry.content,
                embedding: embedding.clone(),
                tags: entry.tags,
                metadata: entry.metadata,
                created_at: entry.created_at,
                updated_at: now,
                namespace: namespace.unwrap_or(DEFAULT_NAMESPACE).to_string(),
                access_count: 0,
                last_accessed: None,
                deprecated: false,
                valid_from: Some(entry.created_at),
                valid_until: None,
                memory_type: "fact".to_string(),
            };

            self.store.insert(&memory)?;

            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            index.insert(&id, &embedding);

            imported += 1;
        }

        // Persist vector index after import completes
        if imported > 0 {
            let mut index = self
                .index
                .lock()
                .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
            index.save()?;
        }

        Ok(ImportResult { imported, skipped })
    }
}

/// Resolve a path to a database string.
fn resolve_db_path(db_path: &Path) -> Result<String, Error> {
    if db_path.to_str() == Some(":memory:") {
        return Ok(":memory:".to_string());
    }

    if db_path.is_dir() || db_path.extension().is_none() {
        std::fs::create_dir_all(db_path).map_err(Error::Io)?;
        Ok(db_path.join("uteke.db").to_string_lossy().to_string())
    } else {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
        }
        Ok(db_path.to_string_lossy().to_string())
    }
}

/// Helper to format bytes.
fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Status of a doctor check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DoctorStatus {
    Ok,
    Warn,
    Error,
}

/// A single check in the doctor report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Name of the check.
    pub name: String,
    /// Status: ok, warn, error.
    pub status: DoctorStatus,
    /// Detail message.
    pub detail: String,
}

/// Result of `uteke doctor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    /// All checks performed.
    pub checks: Vec<DoctorCheck>,
}

/// Result of `uteke verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Count of memories in SQLite.
    pub db_count: usize,
    /// Count of vectors in usearch index.
    pub index_count: usize,
    /// Whether they match.
    pub consistent: bool,
}

/// Result of `uteke repair`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairReport {
    /// Count of memories in DB.
    pub db_count: usize,
    /// Index count before repair.
    pub index_before: usize,
    /// Index count after repair.
    pub index_after: usize,
}

/// Uteke error type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Database error: {0}")]
    Database(String),
    #[error("Embedding error: {0}")]
    Embedding(String),
}

/// Compute cosine similarity between two vectors.
fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
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

    #[test]
    fn test_memory_types_serialization() {
        let now = chrono::Utc::now();
        let m = Memory {
            id: "test-id".to_string(),
            content: "hello".to_string(),
            embedding: vec![0.1; 768],
            tags: vec!["a".to_string(), "b".to_string()],
            metadata: serde_json::json!({"key": "value"}),
            created_at: now,
            updated_at: now,
            namespace: DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
        };

        let json = serde_json::to_string(&m).unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Embedding is skipped in JSON output (skip_serializing)
        assert!(
            !v.get("embedding").is_some(),
            "embedding should not be in JSON output"
        );

        // Other fields should serialize correctly
        assert_eq!(v["id"], m.id);
        assert_eq!(v["content"], m.content);
        assert_eq!(v["tags"].as_array().unwrap().len(), 2);

        // Deserialization produces empty embedding (expected — populated programmatically)
        let restored: Memory = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, m.id);
        assert_eq!(restored.content, m.content);
        assert_eq!(restored.tags, m.tags);
        assert!(restored.embedding.is_empty());
    }

    #[test]
    fn test_search_result_type() {
        let now = chrono::Utc::now();
        let m = Memory {
            id: "sr-test".to_string(),
            content: "test content".to_string(),
            embedding: vec![0.0; 768],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
            namespace: DEFAULT_NAMESPACE.to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
        };

        let sr = SearchResult {
            memory: m,
            score: 0.95,
        };
        assert_eq!(sr.score, 0.95);
        assert_eq!(sr.memory.id, "sr-test");
    }

    #[test]
    fn test_store_stats_type() {
        let stats = StoreStats {
            total_memories: 42,
            unique_tags: 5,
            db_size_bytes: 1024,
            hot: 10,
            warm: 15,
            cold: 17,
        };

        let json = serde_json::to_string(&stats).unwrap();
        let restored: StoreStats = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.total_memories, 42);
        assert_eq!(restored.unique_tags, 5);
    }
}
