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

    /// [`Self::recall_at_time`] in history mode (#1598): memories deprecated
    /// after `point_in_time` are candidates too, as the `at` rule
    /// ([`memory_existed_at`]) has said since #1086. Deprecation removes a
    /// memory's vector from the index, so those rows come from SQL and are
    /// scored by cosine similarity against the embedding kept in the row
    /// (with the same hot boost), then merged with the index results by
    /// score. The extra cost is one query plus one dot product per memory
    /// deprecated after `point_in_time`, a set `prune` bounds. The default
    /// `at` on HTTP and MCP stays [`Self::recall_at_time`] until the contract
    /// changes (uteke-cloud#458).
    #[allow(clippy::too_many_arguments)]
    pub fn recall_at_time_history(
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
        let mut results = self.recall_at_time(
            query,
            limit,
            tags_filter,
            namespace,
            point_in_time,
            min_score,
            entity_filter,
            category_filter,
        )?;
        let meta_is = |m: &crate::memory::types::Memory, key: &str, want: Option<&str>| {
            want.is_none_or(|w| m.metadata.get(key).and_then(|v| v.as_str()) == Some(w))
        };
        let retired: Vec<_> = self
            .store
            .deprecated_after(namespace, point_in_time)?
            .into_iter()
            .filter(|m| memory_existed_at(m, point_in_time))
            .filter(|m| {
                tags_filter.is_none_or(|ft| ft.iter().any(|f| m.tags.iter().any(|t| t == f)))
            })
            .filter(|m| {
                meta_is(m, "entity", entity_filter) && meta_is(m, "category", category_filter)
            })
            .filter(|m| !results.iter().any(|r| r.memory.id == m.id))
            .collect();
        if retired.is_empty() {
            return Ok(results);
        }
        let query_embedding = self.embed(query)?;
        for memory in retired {
            // A vector from another embedding model cannot be compared.
            if memory.embedding.len() != query_embedding.len() {
                continue;
            }
            let similarity =
                crate::consolidate::cosine_similarity(&query_embedding, &memory.embedding)
                    .clamp(0.0, 1.0);
            let score = self.hot_boosted(&memory, similarity);
            if min_score > 0.0 && score < min_score {
                continue;
            }
            results.push(SearchResult { memory, score });
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// [`Self::list_at_time`], optionally in history mode (#1598): with
    /// `history` the memories deprecated after `point_in_time` are listed
    /// too.
    pub fn list_at_time_with(
        &self,
        tag: Option<&str>,
        limit: usize,
        offset: usize,
        namespace: Option<&str>,
        point_in_time: chrono::DateTime<chrono::Utc>,
        history: bool,
    ) -> Result<Vec<Memory>, Error> {
        self.store
            .list_at_time_with(tag, namespace, limit, offset, point_in_time, history)
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

#[cfg(test)]
mod history_tests {
    //! #1598: history mode sees memories deprecated after the instant, end to
    //! end through the real deprecation paths (which drop the vector).
    use crate::Uteke;
    use crate::test_support::open_keyword;
    use chrono::{Duration, Utc};

    /// Move a memory's write time (and default validity start) into the past.
    fn backdate(u: &Uteke, id: &str, hours: i64) {
        let t = (Utc::now() - Duration::hours(hours)).to_rfc3339();
        u.store
            .conn
            .execute(
                "UPDATE memories SET created_at = ?1, valid_from = ?1 WHERE id = ?2",
                rusqlite::params![t, id],
            )
            .unwrap();
    }

    fn ids(results: &[crate::SearchResult]) -> Vec<&str> {
        results.iter().map(|r| r.memory.id.as_str()).collect()
    }

    /// Two memories written two hours ago, one forgotten and one superseded
    /// now, plus one that stays live. Returns (forgotten, superseded, live).
    fn seed(u: &Uteke) -> (String, String, String) {
        let forgotten = u
            .remember("deploy port quartz is 8080", &["ops"], None, None)
            .unwrap();
        let superseded = u
            .remember("deploy host quartz is alpha", &["ops"], None, None)
            .unwrap();
        let live = u
            .remember("deploy region quartz is jakarta", &["ops"], None, None)
            .unwrap();
        for id in [&forgotten, &superseded, &live] {
            backdate(u, id, 2);
        }
        u.soft_forget(&forgotten, "outdated").unwrap();
        let newer = u
            .remember("deploy host quartz is beta", &["ops"], None, None)
            .unwrap();
        u.supersede(&superseded, &newer, Some("moved")).unwrap();
        (forgotten, superseded, live)
    }

    #[test]
    fn history_recall_returns_rows_deprecated_after_the_instant() {
        let u = open_keyword();
        let (forgotten, superseded, live) = seed(&u);
        let hour_ago = Utc::now() - Duration::hours(1);

        // Default `at` is unchanged: the index lost both vectors.
        let plain = u
            .recall_at_time("deploy quartz", 10, None, None, hour_ago, 0.0, None, None)
            .unwrap();
        assert_eq!(ids(&plain), vec![live.as_str()]);

        let history = u
            .recall_at_time_history("deploy quartz", 10, None, None, hour_ago, 0.0, None, None)
            .unwrap();
        let mut got = ids(&history);
        got.sort_unstable();
        let mut want = vec![forgotten.as_str(), superseded.as_str(), live.as_str()];
        want.sort_unstable();
        assert_eq!(got, want);
        assert!(history.windows(2).all(|w| w[0].score >= w[1].score));
        assert!(history.iter().all(|r| r.score > 0.0));

        // After the deprecation they are gone again; the replacement shows.
        let later = Utc::now() + Duration::seconds(1);
        let after = u
            .recall_at_time_history("deploy quartz", 10, None, None, later, 0.0, None, None)
            .unwrap();
        assert!(!ids(&after).contains(&forgotten.as_str()));
        assert!(!ids(&after).contains(&superseded.as_str()));
        assert!(ids(&after).contains(&live.as_str()));
    }

    #[test]
    fn history_recall_applies_the_filters_and_the_limit() {
        let u = open_keyword();
        let (forgotten, _, _) = seed(&u);
        let hour_ago = Utc::now() - Duration::hours(1);
        let other_tag = u
            .recall_at_time_history(
                "deploy quartz",
                10,
                Some(&["elsewhere"]),
                None,
                hour_ago,
                0.0,
                None,
                None,
            )
            .unwrap();
        assert!(other_tag.is_empty(), "{:?}", ids(&other_tag));
        let other_ns = u
            .recall_at_time_history(
                "deploy quartz",
                10,
                None,
                Some("elsewhere"),
                hour_ago,
                0.0,
                None,
                None,
            )
            .unwrap();
        assert!(other_ns.is_empty());
        let one = u
            .recall_at_time_history("deploy port 8080", 1, None, None, hour_ago, 0.0, None, None)
            .unwrap();
        assert_eq!(one.len(), 1);
        // Before the memory was written it is not there in history mode either.
        let before = Utc::now() - Duration::hours(3);
        let early = u
            .recall_at_time_history("deploy quartz", 10, None, None, before, 0.0, None, None)
            .unwrap();
        assert!(!ids(&early).contains(&forgotten.as_str()));
    }

    #[test]
    fn history_list_includes_rows_deprecated_after_the_instant() {
        let u = open_keyword();
        let (forgotten, superseded, live) = seed(&u);
        let hour_ago = Utc::now() - Duration::hours(1);
        let listed = |tag: Option<&str>, ns: Option<&str>, history: bool| {
            let mut v: Vec<String> = u
                .list_at_time_with(tag, 50, 0, ns, hour_ago, history)
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect();
            v.sort_unstable();
            v
        };
        let mut all = vec![forgotten.clone(), superseded.clone(), live.clone()];
        all.sort_unstable();
        // Every branch of the query (tag and/or namespace).
        for (tag, ns) in [
            (None, None),
            (Some("ops"), None),
            (None, Some("default")),
            (Some("ops"), Some("default")),
        ] {
            assert_eq!(listed(tag, ns, false), vec![live.clone()], "{tag:?} {ns:?}");
            assert_eq!(listed(tag, ns, true), all, "{tag:?} {ns:?}");
        }
        // The plain wrapper is the non-history query.
        assert_eq!(
            u.list_at_time(None, 50, 0, None, hour_ago).unwrap().len(),
            1
        );
        let later = Utc::now() + Duration::seconds(1);
        let after: Vec<String> = u
            .list_at_time_with(None, 50, 0, None, later, true)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(!after.contains(&forgotten) && !after.contains(&superseded));
    }
}
