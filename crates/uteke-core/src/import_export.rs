//! Import and export memories in JSONL format.

use crate::error::Error;
use crate::index_sync::{self, SyncPolicy};
use crate::memory::types::{ExportEntry, ImportOptions, ImportResult, Memory};

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
    ///
    /// Rooms are cross-namespace collaboration spaces (#392): a room is not
    /// owned by a namespace, and `remember_in_room`, `POST /room/remember` and
    /// `POST /room/memory` accept any existing room for any memory. Import is a
    /// write like those (a read-only token cannot call it), so it follows the
    /// same rule rather than checking the room's namespace against the file's.
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

/// A document row of an import file (#1632): the Uteke Cloud export shape
/// `{"type":"document", slug, title, content, tags, parent, ...}`.
#[derive(serde::Deserialize)]
struct DocumentRow {
    #[serde(default)]
    slug: String,
    #[serde(default)]
    title: Option<String>,
    content: String,
    #[serde(default)]
    tags: Vec<String>,
    /// The parent's slug (or id); `null` for a root document.
    #[serde(default)]
    parent: Option<String>,
    #[serde(default)]
    author: Option<String>,
}

impl DocumentRow {
    fn parent_ref(&self) -> Option<&str> {
        self.parent
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
    }
}

/// A room row of an import file (#1632): the Uteke Cloud export shape
/// `{"type":"room", id, title, description, doc_slugs}`.
#[derive(serde::Deserialize)]
struct RoomRow {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    doc_slugs: Vec<String>,
    #[serde(default)]
    namespace: Option<String>,
}

/// One row of an import file.
enum Row {
    Memory(Box<ExportEntry>),
    Document(DocumentRow),
    Room(RoomRow),
    /// The first line of a Uteke Cloud export (or of a bundle): skipped
    /// without a warning.
    Manifest,
}

/// The rows of an import file by kind, and how many could not be read.
#[derive(Default)]
struct ParsedInput {
    memories: Vec<ExportEntry>,
    documents: Vec<DocumentRow>,
    rooms: Vec<RoomRow>,
    failed: usize,
}

impl ParsedInput {
    fn push(&mut self, row: Row) {
        match row {
            Row::Memory(e) => self.memories.push(*e),
            Row::Document(d) => self.documents.push(d),
            Row::Room(r) => self.rooms.push(r),
            Row::Manifest => {}
        }
    }
}

/// What a row is (#1632): `type: "document"` and `type: "room"` rows of a
/// Uteke Cloud export, a memory (any row with `content`), or a manifest line.
fn classify(v: serde_json::Value) -> Result<Row, String> {
    let read_err = |e: serde_json::Error| e.to_string();
    match v.get("type").and_then(|t| t.as_str()) {
        Some("document") => serde_json::from_value(v)
            .map(Row::Document)
            .map_err(read_err),
        Some("room") => serde_json::from_value(v).map(Row::Room).map_err(read_err),
        _ if v.get("content").is_some() => serde_json::from_value(v)
            .map(|e| Row::Memory(Box::new(e)))
            .map_err(read_err),
        _ if ["format_version", "exported_at", "uteke_export"]
            .iter()
            .any(|k| v.get(k).is_some()) =>
        {
            Ok(Row::Manifest)
        }
        _ => Err("missing field `content`".to_string()),
    }
}

/// The documents and rooms one import stored (#1632).
#[derive(Default)]
struct StructureCounts {
    documents_imported: usize,
    documents_updated: usize,
    documents_unchanged: usize,
    parent_unresolved: usize,
    rooms_imported: usize,
    rooms_existing: usize,
    room_documents_linked: usize,
    room_documents_missing: usize,
}

/// The id of an imported row (#1633): a UUID, stored in the canonical
/// lowercase hyphenated form. Anything else is dropped and the row gets a
/// new id, as rows from older export files do.
fn import_id(raw: &str) -> Option<String> {
    uuid::Uuid::parse_str(raw.trim())
        .ok()
        .map(|u| u.hyphenated().to_string())
}

/// A content type from a file is kept when it looks like one (`text`,
/// `json`, `markdown`, a MIME type); otherwise the row is `text` (#1633).
fn import_content_type(t: &str) -> bool {
    (1..=64).contains(&t.len())
        && t.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'/' | b'+' | b'.' | b'-')
        })
}

/// A memory slug from a file (#1633): trimmed, 1 to 256 bytes, no control
/// characters; otherwise the row has none.
fn import_slug(raw: &str) -> Option<String> {
    let slug = raw.trim();
    ((1..=256).contains(&slug.len()) && !slug.chars().any(char::is_control))
        .then(|| slug.to_string())
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
                // Identity and fields survive a round-trip (#1633).
                id: Some(m.id),
                memory_type: Some(m.memory_type),
                importance: Some(m.importance),
                pinned: Some(m.pinned),
                content_type: Some(m.content_type),
                slug: m.slug,
                deprecated: false,
                deprecated_at: None,
                // Valid time round-trips (#1598); only live rows are exported,
                // so an end here is the caller's, not deprecation bookkeeping.
                valid_from: m.valid_from,
                valid_until: m.valid_until,
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
        self.import_with(input, namespace, ImportOptions::default())
    }

    /// [`Self::import`] with options. With `defer_embedding` (#1608) every row
    /// is stored without a vector, so the import does not wait on the
    /// embedder and a backend that is down does not stop it; the rows are
    /// keyword-searchable at once and `uteke repair --reembed` adds the
    /// vectors later. Duplicates are then caught by exact content (the
    /// provenance hash), not by the cosine check.
    pub fn import_with(
        &self,
        input: &str,
        namespace: Option<&str>,
        options: ImportOptions,
    ) -> Result<ImportResult, Error> {
        let ParsedInput {
            memories: entries,
            documents,
            rooms,
            failed: mut skipped,
        } = Self::parse_input(input)?;

        // Documents, then rooms (their `doc_slugs` name documents), then the
        // memories, whose `rooms` must find the rooms of the same file (#1632).
        let mut structure = StructureCounts::default();
        let defer_chunks = options.defer_embedding || self.embedder_backend.is_empty();
        self.import_documents(documents, defer_chunks, &mut structure, &mut skipped);
        self.import_rooms(rooms, namespace, &mut structure, &mut skipped);

        let mut imported = 0;
        let mut deduped = 0;
        let mut id_conflicts = 0;
        let mut soft_deleted = 0;
        let mut redacted = 0;
        let mut rejected_secret = 0;
        let mut rejected_tombstone = 0;
        let mut deferred_embedding = 0;
        let mut room_links = RoomLinks::default();
        // Exact-content duplicates for a deferred import: hash -> stored id,
        // extended with every row this import stores.
        let mut known_hashes = if options.defer_embedding {
            self.store.active_content_hashes(namespace)?
        } else {
            std::collections::HashMap::new()
        };
        // The exact content of every stored row, live or soft-deleted, per
        // namespace (#1633), extended with every row this import stores.
        let mut stored_content = self.store.content_hashes_with_state(namespace)?;

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

            // A value forgotten as wrong is skipped and counted (#1597),
            // before any embedding work. `commit_memory` checks again.
            if let Err(e) = self.check_not_rejected(&entry.content) {
                tracing::warn!("Skipping import entry: {e}");
                rejected_tombstone += 1;
                skipped += 1;
                continue;
            }

            // Namespace resolution (#1036): a caller-SUPPLIED namespace is an
            // explicit override — every row lands there (e.g. CLI --namespace
            // bulk import). Without an override, the per-row namespace from
            // the export file wins, so round-trips reconstruct namespaces.
            // Rows from OLD export files (no namespace key) deserialize with
            // the "default" fallback via serde, keeping them importable.
            let target_ns = namespace
                .map(str::to_string)
                .unwrap_or_else(|| entry.namespace.clone());
            let mut row_id = entry.id.as_deref().and_then(import_id);
            let row_deprecated = entry.deprecated;

            // Identity first (#1633): a stored id is never overwritten. The
            // same content is a duplicate, other content an id conflict, and
            // a live row whose stored copy was forgotten stays forgotten.
            // Ids are unique across namespaces, so an id stored in another
            // namespace is not this row's identity in the target (a copy into
            // another namespace, `--namespace`): the row gets a new id and
            // the content checks of the target namespace below.
            if let Some(id) = row_id.clone()
                && let Some((stored_ns, stored, stored_deprecated)) =
                    self.store.content_and_state(&id)?
            {
                if stored_ns != target_ns {
                    row_id = None;
                } else {
                    if stored != entry.content {
                        id_conflicts += 1;
                    } else if stored_deprecated && !row_deprecated {
                        soft_deleted += 1;
                    } else {
                        deduped += 1;
                        if !stored_deprecated {
                            room_links.apply(self, &id, &rooms, &link_author);
                        }
                    }
                    continue;
                }
            }
            // The exact content in the target namespace, live or forgotten.
            // Only a live copy (or a deprecated row over a forgotten one)
            // is skipped; a live row whose copy was forgotten is inserted.
            let content_key = (
                target_ns.clone(),
                crate::provenance::content_hash(&entry.content),
            );
            match stored_content.get(&content_key) {
                Some((existing_id, false)) => {
                    deduped += 1;
                    if !row_deprecated {
                        room_links.apply(self, existing_id, &rooms, &link_author);
                    }
                    continue;
                }
                Some((_, true)) if row_deprecated => {
                    deduped += 1;
                    continue;
                }
                // A live row over a deprecated copy is skipped only on request
                // (`skip_deprecated_copies`, #1716): a file may carry text that
                // was replaced on purpose. Its rooms are not linked: the stored
                // copy is not live.
                Some((_, true)) if options.skip_deprecated_copies => {
                    deduped += 1;
                    continue;
                }
                // Otherwise it is a new memory: a plain forget is recoverable,
                // only `forget --reason` rejects (#1689).
                Some((_, true)) | None => {}
            }

            // Re-embed the content with retry (consistent with remember path, #1005).
            // A store with no embedding backend (#1166) imports FTS5-only, as
            // `remember` stores there: no vector, so no duplicate check either.
            // A deprecated row gets no vector: nothing recalls it.
            let embedding =
                if self.embedder_backend.is_empty() || options.defer_embedding || row_deprecated {
                    Vec::new()
                } else {
                    self.embed(&entry.content)?
                };
            let content_hash = (options.defer_embedding && !row_deprecated)
                .then(|| crate::provenance::content_hash(&entry.content));
            if let Some(existing_id) = content_hash.as_ref().and_then(|h| known_hashes.get(h)) {
                deduped += 1;
                room_links.apply(self, existing_id, &rooms, &link_author);
                continue;
            }

            // Dedup check: skip if cosine >= 0.95 to existing memory (#1005).
            // A row with an id is matched on its id and exact content only.
            if row_id.is_none()
                && !embedding.is_empty()
                && let Some(existing_id) = self.check_duplicate(&embedding, namespace)?
            {
                deduped += 1;
                // The stored memory is the one that belongs in the rooms (#1588).
                room_links.apply(self, &existing_id, &rooms, &link_author);
                continue;
            }

            let id = row_id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string());
            let now = chrono::Utc::now();

            // Valid time (#1598): the start defaults to the creation time, an
            // end that does not follow the start is dropped, not the row.
            let valid_from = entry.valid_from.unwrap_or(entry.created_at);
            let valid_until = entry.valid_until.filter(|u| *u > valid_from);

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
                deprecated: row_deprecated,
                deprecated_at: row_deprecated.then(|| entry.deprecated_at.unwrap_or(now)),
                valid_from: Some(valid_from),
                valid_until,
                // A missing or bad value falls back to the default (#1633).
                memory_type: entry
                    .memory_type
                    .as_deref()
                    .and_then(crate::memory::types::MemoryType::from_str_opt)
                    .map_or("fact", |t| t.as_str())
                    .to_string(),
                importance: entry
                    .importance
                    .filter(|i| (0.0..=1.0).contains(i))
                    .unwrap_or(0.5),
                pinned: entry.pinned.unwrap_or(false),
                content_type: entry
                    .content_type
                    .filter(|t| import_content_type(t))
                    .unwrap_or_else(|| "text".to_string()),
                slug: entry.slug.as_deref().and_then(import_slug),
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
            if row_deprecated {
                stored_content
                    .entry(content_key)
                    .or_insert((id.clone(), true));
            } else {
                room_links.apply(self, &id, &rooms, &link_author);
                stored_content.insert(content_key, (id.clone(), false));
            }

            if let Some(hash) = content_hash {
                known_hashes.insert(hash, id.clone());
                deferred_embedding += 1;
            }
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

        if skipped > 0 || deduped > 0 || id_conflicts > 0 || soft_deleted > 0 {
            tracing::info!(
                "Import completed: {imported} imported, {deduped} duplicates skipped, {id_conflicts} id conflicts skipped, {soft_deleted} soft-deleted skipped, {skipped} errored entries."
            );
        }

        Ok(ImportResult {
            imported,
            skipped,
            duplicates_skipped: deduped,
            id_conflicts_skipped: id_conflicts,
            soft_deleted_skipped: soft_deleted,
            redacted,
            rejected_secret,
            rejected_tombstone,
            room_links: room_links.linked,
            rooms_missing: room_links.missing.into_iter().collect(),
            deferred_embedding,
            documents_imported: structure.documents_imported,
            documents_updated: structure.documents_updated,
            documents_unchanged: structure.documents_unchanged,
            parent_unresolved: structure.parent_unresolved,
            rooms_imported: structure.rooms_imported,
            rooms_existing: structure.rooms_existing,
            room_documents_linked: structure.room_documents_linked,
            room_documents_missing: structure.room_documents_missing,
        })
    }

    /// Parse the input: a JSON array, one JSON object, or JSONL (one object
    /// per line). A single object that does not parse falls back to JSONL,
    /// which covers JSONL whose first line starts with `{` and pretty-printed
    /// objects. An invalid element of an array fails the import; an invalid
    /// JSONL line is counted in `failed`.
    fn parse_input(input: &str) -> Result<ParsedInput, Error> {
        let trimmed = input.trim();
        if trimmed.starts_with('[') {
            let invalid = |e: String| Error::validation(format!("Invalid JSON array: {e}"));
            let rows: Vec<serde_json::Value> =
                serde_json::from_str(trimmed).map_err(|e| invalid(e.to_string()))?;
            let mut parsed = ParsedInput::default();
            for row in rows {
                parsed.push(classify(row).map_err(invalid)?);
            }
            return Ok(parsed);
        }
        if trimmed.starts_with('{')
            && let Some(Ok(row)) = serde_json::from_str::<serde_json::Value>(trimmed)
                .ok()
                .map(classify)
        {
            let mut parsed = ParsedInput::default();
            parsed.push(row);
            return Ok(parsed);
        }
        Ok(Self::parse_jsonl(input))
    }

    /// Parse JSONL input: one self-contained JSON object per line.
    /// Lines that fail to parse are counted as failed (not fatal).
    fn parse_jsonl(input: &str) -> ParsedInput {
        let mut parsed = ParsedInput::default();
        for line in input.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<serde_json::Value>(line)
                .map_err(|e| e.to_string())
                .and_then(classify)
            {
                Ok(row) => parsed.push(row),
                Err(e) => {
                    tracing::warn!("Skipping invalid JSONL line: {e}");
                    parsed.failed += 1;
                }
            }
        }
        parsed
    }

    /// Store the document rows of an import (#1632), parents before their
    /// children whatever the order of the file. A document is matched on its
    /// slug: an unchanged one is left alone (no new version), a changed one is
    /// updated in place. A parent named by slug (or id) that is neither in
    /// the file nor in the store is counted in `parent_unresolved` and the
    /// document lands at the root (an existing one keeps its parent).
    fn import_documents(
        &self,
        rows: Vec<DocumentRow>,
        defer_embedding: bool,
        counts: &mut StructureCounts,
        skipped: &mut usize,
    ) {
        let mut pending: Vec<(String, DocumentRow)> = Vec::new();
        for row in rows {
            match import_slug(&row.slug) {
                Some(slug) => pending.push((slug, row)),
                None => {
                    tracing::warn!("Skipping import document: invalid slug {:?}", row.slug);
                    *skipped += 1;
                }
            }
        }
        let in_file: std::collections::HashSet<String> =
            pending.iter().map(|(slug, _)| slug.clone()).collect();
        let mut done: std::collections::HashSet<String> = std::collections::HashSet::new();

        while !pending.is_empty() {
            // Rows whose parent is stored already or is not in the file; when
            // none is left (a cycle) the rest goes in as it is.
            let ready = |(slug, row): &(String, DocumentRow)| match row.parent_ref() {
                Some(p) if p != slug.as_str() && in_file.contains(p) => done.contains(p),
                _ => true,
            };
            let (now, later): (Vec<_>, Vec<_>) = if pending.iter().any(ready) {
                pending.into_iter().partition(|r| ready(r))
            } else {
                (std::mem::take(&mut pending), Vec::new())
            };
            pending = later;
            for (slug, row) in now {
                self.import_document(&slug, &row, defer_embedding, counts, skipped);
                done.insert(slug);
            }
        }
    }

    /// One document row of [`Self::import_documents`].
    fn import_document(
        &self,
        slug: &str,
        row: &DocumentRow,
        defer_embedding: bool,
        counts: &mut StructureCounts,
        skipped: &mut usize,
    ) {
        let existing = match self.store.get_document_by_slug(slug) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!("Skipping import document {slug}: {e}");
                *skipped += 1;
                return;
            }
        };
        // The parent as a stored document: by slug, or by id (a Cloud export
        // also accepts the parent's UUID). Not itself, and only once stored.
        let parent = row
            .parent_ref()
            .filter(|p| *p != slug)
            .and_then(|p| match self.doc_get(p) {
                Ok(Some(d)) if d.slug != slug => Some(d),
                _ => None,
            });
        let parent = match (row.parent_ref(), parent) {
            (None, _) => None,
            (Some(_), Some(p)) => Some(p),
            (Some(p), None) => {
                tracing::warn!(
                    "Import document {slug}: parent {p:?} not found, stored at the root"
                );
                counts.parent_unresolved += 1;
                // An existing document keeps the parent it has.
                existing
                    .as_ref()
                    .and_then(|d| d.parent_id.as_deref())
                    .and_then(|id| self.doc_get(id).ok().flatten())
            }
        };
        let title = row
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .unwrap_or(slug);
        let parent_id = parent.as_ref().map(|p| p.id.clone());
        if let Some(d) = &existing
            && d.title == title
            && d.content == row.content
            && d.tags == row.tags
            && d.parent_id == parent_id
        {
            counts.documents_unchanged += 1;
            return;
        }
        let tags: Vec<&str> = row.tags.iter().map(String::as_str).collect();
        let author = row
            .author
            .as_deref()
            .and_then(|a| crate::memory::types::validate_author(a).ok());
        match self.doc_upsert_inner(
            slug,
            title,
            &row.content,
            &tags,
            author,
            parent.as_ref().map(|p| p.slug.as_str()),
            defer_embedding,
        ) {
            Ok(_) if existing.is_some() => counts.documents_updated += 1,
            Ok(_) => counts.documents_imported += 1,
            Err(e) => {
                tracing::warn!("Skipping import document {slug}: {e}");
                *skipped += 1;
            }
        }
    }

    /// Create the rooms of an import (#1632) and link their documents. A room
    /// that exists is left as it is (its `doc_slugs` are still linked); a
    /// document slug that is not stored is counted in `room_documents_missing`.
    fn import_rooms(
        &self,
        rows: Vec<RoomRow>,
        namespace: Option<&str>,
        counts: &mut StructureCounts,
        skipped: &mut usize,
    ) {
        for row in rows {
            let Some(id) = import_slug(&row.id) else {
                tracing::warn!("Skipping import room: invalid id {:?}", row.id);
                *skipped += 1;
                continue;
            };
            let text = |v: &Option<String>| {
                v.as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .map(str::to_string)
            };
            let (title, description) = (text(&row.title), text(&row.description));
            match self.store.get_room(&id) {
                Ok(Some(_)) => counts.rooms_existing += 1,
                Ok(None) => {
                    let ns = namespace
                        .or(row.namespace.as_deref())
                        .unwrap_or(crate::memory::types::DEFAULT_NAMESPACE);
                    let created = self.create_room(&id, title.as_deref(), ns).and_then(|()| {
                        if description.is_some() {
                            self.update_room(&id, title.as_deref(), description.as_deref())
                                .map(|_| ())
                        } else {
                            Ok(())
                        }
                    });
                    if let Err(e) = created {
                        tracing::warn!("Skipping import room {id}: {e}");
                        *skipped += 1;
                        continue;
                    }
                    counts.rooms_imported += 1;
                }
                Err(e) => {
                    tracing::warn!("Skipping import room {id}: {e}");
                    *skipped += 1;
                    continue;
                }
            }
            if row.doc_slugs.is_empty() {
                continue;
            }
            let linked: std::collections::HashSet<String> = self
                .room_list_documents(&id)
                .unwrap_or_default()
                .into_iter()
                .collect();
            for slug in &row.doc_slugs {
                if linked.contains(slug) {
                    continue;
                }
                if !matches!(self.store.get_document_by_slug(slug), Ok(Some(_))) {
                    counts.room_documents_missing += 1;
                    continue;
                }
                match self.room_add_document(&id, slug) {
                    Ok(()) => counts.room_documents_linked += 1,
                    Err(e) => tracing::warn!("Could not link document {slug} to room {id}: {e}"),
                }
            }
        }
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
            id: None,
            memory_type: None,
            importance: None,
            pinned: None,
            content_type: None,
            slug: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
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

    /// #1598: the caller's validity interval survives export and import; an
    /// old file without it falls back to the creation time, and an end that
    /// does not follow the start is dropped without failing the row.
    #[test]
    fn test_validity_survives_export_import() {
        let src = crate::test_support::open_keyword();
        let a = src
            .remember("a fact with an interval", &[], None, None)
            .unwrap();
        src.remember("a plain fact", &[], None, None).unwrap();
        let v = crate::memory::types::Validity::parse(
            Some("2025-01-01T00:00:00Z"),
            Some("2026-01-01T00:00:00Z"),
        )
        .unwrap()
        .unwrap();
        assert!(src.set_validity(&a, &v).unwrap());
        let exported = src.export(None).unwrap();

        let dst = crate::test_support::open_keyword();
        dst.import(&exported, None).unwrap();
        let got = dst.get_by_id(&a).unwrap().unwrap();
        assert_eq!(got.valid_from, Some(v.from.unwrap()));
        assert_eq!(got.valid_until, Some(v.until.unwrap()));
        let plain = src.export(None).unwrap();
        let line = plain.lines().find(|l| l.contains("a plain fact")).unwrap();
        let row: serde_json::Value = serde_json::from_str(line).unwrap();
        assert!(row.get("valid_until").is_none(), "{row}");

        // Old file: no validity keys, the start is the creation time.
        let old = dst_old_file();
        let u = crate::test_support::open_keyword();
        u.import(&old, None).unwrap();
        let m = u.list(None, 10, 0, None).unwrap().remove(0);
        assert_eq!(m.valid_from, Some(m.created_at));
        assert_eq!(m.valid_until, None);

        // An end that is not after the start is dropped, the row stays.
        let bad = r#"{"content":"inverted","created_at":"2025-01-01T00:00:00Z","valid_from":"2025-06-01T00:00:00Z","valid_until":"2025-02-01T00:00:00Z"}"#;
        let u = crate::test_support::open_keyword();
        assert_eq!(u.import(bad, None).unwrap().imported, 1);
        let m = u.list(None, 10, 0, None).unwrap().remove(0);
        assert_eq!(m.valid_until, None);
    }

    fn dst_old_file() -> String {
        r#"{"content":"from an old export","created_at":"2024-03-01T00:00:00Z"}"#.to_string()
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
        let parsed = crate::Uteke::parse_jsonl(&exported);
        let (entries, failed) = (parsed.memories, parsed.failed);
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
        let parsed = crate::Uteke::parse_jsonl(jsonl);
        let (entries, failed) = (parsed.memories, parsed.failed);
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

    /// #1166: a store opened without an embedding backend imports FTS5-only
    /// instead of failing on the first row.
    #[test]
    fn a_store_without_an_embedder_imports_keyword_only() {
        let u = crate::Uteke::open_with_backend(":memory:", None).unwrap();
        let file = "{\"content\":\"alpha beta gamma\"}\n{\"content\":\"delta epsilon zeta\"}";
        let r = u.import(file, None).unwrap();
        assert_eq!((r.imported, r.skipped), (2, 0), "{r:?}");
        assert_eq!(u.count(None).unwrap(), 2);
        let found = u.search("alpha", 5, None, None).unwrap();
        assert_eq!(found.len(), 1, "the rows are keyword-searchable");
    }
}

/// An import that defers embedding (#1608) stores first and embeds later.
#[cfg(test)]
mod defer_embedding_tests {
    use crate::test_support::{RecordingEmbedder, open_with_embedder};
    use crate::{ImportOptions, ReembedOptions};

    const DEFER: ImportOptions = ImportOptions {
        defer_embedding: true,
        skip_deprecated_copies: false,
    };

    fn vectors(u: &crate::Uteke) -> i64 {
        u.graph_store()
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE embedding IS NOT NULL AND length(embedding) > 0",
                [],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn rows_are_stored_without_calling_the_embedder() {
        let (emb, seen) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let input = "{\"content\":\"the quokka is a small marsupial\"}\n{\"content\":\"rust has no garbage collector\"}\n";
        let r = u.import_with(input, None, DEFER).unwrap();
        assert_eq!((r.imported, r.deferred_embedding, r.skipped), (2, 2, 0));
        assert!(seen.lock().unwrap().is_empty(), "the import embedded");
        assert_eq!(vectors(&u), 0);
        // keyword search finds them at once
        let hits = u.search("quokka", 5, None, None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn exact_copies_are_skipped_in_the_store_and_in_the_file() {
        let (emb, _) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        u.remember("already stored", &[], None, None).unwrap();
        let input = "{\"content\":\"already stored\"}\n{\"content\":\"new fact\"}\n{\"content\":\"new fact\"}\n";
        let r = u.import_with(input, None, DEFER).unwrap();
        assert_eq!((r.imported, r.deferred_embedding), (1, 1));
        assert_eq!(u.count(None).unwrap(), 2);
    }

    #[test]
    fn a_default_import_still_embeds_and_reports_nothing_deferred() {
        let (emb, seen) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let r = u.import("{\"content\":\"embedded now\"}\n", None).unwrap();
        assert_eq!((r.imported, r.deferred_embedding), (1, 0));
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(vectors(&u), 1);
        let json = serde_json::to_value(&r).unwrap();
        assert!(json.get("deferred_embedding").is_none(), "{json}");
    }

    #[test]
    fn reembed_with_a_limit_continues_where_it_stopped() {
        let (emb, seen) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let input: String = (0..5)
            .map(|i| format!("{{\"content\":\"deferred fact number {i}\"}}\n"))
            .collect();
        u.import_with(&input, None, DEFER).unwrap();

        let limited = ReembedOptions {
            limit: Some(2),
            pause: std::time::Duration::ZERO,
        };
        let first = u.reembed_missing_with(limited).unwrap();
        assert_eq!(
            (first.missing_count, first.reembedded, first.remaining),
            (5, 2, 3)
        );
        assert_eq!(vectors(&u), 2);

        let second = u.reembed_missing_with(limited).unwrap();
        assert_eq!(
            (second.missing_count, second.reembedded, second.remaining),
            (3, 2, 1)
        );
        let last = u.reembed_missing_with(ReembedOptions::default()).unwrap();
        assert_eq!((last.reembedded, last.remaining), (1, 0));
        assert_eq!(vectors(&u), 5);
        assert_eq!(seen.lock().unwrap().len(), 5, "each row embedded once");
        // now found by meaning too
        assert!(
            !u.recall("deferred fact", 5, None, None, 0.0, None, None)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn a_limit_of_zero_embeds_nothing() {
        let (emb, seen) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        u.import_with("{\"content\":\"waits\"}\n", None, DEFER)
            .unwrap();
        let r = u
            .reembed_missing_with(ReembedOptions {
                limit: Some(0),
                pause: std::time::Duration::ZERO,
            })
            .unwrap();
        assert_eq!((r.reembedded, r.remaining), (0, 1));
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn a_duplicate_row_links_its_rooms_to_the_stored_memory() {
        let (emb, _) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let id = u.remember("shared fact", &[], None, None).unwrap();
        u.create_room("r1", None, "default").unwrap();
        let r = u
            .import_with(
                "{\"content\":\"shared fact\",\"rooms\":[\"r1\"]}\n",
                None,
                DEFER,
            )
            .unwrap();
        assert_eq!((r.imported, r.room_links), (0, 1));
        let linked: i64 = u
            .graph_store()
            .query_row(
                "SELECT COUNT(*) FROM room_memories WHERE room_id = 'r1' AND memory_id = ?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(linked, 1);
    }

    #[test]
    fn duplicates_follow_the_current_content_not_the_insert_time_hash() {
        let (emb, _) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let id = u.remember("old text", &[], None, None).unwrap();
        // an update rewrites the content but keeps source_hash (tamper audit)
        u.graph_store()
            .execute(
                "UPDATE memories SET content = 'new text' WHERE id = ?1",
                [&id],
            )
            .unwrap();
        let old = u
            .import_with("{\"content\":\"old text\"}\n", None, DEFER)
            .unwrap();
        assert_eq!(old.imported, 1, "the old text is no longer stored");
        let new = u
            .import_with("{\"content\":\"new text\"}\n", None, DEFER)
            .unwrap();
        assert_eq!(new.imported, 0, "the current text is a duplicate");
    }

    #[test]
    fn rows_that_fail_do_not_use_up_the_limit() {
        let (emb, _) = RecordingEmbedder::failing_first(2);
        let u = open_with_embedder(Box::new(emb));
        let input: String = (0..3)
            .map(|i| format!("{{\"content\":\"row {i}\"}}\n"))
            .collect();
        u.import_with(&input, None, DEFER).unwrap();
        let one = ReembedOptions {
            limit: Some(1),
            pause: std::time::Duration::ZERO,
        };
        let r = u.reembed_missing_with(one).unwrap();
        assert_eq!((r.failed, r.reembedded, r.remaining), (2, 1, 0));
        assert_eq!(vectors(&u), 1);
    }

    #[test]
    fn chunks_left_by_the_limit_are_counted_as_remaining() {
        let (emb, _) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        u.doc_upsert("guide", "Guide", "first part of the guide", &[], None)
            .unwrap();
        let chunks: i64 = u
            .graph_store()
            .execute("UPDATE document_chunks SET embedding = NULL", [])
            .unwrap() as i64;
        assert!(chunks > 0);
        u.import_with("{\"content\":\"a deferred memory\"}\n", None, DEFER)
            .unwrap();
        let r = u
            .reembed_missing_with(ReembedOptions {
                limit: Some(1),
                pause: std::time::Duration::ZERO,
            })
            .unwrap();
        assert_eq!((r.reembedded, r.chunks_reembedded), (1, 0));
        assert_eq!(r.remaining as i64, chunks);
        let rest = u.reembed_missing_with(ReembedOptions::default()).unwrap();
        assert_eq!((rest.chunks_reembedded as i64, rest.remaining), (chunks, 0));
    }

    #[test]
    fn reembedded_rows_get_the_cosine_links_remember_makes() {
        let texts = [
            "the quokka lives on rottnest island in western australia",
            "the quokka lives on rottnest island near perth in western australia",
        ];
        // control: `remember` links these two
        let (emb, _) = RecordingEmbedder::new();
        let control = open_with_embedder(Box::new(emb));
        for t in texts {
            control.remember(t, &[], None, None).unwrap();
        }
        assert!(
            control.count_edges().unwrap() > 0,
            "fixture texts must link"
        );

        let (emb, _) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let input: String = texts
            .iter()
            .map(|t| format!("{{\"content\":\"{t}\"}}\n"))
            .collect();
        u.import_with(&input, None, DEFER).unwrap();
        assert_eq!(u.count_edges().unwrap(), 0);
        u.reembed_missing_with(ReembedOptions::default()).unwrap();
        assert!(u.count_edges().unwrap() > 0);
    }
}

/// Round-trip fidelity (#1633): a row's id, type, importance, pinned flag,
/// content type, slug and deprecated state survive an import, and a row that
/// is soft-deleted in the target is not brought back.
#[cfg(test)]
mod fidelity_tests {
    use crate::test_support::open_keyword;

    const ID_A: &str = "0b9a4f3e-6a52-4c1f-9d1e-3f1d2c4b5a61";
    const ID_B: &str = "1c8b5e4f-7b63-4d2a-8e2f-4a2e3d5c6b72";
    const ID_C: &str = "2d7c6f50-8c74-4e3b-9f30-5b3f4e6d7c83";

    fn fixture() -> String {
        [
            format!(
                r#"{{"id":"{ID_A}","content":"we chose sqlite for the local store","memory_type":"decision","importance":0.9,"pinned":true,"slug":"store-choice"}}"#
            ),
            format!(
                r#"{{"id":"{ID_B}","content":"run cargo fmt before every commit","memory_type":"procedure","importance":0.7,"content_type":"text"}}"#
            ),
            format!(
                r#"{{"id":"{ID_C}","content":"{{\"editor\":\"helix\"}}","memory_type":"preference","importance":0.2,"content_type":"json"}}"#
            ),
        ]
        .join("\n")
    }

    #[test]
    fn ids_types_importance_and_pinned_survive_and_a_reimport_adds_nothing() {
        let u = open_keyword();
        let r = u.import(&fixture(), None).unwrap();
        assert_eq!(
            (
                r.imported,
                r.skipped,
                r.duplicates_skipped,
                r.id_conflicts_skipped
            ),
            (3, 0, 0, 0),
            "{r:?}"
        );
        let a = u.get_by_id(ID_A).unwrap().expect("id A kept");
        assert_eq!(a.memory_type, "decision");
        assert!((a.importance - 0.9).abs() < 1e-9, "{}", a.importance);
        assert!(a.pinned);
        assert_eq!(a.slug.as_deref(), Some("store-choice"));
        let b = u.get_by_id(ID_B).unwrap().expect("id B kept");
        assert_eq!(
            (b.memory_type.as_str(), b.pinned, b.content_type.as_str()),
            ("procedure", false, "text")
        );
        let c = u.get_by_id(ID_C).unwrap().expect("id C kept");
        assert_eq!(
            (c.memory_type.as_str(), c.content_type.as_str()),
            ("preference", "json")
        );

        let again = u.import(&fixture(), None).unwrap();
        assert_eq!(
            (
                again.imported,
                again.duplicates_skipped,
                again.id_conflicts_skipped
            ),
            (0, 3, 0),
            "{again:?}"
        );
        assert_eq!(u.count(None).unwrap(), 3, "no new rows");
        let json = serde_json::to_value(&again).unwrap();
        assert_eq!(json["duplicates_skipped"], 3, "{json}");
        assert_eq!(json["id_conflicts_skipped"], 0, "{json}");
    }

    #[test]
    fn a_plain_export_without_ids_still_imports_with_generated_ids() {
        let u = open_keyword();
        let r = u
            .import(r#"{"content":"an old export row without id"}"#, None)
            .unwrap();
        assert_eq!(r.imported, 1, "{r:?}");
        let m = &u.list(None, 10, 0, None).unwrap()[0];
        assert!(uuid::Uuid::parse_str(&m.id).is_ok(), "{}", m.id);
        assert_eq!(
            (m.memory_type.as_str(), m.pinned, m.content_type.as_str()),
            ("fact", false, "text")
        );
        assert!((m.importance - 0.5).abs() < 1e-9);
        assert_eq!(m.slug, None);
    }

    #[test]
    fn an_existing_id_with_other_content_is_an_id_conflict_not_a_merge() {
        let u = open_keyword();
        u.import(&fixture(), None).unwrap();
        let file = format!(r#"{{"id":"{ID_A}","content":"a different text under the same id"}}"#);
        let r = u.import(&file, None).unwrap();
        assert_eq!(
            (r.imported, r.duplicates_skipped, r.id_conflicts_skipped),
            (0, 0, 1),
            "{r:?}"
        );
        let a = u.get_by_id(ID_A).unwrap().unwrap();
        assert_eq!(a.content, "we chose sqlite for the local store");
    }

    #[test]
    fn a_row_with_an_id_skips_the_exact_copy_stored_under_another_id() {
        let u = open_keyword();
        let old = u
            .remember("an exact copy already stored", &[], None, None)
            .unwrap();
        let file = format!(r#"{{"id":"{ID_A}","content":"an exact copy already stored"}}"#);
        let r = u.import(&file, None).unwrap();
        assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
        assert!(u.get_by_id(ID_A).unwrap().is_none());
        assert!(u.get_by_id(&old).unwrap().is_some());
    }

    #[test]
    fn a_row_soft_deleted_in_the_target_is_not_resurrected() {
        let u = open_keyword();
        // with the same id: matched on the id
        u.import(&fixture(), None).unwrap();
        u.forget(ID_B).unwrap();
        let r = u.import(&fixture(), None).unwrap();
        assert_eq!(
            (r.imported, r.duplicates_skipped, r.soft_deleted_skipped),
            (0, 2, 1),
            "{r:?}"
        );
        assert!(u.get_by_id(ID_B).unwrap().unwrap().deprecated);
    }

    /// #1689: a plain forget is recoverable. Content without an id that was
    /// forgotten imports as a new live row; the tombstone stays deprecated.
    #[test]
    fn an_anonymous_row_matching_a_forgotten_one_imports_as_a_new_live_row() {
        let u = open_keyword();
        let file = r#"{"content":"content forgotten then imported again"}"#;
        assert_eq!(u.import(file, None).unwrap().imported, 1);
        let first = u.list(None, 10, 0, None).unwrap()[0].id.clone();
        u.forget(&first).unwrap();
        assert_eq!(u.count(None).unwrap(), 0);
        let r = u.import(file, None).unwrap();
        assert_eq!((r.imported, r.soft_deleted_skipped), (1, 0), "{r:?}");
        let live = u.list(None, 10, 0, None).unwrap();
        assert_eq!(live.len(), 1);
        assert_ne!(live[0].id, first, "a new row, not the tombstone");
        assert!(u.get_by_id(&first).unwrap().unwrap().deprecated);
        assert!(!live[0].deprecated);
        // a third import is an exact duplicate of the live row again
        let r = u.import(file, None).unwrap();
        assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
    }

    // ── skip_deprecated_copies (#1716) ──────────────────────────────────

    use crate::ImportOptions;

    const SKIP: ImportOptions = ImportOptions {
        defer_embedding: false,
        skip_deprecated_copies: true,
    };

    /// Import one anonymous row, then deprecate the stored copy the way
    /// `how` names it: a plain forget, a supersession, or a bare deprecate
    /// (what consolidation and pruning end in).
    fn import_then_deprecate(u: &crate::Uteke, content: &str, how: &str) -> String {
        let file = format!(r#"{{"content":"{content}"}}"#);
        assert_eq!(u.import(&file, None).unwrap().imported, 1);
        let old = u.list(None, 10, 0, None).unwrap()[0].id.clone();
        match how {
            "forget" => {
                u.forget(&old).unwrap();
            }
            "supersede" => {
                let newer = u
                    .remember(&format!("{content} replaced"), &[], None, None)
                    .unwrap();
                u.supersede(&old, &newer, None).unwrap();
            }
            "deprecate" => u.store.deprecate(&old).unwrap(),
            other => panic!("unknown way {other}"),
        }
        assert!(u.get_by_id(&old).unwrap().unwrap().deprecated, "{how}");
        old
    }

    #[test]
    fn by_default_a_live_row_over_any_deprecated_copy_is_imported_as_new() {
        for how in ["forget", "supersede", "deprecate"] {
            let u = open_keyword();
            let content = format!("text kept by {how}");
            let old = import_then_deprecate(&u, &content, how);
            let live_before = u.count(None).unwrap();
            let r = u
                .import(&format!(r#"{{"content":"{content}"}}"#), None)
                .unwrap();
            assert_eq!(
                (r.imported, r.duplicates_skipped, r.soft_deleted_skipped),
                (1, 0, 0),
                "{how}: {r:?}"
            );
            assert_eq!(u.count(None).unwrap(), live_before + 1, "{how}");
            assert!(u.get_by_id(&old).unwrap().unwrap().deprecated, "{how}");
        }
    }

    #[test]
    fn skip_deprecated_copies_skips_the_row_and_counts_a_duplicate() {
        for how in ["forget", "supersede", "deprecate"] {
            let u = open_keyword();
            let content = format!("text replaced by {how}");
            import_then_deprecate(&u, &content, how);
            let live_before = u.count(None).unwrap();
            let r = u
                .import_with(&format!(r#"{{"content":"{content}"}}"#), None, SKIP)
                .unwrap();
            assert_eq!(
                (r.imported, r.duplicates_skipped, r.soft_deleted_skipped),
                (0, 1, 0),
                "{how}: {r:?}"
            );
            assert_eq!(u.count(None).unwrap(), live_before, "{how}");
        }
    }

    #[test]
    fn a_skipped_deprecated_copy_links_no_rooms_and_an_imported_one_does() {
        let u = open_keyword();
        u.create_room("team", None, "default").unwrap();
        import_then_deprecate(&u, "room text once forgotten", "forget");
        let file = r#"{"content":"room text once forgotten","rooms":["team"]}"#;
        let r = u.import_with(file, None, SKIP).unwrap();
        assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
        assert!(u.recall_room("team", None, 0).unwrap().is_empty());
        let r = u.import(file, None).unwrap();
        assert_eq!(r.imported, 1, "{r:?}");
        assert_eq!(u.recall_room("team", None, 0).unwrap().len(), 1);
    }

    #[test]
    fn the_flag_changes_nothing_for_ids_live_duplicates_and_deprecated_rows() {
        let u = open_keyword();
        // A row that carries the id of a forgotten memory stays skipped.
        let id_file = format!(r#"{{"id":"{ID_B}","content":"identity case"}}"#);
        assert_eq!(u.import(&id_file, None).unwrap().imported, 1);
        u.forget(ID_B).unwrap();
        for options in [ImportOptions::default(), SKIP] {
            let r = u.import_with(&id_file, None, options).unwrap();
            assert_eq!(
                (r.imported, r.duplicates_skipped, r.soft_deleted_skipped),
                (0, 0, 1),
                "{r:?}"
            );
        }
        // An exact copy of a live row is a duplicate either way.
        let live = r#"{"content":"a live row"}"#;
        assert_eq!(u.import(live, None).unwrap().imported, 1);
        for options in [ImportOptions::default(), SKIP] {
            let r = u.import_with(live, None, options).unwrap();
            assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
        }
        // A deprecated row over a deprecated copy is skipped either way.
        import_then_deprecate(&u, "both ends deprecated", "forget");
        let dep = r#"{"content":"both ends deprecated","deprecated":true}"#;
        for options in [ImportOptions::default(), SKIP] {
            let r = u.import_with(dep, None, options).unwrap();
            assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
        }
    }

    /// A deprecated row in the file stays skipped over a forgotten copy.
    #[test]
    fn a_deprecated_row_over_a_forgotten_copy_is_skipped() {
        let u = open_keyword();
        let gone = u
            .remember("forgotten at both ends", &[], None, None)
            .unwrap();
        u.forget(&gone).unwrap();
        let r = u
            .import(
                r#"{"content":"forgotten at both ends","deprecated":true}"#,
                None,
            )
            .unwrap();
        assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
    }

    #[test]
    fn a_deprecated_row_is_imported_deprecated() {
        let u = open_keyword();
        let file = format!(
            r#"{{"id":"{ID_A}","content":"a memory that was forgotten at the source","deprecated":true,"deprecated_at":"2026-10-01T00:00:00Z"}}"#
        );
        let r = u.import(&file, None).unwrap();
        assert_eq!(r.imported, 1, "{r:?}");
        let m = u.get_by_id(ID_A).unwrap().unwrap();
        assert!(m.deprecated);
        assert_eq!(
            m.deprecated_at.map(|t| t.to_rfc3339()),
            Some("2026-10-01T00:00:00+00:00".to_string())
        );
        assert_eq!(u.count(None).unwrap(), 0, "not live");
        let again = u.import(&file, None).unwrap();
        assert_eq!(
            (again.imported, again.duplicates_skipped),
            (0, 1),
            "{again:?}"
        );
    }

    #[test]
    fn invalid_field_values_fall_back_to_the_defaults() {
        let u = open_keyword();
        let file = r#"{"id":"not-a-uuid","content":"a row with bad fields","memory_type":"bogus","importance":7.5,"content_type":"no spaces allowed","slug":"   "}"#;
        let r = u.import(file, None).unwrap();
        assert_eq!(r.imported, 1, "{r:?}");
        let m = &u.list(None, 10, 0, None).unwrap()[0];
        assert_ne!(m.id, "not-a-uuid");
        assert_eq!(
            (m.memory_type.as_str(), m.content_type.as_str()),
            ("fact", "text")
        );
        assert!((m.importance - 0.5).abs() < 1e-9);
        assert_eq!(m.slug, None);
    }

    #[test]
    fn an_oss_export_round_trips_ids_and_fields() {
        let src = open_keyword();
        src.import(&fixture(), None).unwrap();
        let file = src.export(None).unwrap();
        let dst = open_keyword();
        let r = dst.import(&file, None).unwrap();
        assert_eq!(r.imported, 3, "{r:?}");
        let a = dst.get_by_id(ID_A).unwrap().expect("id survives export");
        assert_eq!(
            (a.memory_type.as_str(), a.pinned, a.slug.as_deref()),
            ("decision", true, Some("store-choice"))
        );
        assert!((a.importance - 0.9).abs() < 1e-9);
    }

    /// Copying a store's own export into another namespace copies the rows:
    /// an id stored in another namespace is not this row's identity there.
    #[test]
    fn an_import_into_another_namespace_copies_the_rows_under_new_ids() {
        let u = open_keyword();
        u.create_room("team", None, "default").unwrap();
        let file = format!(
            r#"{{"id":"{ID_A}","content":"copy me into another namespace","memory_type":"decision","rooms":["team"]}}"#
        );
        assert_eq!(u.import(&file, None).unwrap().imported, 1);
        let r = u.import(&file, Some("other")).unwrap();
        assert_eq!(
            (r.imported, r.duplicates_skipped, r.id_conflicts_skipped),
            (1, 0, 0),
            "{r:?}"
        );
        let copy = &u.list(None, 10, 0, Some("other")).unwrap()[0];
        assert_ne!(copy.id, ID_A, "a new id in the other namespace");
        assert_eq!(copy.memory_type, "decision");
        assert_eq!(u.get_by_id(ID_A).unwrap().unwrap().namespace, "default");
        // the copy is linked, the original keeps its one link
        assert_eq!(u.recall_room("team", None, 0).unwrap().len(), 2);
        // and a second copy is a duplicate in that namespace
        let again = u.import(&file, Some("other")).unwrap();
        assert_eq!(
            (again.imported, again.duplicates_skipped),
            (0, 1),
            "{again:?}"
        );
    }

    #[test]
    fn rows_without_an_id_keep_the_cosine_duplicate_check() {
        let u = open_keyword();
        u.remember("alpha beta gamma delta epsilon", &[], None, None)
            .unwrap();
        let r = u
            .import(r#"{"content":"alpha beta gamma delta epsilon"}"#, None)
            .unwrap();
        assert_eq!((r.imported, r.duplicates_skipped), (0, 1), "{r:?}");
    }
}

/// A Uteke Cloud export (#1632): documents become documents, room rows become
/// rooms, both before the memories' room links, and the manifest is skipped.
#[cfg(test)]
mod cloud_export_tests {
    use crate::test_support::{RecordingEmbedder, open_keyword, open_with_embedder};

    /// The Cloud row order: manifest, memories, documents, rooms. The
    /// grandchild comes before its parent.
    fn cloud_export() -> String {
        [
            r#"{"format_version":1,"exported_at":"2026-11-02T00:00:00Z","workspace_id":"w1","room":null,"live_only":false,"note":"uteke-cloud export"}"#,
            r#"{"id":"0b9a4f3e-6a52-4c1f-9d1e-3f1d2c4b5a61","namespace":"default","content":"the team ships on fridays","memory_type":"decision","importance":0.8,"rooms":["r1"],"workspace_id":"w1"}"#,
            r#"{"id":"1c8b5e4f-7b63-4d2a-8e2f-4a2e3d5c6b72","namespace":"default","content":"a memory in no room","workspace_id":"w1"}"#,
            r#"{"type":"document","id":"d0","namespace":"default","slug":"grandchild","title":"Grandchild","content":"the lowest page","tags":["t"],"metadata":{},"parent":"child"}"#,
            r#"{"type":"document","id":"d1","namespace":"default","slug":"root","title":"Root","content":"the top page","tags":[],"metadata":{},"parent":null}"#,
            r#"{"type":"document","id":"d2","namespace":"default","slug":"child","title":"Child","content":"the middle page","tags":[],"metadata":{},"parent":"root"}"#,
            r#"{"type":"room","id":"r1","title":"Team","description":"the team room","created_at":"2026-10-01T00:00:00Z","doc_slugs":["root","not-exported"]}"#,
            r#"{"type":"room","id":"r2","title":null,"description":null,"created_at":"2026-10-01T00:00:00Z"}"#,
        ]
        .join("\n")
    }

    #[test]
    fn documents_rooms_and_links_survive_and_the_manifest_is_silent() {
        let u = open_keyword();
        let r = u.import(&cloud_export(), None).unwrap();
        assert_eq!((r.imported, r.skipped), (2, 0), "{r:?}");
        assert_eq!((r.documents_imported, r.parent_unresolved), (3, 0), "{r:?}");
        assert_eq!((r.rooms_imported, r.rooms_existing), (2, 0), "{r:?}");
        assert_eq!(
            (r.room_documents_linked, r.room_documents_missing),
            (1, 1),
            "{r:?}"
        );
        assert_eq!(r.room_links, 1, "{r:?}");
        assert!(r.rooms_missing.is_empty(), "{r:?}");

        assert_eq!(u.count(None).unwrap(), 2, "documents are not memories");
        let root = u.doc_get("root").unwrap().unwrap();
        let child = u.doc_get("child").unwrap().unwrap();
        let grandchild = u.doc_get("grandchild").unwrap().unwrap();
        assert_eq!(child.parent_id.as_deref(), Some(root.id.as_str()));
        assert_eq!(grandchild.parent_id.as_deref(), Some(child.id.as_str()));
        assert_eq!(grandchild.depth, 2);
        assert_eq!(grandchild.title, "Grandchild");

        let room = u.get_room("r1").unwrap().unwrap();
        assert_eq!(room.title.as_deref(), Some("Team"));
        assert_eq!(room.description.as_deref(), Some("the team room"));
        assert_eq!(u.room_list_documents("r1").unwrap(), vec!["root"]);
        assert_eq!(u.recall_room("r1", None, 0).unwrap().len(), 1);
        assert!(u.get_room("r2").unwrap().is_some());
    }

    #[test]
    fn a_second_import_changes_nothing() {
        let u = open_keyword();
        u.import(&cloud_export(), None).unwrap();
        let version = u.doc_get("root").unwrap().unwrap().version;
        let r = u.import(&cloud_export(), None).unwrap();
        assert_eq!((r.imported, r.duplicates_skipped), (0, 2), "{r:?}");
        assert_eq!(
            (
                r.documents_imported,
                r.documents_updated,
                r.documents_unchanged
            ),
            (0, 0, 3),
            "{r:?}"
        );
        assert_eq!((r.rooms_imported, r.rooms_existing), (0, 2), "{r:?}");
        assert_eq!(r.room_documents_linked, 0, "{r:?}");
        assert_eq!(u.doc_get("root").unwrap().unwrap().version, version);
    }

    #[test]
    fn a_changed_document_is_updated_and_an_existing_room_is_left_alone() {
        let u = open_keyword();
        u.create_room("r1", Some("Mine"), "default").unwrap();
        u.import(&cloud_export(), None).unwrap();
        assert_eq!(
            u.get_room("r1").unwrap().unwrap().title.as_deref(),
            Some("Mine")
        );
        let changed =
            r#"{"type":"document","slug":"root","title":"Root","content":"the top page, edited"}"#;
        let r = u.import(changed, None).unwrap();
        assert_eq!((r.documents_imported, r.documents_updated), (0, 1), "{r:?}");
        assert_eq!(
            u.doc_get("root").unwrap().unwrap().content,
            "the top page, edited"
        );
        // the update keeps the document where it is in the tree
        let child = u.doc_get("child").unwrap().unwrap();
        assert_eq!(
            child.parent_id,
            Some(u.doc_get("root").unwrap().unwrap().id)
        );
    }

    #[test]
    fn an_unknown_parent_lands_at_the_root_and_is_counted() {
        let u = open_keyword();
        let r = u
            .import(
                r#"{"type":"document","slug":"orphan","title":"Orphan","content":"x","parent":"nowhere"}"#,
                None,
            )
            .unwrap();
        assert_eq!((r.documents_imported, r.parent_unresolved), (1, 1), "{r:?}");
        assert_eq!(u.doc_get("orphan").unwrap().unwrap().parent_id, None);
    }

    #[test]
    fn invalid_document_and_room_rows_are_skipped() {
        let u = open_keyword();
        let file = [
            r#"{"type":"document","slug":"","content":"no slug"}"#,
            r#"{"type":"document","slug":"no-content"}"#,
            r#"{"type":"room","id":"   "}"#,
            r#"{"neither":"memory nor manifest"}"#,
        ]
        .join("\n");
        let r = u.import(&file, None).unwrap();
        assert_eq!((r.imported, r.skipped), (0, 4), "{r:?}");
        assert!(u.list_rooms(None).unwrap().is_empty());
    }

    #[test]
    fn a_json_array_export_imports_documents_and_rooms_too() {
        let u = open_keyword();
        let rows: Vec<serde_json::Value> = cloud_export()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        let r = u
            .import(&serde_json::to_string(&rows).unwrap(), None)
            .unwrap();
        assert_eq!(
            (
                r.imported,
                r.documents_imported,
                r.rooms_imported,
                r.skipped
            ),
            (2, 3, 2, 0),
            "{r:?}"
        );
    }

    #[test]
    fn a_deferred_import_stores_document_chunks_without_vectors() {
        let (emb, seen) = RecordingEmbedder::new();
        let u = open_with_embedder(Box::new(emb));
        let r = u
            .import_with(
                &cloud_export(),
                None,
                crate::ImportOptions {
                    defer_embedding: true,
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(r.documents_imported, 3, "{r:?}");
        assert!(seen.lock().unwrap().is_empty(), "nothing embedded");
        assert!(!u.store.chunks_missing_embeddings().unwrap().is_empty());
        // keyword search finds the document at once
        assert!(!u.doc_search("middle", 5, "fts").unwrap().is_empty());
    }

    #[test]
    fn a_plain_import_reports_no_document_or_room_counts() {
        let u = open_keyword();
        let r = u.import(r#"{"content":"just a memory"}"#, None).unwrap();
        let json = serde_json::to_value(&r).unwrap();
        for key in [
            "documents_imported",
            "documents_updated",
            "documents_unchanged",
            "parent_unresolved",
            "rooms_imported",
            "rooms_existing",
            "room_documents_linked",
            "room_documents_missing",
        ] {
            assert!(json.get(key).is_none(), "{key}: {json}");
        }
    }
}
