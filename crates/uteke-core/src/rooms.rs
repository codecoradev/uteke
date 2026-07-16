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
        // limit=0 means "return all" — use a generous fetch limit (#623).
        let effective_limit = if limit == 0 { 1000 } else { limit };
        let fetch_limit = (effective_limit * 5).min(200).max(effective_limit);
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

        // 6. Truncate to limit (0 = return all, no truncation)
        if limit > 0 {
            filtered.truncate(limit);
        }

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

    /// Get room summary with referenced documents populated.
    pub fn room_summary_with_docs(&self, room_id: &str) -> Result<Option<RoomSummary>, Error> {
        self.store.room_summary_with_docs(room_id)
    }

    /// Generate a structured document from room memories.
    pub fn room_document(&self, room_id: &str) -> Result<Option<RoomDocument>, Error> {
        self.store.room_document(room_id)
    }

    // ── Room ↔ Document junction (v15, #689) ─────────────────────────────

    /// Link a document to a room. No-op if already linked.
    pub fn room_add_document(&self, room_id: &str, doc_slug: &str) -> Result<(), Error> {
        self.store.room_add_document(room_id, doc_slug)
    }

    /// Unlink a document from a room.
    pub fn room_remove_document(&self, room_id: &str, doc_slug: &str) -> Result<(), Error> {
        self.store.room_remove_document(room_id, doc_slug)
    }

    /// List document slugs linked to a room.
    pub fn room_list_documents(&self, room_id: &str) -> Result<Vec<String>, Error> {
        self.store.room_list_documents(room_id)
    }

    /// List room IDs that have a given document linked.
    pub fn document_list_rooms(&self, doc_slug: &str) -> Result<Vec<String>, Error> {
        self.store.document_list_rooms(doc_slug)
    }
}

#[cfg(test)]
mod tests {

    /// Create an Uteke instance backed by an in-memory store.
    /// The embedder is lazy-loaded on first use, so tests that only
    /// exercise CRUD methods (no embedding) don't need the ONNX model.
    fn open_in_memory() -> crate::Uteke {
        crate::Uteke::open(":memory:").unwrap()
    }

    // ── Room CRUD ──────────────────────────────────────────────────

    #[test]
    fn create_and_get_room() {
        let uteke = open_in_memory();
        uteke
            .create_room("room-1", Some("Test"), "default")
            .unwrap();
        let room = uteke.get_room("room-1").unwrap().unwrap();
        assert_eq!(room.id, "room-1");
        assert_eq!(room.title, Some("Test".to_string()));
        assert_eq!(room.namespace, "default");
    }

    #[test]
    fn list_rooms_with_namespace_filter() {
        let uteke = open_in_memory();
        uteke.create_room("r1", None, "ns-a").unwrap();
        uteke.create_room("r2", None, "ns-b").unwrap();
        uteke.create_room("r3", None, "ns-a").unwrap();

        let all = uteke.list_rooms(None).unwrap();
        assert_eq!(all.len(), 3);

        let filtered = uteke.list_rooms(Some("ns-a")).unwrap();
        assert_eq!(filtered.len(), 2);
    }

    #[test]
    fn delete_room() {
        let uteke = open_in_memory();
        uteke.create_room("del", None, "default").unwrap();
        uteke.delete_room("del").unwrap();
        assert!(uteke.get_room("del").unwrap().is_none());
    }

    #[test]
    fn room_stats() {
        let uteke = open_in_memory();
        uteke
            .create_room("stats-room", Some("Stats"), "default")
            .unwrap();

        // No memories linked
        let stats = uteke.room_stats("stats-room").unwrap().unwrap();
        assert_eq!(stats.memory_count, 0);
        assert_eq!(stats.title, Some("Stats".to_string()));

        // Nonexistent room → None
        assert!(uteke.room_stats("nope").unwrap().is_none());
    }

    #[test]
    fn room_summary_empty() {
        let uteke = open_in_memory();
        uteke.create_room("sum-empty", None, "default").unwrap();

        let summary = uteke.room_summary("sum-empty").unwrap().unwrap();
        assert_eq!(summary.total_memories, 0);
        assert!(summary.clusters.is_empty());
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn room_summary_with_memories() {
        let uteke = open_in_memory();
        uteke
            .remember_in_room(
                "Architecture decision",
                &["arch"],
                None,
                None,
                "decision",
                "sum-pop",
                "alice",
            )
            .unwrap();

        let summary = uteke.room_summary("sum-pop").unwrap().unwrap();
        assert_eq!(summary.total_memories, 1);
        assert_eq!(summary.recent_decisions.len(), 1);
        assert!(summary.recent_decisions[0].contains("Architecture decision"));
    }

    #[test]
    fn room_document() {
        let uteke = open_in_memory();
        uteke
            .create_room("doc-room", Some("Doc"), "default")
            .unwrap();

        // Empty room → no sections
        let doc = uteke.room_document("doc-room").unwrap().unwrap();
        assert!(doc.sections.is_empty());
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn room_document_with_memories() {
        let uteke = open_in_memory();
        uteke
            .remember_in_room("Some fact", &[], None, None, "fact", "doc-room", "bob")
            .unwrap();

        let doc = uteke.room_document("doc-room").unwrap().unwrap();
        assert_eq!(doc.room_id, "doc-room");
        assert_eq!(doc.sections.len(), 1); // Research & Facts
    }
    #[test]
    fn room_document_nonexistent_returns_none() {
        let uteke = open_in_memory();
        assert!(uteke.room_document("nope").unwrap().is_none());
    }

    // ── Room ↔ Document junction ──────────────────────────────────
    // NOTE: room_add_document / room_remove_document / document_list_rooms
    // call Store::room_add_document which validates document slug existence.
    // Creating documents requires the ONNX embedder via Uteke::doc_upsert,
    // so these junction tests live in memory/rooms.rs (Store-level) where
    // we can use Store::upsert_document directly without an embedder.

    // ── remember_in_room (requires embedder) ───────────────────────

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn remember_in_room_stores_and_links() {
        let uteke = open_in_memory();
        let mem_id = uteke
            .remember_in_room(
                "Hello world",
                &["greeting"],
                None,
                None,
                "fact",
                "room-x",
                "alice",
            )
            .unwrap();

        // Memory was stored and linked
        let recalled = uteke.recall_room("room-x", None, 0).unwrap();
        assert_eq!(recalled.len(), 1);
        assert_eq!(recalled[0].id, mem_id);

        // Room was auto-created
        let room = uteke.get_room("room-x").unwrap().unwrap();
        assert_eq!(room.id, "room-x");
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn recall_room_with_author_filter() {
        let uteke = open_in_memory();
        uteke
            .remember_in_room("From alice", &[], None, None, "fact", "ar", "alice")
            .unwrap();
        uteke
            .remember_in_room("From bob", &[], None, None, "fact", "ar", "bob")
            .unwrap();

        let alice = uteke.recall_room("ar", Some("alice"), 0).unwrap();
        assert_eq!(alice.len(), 1);
        assert!(alice[0].content.contains("alice"));
    }
}
