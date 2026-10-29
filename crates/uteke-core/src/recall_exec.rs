//! Running a decoded recall request (#1439, PR C).
//!
//! The HTTP handler used to own the decision tree that turns a
//! [`RecallRequest`] into core calls (explain, pack, unified, time travel,
//! the memory-only path with its post-filters), and MCP had a smaller copy
//! that honoured fewer parameters. This module is that tree, once. A surface
//! decodes its wire format into a [`RecallRequest`], calls
//! [`Uteke::run_recall`], and renders the [`RecallOutput`] its own way.
//!
//! The decision tree is the HTTP one, unchanged: the same guards in the same
//! order, the same core function per branch. A guard failure is an
//! [`Error::Validation`] carrying the exact text HTTP returned.

use crate::memory::types::{SearchResult, SearchType};
use crate::pack_mode::ContextPack;
use crate::recall_explain::ExplainedRecall;
use crate::{Error, RecallRequest, UnifiedSearchResult, Uteke};

/// The `[recall]` values a request falls back to.
#[derive(Debug, Clone, Copy, Default)]
pub struct RecallDefaults<'a> {
    pub min_score: Option<f64>,
    pub min_score_strict: Option<f64>,
    pub default_strategy: Option<&'a str>,
}

/// What a recall produced, before a surface renders it.
#[derive(Debug)]
pub enum RecallOutput {
    /// `explain`: per-result ranking signals.
    Explained(Vec<ExplainedRecall>),
    /// `pack`: a budgeted context pack.
    Packed(ContextPack),
    /// Unified recall over memories and documents (a `search_type` was given).
    Unified(Vec<UnifiedSearchResult>),
    /// Memory-only recall. `min_score` is the floor that was applied, so a
    /// surface can say "nothing above the threshold" when it matters.
    Memories {
        results: Vec<SearchResult>,
        min_score: f32,
    },
}

fn bad(msg: &str) -> Error {
    Error::Validation(msg.to_string())
}

impl Uteke {
    /// Run `req` as `POST /recall` always has. See the module documentation.
    pub fn run_recall(
        &self,
        req: &RecallRequest,
        defaults: RecallDefaults<'_>,
    ) -> Result<RecallOutput, Error> {
        let limit = req.limit;
        let tag_vec = req.tag_filter();
        let tags_filter = tag_vec.as_deref();
        let min_score = req.resolve_min_score(defaults.min_score, defaults.min_score_strict);
        let namespace = req.namespace.as_deref();
        let entity_filter = req.entity.as_deref();
        let category_filter = req.category.as_deref();
        let point_in_time = req.at;
        let (after_ts, before_ts) = (req.after, req.before);
        let has_temporal = after_ts.is_some() || before_ts.is_some();

        // One strategy for every path (#1034): an unknown value is an error on
        // all of them, never a silent no-op.
        let strategy = req
            .strategy(defaults.default_strategy)
            .map_err(|e| Error::Validation(e.to_string()))?;

        // Explain (#1160): memory-only, exclusive with the unified, time
        // travel and temporal surfaces.
        if req.explain {
            if req.search_type_raw().is_some() {
                return Err(bad("explain is memory-only: omit search_type"));
            }
            if point_in_time.is_some() || has_temporal {
                return Err(bad("explain cannot be combined with at/before/after"));
            }
            if entity_filter.is_some() || category_filter.is_some() || req.enrich {
                return Err(bad(
                    "explain cannot be combined with entity/category/enrich",
                ));
            }
            return self
                .recall_explained(
                    &req.query,
                    limit,
                    tags_filter,
                    namespace,
                    strategy,
                    min_score,
                )
                .map(RecallOutput::Explained);
        }

        // Budgeted context pack (#1281): not composable with the time filters.
        if req.pack {
            if point_in_time.is_some() || has_temporal {
                return Err(bad(
                    "pack is not supported with at/after/before filters; rerun without time filters",
                ));
            }
            let search_type = req
                .search_type()
                .map_err(|e| Error::Validation(e.to_string()))?;
            return self
                .recall_unified_packed(
                    &req.query,
                    limit,
                    tags_filter,
                    namespace,
                    min_score,
                    search_type,
                    entity_filter,
                    category_filter,
                    req.enrich,
                    strategy,
                    req.budget_chars_or_default(),
                    &req.exclude_ids,
                )
                .map(RecallOutput::Packed);
        }

        // Unified search (#531): when a search_type was given and there is no
        // point in time. Entity/category are pushed into the core candidate
        // loop (#663).
        if req.search_type_raw().is_some() && point_in_time.is_none() {
            let search_type: SearchType = req
                .search_type()
                .map_err(|e| Error::Validation(e.to_string()))?;
            let mut results = self.recall_unified(
                &req.query,
                limit,
                tags_filter,
                namespace,
                min_score,
                search_type,
                entity_filter,
                category_filter,
                req.enrich,
                strategy,
            )?;
            if has_temporal {
                results.retain(|r| {
                    r.created_at.as_ref().is_some_and(|t| {
                        after_ts.is_none_or(|a| t >= &a) && before_ts.is_none_or(|b| t <= &b)
                    })
                });
            }
            return Ok(RecallOutput::Unified(results));
        }

        // Memory-only recall (#1034): routed through recall_hybrid so the
        // resolved strategy applies. recall_hybrid has no native
        // entity/category support, so they are post-filters with a 3x
        // over-fetch (as in recall_unified_memories).
        let needs_post_filter = entity_filter.is_some() || category_filter.is_some();
        let fetch_limit = if needs_post_filter {
            limit.saturating_mul(3).max(limit + 10)
        } else {
            limit
        };
        let mut results = if let Some(pit) = point_in_time {
            // Time travel: the strategy does not apply to point-in-time recall.
            self.recall_at_time(
                &req.query,
                limit,
                tags_filter,
                namespace,
                pit,
                min_score,
                entity_filter,
                category_filter,
            )?
        } else {
            self.recall_hybrid(
                &req.query,
                fetch_limit,
                tags_filter,
                namespace,
                strategy,
                min_score,
            )?
        };
        if needs_post_filter && point_in_time.is_none() {
            results.retain(|sr| {
                entity_filter.is_none_or(|ent| {
                    sr.memory.metadata.get("entity").and_then(|v| v.as_str()) == Some(ent)
                }) && category_filter.is_none_or(|cat| {
                    sr.memory.metadata.get("category").and_then(|v| v.as_str()) == Some(cat)
                })
            });
            results.truncate(limit);
        }
        if has_temporal {
            results.retain(|r| {
                let t = &r.memory.created_at;
                after_ts.is_none_or(|a| t >= &a) && before_ts.is_none_or(|b| t <= &b)
            });
        }
        Ok(RecallOutput::Memories { results, min_score })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::open_keyword;
    use crate::{RecallInput, RecallPolicy};

    fn req(f: impl FnOnce(&mut RecallInput)) -> RecallRequest {
        let mut i = RecallInput {
            query: "rust memory safety".into(),
            ..RecallInput::default()
        };
        f(&mut i);
        RecallRequest::decode(i, RecallPolicy::HTTP).unwrap()
    }

    fn run(u: &Uteke, r: &RecallRequest) -> Result<RecallOutput, Error> {
        u.run_recall(r, RecallDefaults::default())
    }

    fn msg(e: Error) -> String {
        match e {
            Error::Validation(m) => m,
            other => panic!("not a validation error: {other}"),
        }
    }

    #[test]
    fn explain_is_memory_only_and_exclusive_with_the_other_surfaces() {
        let u = open_keyword();
        type Setter = Box<dyn Fn(&mut RecallInput)>;
        let cases: [(Setter, &str); 4] = [
            (
                Box::new(|i| i.search_type = Some("memory".into())),
                "explain is memory-only: omit search_type",
            ),
            (
                Box::new(|i| i.at = Some("2026-01-01T00:00:00Z".into())),
                "explain cannot be combined with at/before/after",
            ),
            (
                Box::new(|i| i.after = Some("2026-01-01T00:00:00Z".into())),
                "explain cannot be combined with at/before/after",
            ),
            (
                Box::new(|i| i.entity = Some("e".into())),
                "explain cannot be combined with entity/category/enrich",
            ),
        ];
        for (set, expected) in cases {
            let r = req(|i| {
                i.explain = true;
                set(i);
            });
            assert_eq!(msg(run(&u, &r).unwrap_err()), expected);
        }
        let ok = req(|i| i.explain = true);
        assert!(matches!(run(&u, &ok), Ok(RecallOutput::Explained(_))));
    }

    #[test]
    fn pack_refuses_time_filters() {
        let u = open_keyword();
        for set in [
            Box::new(|i: &mut RecallInput| i.at = Some("2026-01-01T00:00:00Z".into()))
                as Box<dyn Fn(&mut RecallInput)>,
            Box::new(|i| i.before = Some("2026-01-01T00:00:00Z".into())),
        ] {
            let r = req(|i| {
                i.pack = true;
                set(i);
            });
            assert_eq!(
                msg(run(&u, &r).unwrap_err()),
                "pack is not supported with at/after/before filters; rerun without time filters"
            );
        }
    }

    #[test]
    fn an_unknown_strategy_or_search_type_is_a_validation_error() {
        let u = open_keyword();
        let r = req(|i| i.strategy = Some("nope".into()));
        assert!(msg(run(&u, &r).unwrap_err()).starts_with("Invalid strategy: 'nope'"));
        let r = req(|i| i.search_type = Some("nope".into()));
        assert!(msg(run(&u, &r).unwrap_err()).starts_with("Invalid search_type: 'nope'"));
    }

    #[test]
    fn the_memory_only_path_applies_entity_category_and_the_temporal_range() {
        let u = open_keyword();
        u.remember(
            "rust gives memory safety without a garbage collector",
            &[],
            Some(serde_json::json!({"entity": "rust", "category": "lang"})),
            None,
        )
        .unwrap();
        u.remember("rust memory safety also helps concurrency", &[], None, None)
            .unwrap();
        // "memory safety" is a phrase in both texts (FTS5 matches phrases).
        let q = |i: &mut RecallInput| i.query = "memory safety".into();
        let all = req(|i| {
            q(i);
            i.strategy = Some("fts5".into());
        });
        let RecallOutput::Memories { results, .. } = run(&u, &all).unwrap() else {
            panic!("memory-only expected");
        };
        assert_eq!(results.len(), 2);

        let by_entity = req(|i| {
            q(i);
            i.strategy = Some("fts5".into());
            i.entity = Some("rust".into());
        });
        let RecallOutput::Memories { results, .. } = run(&u, &by_entity).unwrap() else {
            panic!()
        };
        assert_eq!(results.len(), 1);
        let none = req(|i| {
            q(i);
            i.strategy = Some("fts5".into());
            i.category = Some("other".into());
        });
        let RecallOutput::Memories { results, .. } = run(&u, &none).unwrap() else {
            panic!()
        };
        assert!(results.is_empty());

        let future = req(|i| {
            q(i);
            i.strategy = Some("fts5".into());
            i.after = Some("2999-01-01T00:00:00Z".into());
        });
        let RecallOutput::Memories { results, .. } = run(&u, &future).unwrap() else {
            panic!()
        };
        assert!(results.is_empty(), "nothing was created after 2999");
        let past = req(|i| {
            q(i);
            i.strategy = Some("fts5".into());
            i.before = Some("2999-01-01T00:00:00Z".into());
        });
        let RecallOutput::Memories { results, .. } = run(&u, &past).unwrap() else {
            panic!()
        };
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn a_search_type_selects_the_unified_path() {
        let u = open_keyword();
        u.remember("rust memory safety", &[], None, None).unwrap();
        let r = req(|i| {
            i.search_type = Some("memory".into());
            i.strategy = Some("fts5".into());
        });
        assert!(matches!(run(&u, &r), Ok(RecallOutput::Unified(_))));
    }
}
