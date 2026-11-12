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
    ///
    /// Every memory hit in the output carries the rooms its memory belongs to
    /// (#1703). They are looked up once, after the ranking and the cut to
    /// `limit`, so the result order and scores are exactly those of the
    /// recall itself.
    pub fn run_recall(
        &self,
        req: &RecallRequest,
        defaults: RecallDefaults<'_>,
    ) -> Result<RecallOutput, Error> {
        let mut output = self.run_recall_unrooted(req, defaults)?;
        match &mut output {
            RecallOutput::Explained(items) => self.attach_rooms_explained(items)?,
            RecallOutput::Packed(pack) => self.attach_rooms_unified(&mut pack.selected)?,
            RecallOutput::Unified(results) => self.attach_rooms_unified(results)?,
            RecallOutput::Memories { results, .. } => self.attach_rooms(results)?,
        }
        Ok(output)
    }

    fn run_recall_unrooted(
        &self,
        req: &RecallRequest,
        defaults: RecallDefaults<'_>,
    ) -> Result<RecallOutput, Error> {
        let limit = req.limit;
        let tag_vec = req.tag_filter();
        let tags_filter = tag_vec.as_deref();
        let min_score = req.resolve_min_score(defaults.min_score, defaults.min_score_strict);
        // `strict` (on similarity, #1650) and `min_trust` (#1651) run before
        // the result is cut to `limit`, and never change `score`.
        let filter = req.result_filter(defaults.min_score_strict);
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
            return filter
                .fetch(
                    self,
                    limit,
                    |n| {
                        self.recall_explained(
                            &req.query,
                            n,
                            tags_filter,
                            namespace,
                            strategy,
                            min_score,
                        )
                    },
                    |f, e| f.keep_memory(&e.result),
                    |e| Some(e.result.memory.id.as_str()),
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
            let results = filter.fetch(
                self,
                limit,
                |n| {
                    self.recall_unified(
                        &req.query,
                        n,
                        tags_filter,
                        namespace,
                        min_score,
                        search_type,
                        entity_filter,
                        category_filter,
                        req.enrich,
                        strategy,
                    )
                },
                |f, r| f.keep_unified(r),
                |r| r.memory_id.as_deref(),
            )?;
            return Ok(RecallOutput::Packed(crate::pack_mode::pack_context(
                results,
                req.budget_chars_or_default(),
                &req.exclude_ids,
            )));
        }

        // Unified search (#531): when a search_type was given and there is no
        // point in time. Entity/category are pushed into the core candidate
        // loop (#663).
        if req.search_type_raw().is_some() && point_in_time.is_none() {
            let search_type: SearchType = req
                .search_type()
                .map_err(|e| Error::Validation(e.to_string()))?;
            let mut results = filter.fetch(
                self,
                limit,
                |n| {
                    self.recall_unified(
                        &req.query,
                        n,
                        tags_filter,
                        namespace,
                        min_score,
                        search_type,
                        entity_filter,
                        category_filter,
                        req.enrich,
                        strategy,
                    )
                },
                |f, r| f.keep_unified(r),
                |r| r.memory_id.as_deref(),
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
        let mut results = filter.fetch(
            self,
            limit,
            |n| {
                let fetch_limit = if needs_post_filter {
                    n.saturating_mul(3).max(n + 10)
                } else {
                    n
                };
                let mut found = if let Some(pit) = point_in_time {
                    // Time travel: the strategy does not apply to point-in-time recall.
                    self.recall_at_time(
                        &req.query,
                        n,
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
                    found.retain(|sr| {
                        entity_filter.is_none_or(|ent| {
                            sr.memory.metadata.get("entity").and_then(|v| v.as_str()) == Some(ent)
                        }) && category_filter.is_none_or(|cat| {
                            sr.memory.metadata.get("category").and_then(|v| v.as_str()) == Some(cat)
                        })
                    });
                    found.truncate(n);
                }
                Ok(found)
            },
            |f, r| f.keep_memory(r),
            |r| Some(r.memory.id.as_str()),
        )?;
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
    use crate::RecallInput;
    use crate::RecallStrategy;
    use crate::TrustTier;
    use crate::test_support::open_keyword;

    fn req(f: impl FnOnce(&mut RecallInput)) -> RecallRequest {
        let mut i = RecallInput {
            query: "rust memory safety".into(),
            ..RecallInput::default()
        };
        f(&mut i);
        RecallRequest::decode(i).unwrap()
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

    // ── #1703: the rooms of each hit ─────────────────────────────────────

    /// A memory in two rooms (one titled, one not) and one in no room, all
    /// matching `rust memory safety`.
    fn roomed_store() -> (Uteke, String) {
        let u = open_keyword();
        u.remember("rust memory safety in no room", &[], None, None)
            .unwrap();
        u.create_room("beta", Some("Beta room"), "default").unwrap();
        let id = u
            .remember_in_room(
                "rust memory safety in two rooms",
                &[],
                None,
                None,
                "fact",
                "alpha",
                "tester",
            )
            .unwrap()
            .id;
        assert!(u.add_memory_to_room(&id, "beta", "tester").unwrap());
        (u, id)
    }

    fn expected_rooms() -> Vec<crate::RoomRef> {
        vec![
            crate::RoomRef {
                room_id: "alpha".into(),
                room_name: "alpha".into(),
            },
            crate::RoomRef {
                room_id: "beta".into(),
                room_name: "Beta room".into(),
            },
        ]
    }

    #[test]
    fn memory_hits_carry_their_rooms_and_the_ranking_is_untouched() {
        let (u, id) = roomed_store();
        let rooms_of = |hits: &[SearchResult]| {
            hits.iter()
                .map(|h| (h.memory.id.clone(), h.rooms.clone()))
                .collect::<Vec<_>>()
        };
        let hits = memories(&u, &req(|i| i.strategy = Some("fts5".into())));
        assert_eq!(hits.len(), 2);
        for (hit_id, rooms) in rooms_of(&hits) {
            if hit_id == id {
                assert_eq!(rooms, expected_rooms());
            } else {
                assert!(rooms.is_empty(), "a memory in no room has none");
            }
        }
        // Same order and scores as the recall without the room lookup.
        let bare = u
            .recall_hybrid(
                "rust memory safety",
                10,
                None,
                None,
                RecallStrategy::Fts5,
                0.0,
            )
            .unwrap();
        let order = |v: &[SearchResult]| v.iter().map(|h| h.memory.id.clone()).collect::<Vec<_>>();
        assert_eq!(order(&hits), order(&bare));
        for (a, b) in hits.iter().zip(&bare) {
            assert_eq!(a.score, b.score);
        }
    }

    #[test]
    fn unified_packed_and_explained_hits_carry_their_rooms() {
        let (u, id) = roomed_store();
        let unified = req(|i| {
            i.search_type = Some("memory".into());
            i.strategy = Some("fts5".into());
        });
        let RecallOutput::Unified(results) = run(&u, &unified).unwrap() else {
            panic!("unified expected")
        };
        let hit = results
            .iter()
            .find(|r| r.memory_id.as_deref() == Some(id.as_str()))
            .unwrap();
        assert_eq!(hit.rooms, expected_rooms());
        assert!(
            results
                .iter()
                .filter(|r| r.memory_id.as_deref() != Some(id.as_str()))
                .all(|r| r.rooms.is_empty())
        );

        let packed = req(|i| {
            i.pack = true;
            i.strategy = Some("fts5".into());
        });
        let RecallOutput::Packed(pack) = run(&u, &packed).unwrap() else {
            panic!("pack expected")
        };
        let hit = pack
            .selected
            .iter()
            .find(|r| r.memory_id.as_deref() == Some(id.as_str()))
            .unwrap();
        assert_eq!(hit.rooms, expected_rooms());

        let explained = req(|i| {
            i.explain = true;
            i.strategy = Some("fts5".into());
        });
        let RecallOutput::Explained(items) = run(&u, &explained).unwrap() else {
            panic!("explain expected")
        };
        let hit = items.iter().find(|e| e.result.memory.id == id).unwrap();
        assert_eq!(hit.result.rooms, expected_rooms());
    }

    #[test]
    fn the_rooms_field_is_omitted_when_empty_and_listed_otherwise() {
        let (u, id) = roomed_store();
        let hits = memories(&u, &req(|i| i.strategy = Some("fts5".into())));
        for hit in &hits {
            let json = serde_json::to_value(hit).unwrap();
            if hit.memory.id == id {
                assert_eq!(
                    json["rooms"],
                    serde_json::json!([
                        {"room_id": "alpha", "room_name": "alpha"},
                        {"room_id": "beta", "room_name": "Beta room"},
                    ])
                );
            } else {
                assert!(json.get("rooms").is_none(), "no key for a roomless hit");
            }
        }
    }

    #[test]
    fn a_stale_cached_recall_never_reports_old_membership() {
        let (u, id) = roomed_store();
        let q = || req(|i| i.strategy = Some("fts5".into()));
        let first = memories(&u, &q());
        assert!(
            !first
                .iter()
                .find(|h| h.memory.id == id)
                .unwrap()
                .rooms
                .is_empty()
        );
        // Leave one room, then ask the same question again.
        u.create_room("gamma", Some("Gamma"), "default").unwrap();
        assert!(u.add_memory_to_room(&id, "gamma", "tester").unwrap());
        let second = memories(&u, &q());
        let rooms = &second.iter().find(|h| h.memory.id == id).unwrap().rooms;
        assert_eq!(rooms.len(), 3, "the new room shows at once: {rooms:?}");
    }

    // ── #1650: strict filters on similarity, #1651: min_trust ───────────

    const ON_TOPIC: &str = "quarterly revenue projections for the board meeting";

    /// One on-topic memory and four that share no word with the query.
    fn finance_store() -> Uteke {
        let u = open_keyword();
        u.remember(ON_TOPIC, &["finance"], None, None).unwrap();
        for (i, topic) in [
            "gardening tulips",
            "zebra savanna",
            "volcano basalt",
            "origami cranes",
        ]
        .iter()
        .enumerate()
        {
            u.remember(&format!("{topic} note {i}"), &["misc"], None, None)
                .unwrap();
        }
        u
    }

    fn memories(u: &Uteke, r: &RecallRequest) -> Vec<SearchResult> {
        match run(u, r).unwrap() {
            RecallOutput::Memories { results, .. } => results,
            other => panic!("memory-only expected, got {other:?}"),
        }
    }

    #[test]
    fn strict_under_fusion_returns_the_on_topic_memory_and_drops_the_rest() {
        let u = finance_store();
        let q = |i: &mut RecallInput| i.query = "quarterly revenue projections".into();
        let plain = memories(&u, &req(q));
        assert!(plain.len() > 1, "without strict every memory is returned");
        // The old strict compared the fused score to 0.5; fusion can never
        // reach it, so strict returned nothing at all.
        assert!(plain.iter().all(|r| r.score < 0.5));

        let strict = memories(
            &u,
            &req(|i| {
                q(i);
                i.strict = true;
            }),
        );
        assert_eq!(strict.len(), 1, "only the on-topic memory is confident");
        assert_eq!(strict[0].memory.content, ON_TOPIC);
        assert!(strict[0].similarity.unwrap() >= 0.5);
        assert!(strict[0].score < 0.5, "score keeps its meaning");
    }

    #[test]
    fn strict_drops_hits_without_a_similarity() {
        let u = finance_store();
        let q = |i: &mut RecallInput| {
            i.query = "quarterly revenue projections".into();
            i.strategy = Some("fts5".into());
        };
        let plain = memories(&u, &req(q));
        assert!(!plain.is_empty(), "the full-text leg finds the memory");
        assert!(plain.iter().all(|r| r.similarity.is_none()));
        let strict = memories(
            &u,
            &req(|i| {
                q(i);
                i.strict = true;
            }),
        );
        assert!(strict.is_empty(), "no similarity, no strict hit");
    }

    #[test]
    fn strict_never_changes_a_score_for_any_strategy() {
        for strategy in ["vector", "hybrid", "fts5", "fusion", "graph"] {
            let q = |i: &mut RecallInput| {
                i.query = "quarterly revenue projections".into();
                i.strategy = Some(strategy.into());
            };
            // A recall touches what it returns (hot boost), so each run gets a
            // fresh store.
            let plain = memories(&finance_store(), &req(q));
            let u = finance_store();
            let strict = memories(
                &u,
                &req(|i| {
                    q(i);
                    i.strict = true;
                    i.min_score = Some(0.0);
                }),
            );
            for r in &strict {
                let same = plain
                    .iter()
                    .find(|p| p.memory.content == r.memory.content)
                    .unwrap_or_else(|| panic!("{strategy}: strict added a hit"));
                assert_eq!(same.score, r.score, "{strategy}: score changed");
            }
        }
    }

    #[test]
    fn min_score_still_filters_on_score_with_and_without_strict() {
        let u = finance_store();
        let r = memories(
            &u,
            &req(|i| {
                i.query = "quarterly revenue projections".into();
                i.strategy = Some("vector".into());
                i.min_score = Some(0.5);
            }),
        );
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].memory.content, ON_TOPIC);
    }

    #[test]
    fn similarity_is_the_unboosted_cosine_and_null_for_fts_only_in_json() {
        let u = finance_store();
        let vec_hit = memories(
            &u,
            &req(|i| {
                i.query = "quarterly revenue projections".into();
                i.strategy = Some("vector".into());
            }),
        );
        let v = serde_json::to_value(&vec_hit[0]).unwrap();
        assert!(v["similarity"].as_f64().unwrap() > 0.5);
        let fts_hit = memories(
            &u,
            &req(|i| {
                i.query = "quarterly revenue projections".into();
                i.strategy = Some("fts5".into());
            }),
        );
        let v = serde_json::to_value(&fts_hit[0]).unwrap();
        assert!(v.get("similarity").is_some_and(|s| s.is_null()));
    }

    #[test]
    fn strict_applies_to_unified_and_pack_results_too() {
        let u = finance_store();
        let q = |i: &mut RecallInput| {
            i.query = "quarterly revenue projections".into();
            i.search_type = Some("memory".into());
            i.strict = true;
        };
        let RecallOutput::Unified(results) = run(&u, &req(q)).unwrap() else {
            panic!("unified expected");
        };
        assert_eq!(results.len(), 1);
        assert!(results[0].similarity.unwrap() >= 0.5);

        let RecallOutput::Packed(pack) = run(
            &u,
            &req(|i| {
                q(i);
                i.pack = true;
            }),
        )
        .unwrap() else {
            panic!("pack expected");
        };
        assert_eq!(pack.selected.len(), 1);
        let v = serde_json::to_value(&pack.selected[0]).unwrap();
        assert!(v["similarity"].as_f64().unwrap() >= 0.5);
    }

    #[test]
    fn a_filtered_recall_counts_only_the_rows_it_returns() {
        // The widened passes fetch discarded candidates; they must not be
        // counted as recalled (#1650 review).
        let u = finance_store();
        let r = req(|i| {
            i.query = "quarterly revenue projections".into();
            i.strict = true;
            i.limit = Some(1);
        });
        let got = memories(&u, &r);
        assert_eq!(got.len(), 1);
        let counts: Vec<(bool, u32)> = u
            .store()
            .list(None, None, 100, 0)
            .unwrap()
            .iter()
            .map(|m| (m.id == got[0].memory.id, m.recall_count))
            .collect();
        assert_eq!(counts.len(), 5);
        for (returned, count) in counts {
            assert_eq!(count, u32::from(returned));
        }
    }

    #[test]
    fn recall_related_keeps_the_seed_similarity_so_strict_does_not_drop_it() {
        let u = finance_store();
        let found = u
            .recall_related("quarterly revenue projections", 5, None, None, 0.0, 1)
            .unwrap();
        let seed = found
            .iter()
            .find(|r| r.memory.content == ON_TOPIC)
            .expect("the seed hit is returned");
        assert!(seed.similarity.unwrap() >= 0.5);
    }

    /// One memory per tier, all matching `tier words`.
    fn tier_store() -> Uteke {
        let u = open_keyword();
        let mk = |text: &str, author: &str, source_type: &str| {
            let id = u.remember(text, &[], None, None).unwrap();
            u.set_author_type(&id, author).unwrap();
            u.set_source(&id, None, source_type).unwrap();
        };
        mk("tier words human statement", "human", "user");
        mk("tier words agent note", "agent", "user");
        mk("tier words external page", "agent", "url");
        mk("tier words untrusted derived", "agent", "derived");
        u
    }

    fn tiers(u: &Uteke, min: Option<&str>) -> Vec<String> {
        let r = req(|i| {
            i.query = "tier words".into();
            i.strategy = Some("vector".into());
            i.min_trust = min.map(String::from);
        });
        let mut t: Vec<String> = memories(u, &r)
            .iter()
            .map(|r| TrustTier::of(&r.memory).as_str().to_string())
            .collect();
        t.sort();
        t
    }

    #[test]
    fn min_trust_returns_each_tiers_expected_set() {
        let u = tier_store();
        assert_eq!(tiers(&u, None).len(), 4, "no filter, nothing dropped");
        assert_eq!(tiers(&u, Some("untrusted")).len(), 4);
        assert_eq!(tiers(&u, Some("external")), ["agent", "external", "human"]);
        assert_eq!(tiers(&u, Some("agent")), ["agent", "human"]);
        assert_eq!(tiers(&u, Some("human")), ["human"]);
    }

    #[test]
    fn trust_tier_on_each_result_matches_the_classifier() {
        let u = tier_store();
        let r = req(|i| {
            i.query = "tier words".into();
            i.strategy = Some("vector".into());
        });
        for sr in memories(&u, &r) {
            let v = serde_json::to_value(&sr).unwrap();
            assert_eq!(v["trust_tier"], TrustTier::of(&sr.memory).as_str());
        }
        let RecallOutput::Unified(rows) = run(
            &u,
            &req(|i| {
                i.query = "tier words".into();
                i.search_type = Some("memory".into());
            }),
        )
        .unwrap() else {
            panic!()
        };
        assert!(rows.iter().all(|r| r.trust_tier.is_some()));
    }

    #[test]
    fn min_trust_runs_before_the_limit() {
        // Ten agent notes outrank one human note on the query, yet limit 1
        // with min_trust=human must still return the human note.
        let u = open_keyword();
        for i in 0..10 {
            u.remember(
                &format!("deploy checklist steps agent {i}"),
                &[],
                None,
                None,
            )
            .unwrap();
        }
        let id = u.remember("deploy checklist", &[], None, None).unwrap();
        u.set_author_type(&id, "human").unwrap();
        let r = req(|i| {
            i.query = "deploy checklist steps agent".into();
            i.strategy = Some("vector".into());
            i.limit = Some(1);
            i.min_trust = Some("human".into());
        });
        let got = memories(&u, &r);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].memory.id, id);
    }

    #[test]
    fn omitting_min_trust_changes_nothing_and_a_bad_value_is_rejected() {
        let u = tier_store();
        let base = req(|i| {
            i.query = "tier words".into();
            i.strategy = Some("vector".into());
        });
        let explicit_none = req(|i| {
            i.query = "tier words".into();
            i.strategy = Some("vector".into());
            i.min_trust = None;
        });
        assert_eq!(
            serde_json::to_string(&memories(&u, &base)).unwrap(),
            serde_json::to_string(&memories(&u, &explicit_none)).unwrap()
        );
        for bad in ["Human", "admin", ""] {
            let e = RecallRequest::decode(RecallInput {
                query: "q".into(),
                min_trust: Some(bad.into()),
                ..RecallInput::default()
            })
            .unwrap_err();
            assert!(e.to_string().starts_with("Invalid min_trust"), "{bad}");
        }
    }
    #[test]
    fn an_entity_filter_over_fetches_by_the_limit_plus_ten() {
        // limit 1 with a post-filter fetches max(3, 1 + 10) = 11 candidates:
        // the one memory with the entity ranks 11th of 11, so a window of 10
        // (limit * 10) misses it and one below 1 + 10 cannot even be computed.
        let u = open_keyword();
        for i in 0..10 {
            u.remember(&format!("alpha alpha alpha item{i}"), &[], None, None)
                .unwrap();
        }
        u.remember(
            "alpha once among many other unrelated words here",
            &[],
            Some(serde_json::json!({"entity": "needle"})),
            None,
        )
        .unwrap();
        let r = req(|i| {
            i.query = "alpha".into();
            i.strategy = Some("fts5".into());
            i.limit = Some(1);
            i.entity = Some("needle".into());
        });
        let RecallOutput::Memories { results, .. } = run(&u, &r).unwrap() else {
            panic!("memory-only expected");
        };
        assert_eq!(results.len(), 1, "{results:?}");
        assert_eq!(
            results[0]
                .memory
                .metadata
                .get("entity")
                .and_then(|v| v.as_str()),
            Some("needle")
        );
    }
    fn remember_as(u: &Uteke, content: &str, author_type: &str, source_type: &str) {
        let r = crate::remember_request::RememberRequest {
            content: content.to_string(),
            tags: vec![],
            namespace: None,
            memory_type: Some("fact".to_string()),
            metadata: None,
            detect_contradiction: false,
            source: None,
            source_type: Some(source_type.to_string()),
            author_type: Some(author_type.to_string()),
            room: None,
            author: None,
            validity: None,
        };
        u.run_remember(&r).unwrap();
    }

    #[test]
    fn min_trust_keeps_the_given_tier_and_the_ones_above() {
        let u = open_keyword();
        remember_as(&u, "tier probe human", "human", "user");
        remember_as(&u, "tier probe agent", "agent", "user");
        remember_as(&u, "tier probe external", "agent", "url");
        remember_as(&u, "tier probe derived", "agent", "derived");
        let r = req(|i| {
            i.query = "tier probe".into();
            i.search_type = Some("memory".into());
            i.strategy = Some("fts5".into());
            i.min_trust = Some("agent".into());
        });
        let RecallOutput::Unified(results) = run(&u, &r).unwrap() else {
            panic!("unified expected");
        };
        let mut tiers: Vec<_> = results.iter().map(|r| r.trust_tier).collect();
        tiers.sort();
        assert_eq!(tiers, vec![Some(TrustTier::Agent), Some(TrustTier::Human)]);
    }
}
