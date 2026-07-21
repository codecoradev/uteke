//! Import and export memories in JSONL format.

use crate::error::Error;
use crate::memory::types::{DEFAULT_NAMESPACE, ExportEntry, ImportResult, Memory};

impl crate::Uteke {
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
                source: m.source,
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
            self.ensure_embedder()?;
            let embedding = self
                .embedder
                .lock()
                .map_err(|_| Error::lock("embedder lock during import"))?
                .as_ref()
                .expect("embedder ensured above")
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
                importance: 0.5,
                pinned: false,
                content_type: "text".to_string(),
                slug: None,
                source: Some(format!("import:{}", entry.source.unwrap_or_default())),
                source_type: "import".to_string(),
            };

            // Write-ahead: vector index first (can be rolled back), then SQLite.
            {
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during import"))?;
                index.insert(&id, &embedding)?;
                // Don't save per-item — we'll persist once after the full import.
            }

            if let Err(e) = self.store.insert(&memory) {
                // Rollback: remove from vector index
                let mut index = self
                    .index
                    .write()
                    .map_err(|_| Error::lock("index write lock during import rollback"))?;
                index.remove(&id);
                // Note: don't save per-entry — save once at end of import.
                // If process crashes, orphan entry is harmless and cleaned by repair.
                tracing::warn!("Skipping import entry (id={id}): {e}");
                skipped += 1;
                continue;
            }

            imported += 1;
        }

        // Persist vector index after import completes
        if imported > 0 {
            let mut index = self
                .index
                .write()
                .map_err(|_| Error::lock("index write lock during import save"))?;
            index.save()?;
        }

        if skipped > 0 {
            tracing::warn!(
                "Import completed with {imported} imported and {skipped} skipped entries. \
                 Check logs above for individual entry errors."
            );
        }

        Ok(ImportResult { imported, skipped })
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
        };
        let json = serde_json::to_string(&entry).unwrap();
        let restored: ExportEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.content, "hello world");
        assert_eq!(restored.tags.len(), 1);
    }
}
