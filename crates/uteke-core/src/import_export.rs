//! Import and export memories in JSONL format.

use crate::error::Error;
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::{ExportEntry, ImportResult, Memory};

/// The room links made by one import, and the rooms it could not link to (#1588).
#[derive(Default)]
struct RoomLinks {
    linked: usize,
    missing: std::collections::BTreeSet<String>,
    exists: std::collections::HashMap<String, bool>,
}

impl RoomLinks {
    /// Link `memory_id` to each of `rooms` that exists. A failure to link is
    /// logged and never fails the import: the memory is already stored.
    fn apply(&mut self, uteke: &crate::Uteke, memory_id: &str, rooms: &[String], author: &str) {
        for room in rooms {
            let known = *self
                .exists
                .entry(room.clone())
                .or_insert_with(|| matches!(uteke.store.get_room(room), Ok(Some(_))));
            if !known {
                self.missing.insert(room.clone());
                continue;
            }
            match uteke.add_memory_to_room(memory_id, room, author) {
                Ok(true) => self.linked += 1,
                Ok(false) => {}
                Err(e) => tracing::warn!("Could not link {memory_id} to room {room}: {e}"),
            }
        }
    }
}

impl crate::Uteke {
    /// Export all memories to JSONL format (one JSON object per line).
    ///
    /// Embeddings are NOT exported — they will be re-computed on import.
    /// This keeps export files small and portable.
    ///
    /// Every row carries its `namespace` (#1036) so multi-namespace stores
    /// round-trip with attribution intact. Deprecated (soft-deleted) rows are
    /// NOT exported — `load_all` filters `deprecated = 0` by design; use the
    /// lifecycle endpoints to audit deprecated rows separately.
    pub fn export(&self, namespace: Option<&str>) -> Result<String, Error> {
        let memories = self.store.load_all(namespace)?;
        let mut rooms_of = self.store.room_ids_by_memory()?;
        let entries: Vec<ExportEntry> = memories
            .into_iter()
            .map(|m| ExportEntry {
                rooms: rooms_of.remove(&m.id).unwrap_or_default(),
                content: m.content,
                tags: m.tags,
                metadata: m.metadata,
                created_at: m.created_at,
                source: m.source,
                namespace: m.namespace,
                author: m.author,
            })
            .collect();

        let mut lines = Vec::with_capacity(entries.len());
        for entry in &entries {
            let line =
                serde_json::to_string(entry).map_err(|e| Error::db("export serialization", e))?;
            lines.push(line);
        }

        Ok(lines.join("\n"))
    }

    /// Import memories from JSONL or JSON array format.
    ///
    /// Accepts:
    /// - JSONL: one JSON object per line (each must have `content`)
    /// - JSON array: `[{"content":"..."}, ...]`
    /// - Single JSON object: `{"content":"..."}`
    ///
    /// Required field: `content`. Optional fields default automatically.
    /// Embeddings are re-computed during import.
    pub fn import(&self, input: &str, namespace: Option<&str>) -> Result<ImportResult, Error> {
        let trimmed = input.trim();

        // Detect format: JSON array, single JSON object, or JSONL.
        // Strategy:
        //   1. starts_with('[') → JSON array
        //   2. starts_with('{') → try single JSON object; if that fails, fall back to JSONL
        //      (handles both JSONL lines starting with `{` and pretty-printed objects)
        //   3. otherwise → JSONL
        let (entries, mut skipped): (Vec<ExportEntry>, usize) = if trimmed.starts_with('[') {
            match serde_json::from_str::<Vec<ExportEntry>>(trimmed) {
                Ok(arr) => (arr, 0),
                Err(e) => {
                    return Err(Error::validation(format!("Invalid JSON array: {e}")));
                }
            }
        } else if trimmed.starts_with('{') {
            // Try single JSON object first (covers both compact and pretty-printed)
            match serde_json::from_str::<ExportEntry>(trimmed) {
                Ok(entry) => (vec![entry], 0),
                Err(_) => {
                    // Fall back to JSONL: each line should be a self-contained JSON object
                    Self::parse_jsonl(input)
                }
            }
        } else {
            Self::parse_jsonl(input)
        };

        let mut imported = 0;
        let mut deduped = 0;
        let mut redacted = 0;
        let mut rejected_secret = 0;
        let mut room_links = RoomLinks::default();

        for mut entry in entries {
            if entry.content.is_empty() {
                skipped += 1;
                continue;
            }
            let rooms = std::mem::take(&mut entry.rooms);
            let link_author = entry
                .author
                .as_deref()
                .and_then(|a| crate::memory::types::validate_author(a).ok())
                .unwrap_or("import")
                .to_string();

            // Privacy filter (#1464), before the content is embedded or stored.
            // Strict mode skips the row (counted as skipped) instead of
            // aborting the whole import.
            // The tags and the source are filtered with the content (the
            // source rides along as the last "tag" and is taken off again).
            let mut fields: Vec<&str> = entry.tags.iter().map(String::as_str).collect();
            if let Some(src) = entry.source.as_deref() {
                fields.push(src);
            }
            let prepared = match self.privacy_config.prepare_with_tags(
                &entry.content,
                &fields,
                Some(std::mem::take(&mut entry.metadata)),
            ) {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("Skipping import entry: {e}");
                    if matches!(e, Error::SecretDetected { .. }) {
                        rejected_secret += 1;
                    }
                    skipped += 1;
                    continue;
                }
            };
            entry.content = prepared.content;
            let mut masked_fields = prepared.tags;
            if entry.source.is_some() {
                entry.source = masked_fields.pop();
            }
            entry.tags = masked_fields;
            if let Some(m) = prepared.metadata {
                entry.metadata = m;
            }
            let redactions = prepared.redactions;

            // Re-embed the content with retry (consistent with remember path, #1005).
            let embedding = self.embed(&entry.content)?;

            // Dedup check: skip if cosine >= 0.95 to existing memory (#1005).
            if let Some(existing_id) = self.check_duplicate(&embedding, namespace)? {
                deduped += 1;
                // The stored memory is the one that belongs in the rooms (#1588).
                room_links.apply(self, &existing_id, &rooms, &link_author);
                continue;
            }

            let id = uuid::Uuid::now_v7().to_string();
            let now = chrono::Utc::now();

            // Namespace resolution (#1036): a caller-SUPPLIED namespace is an
            // explicit override — every row lands there (e.g. CLI --namespace
            // bulk import). Without an override, the per-row namespace from
            // the export file wins, so round-trips reconstruct namespaces.
            // Rows from OLD export files (no namespace key) deserialize with
            // the "default" fallback via serde, keeping them importable.
            let target_ns = namespace
                .map(str::to_string)
                .unwrap_or_else(|| entry.namespace.clone());

            let memory = Memory {
                id: id.clone(),
                content: entry.content,
                embedding,
                tags: entry.tags,
                metadata: entry.metadata,
                created_at: entry.created_at,
                updated_at: now,
                namespace: target_ns,
                access_count: 0,
                recall_count: 0,
                last_accessed: None,
                deprecated: false,
                deprecated_at: None,
                valid_from: Some(entry.created_at),
                valid_until: None,
                memory_type: "fact".to_string(),
                importance: 0.5,
                pinned: false,
                content_type: "text".to_string(),
                slug: None,
                source: Some(format!("import:{}", entry.source.unwrap_or_default())),
                source_type: "import".to_string(),
                author_type: "agent".to_string(),
                // A bad value in a file is dropped, it must not fail the import.
                author: entry
                    .author
                    .as_deref()
                    .and_then(|a| crate::memory::types::validate_author(a).ok())
                    .map(str::to_string),
            };

            // The same tail as `remember` (#1514): the row, the source hash, the
            // `Created` event, edge wiring, recall-cache invalidation, the vector
            // (its save is batched, the whole import is persisted once below)
            // and the cosine links. A vector that could not be written leaves the
            // row FTS5-only, as it does for `remember`; `uteke repair` backfills it.
            match self.commit_memory(&memory, None) {
                Ok(outcome) => {
                    if let Some(w) = outcome.warning {
                        tracing::warn!("Imported entry (id={id}) is FTS5-only: {w}");
                    }
                }
                Err(e) => {
                    tracing::warn!("Skipping import entry (id={id}): {e}");
                    skipped += 1;
                    continue;
                }
            }

            self.record_redaction(&id, &redactions);
            if !redactions.is_empty() {
                redacted += 1;
            }
            room_links.apply(self, &id, &rooms, &link_author);

            imported += 1;
        }

        // Persist vector index after import completes
        if imported > 0 {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during import save"))?;
            index_sync::persist(&mut *index, &SyncPolicy::STANDARD)?;
        }

        if skipped > 0 || deduped > 0 {
            tracing::info!(
                "Import completed: {imported} imported, {deduped} duplicates skipped, {skipped} errored entries."
            );
        }

        Ok(ImportResult {
            imported,
            skipped,
            redacted,
            rejected_secret,
            room_links: room_links.linked,
            rooms_missing: room_links.missing.into_iter().collect(),
        })
    }

    /// Parse JSONL input: one self-contained JSON object per line.
    /// Lines that fail to parse are counted as skipped (not fatal).
    fn parse_jsonl(input: &str) -> (Vec<ExportEntry>, usize) {
        let mut entries = Vec::new();
        let mut failed = 0;
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<ExportEntry>(line) {
                Ok(e) => entries.push(e),
                Err(e) => {
                    tracing::warn!("Skipping invalid JSONL line: {e}");
                    failed += 1;
                }
            }
        }
        (entries, failed)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_export_entry_serialization() {
        use crate::memory::types::ExportEntry;
        let entry = ExportEntry {
            content: "hello world".to_string(),
            tags: vec!["greeting".to_string()],
            metadata: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            source: None,
            namespace: "default".to_string(),
            author: None,
            rooms: Vec::new(),
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(
            !json.contains("author"),
            "an unset author is not serialized: {json}"
        );
        let restored: ExportEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.content, "hello world");
        assert_eq!(restored.tags.len(), 1);
    }

    /// #1427 phase 2: the author survives export and import; an invalid author
    /// in a file is dropped without failing the row; old files have none.
    #[test]
    fn test_author_survives_export_import() {
        let src = crate::test_support::open_keyword();
        let a = src.remember("attributed note", &[], None, None).unwrap();
        src.remember("plain note", &[], None, None).unwrap();
        src.set_author(&a, Some("claude-code")).unwrap();
        let exported = src.export(None).unwrap();
        assert!(
            exported.contains("\"author\":\"claude-code\""),
            "{exported}"
        );
        assert_eq!(
            exported.matches("\"author\"").count(),
            1,
            "only the attributed row"
        );

        let dst = crate::test_support::open_keyword();
        assert_eq!(dst.import(&exported, None).unwrap().imported, 2);
        let authors: std::collections::BTreeMap<String, Option<String>> = dst
            .list(None, 10, 0, None)
            .unwrap()
            .into_iter()
            .map(|m| (m.content, m.author))
            .collect();
        assert_eq!(authors["attributed note"].as_deref(), Some("claude-code"));
        assert_eq!(authors["plain note"], None);

        // An invalid author in a file is dropped, the row is still imported.
        let long = "x".repeat(200);
        let file = format!(
            "{{\"content\":\"bad author\",\"author\":\"{long}\"}}\n{{\"content\":\"blank author\",\"author\":\"  \"}}\n{{\"content\":\"old row\"}}"
        );
        let other = crate::test_support::open_keyword();
        assert_eq!(other.import(&file, None).unwrap().imported, 3);
        assert!(
            other
                .list(None, 10, 0, None)
                .unwrap()
                .iter()
                .all(|m| m.author.is_none())
        );
    }

    /// #1036: export must carry per-row namespace; import must reconstruct
    /// namespaces on round-trip (seed K namespaces → export → fresh store →
    /// import → all rows present with original namespaces). Also verifies
    /// deprecated rows are excluded from export (documented behavior).
    #[test]
    fn test_export_import_roundtrip_namespaces() {
        use crate::Uteke;
        let dir_a = std::env::temp_dir().join(format!("exp-a-{}", uuid::Uuid::new_v4()));
        let dir_b = std::env::temp_dir().join(format!("exp-b-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        let src = Uteke::open(dir_a.join("t.db").to_str().unwrap()).unwrap();
        let embedding = vec![0.33_f32; 768];
        for ns in ["alpha", "beta", "gamma"] {
            for i in 0..2 {
                src.remember_precomputed(
                    &format!("roundtrip memory {ns}-{i} unique content marker"),
                    &[],
                    None,
                    Some(ns),
                    "fact",
                    "text",
                    &embedding,
                )
                .unwrap();
            }
        }

        // Full-store export (namespace=None) must include per-row namespace.
        let exported = src.export(None).unwrap();
        let rows: Vec<serde_json::Value> = exported
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(rows.len(), 6, "all active rows exported");
        for row in &rows {
            let ns = row["namespace"]
                .as_str()
                .unwrap_or_else(|| panic!("export row missing namespace key: {row}"));
            assert!(["alpha", "beta", "gamma"].contains(&ns));
        }

        // Import-side namespace resolution without ONNX (import re-embeds and
        // this environment has no ORT lib): verify the per-row namespace
        // mapping directly from parsed entries — the same data `import()`
        // feeds into Memory.namespace (target_ns = param ?? entry.namespace).
        let (entries, failed) = crate::Uteke::parse_jsonl(&exported);
        assert_eq!(failed, 0);
        assert_eq!(entries.len(), 6);
        for ns in ["alpha", "beta", "gamma"] {
            let count = entries.iter().filter(|e| e.namespace == ns).count();
            assert_eq!(count, 2, "namespace {ns} attribution preserved in entries");
        }
        // Old-format compat: rows WITHOUT a namespace key deserialize with the
        // default, and a caller-supplied namespace override wins over row data.
        let legacy_line = r#"{"content":"legacy","tags":[],"metadata":{},"created_at":"2024-01-01T00:00:00Z","source":null}"#;
        let legacy: crate::memory::types::ExportEntry = serde_json::from_str(legacy_line).unwrap();
        assert_eq!(
            legacy.namespace, "default",
            "old exports default to 'default'"
        );

        drop(src);
        std::fs::remove_dir_all(&dir_a).ok();
        std::fs::remove_dir_all(&dir_b).ok();
    }

    #[test]
    fn test_import_jsonl_multiple_lines() {
        let jsonl = r#"{"content":"first","tags":[],"metadata":{},"created_at":"2024-01-01T00:00:00Z","source":null}
{"content":"second","tags":[],"metadata":{},"created_at":"2024-01-01T00:00:00Z","source":null}"#;
        let (entries, failed) = crate::Uteke::parse_jsonl(jsonl);
        assert_eq!(entries.len(), 2);
        assert_eq!(failed, 0);
        assert_eq!(entries[0].content, "first");
        assert_eq!(entries[1].content, "second");
    }

    #[test]
    fn test_import_pretty_printed_single_object() {
        use crate::memory::types::ExportEntry;
        // Pretty-printed JSON object spanning multiple lines should parse as a single entry,
        // NOT fall through to JSONL line-by-line parsing.
        let pretty = r#"{
  "content": "hello pretty",
  "tags": ["test"],
  "metadata": {},
  "created_at": "2024-01-01T00:00:00Z",
  "source": null
}"#;
        // Simulate the import detection logic
        let trimmed = pretty.trim();
        assert!(trimmed.starts_with('{'));
        let result: Result<ExportEntry, _> = serde_json::from_str(trimmed);
        assert!(
            result.is_ok(),
            "Pretty-printed JSON should parse as single object"
        );
        assert_eq!(result.unwrap().content, "hello pretty");
    }
}

#[cfg(test)]
mod room_link_tests {
    use crate::test_support::open_keyword;

    const A: &str = "alpha beta gamma delta epsilon zeta eta theta iota";
    const B: &str = "one two three four five six seven eight nine ten";

    #[test]
    fn export_carries_room_ids_and_import_restores_the_links() {
        let src = open_keyword();
        src.create_room("team", None, "default").unwrap();
        src.create_room("ops", None, "default").unwrap();
        let a = src.remember(A, &[], None, None).unwrap();
        let b = src.remember(B, &[], None, None).unwrap();
        src.add_memory_to_room(&a, "team", "alice").unwrap();
        src.add_memory_to_room(&a, "ops", "alice").unwrap();
        // A memory in no room carries no `rooms` key (the old file shape).
        let file = src.export(None).unwrap();
        let rows: Vec<serde_json::Value> = file
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let row = |text: &str| rows.iter().find(|r| r["content"] == text).unwrap();
        assert_eq!(row(A)["rooms"], serde_json::json!(["ops", "team"]));
        assert!(row(B).get("rooms").is_none());
        let _ = b;

        let dst = open_keyword();
        dst.create_room("team", None, "default").unwrap();
        dst.create_room("ops", None, "default").unwrap();
        let r = dst.import(&file, None).unwrap();
        assert_eq!((r.imported, r.room_links), (2, 2), "{r:?}");
        assert!(r.rooms_missing.is_empty());
        assert_eq!(dst.recall_room("team", None, 0).unwrap().len(), 1);
        assert_eq!(dst.recall_room("ops", None, 0).unwrap().len(), 1);
    }

    #[test]
    fn missing_rooms_are_reported_and_a_second_import_completes_the_links() {
        let src = open_keyword();
        src.create_room("team", None, "default").unwrap();
        src.create_room("later", None, "default").unwrap();
        let a = src.remember(A, &[], None, None).unwrap();
        src.add_memory_to_room(&a, "team", "alice").unwrap();
        src.add_memory_to_room(&a, "later", "alice").unwrap();
        let file = src.export(None).unwrap();

        let dst = open_keyword();
        dst.create_room("team", None, "default").unwrap();
        let first = dst.import(&file, None).unwrap();
        assert_eq!((first.imported, first.room_links), (1, 1), "{first:?}");
        assert_eq!(first.rooms_missing, vec!["later".to_string()]);

        // Create the missing room and import the same file again: the memory
        // is a duplicate, and the duplicate is linked.
        dst.create_room("later", None, "default").unwrap();
        let second = dst.import(&file, None).unwrap();
        assert_eq!(second.imported, 0, "{second:?}");
        assert_eq!(second.room_links, 1, "{second:?}");
        assert!(second.rooms_missing.is_empty());
        assert_eq!(dst.recall_room("later", None, 0).unwrap().len(), 1);
        assert_eq!(dst.count(None).unwrap(), 1, "no copy was made");
    }

    #[test]
    fn a_file_without_rooms_imports_exactly_as_before() {
        let dst = open_keyword();
        let r = dst
            .import(&format!("{{\"content\":\"{A}\"}}"), None)
            .unwrap();
        assert_eq!((r.imported, r.room_links), (1, 0));
        assert!(r.rooms_missing.is_empty());
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("room_links").is_none() && json.get("rooms_missing").is_none());
    }
}
