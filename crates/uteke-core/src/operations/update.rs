//! Updating a memory in place (#1202): fields, privacy filter, re-embed when the content changes.

use super::*;

impl crate::Uteke {
    /// Update an existing memory with partial fields (#659).
    ///
    /// Only provided fields are changed. If `content` is changed, the
    /// embedding is regenerated and the vector index is updated.
    /// Returns `Ok(true)` if the memory was found and updated,
    /// `Ok(false)` if the memory ID doesn't exist. Use
    /// [`update_memory_detailed`](Self::update_memory_detailed) to also learn
    /// what the privacy filter masked.
    ///
    /// Acceptance criteria:
    /// - Partial update semantics (only provided fields changed)
    /// - Content update regenerates embedding
    /// - 404 if not found (caller checks return value)
    #[allow(clippy::too_many_arguments)]
    pub fn update_memory(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
        metadata: Option<&serde_json::Value>,
        importance: Option<f64>,
        pinned: Option<bool>,
        memory_type: Option<&str>,
    ) -> Result<bool, Error> {
        self.update_memory_detailed(id, content, tags, metadata, importance, pinned, memory_type)
            .map(|o| o.updated)
    }

    /// [`update_memory`](Self::update_memory) that returns an [`UpdateOutcome`]:
    /// whether the memory was updated, and what the privacy filter masked in
    /// the new content, tags or metadata (#1464).
    #[allow(clippy::too_many_arguments)]
    pub fn update_memory_detailed(
        &self,
        id: &str,
        content: Option<&str>,
        tags: Option<&[String]>,
        metadata: Option<&serde_json::Value>,
        importance: Option<f64>,
        pinned: Option<bool>,
        memory_type: Option<&str>,
    ) -> Result<UpdateOutcome, Error> {
        // Validate memory exists
        let existing = self
            .store
            .get_by_id(id)?
            .ok_or_else(|| Error::not_found(format!("Memory not found: {id}")))?;

        // Validate new content if provided
        if let Some(c) = content {
            crate::validate_input(c, &[] as &[&str])?;
        }

        // Validate new tags if provided
        if let Some(t) = tags {
            let tag_refs: Vec<&str> = t.iter().map(|s| s.as_str()).collect();
            crate::validate_input(content.unwrap_or(&existing.content), &tag_refs)?;
        }

        // Validate new memory_type if provided
        if let Some(mt) = memory_type {
            crate::memory::types::MemoryType::from_str_opt(mt).ok_or_else(|| {
                Error::Validation(format!(
                    "Unknown memory type '{mt}'. Valid types: fact, procedure, preference, decision, context, note, insight, reference, event"
                ))
            })?;
        }

        // Validate importance range
        if let Some(imp) = importance {
            if !(0.0..=1.0).contains(&imp) {
                return Err(Error::Validation(format!(
                    "Importance must be between 0.0 and 1.0, got {imp}"
                )));
            }
        }

        // Privacy filter (#1464) on the new content, before it is embedded or
        // stored. A masked update also records `redacted` in the metadata:
        // the caller's metadata when given, else the memory's current one.
        let prepared = match content {
            Some(c) => Some(self.privacy_config.prepare(c, None)?),
            None => None,
        };
        // The tags and the metadata strings are filtered too; what they hold
        // counts with the content's redactions.
        let (masked_metadata, meta_redactions) = match metadata {
            Some(m) => {
                let (v, r) = self.privacy_config.mask_metadata(m)?;
                (Some(v), r)
            }
            None => (None, Vec::new()),
        };
        let metadata: Option<&serde_json::Value> = masked_metadata.as_ref();
        let (masked_tags, tag_redactions) = match tags {
            Some(t) => {
                let (v, r) = self.privacy_config.mask_tags(t)?;
                (Some(v), r)
            }
            None => (None, Vec::new()),
        };
        let tags: Option<&[String]> = masked_tags.as_deref();
        let mut merged: std::collections::BTreeMap<&'static str, usize> = Default::default();
        for r in prepared
            .iter()
            .flat_map(|p| p.redactions.iter())
            .chain(&meta_redactions)
            .chain(&tag_redactions)
        {
            *merged.entry(r.kind).or_default() += r.count;
        }
        let redactions: Vec<crate::privacy::RedactionCount> = merged
            .into_iter()
            .map(|(kind, count)| crate::privacy::RedactionCount { kind, count })
            .collect();
        let masked_content: Option<String> = prepared.map(|p| p.content);
        // The same text as what is stored is not a content change: no new vector
        // and no embedding call, only the other fields are updated.
        let content: Option<&str> = masked_content.as_deref().filter(|c| *c != existing.content);
        let flagged_metadata =
            crate::redaction_flags::resolve(content, metadata, &existing.metadata, &redactions);
        let metadata: Option<&serde_json::Value> = flagged_metadata.as_ref().or(metadata);

        let now = chrono::Utc::now();

        // Re-embed if content changed
        if let Some(c) = content {
            let content_type = crate::memory::crud::detect_content_type(c);
            let embed_text = if content_type == "json" {
                crate::memory::crud::flatten_json_for_embedding(c)
            } else {
                c.to_string()
            };
            let new_embedding = self.embed(&embed_text)?;

            // Update SQLite first, then vector index
            let updated = self.store.update_fields(
                id,
                content,
                tags,
                metadata,
                importance,
                pinned,
                memory_type,
                now,
            )?;

            if !updated {
                return Ok(UpdateOutcome::default());
            }

            // Update vector index (insert handles dedup by removing old entry)
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during update_memory"))?;
            let sync = index_sync::upsert(&mut *index, id, &new_embedding, &SyncPolicy::STANDARD);
            if let Some(e) = &sync.insert_error {
                return Err(Error::embed_msg(format!(
                    "Memory {id} was updated in the database, but its vector could not be \
                     written to the index: {e}. Run `uteke repair` to resync."
                )));
            }
            if let Some(msg) = sync.error_message() {
                tracing::error!(
                    "{msg} (update_memory id={id}). Index may be stale on next launch."
                );
            }
        } else {
            // No content change — just update SQLite fields
            let updated = self.store.update_fields(
                id,
                None,
                tags,
                metadata,
                importance,
                pinned,
                memory_type,
                now,
            )?;
            if !updated {
                return Ok(UpdateOutcome::default());
            }
        }

        self.record_redaction(id, &redactions);

        // Invalidate recall cache for the memory's namespace
        self.recall_cache.invalidate_namespace(&existing.namespace);

        Ok(UpdateOutcome {
            updated: true,
            redactions,
        })
    }
}
