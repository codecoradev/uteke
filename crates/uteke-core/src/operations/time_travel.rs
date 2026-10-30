//! Point-in-time recall and listing.

use super::*;

impl crate::Uteke {
    /// Recall memories that existed at a specific point in time.
    ///
    /// Runs a semantic recall to gather candidates, then post-filters by
    /// temporal validity at `point_in_time`:
    /// - `created_at <= point_in_time`
    /// - `valid_until IS NULL OR valid_until > point_in_time`
    /// - `valid_from IS NULL OR valid_from <= point_in_time`
    /// - `deprecated = false`
    #[allow(clippy::too_many_arguments)]
    pub fn recall_at_time(
        &self,
        query: &str,
        limit: usize,
        tags_filter: Option<&[&str]>,
        namespace: Option<&str>,
        point_in_time: chrono::DateTime<chrono::Utc>,
        min_score: f32,
        entity_filter: Option<&str>,
        category_filter: Option<&str>,
    ) -> Result<Vec<SearchResult>, Error> {
        // Retry loop: over-fetch with increasing multipliers to compensate
        // for temporal filtering removing candidates. If post-filtering
        // yields fewer than `limit` results, expand the search scope.
        let mut multiplier = 3usize;
        let mut attempt = 0;

        loop {
            let fetch_limit = (limit * multiplier).max(50);
            let candidates = self.recall_inner(
                query,
                fetch_limit,
                tags_filter,
                namespace,
                min_score,
                entity_filter,
                category_filter,
                true,
            )?;
            let candidates_len = candidates.len();

            let mut results: Vec<SearchResult> = candidates
                .into_iter()
                .filter(|r| memory_existed_at(&r.memory, point_in_time))
                .collect();

            // Stop if we have enough results or exhausted retry budget.
            if results.len() >= limit || attempt >= 2 {
                results.truncate(limit);
                return Ok(results);
            }

            // If the fetch returned fewer candidates than fetch_limit, the
            // index is exhausted — expanding the search scope won't help.
            if candidates_len < fetch_limit {
                results.truncate(limit);
                return Ok(results);
            }

            attempt += 1;
            multiplier *= 3;
        }
    }

    /// List memories that existed at a specific point in time.
    ///
    /// Thin wrapper around the store-level temporal query.
    pub fn list_at_time(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
        namespace: Option<&str>,
        point_in_time: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<Memory>, Error> {
        self.store
            .list_at_time(tag, namespace, limit, offset, point_in_time)
    }
}

/// Temporal predicate shared by `recall_at_time` (core) and
/// `memory_exists_at` (server): did this memory exist at `pit`?
/// Deprecated-before-pit excludes; deprecated-after-pit includes (#1086).
pub fn memory_existed_at(
    m: &crate::memory::types::Memory,
    pit: chrono::DateTime<chrono::Utc>,
) -> bool {
    if m.created_at > pit {
        return false;
    }
    if let Some(valid_until) = m.valid_until {
        if valid_until <= pit {
            return false;
        }
    }
    if m.deprecated {
        match m.deprecated_at {
            Some(dep_at) if dep_at <= pit => return false,
            _ => {}
        }
    }
    if let Some(valid_from) = m.valid_from {
        if valid_from > pit {
            return false;
        }
    }
    true
}
