//! Memory lookup and counts: by id, short id, batch and tag.

use super::*;

impl crate::Uteke {
    /// Count memories by tag in a namespace.
    pub fn count_by_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count_by_tag(tag, namespace)
    }

    /// Count total memories, optionally filtered by namespace.
    pub fn count(&self, namespace: Option<&str>) -> Result<usize, Error> {
        self.store.count(namespace)
    }

    /// Get a memory by ID (without touching access count — used for internal lookups).
    pub fn get_by_id(&self, id: &str) -> Result<Option<Memory>, Error> {
        self.store.get_by_id(id)
    }

    /// Batch-fetch multiple memories by ID (eliminates N+1 in graph traversal).
    pub fn get_by_ids(&self, ids: &[&str]) -> Result<Vec<Memory>, Error> {
        self.store.get_by_ids(ids)
    }

    /// Resolve a short ID prefix (first 8 chars) to a full memory ID (#794).
    ///
    /// Returns `Ok(Some(id))` for exact match, `Ok(None)` if not found,
    /// or `Err(Validation)` if ambiguous.
    pub fn resolve_id_prefix(&self, prefix: &str) -> Result<Option<String>, Error> {
        self.store.resolve_id_prefix(prefix)
    }
}
