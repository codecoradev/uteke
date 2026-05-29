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

pub use memory::types::{Memory, SearchResult, StoreStats};

use embed::EmbeddingEngine;
use memory::store::Store;
use memory::vector::euclidean_to_cosine;
use memory::VectorIndex;

use std::path::Path;
use std::sync::Mutex;

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
        // Load existing memories into HNSW index
        let all_memories = store.load_all()?;
        let mut index = VectorIndex::new();
        if !all_memories.is_empty() {
            let items: Vec<(String, Vec<f32>)> = all_memories
                .into_iter()
                .map(|m| (m.id, m.embedding))
                .collect();
            index.build(&items);
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
        };

        self.store.insert(&memory)?;

        // Add to HNSW index
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        index.insert(&id, &embedding);

        Ok(id)
    }

    /// Recall memories relevant to a query using vector similarity.
    ///
    /// Optionally filter by tags.
    pub fn recall(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
    ) -> Result<Vec<SearchResult>, Error> {
        let query_embedding = self
            .embedder
            .lock()
            .map_err(|_| Error::Database("Failed to acquire embedder lock".into()))?
            .embed(query)?;

        // Search HNSW with higher ef for better recall
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;
        let candidates = index.search(&query_embedding, limit * 3, limit * 4);

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
            results.push(SearchResult { memory, score });
        }

        // Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(results)
    }

    /// Search memories by content text (LIKE-based for v2).
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, Error> {
        let memories = self.store.search_content(query, limit)?;

        let results = memories
            .into_iter()
            .map(|memory| SearchResult {
                memory,
                score: 1.0, // Text search doesn't have meaningful scores
            })
            .collect();

        Ok(results)
    }

    /// Delete a memory by ID.
    pub fn forget(&self, id: &str) -> Result<(), Error> {
        self.store.delete(id)?;

        // Rebuild index (simple approach for v2)
        let all_memories = self.store.load_all()?;
        let mut index = self
            .index
            .lock()
            .map_err(|_| Error::Database("Failed to acquire index lock".into()))?;

        let items: Vec<(String, Vec<f32>)> = all_memories
            .into_iter()
            .map(|m| (m.id, m.embedding))
            .collect();
        index.build(&items);

        Ok(())
    }

    /// List memories with optional tag filter and pagination.
    pub fn list(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Memory>, Error> {
        self.store.list(tag, limit, offset)
    }

    /// Get a single memory by ID.
    pub fn get(&self, id: &str) -> Result<Memory, Error> {
        self.store
            .get_by_id(id)?
            .ok_or_else(|| Error::Database(format!("Memory not found: {id}")))
    }

    /// Get statistics about the memory store.
    pub fn stats(&self) -> Result<StoreStats, Error> {
        let total_memories = self.store.count()?;
        let unique_tags = self.store.unique_tags()?.len();

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
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_types_serialization() {
        let now = chrono::Utc::now();
        let m = Memory {
            id: "test-id".to_string(),
            content: "hello".to_string(),
            embedding: vec![0.1; 384],
            tags: vec!["a".to_string(), "b".to_string()],
            metadata: serde_json::json!({"key": "value"}),
            created_at: now,
            updated_at: now,
        };

        let json = serde_json::to_string(&m).unwrap();
        let restored: Memory = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.id, m.id);
        assert_eq!(restored.content, m.content);
        assert_eq!(restored.tags, m.tags);
        assert_eq!(restored.embedding.len(), 384);
    }

    #[test]
    fn test_search_result_type() {
        let now = chrono::Utc::now();
        let m = Memory {
            id: "sr-test".to_string(),
            content: "test content".to_string(),
            embedding: vec![0.0; 384],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: now,
            updated_at: now,
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
        };

        let json = serde_json::to_string(&stats).unwrap();
        let restored: StoreStats = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.total_memories, 42);
        assert_eq!(restored.unique_tags, 5);
    }
}
