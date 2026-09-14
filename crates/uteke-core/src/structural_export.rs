//! Structural export/import — full-store round-trip (#1057).
//!
//! Single NDJSON stream with a manifest first line and tagged rows after:
//!
//! ```text
//! {"uteke_export":{"format_version":1,"sections":{...}}}
//! {"type":"memory",...}
//! {"type":"room",...}
//! {"type":"room_memory",...}
//! ...
//! ```
//!
//! Design notes:
//! - Single stream (no zip/tar dependency) — every line is a JSON object,
//!   filterable with standard ndjson tooling (`grep '"type":"room"'`).
//! - `manifest.format_version` gives future migrations a hook.
//! - Rows are dumped from SQLite directly (junction tables included) so
//!   cross-referenced ids survive the round-trip verbatim.
//! - Import detects the manifest line; a plain `ExportEntry` stream (old
//!   format) falls through to the existing `import()` path unchanged.
//!
//! Deprecated (soft-deleted) memories are excluded from the memory section
//! (same policy as `export()`); every other table dumps in full.

use crate::error::Error;

/// Current structural export format version.
pub const STRUCTURAL_EXPORT_VERSION: u32 = 1;

/// Section names in canonical dump order.
pub const SECTIONS: &[&str] = &[
    "memories",
    "rooms",
    "room_memories",
    "room_documents",
    "graph_nodes",
    "graph_edges",
    "memory_edges",
    "documents",
    "document_chunks",
    "timeline_events",
];

impl crate::Uteke {
    /// Export the FULL store structure: memories, rooms (+junctions), graph,
    /// memory edges, documents (+chunks), timeline events (#1057).
    ///
    /// Embeddings are not exported (memories re-embed on import via the
    /// legacy path); all other columns are preserved verbatim.
    pub fn export_full(&self) -> Result<String, Error> {
        let conn = self.graph_store();
        let mut lines: Vec<String> = Vec::new();

        // Manifest first line — import detects this to route here.
        let mut sections = serde_json::Map::new();
        for s in SECTIONS {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {s}"), [], |r| r.get(0))
                .unwrap_or(0);
            sections.insert((*s).to_string(), serde_json::json!(count));
        }
        let manifest = serde_json::json!({
            "uteke_export": {
                "format_version": STRUCTURAL_EXPORT_VERSION,
                "sections": sections,
            }
        });
        lines.push(
            serde_json::to_string(&manifest).map_err(|e| Error::db("manifest serialize", e))?,
        );

        // memories — active rows only, all fields, embedding dropped (re-embeds).
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, content, tags, metadata, created_at, updated_at, namespace, \
                     access_count, last_accessed, deprecated, valid_from, valid_until, \
                     memory_type, importance, pinned, content_type, slug, source, source_type \
                     FROM memories WHERE deprecated = 0",
                )
                .map_err(|e| Error::db("dump memories prepare", e))?;
            let rows = stmt
                .query_map([], |row| {
                    let tags_raw = row.get::<_, String>(2).unwrap_or_default();
                    let tags: Vec<String> =
                        serde_json::from_str(tags_raw.as_str()).unwrap_or_default();
                    let meta_raw = row.get::<_, String>(3).unwrap_or_default();
                    let metadata: serde_json::Value = serde_json::from_str(meta_raw.as_str())
                        .unwrap_or(serde_json::Value::Object(Default::default()));
                    let id = row.get::<_, String>(0)?;
                    let content = row.get::<_, String>(1)?;
                    let created_at = row.get::<_, String>(4)?;
                    let updated_at = row.get::<_, String>(5)?;
                    let namespace = row.get::<_, String>(6)?;
                    let access_count = row.get::<_, i64>(7)?;
                    let last_accessed = row.get::<_, Option<String>>(8)?;
                    let valid_from = row.get::<_, Option<String>>(10)?;
                    let valid_until = row.get::<_, Option<String>>(11)?;
                    let memory_type = row.get::<_, String>(12)?;
                    let importance = row.get::<_, f64>(13)?;
                    let pinned = row.get::<_, i64>(14)? != 0;
                    let content_type = row.get::<_, String>(15)?;
                    let slug = row.get::<_, Option<String>>(16)?;
                    let source = row.get::<_, Option<String>>(17)?;
                    let source_type = row.get::<_, String>(18)?;
                    Ok(serde_json::json!({
                        "type": "memory",
                        "id": id,
                        "content": content,
                        "tags": tags,
                        "metadata": metadata,
                        "created_at": created_at,
                        "updated_at": updated_at,
                        "namespace": namespace,
                        "access_count": access_count,
                        "last_accessed": last_accessed,
                        // col 9 = deprecated (always 0 here — filtered above)
                        "valid_from": valid_from,
                        "valid_until": valid_until,
                        "memory_type": memory_type,
                        "importance": importance,
                        "pinned": pinned,
                        "content_type": content_type,
                        "slug": slug,
                        "source": source,
                        "source_type": source_type,
                    }))
                })
                .map_err(|e| Error::db("dump memories query", e))?;
            for row in rows {
                let v = row.map_err(|e| Error::db("dump memories row", e))?;
                lines.push(
                    serde_json::to_string(&v)
                        .map_err(|e| Error::db("dump memories serialize", e))?,
                );
            }
        }

        // Simple full-table dumps for the structural tables.
        let simple_tables: &[(&str, &str)] = &[
            (
                "room",
                "SELECT id, title, namespace, created_at, updated_at, description \
                 FROM rooms",
            ),
            (
                "room_memory",
                "SELECT room_id, memory_id, author, joined_at FROM room_memories",
            ),
            (
                "room_document",
                "SELECT room_id, doc_slug, added_at FROM room_documents",
            ),
            (
                "graph_node",
                "SELECT id, label, entity_type, properties_json, memory_id, created_at FROM graph_nodes",
            ),
            (
                "graph_edge",
                "SELECT id, source_id, target_id, relation, weight, created_at FROM graph_edges",
            ),
            (
                "memory_edge",
                "SELECT source_id, target_id, edge_type, created_at FROM memory_edges",
            ),
            (
                "timeline_event",
                "SELECT id, memory_id, event_type, event_data, created_at FROM timeline_events",
            ),
        ];

        for (tag, sql) in simple_tables {
            let mut stmt = conn
                .prepare(sql)
                .map_err(|e| Error::db_msg(format!("dump {tag} prepare: {e}")))?;
            let col_count = stmt.column_count();
            let rows = stmt
                .query_map([], |row| {
                    let mut arr: Vec<serde_json::Value> = Vec::with_capacity(col_count);
                    for i in 0..col_count {
                        let v = match row.get_ref(i)? {
                            rusqlite::types::ValueRef::Null => serde_json::Value::Null,
                            rusqlite::types::ValueRef::Integer(n) => serde_json::json!(n),
                            rusqlite::types::ValueRef::Real(f) => serde_json::json!(f),
                            rusqlite::types::ValueRef::Text(t) => {
                                serde_json::json!(String::from_utf8_lossy(t))
                            }
                            rusqlite::types::ValueRef::Blob(_) => serde_json::Value::Null,
                        };
                        arr.push(v);
                    }
                    Ok((tag, arr))
                })
                .map_err(|e| Error::db_msg(format!("dump {tag} query: {e}")))?;
            for row in rows {
                let (tag, arr) = row.map_err(|e| Error::db_msg(format!("dump {tag} row: {e}")))?;
                let v = serde_json::json!({"type": tag, "row": arr});
                lines.push(
                    serde_json::to_string(&v)
                        .map_err(|e| Error::db_msg(format!("dump {tag} serialize: {e}")))?,
                );
            }
        }

        // documents (full content) + document_chunks (content for re-chunking).
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, slug, title, content, namespace, author, tags, metadata, \
                     version, content_type, created_at, updated_at, parent_id, depth, \
                     sort_order, has_children FROM documents",
                )
                .map_err(|e| Error::db("dump documents prepare", e))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(serde_json::json!({
                        "type": "document",
                        "id": row.get::<_, String>(0)?,
                        "slug": row.get::<_, String>(1)?,
                        "title": row.get::<_, String>(2)?,
                        "content": row.get::<_, String>(3)?,
                        "namespace": row.get::<_, Option<String>>(4)?,
                        "author": row.get::<_, Option<String>>(5)?,
                        "tags": serde_json::from_str::<Vec<String>>(
                            row.get::<_, String>(6).unwrap_or_default().as_str()
                        ).unwrap_or_default(),
                        "metadata": serde_json::from_str(
                            row.get::<_, String>(7).unwrap_or_default().as_str()
                        ).unwrap_or(serde_json::json!({})),
                        "version": row.get::<_, i64>(8)?,
                        "content_type": row.get::<_, String>(9)?,
                        "created_at": row.get::<_, String>(10)?,
                        "updated_at": row.get::<_, String>(11)?,
                        "parent_id": row.get::<_, Option<String>>(12)?,
                        "depth": row.get::<_, i64>(13)?,
                        "sort_order": row.get::<_, i64>(14)?,
                        "has_children": row.get::<_, i64>(15)? != 0,
                    }))
                })
                .map_err(|e| Error::db("dump documents query", e))?;
            for row in rows {
                let v = row.map_err(|e| Error::db("dump documents row", e))?;
                lines.push(
                    serde_json::to_string(&v)
                        .map_err(|e| Error::db("dump documents serialize", e))?,
                );
            }
        }
        {
            let mut stmt = conn
                .prepare(
                    "SELECT id, document_id, chunk_index, heading, content, char_start, char_end, tags, created_at \
                     FROM document_chunks",
                )
                .map_err(|e| Error::db("dump chunks prepare", e))?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(serde_json::json!({
                        "type": "document_chunk",
                        "id": row.get::<_, String>(0)?,
                        "document_id": row.get::<_, String>(1)?,
                        "chunk_index": row.get::<_, i64>(2)?,
                        "heading": row.get::<_, String>(3)?,
                        "content": row.get::<_, String>(4)?,
                        "char_start": row.get::<_, i64>(5)?,
                        "char_end": row.get::<_, i64>(6)?,
                        "tags": serde_json::from_str::<Vec<String>>(
                            row.get::<_, String>(7).unwrap_or_default().as_str()
                        ).unwrap_or_default(),
                        "created_at": row.get::<_, String>(8)?,
                    }))
                })
                .map_err(|e| Error::db("dump chunks query", e))?;
            for row in rows {
                let v = row.map_err(|e| Error::db("dump chunks row", e))?;
                lines.push(
                    serde_json::to_string(&v).map_err(|e| Error::db("dump chunks serialize", e))?,
                );
            }
        }

        Ok(lines.join("\n"))
    }

    /// Import a structural export produced by [`Uteke::export_full`] (#1057).
    ///
    /// Restores all sections with original ids intact (rooms point at the
    /// restored memories; graph/edges/junctions reference the same ids).
    /// Existing rows with the same primary key are skipped (idempotent-ish
    /// merge, not a wipe-and-replace).
    ///
    /// Returns per-section inserted counts.
    pub fn import_full(&self, input: &str) -> Result<serde_json::Value, Error> {
        let conn = self.graph_store();

        // Atomic restore: everything in one transaction — a failure at any
        // section rolls the whole import back (partial restores must not
        // leave dangling junction rows).
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| Error::db_msg(format!("import_full begin tx: {e}")))?;
        let result = Self::import_full_tx(&tx, input)?;
        tx.commit()
            .map_err(|e| Error::db_msg(format!("import_full commit: {e}")))?;
        Ok(result)
    }

    /// Transactional body of [`Uteke::import_full`] — all statements run
    /// against `conn` (the open transaction); commit happens in the caller.
    fn import_full_tx(
        conn: &rusqlite::Connection,
        input: &str,
    ) -> Result<serde_json::Value, Error> {
        let mut counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();

        // Two-pass import: collect all rows first, then restore in
        // dependency order — memories BEFORE room_memories/memory_edges/
        // timeline_events (FK REFERENCES memories), documents BEFORE
        // document_chunks, graph nodes BEFORE graph edges.
        let mut pending_memories: Vec<serde_json::Value> = Vec::new();
        let mut pending_documents: Vec<serde_json::Value> = Vec::new();
        let mut pending_chunks: Vec<serde_json::Value> = Vec::new();
        let mut pending_nodes: Vec<serde_json::Value> = Vec::new();
        let mut pending_graph_edges: Vec<serde_json::Value> = Vec::new();
        // room_documents has no FK on doc_slug, but restore it after documents
        // (pass 2) for clarity of dependency ordering (#1078).
        let mut pending_room_documents: Vec<serde_json::Value> = Vec::new();
        let mut rest: Vec<serde_json::Value> = Vec::new();

        for line in input.lines().filter(|l| !l.trim().is_empty()) {
            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| Error::db_msg(format!("structural import parse: {e}")))?;

            if v.get("uteke_export").is_some() {
                continue; // manifest line
            }
            let tag = v["type"].as_str().unwrap_or("");
            match tag {
                "memory" => pending_memories.push(v),
                "document" => pending_documents.push(v),
                "document_chunk" => pending_chunks.push(v),
                "graph_node" => pending_nodes.push(v),
                "graph_edge" => pending_graph_edges.push(v),
                "room_document" => pending_room_documents.push(v),
                _ => rest.push(v),
            }
        }

        // ── Pass 2: restore in dependency order ─────────────────────────
        // memories first (sentinel embeddings; repair --reembed regenerates).
        {
            for m in &pending_memories {
                let tags = serde_json::to_string(&m["tags"]).unwrap_or_else(|_| "[]".into());
                let metadata =
                    serde_json::to_string(&m["metadata"]).unwrap_or_else(|_| "{}".into());
                // Embedding is deliberately NULL: load_all() filters NULL
                // embeddings out of index builds (#992), and repair --reembed
                // scans for exactly these rows. A sentinel blob would leak
                // into index.build() and crash on dimension mismatch.
                let n = conn
                    .execute(
                        "INSERT OR IGNORE INTO memories (id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, last_accessed, deprecated, valid_from, valid_until, memory_type, importance, pinned, content_type, slug, source, source_type) \
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11,?12,?13,?14,?15,?16,?17,?18,?19)",
                        rusqlite::params![
                            m["id"].as_str().unwrap_or(""),
                            m["content"].as_str().unwrap_or(""),
                            Option::<Vec<u8>>::None, // embedding NULL (see comment above)
                            tags,
                            metadata,
                            m["created_at"].as_str().unwrap_or(""),
                            m["updated_at"].as_str().unwrap_or(""),
                            m["namespace"].as_str().unwrap_or("default"),
                            m["access_count"].as_i64().unwrap_or(0),
                            m["last_accessed"].as_str(),
                            m["valid_from"].as_str(),
                            m["valid_until"].as_str(),
                            m["memory_type"].as_str().unwrap_or("fact"),
                            m["importance"].as_f64().unwrap_or(0.5),
                            m["pinned"].as_bool().unwrap_or(false),
                            m["content_type"].as_str().unwrap_or("text"),
                            m["slug"].as_str(),
                            m["source"].as_str(),
                            m["source_type"].as_str().unwrap_or("import"),
                        ],
                    )
                    .map_err(|e| Error::db_msg(format!("import memory: {e}")))?;
                *counts.entry("memories".into()).or_default() += n;
            }
        }

        // rooms + junctions + memory_edges + timeline (all FK-safe now).
        for v in &rest {
            let tag = v["type"].as_str().unwrap_or("");
            match tag {
                "room" => {
                    let r = &v["row"];
                    // Column-count aware: exports predating description
                    // (#1202) carry 5 columns; new exports carry 6. Both
                    // import cleanly (description defaults to NULL).
                    let description = r.get(5).and_then(|v| v.as_str());
                    let n = conn
                        .execute(
                            "INSERT OR IGNORE INTO rooms (id, title, namespace, created_at, updated_at, description) VALUES (?1,?2,?3,?4,?5,?6)",
                            rusqlite::params![
                                r[0].as_str().unwrap_or(""),
                                r[1].as_str(),
                                r[2].as_str().unwrap_or("default"),
                                r[3].as_str().unwrap_or(""),
                                r[4].as_str().unwrap_or(""),
                                description,
                            ],
                        )
                        .map_err(|e| Error::db_msg(format!("import room: {e}")))?;
                    *counts.entry("rooms".into()).or_default() += n;
                }
                "room_memory" => {
                    let r = &v["row"];
                    let n = conn
                        .execute(
                            "INSERT OR IGNORE INTO room_memories (room_id, memory_id, author, joined_at) VALUES (?1,?2,?3,?4)",
                            rusqlite::params![
                                r[0].as_str().unwrap_or(""),
                                r[1].as_str().unwrap_or(""),
                                r[2].as_str().unwrap_or("imported"),
                                r[3].as_str().unwrap_or(""),
                            ],
                        )
                        .map_err(|e| Error::db_msg(format!("import room_memory: {e}")))?;
                    *counts.entry("room_memories".into()).or_default() += n;
                }
                "memory_edge" => {
                    let r = &v["row"];
                    let n = conn
                        .execute(
                            "INSERT OR IGNORE INTO memory_edges (source_id, target_id, edge_type, created_at) VALUES (?1,?2,?3,?4)",
                            rusqlite::params![
                                r[0].as_str().unwrap_or(""),
                                r[1].as_str().unwrap_or(""),
                                r[2].as_str().unwrap_or(""),
                                r[3].as_str().unwrap_or(""),
                            ],
                        )
                        .map_err(|e| Error::db_msg(format!("import memory_edge: {e}")))?;
                    *counts.entry("memory_edges".into()).or_default() += n;
                }
                "timeline_event" => {
                    let r = &v["row"];
                    let n = conn
                        .execute(
                            "INSERT OR IGNORE INTO timeline_events (id, memory_id, event_type, event_data, created_at) VALUES (?1,?2,?3,?4,?5)",
                            rusqlite::params![
                                r[0].as_i64().unwrap_or(0),
                                r[1].as_str().unwrap_or(""),
                                r[2].as_str().unwrap_or(""),
                                r[3].as_str(),
                                r[4].as_str().unwrap_or(""),
                            ],
                        )
                        .map_err(|e| Error::db_msg(format!("import timeline_event: {e}")))?;
                    *counts.entry("timeline_events".into()).or_default() += n;
                }
                _ => {
                    // Unknown tag — ignore for forward compatibility (newer
                    // export format version writing sections this build does
                    // not know must not fail the whole import).
                }
            }
        }

        // documents (before chunks — FK document_id → documents.id)
        for v in &pending_documents {
            let tags = serde_json::to_string(&v["tags"]).unwrap_or_else(|_| "[]".into());
            let metadata = serde_json::to_string(&v["metadata"]).unwrap_or_else(|_| "{}".into());
            let n = conn
                .execute(
                    "INSERT OR IGNORE INTO documents (id, slug, title, content, namespace, author, tags, metadata, version, content_type, created_at, updated_at, parent_id, depth, sort_order, has_children) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",
                    rusqlite::params![
                        v["id"].as_str().unwrap_or(""),
                        v["slug"].as_str().unwrap_or(""),
                        v["title"].as_str().unwrap_or(""),
                        v["content"].as_str().unwrap_or(""),
                        v["namespace"].as_str(),
                        v["author"].as_str(),
                        tags,
                        metadata,
                        v["version"].as_i64().unwrap_or(1),
                        v["content_type"].as_str().unwrap_or("markdown"),
                        v["created_at"].as_str().unwrap_or(""),
                        v["updated_at"].as_str().unwrap_or(""),
                        v["parent_id"].as_str(),
                        v["depth"].as_i64().unwrap_or(0),
                        v["sort_order"].as_i64().unwrap_or(0),
                        v["has_children"].as_bool().unwrap_or(false),
                    ],
                )
                .map_err(|e| Error::db_msg(format!("import document: {e}")))?;
            *counts.entry("documents".into()).or_default() += n;
        }

        // document_chunks
        for v in &pending_chunks {
            let tags = serde_json::to_string(&v["tags"]).unwrap_or_else(|_| "[]".into());
            let n = conn
                .execute(
                    "INSERT OR IGNORE INTO document_chunks (id, document_id, chunk_index, heading, content, char_start, char_end, tags, created_at) \
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    rusqlite::params![
                        v["id"].as_str().unwrap_or(""),
                        v["document_id"].as_str().unwrap_or(""),
                        v["chunk_index"].as_i64().unwrap_or(0),
                        v["heading"].as_str().unwrap_or(""),
                        v["content"].as_str().unwrap_or(""),
                        v["char_start"].as_i64().unwrap_or(0),
                        v["char_end"].as_i64().unwrap_or(0),
                        tags,
                        v["created_at"].as_str().unwrap_or(""),
                    ],
                )
                .map_err(|e| Error::db_msg(format!("import document_chunk: {e}")))?;
            *counts.entry("document_chunks".into()).or_default() += n;
        }

        // graph_nodes (before graph_edges — FK source/target → graph_nodes.id)
        for v in &pending_nodes {
            let r = &v["row"];
            // properties_json is stored as a JSON *string* in SQLite and
            // dumped verbatim into the row array — use it as-is; re-encoding
            // the parsed value would double-encode (cora finding).
            let props = r[3].as_str().unwrap_or("{}").to_string();
            let n = conn
                .execute(
                    "INSERT OR IGNORE INTO graph_nodes (id, label, entity_type, properties_json, memory_id, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![
                        r[0].as_str().unwrap_or(""),
                        r[1].as_str().unwrap_or(""),
                        r[2].as_str(),
                        props,
                        r[4].as_str(),
                        r[5].as_str().unwrap_or(""),
                    ],
                )
                .map_err(|e| Error::db_msg(format!("import graph_node: {e}")))?;
            *counts.entry("graph_nodes".into()).or_default() += n;
        }

        // graph_edges
        for v in &pending_graph_edges {
            let r = &v["row"];
            let n = conn
                .execute(
                    "INSERT OR IGNORE INTO graph_edges (id, source_id, target_id, relation, weight, created_at) VALUES (?1,?2,?3,?4,?5,?6)",
                    rusqlite::params![
                        r[0].as_str().unwrap_or(""),
                        r[1].as_str().unwrap_or(""),
                        r[2].as_str().unwrap_or(""),
                        r[3].as_str().unwrap_or(""),
                        r[4].as_f64().unwrap_or(1.0),
                        r[5].as_str().unwrap_or(""),
                    ],
                )
                .map_err(|e| Error::db_msg(format!("import graph_edge: {e}")))?;
            *counts.entry("graph_edges".into()).or_default() += n;
        }

        for v in &pending_room_documents {
            let r = &v["row"];
            let n = conn
                .execute(
                    "INSERT OR IGNORE INTO room_documents (room_id, doc_slug, added_at) VALUES (?1,?2,?3)",
                    rusqlite::params![
                        r[0].as_str().unwrap_or(""),
                        r[1].as_str().unwrap_or(""),
                        r[2].as_str().unwrap_or(""),
                    ],
                )
                .map_err(|e| Error::db_msg(format!("import room_document: {e}")))?;
            *counts.entry("room_documents".into()).or_default() += n;
        }
        // Sentinel-embedded rows must NOT enter the vector index (dimension
        // mismatch); load_all filters NULL embeddings but these are non-NULL.
        // Repair with --reembed fixes them; flag the count in the result.
        let sentinel_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE embedding IS NULL OR LENGTH(embedding) = 0",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        // FTS rebuild so the restored memories are keyword-searchable.
        conn.execute_batch("INSERT INTO memories_fts(memories_fts) VALUES ('rebuild');")
            .map_err(|e| Error::db_msg(format!("fts rebuild after structural import: {e}")))?;

        Ok(serde_json::json!({
            "imported": counts,
            "needs_reembed": sentinel_count,
            "hint": if sentinel_count > 0 { "run `uteke repair --reembed` to regenerate embeddings" } else { "" },
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::Uteke;

    /// #1057: full round-trip — seed store with memory+room+doc, export,
    /// import into a FRESH store, verify ids and structure survived.
    #[test]
    fn test_structural_export_import_roundtrip() {
        let dir_a = std::env::temp_dir().join(format!("sx-a-{}", std::process::id()));
        let dir_b = std::env::temp_dir().join(format!("sx-b-{}", std::process::id()));
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let src = Uteke::open(dir_a.join("t.db").to_str().unwrap()).unwrap();
        let conn = src.graph_store();

        // Seed: memory, room + link, memory edge, document + chunk.
        let now = "2026-08-17T00:00:00Z";
        conn.execute(
            "INSERT INTO memories (id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, deprecated, memory_type, importance, pinned, content_type, source_type) \
             VALUES ('m1','structural roundtrip content',NULL,'[\"t\"]','{}',?1,?1,'sx-ns',0,0,'fact',0.5,0,'text','user')",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO rooms (id, title, namespace, created_at, updated_at) VALUES ('r1','roundtrip room','sx-ns',?1,?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO room_memories (room_id, memory_id, author, joined_at) VALUES ('r1','m1','alice',?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO memories (id, content, embedding, tags, metadata, created_at, updated_at, namespace, access_count, deprecated, memory_type, importance, pinned, content_type, source_type) \
             VALUES ('m2','second memory',NULL,'[]','{}',?1,?1,'sx-ns',0,0,'fact',0.5,1,'text','user')",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO memory_edges (source_id, target_id, edge_type, created_at) VALUES ('m1','m2','related',?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO documents (id, slug, title, content, namespace, author, tags, metadata, version, content_type, created_at, updated_at, depth, sort_order, has_children) \
             VALUES ('d1','rt-doc','RT Doc','# Title\ncontent body',NULL,NULL,'[]','{}',1,'markdown',?1,?1,0,0,0)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO document_chunks (id, document_id, chunk_index, heading, content, char_start, char_end, tags, created_at) \
             VALUES ('c1','d1',0,'# Title','# Title\ncontent body',0,20,'[]',?1)",
            [now],
        ).unwrap();
        // Seed every remaining exported section so the round-trip exercises
        // each match arm end-to-end (#1078: a section missing from the test
        // fixture leaves delete-arm mutants uncatchable).
        conn.execute(
            "INSERT INTO room_documents (room_id, doc_slug, added_at) VALUES ('r1','rt-doc',?1)",
            [now],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO graph_nodes (id, label, entity_type, properties_json, created_at) VALUES ('gn1','n1','concept','{}',?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO graph_nodes (id, label, entity_type, properties_json, created_at) VALUES ('gn2','n2','concept','{}',?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO graph_edges (id, source_id, target_id, relation, weight, created_at) VALUES ('ge1','gn1','gn2','relates',0.5,?1)",
            [now],
        ).unwrap();
        conn.execute(
            "INSERT INTO timeline_events (memory_id, event_type, event_data, created_at) VALUES ('m1','created','{}',?1)",
            [now],
        ).unwrap();

        let exported = src.export_full().unwrap();
        let first_line = exported.lines().next().unwrap();
        let manifest: serde_json::Value = serde_json::from_str(first_line).unwrap();
        assert!(manifest["uteke_export"].is_object(), "manifest first line");
        assert_eq!(manifest["uteke_export"]["format_version"], 1);
        assert_eq!(manifest["uteke_export"]["sections"]["memories"], 2);
        for (section, expected) in [
            ("memories", 2),
            ("rooms", 1),
            ("room_memories", 1),
            ("room_documents", 1),
            ("memory_edges", 1),
            ("documents", 1),
            ("document_chunks", 1),
            ("graph_nodes", 2),
            ("graph_edges", 1),
            ("timeline_events", 1),
        ] {
            assert_eq!(
                manifest["uteke_export"]["sections"][section], expected,
                "manifest count for {section}"
            );
        }

        // Import into a fresh store.
        let dst = Uteke::open(dir_b.join("t.db").to_str().unwrap()).unwrap();
        let result = dst.import_full(&exported).unwrap();
        assert_eq!(result["imported"]["memories"], 2);
        assert_eq!(result["imported"]["rooms"], 1);
        assert_eq!(result["imported"]["room_memories"], 1);
        assert_eq!(result["imported"]["memory_edges"], 1);
        assert_eq!(result["imported"]["documents"], 1);
        assert_eq!(result["imported"]["document_chunks"], 1);
        for (section, expected) in [
            ("memories", 2),
            ("rooms", 1),
            ("room_memories", 1),
            ("room_documents", 1),
            ("memory_edges", 1),
            ("documents", 1),
            ("document_chunks", 1),
            ("graph_nodes", 2),
            ("graph_edges", 1),
            ("timeline_events", 1),
        ] {
            assert_eq!(
                result["imported"][section], expected,
                "imported count for {section}"
            );
        }

        // ids intact: room links at restored memory ids.
        let dconn = dst.graph_store();
        let linked: String = dconn
            .query_row(
                "SELECT memory_id FROM room_memories WHERE room_id='r1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(linked, "m1", "room points at restored memory id");
        // pinned flag must survive the round-trip verbatim
        let pinned: i64 = dconn
            .query_row("SELECT pinned FROM memories WHERE id='m2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(pinned, 1, "pinned flag round-trips");
        let doc: String = dconn
            .query_row("SELECT slug FROM documents WHERE id='d1'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(doc, "rt-doc");
        // chunk created_at must survive the round-trip (code-scanning alert:
        // a missing column in the dump SELECT silently restored empty strings)
        let chunk_ts: String = dconn
            .query_row(
                "SELECT created_at FROM document_chunks WHERE id='c1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(chunk_ts, now, "chunk created_at round-trips verbatim");

        // Idempotent: re-import inserts nothing (OR IGNORE).
        let again = dst.import_full(&exported).unwrap();
        assert_eq!(again["imported"]["memories"], 0, "second import is a no-op");

        drop(src);
        drop(dst);
        std::fs::remove_dir_all(&dir_a).ok();
        std::fs::remove_dir_all(&dir_b).ok();
    }

    /// Old-format compatibility: a plain ExportEntry stream (no manifest)
    /// must NOT route into structural import — detectable via the manifest
    /// check helper.
    #[test]
    fn test_structural_export_detect() {
        // (detection is embedded in import() dispatch; here we pin the
        // manifest-shape check used for routing)
        let legacy = r#"{"content":"old format","tags":[],"metadata":{},"created_at":"2024-01-01T00:00:00Z","source":null,"namespace":"default"}"#;
        let v: serde_json::Value = serde_json::from_str(legacy).unwrap();
        assert!(
            v.get("uteke_export").is_none(),
            "legacy rows carry no manifest marker"
        );
    }
}
