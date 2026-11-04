#![allow(clippy::too_many_arguments)]
//! Recall and Search commands — semantic and keyword search.

use crate::cli::Cli;
use crate::config::Config;
use crate::output;
use uteke_core::{RecallStrategy, SearchType, Uteke};

/// Run the shared recall decode (#1439) over the arguments the local path has,
/// and return the limit to use plus a warning when it was capped.
fn decode_local(
    query: &str,
    limit: usize,
    tags: &[String],
    at: Option<&str>,
) -> Result<(usize, Option<String>), String> {
    let typed = uteke_core::RecallRequest::decode(uteke_core::RecallInput {
        query: query.to_string(),
        limit: Some(limit),
        tags: Some(tags.to_vec()),
        at: at.map(str::to_string),
        ..Default::default()
    })
    .map_err(|e| e.to_string())?;
    let warning = (typed.limit < limit).then(|| {
        format!(
            "Warning: --limit {limit} is capped at {} (the same cap as the server and MCP).",
            typed.limit
        )
    });
    Ok((typed.limit, warning))
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_recall(
    cli: &Cli,
    uteke: &mut Uteke,
    ns: Option<&str>,
    query: &str,
    limit: usize,
    tags: &[String],
    entity: Option<&str>,
    category: Option<&str>,
    min: Option<f32>,
    strict: bool,
    strategy: Option<&str>,
    config: &Config,
    related: bool,
    depth: usize,
    context: bool,
    at: Option<&str>,
    content_format: &str,
    where_filter: Option<&str>,
    salience: Option<bool>,
    recency: Option<bool>,
    search_type: Option<&str>,
    enrich: bool,
    explain: bool,
    pack: bool,
    budget: usize,
    exclude_ids: &[String],
) -> Result<(), String> {
    // The same checks as HTTP and MCP (#1439): a blank query is refused, `--limit`
    // is capped at 100 (with a warning), and `--at` must be RFC3339. Decoding only
    // validates; it never changes which core function runs below.
    let limit = {
        let (capped, warning) = decode_local(query, limit, tags, at)?;
        if let Some(w) = warning {
            eprintln!("{w}");
        }
        capped
    };

    // Resolve search type: --type flag > default (All = unified)
    let resolved_search_type = match search_type {
        Some("memory") => SearchType::Memory,
        Some("doc") => SearchType::Document,
        Some("all") | None => SearchType::All,
        Some(other) => {
            return Err(format!(
                "Invalid --type: '{other}'. Use 'all', 'memory', or 'doc'."
            ));
        }
    };

    // When --type is explicitly set (not default unified), route to recall_unified.
    // When unified (default), use existing recall path for backward compat with
    // --strategy, --at, --related, --entity, --category, --where flags
    // which only apply to memory recall.
    // --salience/--recency <true|false> work on both paths (#721).
    let use_unified = match resolved_search_type {
        SearchType::All => {
            // Use unified only when no memory-only flags are active.
            // Memory-only flags: --at, --related, --entity, --category, --where
            at.is_none()
                && !related
                && entity.is_none()
                && category.is_none()
                && where_filter.is_none()
        }
        SearchType::Memory | SearchType::Document => true,
    };

    // Score floor: --min > config min_score > 0.0. `--strict` does not touch
    // it: it filters on similarity instead (#1650), before the result is cut to
    // --limit.
    let min_score = min.unwrap_or(config.engine.recall.min_score as f32);
    let filter = uteke_core::RecallRequest::decode(uteke_core::RecallInput {
        query: query.to_string(),
        strict,
        ..Default::default()
    })
    .map_err(|e| e.to_string())?
    .result_filter(Some(config.engine.recall.min_score_strict));

    tracing::info!(
        "Recalling: {query} (limit: {limit}, min_score: {min_score}, type: {:?})",
        resolved_search_type
    );
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tags_filter = if tag_refs.is_empty() {
        None
    } else {
        Some(tag_refs.as_slice())
    };

    // Resolve strategy: --strategy flag > config [recall].default_strategy
    // > built-in default ("hybrid"). Unknown values fall back to vector with
    // a warning so a typo never silently changes recall semantics.
    let strategy_name = strategy.unwrap_or(&config.engine.recall.default_strategy);
    let resolved_strategy = match RecallStrategy::from_str_opt(strategy_name) {
        Some(s) => s,
        None => {
            return Err(format!(
                "Invalid strategy '{strategy_name}'. Valid options: vector, fts5, hybrid, graph."
            ));
        }
    };

    // #352/#721: dual-axis salience/recency boost.
    // Tri-state via Option<bool>: None (absent) = use default (0.1),
    // Some(true) = use config weight (0.15), Some(false) = 0.0.
    uteke.set_salience_recency_config(uteke_core::SalienceRecencyConfig {
        salience_weight: match salience {
            Some(true) => config.engine.recall.salience_weight, // explicit --salience
            Some(false) => 0.0,                                 // explicit --salience false
            None => uteke_core::SalienceRecencyConfig::default().salience_weight, // default 0.1
        },
        recency_weight: match recency {
            Some(true) => config.engine.recall.recency_weight, // explicit --recency
            Some(false) => 0.0,                                // explicit --recency false
            None => uteke_core::SalienceRecencyConfig::default().recency_weight, // default 0.1
        },
    });

    // --explain (#1160): memory-recall explanation mode. Runs the real
    // strategy pipeline with per-stage instrumentation and prints the
    // signals behind each result. Mutually exclusive with the unified
    // search surface (docs/entities) — explanation is memory-only. An
    // omitted --type (the default invocation) is accepted and treated as
    // memory recall; only explicit --type doc is rejected (code-scanning
    // fix: None maps to SearchType::All, which the old guard rejected).
    if explain {
        if resolved_search_type == SearchType::Document {
            return Err(
                "--explain works on memory recall; --type doc is not supported.".to_string(),
            );
        }
        if at.is_some() {
            return Err("--explain and --at cannot be used together".to_string());
        }
        if related {
            return Err("--explain and --related cannot be used together".to_string());
        }
        if entity.is_some() {
            return Err("--explain and --entity cannot be used together".to_string());
        }
        if category.is_some() {
            return Err("--explain and --category cannot be used together".to_string());
        }
        if where_filter.is_some() {
            return Err("--explain and --where cannot be used together".to_string());
        }
        if context {
            return Err("--explain and --context cannot be used together".to_string());
        }
        if enrich {
            return Err("--explain and --enrich cannot be used together".to_string());
        }
        let explained = filter
            .fetch(
                uteke,
                limit,
                |n| uteke.recall_explained(query, n, tags_filter, ns, resolved_strategy, min_score),
                |f, e| f.keep_memory(&e.result),
                |e| Some(e.result.memory.id.as_str()),
            )
            .map_err(|e| format!("Failed to recall: {e}"))?;
        uteke.reset_salience_recency_config();

        if explained.is_empty() {
            if cli.json {
                output::print_json(&explained);
            } else {
                println!("No matching memories found.");
            }
            return Ok(());
        }

        if cli.json {
            output::print_json(&explained);
        } else {
            for (i, er) in explained.iter().enumerate() {
                let ex = &er.explanation;
                println!("{}. {}", i + 1, er.result.memory.content);
                println!("   Final score: {:.4} ({})", ex.final_score, ex.strategy);
                if let Some(sim) = ex.vector_similarity {
                    println!("   Vector similarity: {:.4}", sim);
                }
                if let Some(rank) = ex.vector_rank {
                    println!("   Vector rank:       #{}", rank);
                }
                if let Some(rank) = ex.fts_rank {
                    println!("   FTS rank:          #{}", rank);
                }
                if let Some(rrf) = ex.rrf_score {
                    println!("   RRF score:         {:.4}", rrf);
                }
                if let Some(c) = ex.fusion_vector_contribution {
                    println!("   Fusion vector contrib: {:.4}", c);
                }
                if let Some(c) = ex.fusion_hybrid_contribution {
                    println!("   Fusion hybrid contrib: {:.4}", c);
                }
                if let Some(b) = ex.jaccard_boost {
                    println!("   Jaccard boost:     +{:.4}", b);
                }
                if let Some(b) = ex.salience_boost {
                    println!("   Salience boost:    +{:.4}", b);
                }
                if let Some(b) = ex.recency_boost {
                    println!("   Recency boost:     +{:.4}", b);
                }
                if let Some(b) = ex.graph_boost {
                    println!("   Graph boost:       +{:.4}", b);
                }
                println!("   Base score:        {:.4}", ex.base_score);
                println!("   ID: {}", er.result.memory.id);
                println!();
            }
        }
        return Ok(());
    }

    // Budgeted context pack (#1281 Phase 1): unified recall + greedy
    // character-budget fill. Deterministic, LLM-free, rank-order preserving.
    // Entity/category filters are honoured by the core post-filter; memory
    // filters that have no packed-path equivalent are rejected loudly
    // instead of being silently dropped.
    if pack {
        if at.is_some() || related || where_filter.is_some() {
            return Err(
                "--pack does not support --at/--related/--where; rerun without them".to_string(),
            );
        }
        let packed = filter
            .fetch(
                uteke,
                limit,
                |n| {
                    uteke.recall_unified(
                        query,
                        n,
                        tags_filter,
                        ns,
                        min_score,
                        resolved_search_type,
                        entity,
                        category,
                        enrich,
                        resolved_strategy,
                    )
                },
                |f, r| f.keep_unified(r),
                |r| r.memory_id.as_deref(),
            )
            .map_err(|e| format!("Failed to recall: {e}"));
        let pack_result = match packed {
            Ok(results) => uteke_core::pack_mode::pack_context(results, budget, exclude_ids),
            Err(e) => {
                uteke.reset_salience_recency_config();
                return Err(e);
            }
        };

        uteke.reset_salience_recency_config();

        if cli.json {
            output::print_json(&pack_result);
        } else {
            output::print_pack_human(&pack_result);
        }
        return Ok(());
    }

    if use_unified {
        // Unified search path (#531)
        let unified_results = filter
            .fetch(
                uteke,
                limit,
                |n| {
                    uteke.recall_unified(
                        query,
                        n,
                        tags_filter,
                        ns,
                        min_score,
                        resolved_search_type,
                        None,
                        None,
                        enrich,
                        resolved_strategy,
                    )
                },
                |f, r| f.keep_unified(r),
                |r| r.memory_id.as_deref(),
            )
            .map_err(|e| format!("Failed to recall: {e}"))?;

        uteke.reset_salience_recency_config();

        if unified_results.is_empty() {
            if cli.json {
                output::print_json(&unified_results);
            } else if min_score > 0.0 {
                println!("No matching results found.");
                println!("(min_score threshold: {:.2})", min_score);
            } else if context {
                println!("{}", format_context(query, &[]));
            } else {
                println!("No matching results found.");
            }
            return Ok(());
        }

        if context {
            // The same layout as the memory-only path below (#1642).
            let rows: Vec<ContextRow<'_>> = unified_results
                .iter()
                .map(|r| ContextRow {
                    score: r.score,
                    content: &r.content,
                    tags: &r.tags,
                    pinned: r.pinned == Some(true),
                    importance: r.importance.unwrap_or(0.0),
                    doc_title: r.doc_title.as_deref(),
                })
                .collect();
            println!("{}", format_context(query, &rows));
        } else if cli.json {
            output::print_json(&unified_results);
        } else {
            output::print_unified_human(&unified_results);
        }
        return Ok(());
    }

    // Existing memory-only recall path (backward compatible)
    // Wrap recall in a closure so reset always runs, even on error paths
    // (CodeCora #387: boost config must not leak on early return).
    let recall_result: Result<_, String> = (|| {
        // Time-travel mode: parse --at as RFC3339 and use recall_at_time
        let results = if let Some(at_str) = at {
            let point_in_time = chrono::DateTime::parse_from_rfc3339(at_str)
                .map_err(|e| {
                    format!(
                        "Invalid --at timestamp: {e}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                    )
                })?
                .with_timezone(&chrono::Utc);
            if related {
                return Err("--at and --related cannot be used together".into());
            }
            filter
                .fetch(
                    uteke,
                    limit,
                    |n| {
                        uteke.recall_at_time(
                            query,
                            n,
                            tags_filter,
                            ns,
                            point_in_time,
                            min_score,
                            None,
                            None,
                        )
                    },
                    |f, r| f.keep_memory(r),
                    |r| Some(r.memory.id.as_str()),
                )
                .map_err(|e| format!("Failed to recall at time: {e}"))?
        } else if related {
            filter
                .fetch(
                    uteke,
                    limit,
                    |n| uteke.recall_related(query, n, tags_filter, ns, min_score, depth),
                    |f, r| f.keep_memory(r),
                    |r| Some(r.memory.id.as_str()),
                )
                .map_err(|e| format!("Failed to recall: {e}"))?
        } else {
            filter
                .fetch(
                    uteke,
                    limit,
                    |n| {
                        uteke.recall_hybrid(query, n, tags_filter, ns, resolved_strategy, min_score)
                    },
                    |f, r| f.keep_memory(r),
                    |r| Some(r.memory.id.as_str()),
                )
                .map_err(|e| format!("Failed to recall: {e}"))?
        };
        Ok(results)
    })();

    // Reset per-query boost config so later recalls on the same Uteke instance
    // aren't affected — runs on EVERY path (CodeCora #387).
    uteke.reset_salience_recency_config();

    let results = recall_result?;

    // Post-filter by entity/category metadata
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|sr| {
            if let Some(ent) = entity {
                let matches = sr
                    .memory
                    .metadata
                    .get("entity")
                    .and_then(|v| v.as_str())
                    .is_some_and(|e| e == ent);
                if !matches {
                    return false;
                }
            }
            if let Some(cat) = category {
                let matches = sr
                    .memory
                    .metadata
                    .get("category")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| c == cat);
                if !matches {
                    return false;
                }
            }
            true
        })
        .collect();

    // JSON field-level filtering: --where key=value
    let filtered: Vec<_> = if let Some(where_clause) = where_filter {
        let (key, value) = parse_where(where_clause)?;
        filtered
            .into_iter()
            .filter(|sr| {
                if sr.memory.content_type != "json" {
                    return false;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sr.memory.content) {
                    if let Some(obj) = v.as_object() {
                        let field = match obj.get(&key) {
                            Some(f) => f,
                            None => return false,
                        };
                        // Match string, number, or bool values
                        let actual = match field {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            serde_json::Value::Null => "null".to_string(),
                            _ => return false, // arrays/objects not supported in key=value filter
                        };
                        return actual == value;
                    }
                }
                false
            })
            .collect()
    } else {
        filtered
    };

    if filtered.is_empty() {
        if cli.json {
            // Always output a JSON array for machine consumers (cora-cli,
            // scripts, MCP). A bare [] is easier to parse than {"results":[]}.
            output::print_json(&filtered);
        } else if min_score > 0.0 {
            println!("No matching memories found.");
            println!("(min_score threshold: {:.2})", min_score);
        } else if context {
            println!("{}", format_context(query, &[]));
        } else {
            println!("No matching memories found.");
        }
        return Ok(());
    }

    if context {
        // Context mode: formatted for AI prompt injection
        let rows: Vec<ContextRow<'_>> = filtered
            .iter()
            .map(|sr| ContextRow {
                score: sr.score,
                content: &sr.memory.content,
                tags: &sr.memory.tags,
                pinned: sr.memory.pinned,
                importance: sr.memory.importance,
                doc_title: None,
            })
            .collect();
        println!("{}", format_context(query, &rows));
    } else if cli.json {
        // Apply content_format: pretty-print JSON content when requested
        if content_format == "json" {
            let mut formatted: Vec<_> = filtered.into_iter().collect();
            for sr in &mut formatted {
                if sr.memory.content_type == "json" {
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&sr.memory.content) {
                        sr.memory.content = serde_json::to_string_pretty(&v)
                            .unwrap_or_else(|_| sr.memory.content.clone());
                    }
                }
            }
            output::print_json(&formatted);
        } else {
            output::print_json(&filtered);
        }
    } else {
        output::print_recall_human(&filtered);
    }
    Ok(())
}

/// One recall hit as `--context` prints it, from either the memory-only or the
/// unified path.
struct ContextRow<'a> {
    score: f32,
    content: &'a str,
    tags: &'a [String],
    pinned: bool,
    importance: f64,
    /// Set for a document hit, printed after the excerpt.
    doc_title: Option<&'a str>,
}

/// The `--context` layout for AI prompt injection, or its empty-result line.
fn format_context(query: &str, rows: &[ContextRow<'_>]) -> String {
    if rows.is_empty() {
        return format!("[No relevant memories found for: {query}]");
    }
    let avg_score: f32 = rows.iter().map(|r| r.score).sum::<f32>() / rows.len() as f32;
    let mut lines = vec![format!(
        "[Relevant Memories ({} results, {:.2} avg score)]",
        rows.len(),
        avg_score
    )];
    for (i, r) in rows.iter().enumerate() {
        let tags = if r.tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", r.tags.join(", "))
        };
        let doc = r
            .doc_title
            .map(|t| format!(" (doc: {t})"))
            .unwrap_or_default();
        let importance = if r.pinned {
            " \u{2605}" // ★
        } else if r.importance > 0.7 {
            " \u{2191}" // ↑
        } else {
            ""
        };
        lines.push(format!(
            "{}. [{:.2}] {}{doc}{tags}{importance}",
            i + 1,
            r.score,
            r.content
        ));
    }
    lines.join("\n")
}

pub(crate) fn run_search(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    query: &str,
    limit: usize,
    tags: &[String],
) -> Result<(), String> {
    tracing::info!("Searching: {query} (limit: {limit})");
    let tag_refs: Vec<&str> = tags.iter().map(|s| s.as_str()).collect();
    let tags_filter = if tag_refs.is_empty() {
        None
    } else {
        Some(tag_refs.as_slice())
    };
    let results = uteke
        .search(query, limit, tags_filter, ns)
        .map_err(|e| format!("Failed to search: {e}"))?;
    if cli.json {
        output::print_json(&results);
    } else {
        output::print_search_human(&results);
    }
    Ok(())
}

/// Parse a `--where` expression in `key=value` format.
fn parse_where(s: &str) -> Result<(String, String), String> {
    let parts: Vec<&str> = s.splitn(2, '=').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid --where expression: '{s}'. Use key=value format."
        ));
    }
    Ok((parts[0].trim().to_string(), parts[1].trim().to_string()))
}

#[cfg(test)]
mod decode_tests {
    use super::decode_local;

    #[test]
    fn a_blank_query_is_refused_like_on_the_server() {
        for q in ["", "  \t"] {
            assert_eq!(
                decode_local(q, 5, &[], None).unwrap_err(),
                "Query must not be empty or whitespace-only"
            );
        }
    }

    #[test]
    fn a_limit_over_the_cap_is_capped_with_a_warning_and_a_small_one_is_untouched() {
        let (l, w) = decode_local("q", 5000, &[], None).unwrap();
        assert_eq!(l, 100);
        assert!(w.unwrap().contains("capped at 100"));
        // The benchmark harness uses --limit 50: unchanged, no warning.
        let (l, w) = decode_local("q", 50, &[], None).unwrap();
        assert_eq!((l, w), (50, None));
    }

    #[test]
    fn an_invalid_at_is_refused_with_the_shared_text() {
        let e = decode_local("q", 5, &[], Some("yesterday")).unwrap_err();
        assert!(e.starts_with("Invalid 'at' timestamp: yesterday"), "{e}");
        assert!(decode_local("q", 5, &[], Some("2026-06-01T12:00:00Z")).is_ok());
    }
}

#[cfg(test)]
mod context_tests {
    use super::{ContextRow, format_context};

    #[test]
    fn no_rows_print_the_empty_line() {
        assert_eq!(
            format_context("q", &[]),
            "[No relevant memories found for: q]"
        );
    }

    #[test]
    fn rows_print_score_content_doc_tags_and_marker() {
        let tags = vec!["a".to_string(), "b".to_string()];
        let rows = [
            ContextRow {
                score: 0.9,
                content: "pinned fact",
                tags: &tags,
                pinned: true,
                importance: 0.1,
                doc_title: None,
            },
            ContextRow {
                score: 0.5,
                content: "an excerpt",
                tags: &[],
                pinned: false,
                importance: 0.8,
                doc_title: Some("Guide"),
            },
            ContextRow {
                score: 0.4,
                content: "plain",
                tags: &[],
                pinned: false,
                importance: 0.7,
                doc_title: None,
            },
        ];
        assert_eq!(
            format_context("q", &rows),
            "[Relevant Memories (3 results, 0.60 avg score)]\n\
             1. [0.90] pinned fact [a, b] \u{2605}\n\
             2. [0.50] an excerpt (doc: Guide) \u{2191}\n\
             3. [0.40] plain"
        );
    }
}
