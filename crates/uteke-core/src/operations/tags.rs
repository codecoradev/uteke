//! Tags: list, rename and delete across memories.

use super::*;

impl crate::Uteke {
    /// List all tags with their usage counts.
    pub fn tags_with_counts(&self, namespace: Option<&str>) -> Result<Vec<TagInfo>, Error> {
        self.store.tags_with_counts(namespace)
    }

    /// Rename a tag across all memories in a namespace.
    pub fn rename_tag(
        &self,
        old: &str,
        new: &str,
        namespace: Option<&str>,
    ) -> Result<usize, Error> {
        let renamed = self.store.rename_tag(old, new, namespace)?;
        if renamed > 0 {
            // Invalidate recall cache — tag filters may reference the old
            // tag name, so cached results are now stale (#849).
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
        }
        Ok(renamed)
    }

    /// Delete a tag from all memories in a namespace.
    pub fn delete_tag(&self, tag: &str, namespace: Option<&str>) -> Result<usize, Error> {
        let deleted = self.store.delete_tag(tag, namespace)?;
        if deleted > 0 {
            // Invalidate recall cache to prevent stale search results (#844).
            // If namespace is specified, invalidate only that namespace.
            // Cross-namespace delete requires full cache flush.
            if let Some(ns) = namespace {
                self.recall_cache.invalidate_namespace(ns);
            } else {
                self.recall_cache.clear();
            }
        }
        Ok(deleted)
    }
}
