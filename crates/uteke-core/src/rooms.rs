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
    ///
    /// Algorithm (two-stage, bounded):
    /// 1. **Primary**: hybrid search with adaptive bounded limit, post-filter to room IDs.
    /// 2. **Fallback**: if primary returns fewer room memories than the room contains,
    ///    load remaining room memories directly and score them with cosine similarity
    ///    against the query embedding. This guarantees complete room coverage without
    ///    unbounded global fetch (#894 follow-up).
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
        let room_size = room_ids.len();
        let id_set: std::collections::HashSet<String> = room_ids.into_iter().collect();

        // 2. Adaptive bounded fetch_limit.
        //
        // Scale with room size (×10 gives generous over-fetch for post-filter)
        // and caller limit (×5). Hard cap at MAX_ROOM_FETCH prevents unbounded
        // scans on large databases (50K+ memories).
        const MAX_ROOM_FETCH: usize = 5_000;
        let effective_limit = if limit == 0 { 1000 } else { limit };
        let total_memories = self.store.count_all_memories()?;
        let fetch_limit = (room_size * 10)
            .max(effective_limit * 5)
            .min(total_memories)
            .min(MAX_ROOM_FETCH);

        // 3. Primary: hybrid search → post-filter to room IDs
        let hybrid_results = self.recall_hybrid(
            query,
            fetch_limit,
            None, // no tag filter
            None, // no namespace filter — rooms are cross-namespace
            RecallStrategy::Hybrid,
            0.0,
        )?;

        let mut results: Vec<SearchResult> = hybrid_results
            .into_iter()
            .filter(|sr| id_set.contains(&sr.memory.id))
            .collect();

        // 4. Fallback: if hybrid missed some room memories, score them directly.
        //
        // This happens when room memories rank below the fetch_limit boundary in
        // global search (e.g. small room in a large DB — the original #894 bug).
        // We embed the query and compute cosine similarity for the missing IDs.
        let found_ids: std::collections::HashSet<&str> =
            results.iter().map(|sr| sr.memory.id.as_str()).collect();
        let missing_ids: Vec<&str> = id_set
            .iter()
            .filter(|id| !found_ids.contains(id.as_str()))
            .map(|s| s.as_str())
            .collect();

        if !missing_ids.is_empty() {
            // Batch-fetch all missing room memories in one query (eliminates N+1).
            let missing_memories: Vec<_> = self
                .store
                .get_by_ids(&missing_ids)?
                .into_iter()
                .filter(|m| !m.deprecated)
                .collect();

            if !missing_memories.is_empty() {
                // Embed query and score each missing memory by cosine similarity.
                self.ensure_embedder()?;
                let query_emb = self
                    .embedder
                    .lock()
                    .map_err(|_| Error::lock("embedder in room fallback"))?
                    .as_ref()
                    .expect("embedder ensured above")
                    .embed(query)?;

                for mem in missing_memories {
                    let score = crate::consolidate::cosine_similarity(&query_emb, &mem.embedding);
                    results.push(SearchResult { memory: mem, score });
                }
            }
        }

        // 5. Apply min_score filter
        if min_score > 0.0 {
            results.retain(|sr| sr.score >= min_score);
        }

        // 6. Sort by score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // 7. Truncate to limit (0 = return all, no truncation)
        if limit > 0 {
            results.truncate(limit);
        }

        Ok(results)
    }

    /// Delete a room (unlink-only).
    /// Returns the number of memory links removed; memories and documents
    /// themselves are NOT deleted — they remain in their namespaces.
    pub fn delete_room(&self, room_id: &str) -> Result<usize, Error> {
        self.store.delete_room(room_id)
    }

    /// Rename a room: the registry row and every member/document link move
    /// in ONE transaction (#1202). Namespace, title, and description are
    /// preserved. Errors when the room is missing or the target ID is taken.
    pub fn rename_room(&self, old_id: &str, new_id: &str) -> Result<Room, Error> {
        self.store.rename_room(old_id, new_id)
    }

    /// Update a room's title and/or description (#1202). `None` fields are
    /// left unchanged. Returns the updated room, or `None` when the room
    /// does not exist.
    pub fn update_room(
        &self,
        room_id: &str,
        title: Option<&str>,
        description: Option<&str>,
    ) -> Result<Option<Room>, Error> {
        self.store.update_room(room_id, title, description)
    }

    /// Move a memory from one room to another (#1202), preserving the link's
    /// author/role/joined_at provenance. Returns `Ok(0)` when the memory is
    /// not a member of `from_room`, `Ok(1)` on success. The target room must
    /// exist; namespace stays untouched (no recall-cache impact).
    pub fn move_memory_to_room(
        &self,
        memory_id: &str,
        from_room: &str,
        to_room: &str,
    ) -> Result<usize, Error> {
        self.store
            .move_memory_to_room(memory_id, from_room, to_room)
    }

    /// Generate a summary of room discussion (topic clustering, no LLM needed).
    /// Enriched with embedding-distance semantic segments (#1088) when
    /// embeddings exist — these are the batching units for future
    /// segment-level LLM consolidation.
    pub fn room_summary(&self, room_id: &str) -> Result<Option<RoomSummary>, Error> {
        let mut summary = match self.store.room_summary(room_id)? {
            Some(s) => s,
            None => return Ok(None),
        };
        if summary.total_memories > 0 && summary.segments.is_none() {
            // Segment only when embeddings are present; otherwise keep None.
            let memories = self.store.recall_room(room_id, None, 0)?;
            if memories.iter().any(|m| !m.embedding.is_empty()) {
                let segmentation = self.room_segments_inner(room_id, &memories, 0.45, 12, 3)?;
                summary.segments = Some(segmentation.segments);
            }
        }
        Ok(Some(summary))
    }

    /// Get room summary with referenced documents populated.
    pub fn room_summary_with_docs(&self, room_id: &str) -> Result<Option<RoomSummary>, Error> {
        self.store.room_summary_with_docs(room_id)
    }

    /// Generate a structured summary document from room memories.
    /// API endpoint: POST /room/summary (#735)
    pub fn room_summary_document(&self, room_id: &str) -> Result<Option<RoomDocument>, Error> {
        self.store.room_summary_document(room_id)
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
        let doc = uteke.room_summary_document("doc-room").unwrap().unwrap();
        assert!(doc.sections.is_empty());
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn room_document_with_memories() {
        let uteke = open_in_memory();
        uteke
            .remember_in_room("Some fact", &[], None, None, "fact", "doc-room", "bob")
            .unwrap();

        let doc = uteke.room_summary_document("doc-room").unwrap().unwrap();
        assert_eq!(doc.room_id, "doc-room");
        assert_eq!(doc.sections.len(), 1); // Research & Facts
    }
    #[test]
    fn room_document_nonexistent_returns_none() {
        let uteke = open_in_memory();
        assert!(uteke.room_summary_document("nope").unwrap().is_none());
    }

    #[test]
    fn count_all_memories_empty() {
        let uteke = open_in_memory();
        assert_eq!(uteke.store.count_all_memories().unwrap(), 0);
    }

    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn count_all_memories_with_data() {
        let uteke = open_in_memory();
        uteke
            .remember_in_room("Fact one", &[], None, None, "fact", "r", "a")
            .unwrap();
        uteke
            .remember_in_room("Fact two", &[], None, None, "fact", "r", "a")
            .unwrap();
        assert_eq!(uteke.store.count_all_memories().unwrap(), 2);
    }

    /// Regression test for #894: semantic room recall must return results
    /// even when the room has few memories in a large database.
    /// The old fixed cap of 200 caused small rooms to return zero.
    #[test]
    #[ignore = "requires ONNX embedder (model download) in CI"]
    fn recall_room_semantic_small_room_large_db() {
        let uteke = open_in_memory();

        // Fill DB with 30+ unrelated memories to ensure the room's memories
        // are a small fraction of the total. The old fixed cap of 200 would
        // miss them; the new dynamic fetch_limit scales to total_memories.
        for i in 0..30 {
            uteke
                .remember(
                    &format!("Noise memory number {i} about weather and cooking"),
                    &[],
                    None,
                    Some("noise-ns"),
                )
                .unwrap();
        }

        // Create a small room with 2 memories
        uteke
            .remember_in_room(
                "Rust is a systems programming language",
                &[],
                None,
                None,
                "fact",
                "small-room",
                "alice",
            )
            .unwrap();
        uteke
            .remember_in_room(
                "Rust uses ownership for memory safety",
                &[],
                None,
                None,
                "fact",
                "small-room",
                "alice",
            )
            .unwrap();

        // Semantic recall must find room memories despite small room size
        let results = uteke
            .recall_room_semantic("small-room", "Rust programming", 10, None, 0.0)
            .unwrap();
        assert!(
            !results.is_empty(),
            "small room must return semantic results (#894)"
        );
        assert!(results.iter().any(|r| r.memory.content.contains("Rust")));
    }

    // ── Room rename / update / memory room-move (#1202) ─────────────

    /// Embedder-less engine — these ops never touch embeddings, and CI has
    /// no ONNX runtime (same pattern as operations::namespace_management_tests).
    fn open_no_embedder() -> crate::Uteke {
        crate::Uteke::open_with_backend(":memory:", None).unwrap()
    }

    #[test]
    fn rename_room_moves_registry_and_links() {
        let uteke = open_no_embedder();
        uteke
            .create_room("old-room", Some("Old"), "default")
            .unwrap();
        let m1 = uteke.remember("m one", &[], None, Some("default")).unwrap();
        let m2 = uteke.remember("m two", &[], None, Some("default")).unwrap();
        uteke
            .store
            .link_memory_to_room("old-room", &m1, "alice", "participant")
            .unwrap();
        uteke
            .store
            .link_memory_to_room("old-room", &m2, "bob", "lead")
            .unwrap();

        let room = uteke.rename_room("old-room", "new-room").unwrap();
        assert_eq!(room.id, "new-room");
        assert_eq!(room.title, Some("Old".to_string()));
        assert_eq!(room.namespace, "default");

        // Old name is gone, members followed.
        assert!(uteke.get_room("old-room").unwrap().is_none());
        assert_eq!(uteke.recall_room("new-room", None, 0).unwrap().len(), 2);
        // Cross-namespace room recall is author-filterable on the new ID.
        assert_eq!(
            uteke.recall_room("new-room", Some("bob"), 0).unwrap().len(),
            1
        );
    }

    #[test]
    fn rename_room_preserves_document_links() {
        let store = crate::memory::Store::open(":memory:").unwrap();
        store.create_room("r-old", None, "default").unwrap();
        let doc = crate::memory::documents::Document {
            id: "doc-1".to_string(),
            slug: "doc-1".to_string(),
            title: "Doc One".to_string(),
            content: "# Doc One\nContent".to_string(),
            namespace: None,
            author: None,
            tags: vec![],
            metadata: serde_json::json!({}),
            version: 1,
            content_type: "markdown".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            parent_id: None,
            path: "/doc-1/".to_string(),
            depth: 0,
            sort_order: 0,
            has_children: false,
        };
        store.upsert_document(&doc).unwrap();
        store.room_add_document("r-old", "doc-1").unwrap();

        store.rename_room("r-old", "r-new").unwrap();

        assert_eq!(store.room_list_documents("r-new").unwrap(), vec!["doc-1"]);
        assert!(store.room_list_documents("r-old").unwrap().is_empty());
    }

    #[test]
    fn rename_room_rejects_missing_source_and_taken_target() {
        let uteke = open_no_embedder();
        uteke.create_room("a", None, "default").unwrap();
        uteke.create_room("b", None, "default").unwrap();

        let missing = uteke.rename_room("nope", "c");
        assert!(matches!(missing, Err(crate::Error::Validation(_))));

        let taken = uteke.rename_room("a", "b");
        assert!(matches!(taken, Err(crate::Error::Validation(_))));

        let same = uteke.rename_room("a", "a");
        assert!(matches!(same, Err(crate::Error::Validation(_))));
    }

    #[test]
    fn update_room_sets_title_and_description() {
        let uteke = open_no_embedder();
        uteke.create_room("r", Some("Title"), "default").unwrap();

        // Description only — title must survive.
        let room = uteke
            .update_room("r", None, Some("First description"))
            .unwrap();
        let room = room.expect("room exists");
        assert_eq!(room.title, Some("Title".to_string()));
        assert_eq!(room.description, Some("First description".to_string()));

        // Overwrite both.
        let room = uteke
            .update_room("r", Some("New title"), Some("Updated desc"))
            .unwrap()
            .unwrap();
        assert_eq!(room.title, Some("New title".to_string()));
        assert_eq!(room.description, Some("Updated desc".to_string()));

        // Missing room → None.
        assert!(
            uteke
                .update_room("ghost", Some("x"), None)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn move_memory_between_rooms_preserves_link_provenance() {
        let uteke = open_no_embedder();
        uteke.create_room("src", None, "default").unwrap();
        uteke.create_room("dst", None, "default").unwrap();
        let mid = uteke
            .remember("traveler", &[], None, Some("default"))
            .unwrap();
        uteke
            .store
            .link_memory_to_room("src", &mid, "carol", "moderator")
            .unwrap();

        let moved = uteke.move_memory_to_room(&mid, "src", "dst").unwrap();
        assert_eq!(moved, 1);

        // Out of source, present in target.
        assert!(uteke.recall_room("src", None, 0).unwrap().is_empty());
        let dst = uteke.recall_room("dst", None, 0).unwrap();
        assert_eq!(dst.len(), 1);
        assert_eq!(dst[0].id, mid);

        // Link provenance (author/role) moved with the link.
        let (author, role): (String, String) = uteke
            .store()
            .conn
            .query_row(
                "SELECT author, role FROM room_memories WHERE room_id = 'dst' AND memory_id = ?1",
                [&mid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!((author.as_str(), role.as_str()), ("carol", "moderator"));

        // Move again — nothing left in source → Ok(0), store untouched.
        assert_eq!(uteke.move_memory_to_room(&mid, "src", "dst").unwrap(), 0);
    }

    #[test]
    fn move_memory_rejects_bad_targets() {
        let uteke = open_no_embedder();
        uteke.create_room("s", None, "default").unwrap();
        let mid = uteke.remember("x", &[], None, Some("default")).unwrap();
        uteke
            .store
            .link_memory_to_room("s", &mid, "a", "participant")
            .unwrap();

        // Target room missing.
        let missing_target = uteke.move_memory_to_room(&mid, "s", "nope");
        assert!(matches!(missing_target, Err(crate::Error::Validation(_))));

        // Same room.
        let same = uteke.move_memory_to_room(&mid, "s", "s");
        assert!(matches!(same, Err(crate::Error::Validation(_))));
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
