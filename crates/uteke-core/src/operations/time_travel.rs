//! Point-in-time recall and listing.

use super::*;

impl crate::Uteke {
    /// The index half of [`Self::recall_at_time`]: a semantic recall to gather
    /// candidates, post-filtered by [`memory_existed_at`]. It only sees what
    /// the vector index holds, i.e. live memories.
    #[allow(clippy::too_many_arguments)]
    fn recall_at_time_index(
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

    /// Recall the memories that existed at `point_in_time` (#1598), the
    /// meaning `at` has had since #1086: `created_at <= at < deprecated_at`
    /// and `valid_from <= at < valid_until` ([`memory_existed_at`]). A memory
    /// deprecated after `at` is therefore a candidate too. Deprecation removes
    /// a memory's vector from the index, so those rows come from SQL and are
    /// scored by cosine similarity against the embedding kept in the row
    /// (with the same hot boost), then merged with the index results by
    /// score. The extra cost is one query plus one dot product per memory
    /// deprecated after `point_in_time`, a set `prune` bounds. This is the
    /// default for `at` on every surface (uteke-cloud#458, Amendment 3).
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
        let mut results = self.recall_at_time_index(
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
            results.push(SearchResult {
                memory,
                score,
                similarity: Some(similarity),
                rooms: Vec::new(),
            });
        }
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        Ok(results)
    }

    /// List memories that existed at a specific point in time, deprecated
    /// ones included when they were deprecated after it (#1598).
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

        // The index alone lost both vectors; that was the gap.
        let plain = u
            .recall_at_time_index("deploy quartz", 10, None, None, hour_ago, 0.0, None, None)
            .unwrap();
        assert_eq!(ids(&plain), vec![live.as_str()]);

        let history = u
            .recall_at_time("deploy quartz", 10, None, None, hour_ago, 0.0, None, None)
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
            .recall_at_time("deploy quartz", 10, None, None, later, 0.0, None, None)
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
            .recall_at_time(
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
            .recall_at_time(
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
            .recall_at_time("deploy port 8080", 1, None, None, hour_ago, 0.0, None, None)
            .unwrap();
        assert_eq!(one.len(), 1);
        // Before the memory was written it is not there in history mode either.
        let before = Utc::now() - Duration::hours(3);
        let early = u
            .recall_at_time("deploy quartz", 10, None, None, before, 0.0, None, None)
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
                .store
                .list_at_time_with(tag, ns, 50, 0, hour_ago, history)
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
        // The default is history: the wrapper lists all three.
        assert_eq!(
            u.list_at_time(None, 50, 0, None, hour_ago).unwrap().len(),
            3
        );
        let later = Utc::now() + Duration::seconds(1);
        let after: Vec<String> = u
            .list_at_time(None, 50, 0, None, later)
            .unwrap()
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert!(!after.contains(&forgotten) && !after.contains(&superseded));
    }

    /// Room recall at an instant (#1598): rows deprecated after it come back,
    /// chronologically and by similarity; before the write or after the
    /// deprecation they do not.
    #[test]
    fn room_recall_at_time_includes_rows_deprecated_after_the_instant() {
        let u = open_keyword();
        u.create_room("ops", None, "default").unwrap();
        let gone = u
            .remember("deploy port quartz is 8080", &["ops"], None, None)
            .unwrap();
        let kept = u
            .remember("deploy region quartz is jakarta", &["ops"], None, None)
            .unwrap();
        for id in [&gone, &kept] {
            u.add_memory_to_room(id, "ops", "tester").unwrap();
            backdate(&u, id, 2);
        }
        u.soft_forget(&gone, "outdated").unwrap();
        let hour_ago = Utc::now() - Duration::hours(1);
        let later = Utc::now() + Duration::seconds(1);
        let before = Utc::now() - Duration::hours(3);

        let chrono_ids = |at| -> Vec<String> {
            u.recall_room_at_time("ops", None, 10, at)
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect()
        };
        let mut then = chrono_ids(hour_ago);
        then.sort_unstable();
        let mut want = vec![gone.clone(), kept.clone()];
        want.sort_unstable();
        assert_eq!(then, want);
        assert_eq!(chrono_ids(later), vec![kept.clone()]);
        assert!(chrono_ids(before).is_empty());
        assert_eq!(
            u.recall_room_at_time("ops", None, 1, hour_ago)
                .unwrap()
                .len(),
            1
        );

        let semantic = |at| -> Vec<String> {
            u.recall_room_semantic_at_time("ops", "deploy quartz", 10, None, 0.0, at)
                .unwrap()
                .into_iter()
                .map(|r| r.memory.id)
                .collect()
        };
        let mut then = semantic(hour_ago);
        then.sort_unstable();
        assert_eq!(then, want);
        assert_eq!(semantic(later), vec![kept]);
        assert!(semantic(before).is_empty());
    }

    /// The `at` rule on hand-built rows: created, validity and deprecation.
    #[test]
    fn existed_at_rule() {
        use super::super::memory_existed_at;
        let u = open_keyword();
        let id = u.remember("rule probe", &[], None, None).unwrap();
        let mut m = u.get_by_id(&id).unwrap().unwrap();
        let now = Utc::now();
        m.created_at = now - Duration::hours(1);
        m.valid_from = None;
        m.valid_until = None;
        assert!(memory_existed_at(&m, now));
        assert!(
            memory_existed_at(&m, m.created_at),
            "created_at is inclusive"
        );
        assert!(!memory_existed_at(&m, now - Duration::hours(2)));
        m.valid_until = Some(now - Duration::minutes(1));
        assert!(!memory_existed_at(&m, now));
        m.valid_until = None;
        m.valid_from = Some(now + Duration::minutes(1));
        assert!(!memory_existed_at(&m, now));
        m.valid_from = None;
        m.deprecated = true;
        m.deprecated_at = Some(now - Duration::seconds(30));
        assert!(!memory_existed_at(&m, now));
        m.deprecated_at = Some(now + Duration::seconds(30));
        assert!(memory_existed_at(&m, now), "deprecated after the instant");
    }
}

/// Point-in-time recall of memories deprecated since (#1729).
#[cfg(test)]
mod retired_filter_tests {
    use crate::test_support::open_keyword;
    use std::time::Duration;

    #[test]
    fn deprecated_memories_are_filtered_by_entity_at_a_point_in_time() {
        let u = open_keyword();
        let a = u
            .remember(
                "shared probe words about x",
                &[],
                Some(serde_json::json!({"entity": "x"})),
                None,
            )
            .unwrap();
        let b = u
            .remember(
                "shared probe words about y",
                &[],
                Some(serde_json::json!({"entity": "y"})),
                None,
            )
            .unwrap();
        std::thread::sleep(Duration::from_millis(50));
        let before_forgetting = chrono::Utc::now();
        std::thread::sleep(Duration::from_millis(50));
        u.soft_forget(&a, "gone").unwrap();
        u.soft_forget(&b, "gone").unwrap();

        let ids = |entity: Option<&str>| {
            let mut ids: Vec<String> = u
                .recall_at_time(
                    "shared probe words",
                    10,
                    None,
                    None,
                    before_forgetting,
                    0.0,
                    entity,
                    None,
                )
                .unwrap()
                .into_iter()
                .map(|r| r.memory.id)
                .collect();
            ids.sort();
            ids
        };
        let mut both = vec![a.clone(), b.clone()];
        both.sort();
        assert_eq!(ids(None), both, "both existed then");
        assert_eq!(ids(Some("x")), vec![a]);
        assert_eq!(ids(Some("y")), vec![b]);
    }
    #[test]
    fn the_index_fetch_widens_when_the_first_window_holds_only_newer_memories() {
        // limit 20 asks for max(20 * 3, 50) = 60 candidates: all 60 are newer
        // than the point in time and are dropped. The next pass (20 * 9) reaches
        // the 10 older memories. A window of limit + multiplier would stay at 50
        // on every pass and return nothing.
        let u = open_keyword();
        for i in 0..10 {
            u.remember(&format!("probe old{i}"), &[], None, None)
                .unwrap();
        }
        std::thread::sleep(Duration::from_millis(50));
        let point_in_time = chrono::Utc::now();
        std::thread::sleep(Duration::from_millis(50));
        for i in 0..60 {
            u.remember(&format!("probe probe probe probe new{i}"), &[], None, None)
                .unwrap();
        }
        let found = u
            .recall_at_time("probe", 20, None, None, point_in_time, 0.0, None, None)
            .unwrap();
        assert_eq!(found.len(), 10, "{found:?}");
        assert!(
            found
                .iter()
                .all(|r| r.memory.content.starts_with("probe old"))
        );
    }
}
