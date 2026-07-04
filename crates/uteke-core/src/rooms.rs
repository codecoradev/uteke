//! Room-based collaborative memory operations.

use crate::error::Error;
use crate::memory::types::{Memory, RecallStrategy, SearchResult};
use crate::memory::{Room, RoomDocument, RoomStats, RoomSummary};

impl crate::Uteke {
    /// Create a new room for collaborative memory.
    pub fn create_room(
        &self,
        room_id: &str,
        title: Option<&str>,
        namespace: &str,
    ) -> Result<(), Error> {
        self.store.create_room(room_id, title, namespace)
    }

    /// Get a room by ID.
    pub fn get_room(&self, room_id: &str) -> Result<Option<Room>, Error> {
        self.store.get_room(room_id)
    }

    /// List rooms for a namespace (or all rooms if namespace is None).
    pub fn list_rooms(&self, namespace: Option<&str>) -> Result<Vec<Room>, Error> {
        self.store.list_rooms(namespace)
    }

    /// Get statistics about a room.
    pub fn room_stats(&self, room_id: &str) -> Result<Option<RoomStats>, Error> {
        self.store.room_stats(room_id)
    }

    /// Store a memory and link it to a room.
    /// Dual-write: memory is stored in the agent's namespace AND linked to the room.
    #[allow(clippy::too_many_arguments)]
    pub fn remember_in_room(
        &self,
        content: &str,
        tags: &[&str],
        metadata: Option<serde_json::Value>,
        namespace: Option<&str>,
        memory_type: &str,
        room_id: &str,
        author: &str,
    ) -> Result<String, Error> {
        // Store the memory normally (lazy-loads embedder if needed)
        let memory_id = self.remember_typed(content, tags, metadata, namespace, memory_type)?;

        // Ensure room exists (auto-create if needed)
        if self.store.get_room(room_id)?.is_none() {
            let ns = namespace.unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
            self.store.create_room(room_id, None, ns)?;
        }

        // Link memory to room
        self.store
            .link_memory_to_room(room_id, &memory_id, author, "participant")?;

        Ok(memory_id)
    }

    /// Recall all memories in a room (cross-namespace).
    /// Optionally filter by author.
    pub fn recall_room(
        &self,
        room_id: &str,
        author: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Memory>, Error> {
        self.store.recall_room(room_id, author, limit)
    }

    /// Semantic recall within room context using hybrid search (vector + FTS5).
    ///
    /// Returns room memories ranked by relevance to query, with scores.
    /// Algorithm: over-fetch via recall_hybrid, post-filter to room members, truncate.
    pub fn recall_room_semantic(
        &self,
        room_id: &str,
        query: &str,
        limit: usize,
        author: Option<&str>,
        min_score: f32,
    ) -> Result<Vec<SearchResult>, Error> {
        // 1. Get room memory IDs (cheap — IDs only)
        let room_ids = self.store.get_room_memory_ids(room_id, author)?;
        if room_ids.is_empty() {
            return Ok(Vec::new());
        }
        let id_set: std::collections::HashSet<String> = room_ids.into_iter().collect();

        // 2. Over-fetch via hybrid recall (no namespace filter — rooms are cross-ns).
        // Cap at 200 to prevent unbounded searches for large rooms (#546).
        let fetch_limit = (limit * 5).min(200).max(limit);
        let results = self.recall_hybrid(
            query,
            fetch_limit,
            None, // no tag filter
            None, // no namespace filter — rooms are cross-namespace
            RecallStrategy::Hybrid,
            0.0, // no min_score at fetch stage, we filter after
        )?;

        // 3. Post-filter: only keep results whose memory ID is in the room
        let mut filtered: Vec<SearchResult> = results
            .into_iter()
            .filter(|sr| id_set.contains(&sr.memory.id))
            .collect();

        // 4. Apply min_score filter
        if min_score > 0.0 {
            filtered.retain(|sr| sr.score >= min_score);
        }

        // 5. Sort by score descending (already sorted by recall_hybrid, but re-sort after filtering)
        filtered.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 6. Truncate to limit
        filtered.truncate(limit);

        Ok(filtered)
    }

    /// Delete a room and all its memory links.
    /// Note: memories themselves are NOT deleted — they remain in their namespaces.
    pub fn delete_room(&self, room_id: &str) -> Result<(), Error> {
        self.store.delete_room(room_id)
    }

    /// Generate a summary of room discussion (topic clustering, no LLM needed).
    pub fn room_summary(&self, room_id: &str) -> Result<Option<RoomSummary>, Error> {
        self.store.room_summary(room_id)
    }

    /// Generate a structured document from room memories.
    pub fn room_document(&self, room_id: &str) -> Result<Option<RoomDocument>, Error> {
        self.store.room_document(room_id)
    }
}
