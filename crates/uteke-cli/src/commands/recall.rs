//! Recall and Search commands — semantic and keyword search.
//!
//! `uteke recall` builds a [`uteke_core::RecallRequest`] from its flags and
//! runs it through [`Uteke::run_recall`], the same entry point as `POST /recall`
//! and the MCP tool, so a decision (which path, which guard, which floor) is
//! made once, in core. What stays here is the part only the CLI owns: reading
//! the flags, and printing the result.

use crate::cli::{Cli, Commands};
use crate::config::Config;
use crate::output;
use uteke_core::recall_exec::{RecallDefaults, RecallOutput};
use uteke_core::{RecallInput, RecallRequest, Uteke};

/// The `uteke recall` flags, borrowed from the parsed command.
pub(crate) struct RecallArgs<'a> {
    pub query: &'a str,
    pub limit: usize,
    pub tags: &'a [String],
    pub entity: Option<&'a str>,
    pub category: Option<&'a str>,
    pub min: Option<f32>,
    pub strict: bool,
    pub min_trust: Option<&'a str>,
    pub strategy: Option<&'a str>,
    pub salience: Option<bool>,
    pub recency: Option<bool>,
    pub related: bool,
    pub depth: usize,
    pub context: bool,
    pub at: Option<&'a str>,
    pub content_format: &'a str,
    pub where_filter: Option<&'a str>,
    pub search_type: Option<&'a str>,
    pub enrich: bool,
    pub explain: bool,
    pub pack: bool,
    pub budget: usize,
    pub exclude_ids: &'a [String],
}

impl<'a> RecallArgs<'a> {
    /// The flags of a `recall` command; `None` for any other command.
    pub(crate) fn from_command(command: &'a Commands) -> Option<Self> {
        let Commands::Recall {
            query,
            limit,
            tags,
            entity,
            category,
            min,
            strict,
            min_trust,
            strategy,
            salience,
            recency,
            related,
            depth,
            context,
            at,
            history: _,
            content_format,
            r#where,
            r#type,
            enrich,
            explain,
            pack,
            budget,
            exclude_ids,
        } = command
        else {
            return None;
        };
        Some(Self {
            query,
            limit: *limit,
            tags,
            entity: entity.as_deref(),
            category: category.as_deref(),
            min: *min,
            strict: *strict,
            min_trust: min_trust.as_deref(),
            strategy: strategy.as_deref(),
            salience: *salience,
            recency: *recency,
            related: *related,
            depth: *depth,
            context: *context,
            at: at.as_deref(),
            content_format,
            where_filter: r#where.as_deref(),
            search_type: r#type.as_deref(),
            enrich: *enrich,
            explain: *explain,
            pack: *pack,
            budget: *budget,
            exclude_ids,
        })
    }

    /// Flags that only mean something on the memory-only path.
    fn memory_only(&self) -> bool {
        self.at.is_some()
            || self.related
            || self.entity.is_some()
            || self.category.is_some()
            || self.where_filter.is_some()
    }

    /// The raw request. `--type` absent or `all` is the default unified search
    /// (memories and documents) unless a memory-only flag or `--explain` is
    /// given, which keep the memory-only path; `memory` and `doc` are passed
    /// on as they are.
    pub(crate) fn input(&self, ns: Option<&str>) -> RecallInput {
        let search_type = match self.search_type {
            Some("all") | None if self.memory_only() || self.explain => None,
            Some("all") | None => Some("all".to_string()),
            Some(other) => Some(other.to_string()),
        };
        RecallInput {
            query: self.query.to_string(),
            limit: Some(self.limit),
            tags: Some(self.tags.to_vec()),
            namespace: ns.map(str::to_string),
            entity: self.entity.map(str::to_string),
            category: self.category.map(str::to_string),
            min_score: self.min,
            strict: self.strict,
            min_trust: self.min_trust.map(str::to_string),
            at: self.at.map(str::to_string),
            search_type,
            enrich: self.enrich,
            pack: self.pack,
            budget_chars: Some(self.budget),
            exclude_ids: self.exclude_ids.to_vec(),
            strategy: self.strategy.map(str::to_string),
            explain: self.explain,
            ..RecallInput::default()
        }
    }

    /// The typed request, plus a warning when `--limit` was capped. Decoding
    /// applies the checks every surface shares (#1439): a blank query is
    /// refused, `--limit` is capped at 100, `--at` must be RFC3339.
    pub(crate) fn request(
        &self,
        ns: Option<&str>,
    ) -> Result<(RecallRequest, Option<String>), String> {
        let mut req = RecallRequest::decode(self.input(ns))
            .map_err(|e| e.to_string())?
            .with_salience_recency(self.salience, self.recency);
        if self.related {
            req = req.with_related(self.depth);
        }
        if let Some(clause) = self.where_filter {
            let (key, value) = parse_where(clause)?;
            req = req.with_where(key, value);
        }
        let warning = (req.limit < self.limit).then(|| {
            format!(
                "Warning: --limit {} is capped at {} (the same cap as the server and MCP).",
                self.limit, req.limit
            )
        });
        Ok((req, warning))
    }

    /// `--explain` is a ranking dump, not a context block.
    pub(crate) fn check_rendering(&self) -> Result<(), String> {
        if self.explain && self.context {
            return Err("--explain and --context cannot be used together".to_string());
        }
        Ok(())
    }
}

/// The `[recall]` values a request falls back to.
fn defaults(config: &Config) -> RecallDefaults<'_> {
    let r = &config.engine.recall;
    RecallDefaults {
        min_score: Some(r.min_score),
        min_score_strict: Some(r.min_score_strict),
        default_strategy: Some(r.default_strategy.as_str()),
        salience_weight: Some(r.salience_weight),
        recency_weight: Some(r.recency_weight),
    }
}

pub(crate) fn run_recall(
    cli: &Cli,
    uteke: &Uteke,
    ns: Option<&str>,
    args: &RecallArgs<'_>,
    config: &Config,
) -> Result<(), String> {
    args.check_rendering()?;
    let (req, warning) = args.request(ns)?;
    if let Some(w) = warning {
        eprintln!("{w}");
    }
    tracing::info!(
        "Recalling: {} (limit: {}, type: {:?})",
        req.query,
        req.limit,
        req.search_type_raw()
    );
    let output = uteke
        .run_recall(&req, defaults(config))
        .map_err(|e| match e {
            // A guard or an unknown strategy/type: the text core gives, with the
            // flag's name for the field's.
            uteke_core::Error::Validation(msg) => msg.replace("search_type", "--type"),
            other => format!("Failed to recall: {other}"),
        })?;
    let min_score = req.resolve_min_score(Some(config.engine.recall.min_score), None);
    render(cli, args, output, min_score);
    Ok(())
}

/// Print a recall result: JSON for `--json`, `--context` for prompt injection,
/// else the human listing. The same for a local store and for the server.
pub(crate) fn render(cli: &Cli, args: &RecallArgs<'_>, output: RecallOutput, min_score: f32) {
    match output {
        RecallOutput::Explained(explained) => render_explained(cli, &explained),
        RecallOutput::Packed(pack) => {
            if cli.json {
                output::print_json(&pack);
            } else {
                output::print_pack_human(&pack);
            }
        }
        RecallOutput::Unified(results) => {
            if results.is_empty() {
                if cli.json {
                    output::print_json(&results);
                } else if min_score > 0.0 {
                    println!("No matching results found.");
                    println!("(min_score threshold: {:.2})", min_score);
                } else if args.context {
                    println!("{}", format_context(args.query, &[]));
                } else {
                    println!("No matching results found.");
                }
            } else if args.context {
                // The same layout as the memory-only path (#1642).
                let rows: Vec<ContextRow<'_>> = results
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
                println!("{}", format_context(args.query, &rows));
            } else if cli.json {
                output::print_json(&results);
            } else {
                output::print_unified_human(&results);
            }
        }
        RecallOutput::Memories { mut results, .. } => {
            if results.is_empty() {
                if cli.json {
                    // Always a JSON array for machine consumers (cora-cli,
                    // scripts, MCP): a bare [] is easier to parse than
                    // {"results":[]}.
                    output::print_json(&results);
                } else if min_score > 0.0 {
                    println!("No matching memories found.");
                    println!("(min_score threshold: {:.2})", min_score);
                } else if args.context {
                    println!("{}", format_context(args.query, &[]));
                } else {
                    println!("No matching memories found.");
                }
            } else if args.context {
                let rows: Vec<ContextRow<'_>> = results
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
                println!("{}", format_context(args.query, &rows));
            } else if cli.json {
                // `--content-format json` pretty-prints JSON content.
                if args.content_format == "json" {
                    for sr in &mut results {
                        if sr.memory.content_type == "json" {
                            if let Ok(v) =
                                serde_json::from_str::<serde_json::Value>(&sr.memory.content)
                            {
                                sr.memory.content = serde_json::to_string_pretty(&v)
                                    .unwrap_or_else(|_| sr.memory.content.clone());
                            }
                        }
                    }
                }
                output::print_json(&results);
            } else {
                output::print_recall_human(&results);
            }
        }
    }
}

fn render_explained(cli: &Cli, explained: &[uteke_core::recall_explain::ExplainedRecall]) {
    if cli.json {
        output::print_json(&explained);
    } else if explained.is_empty() {
        println!("No matching memories found.");
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
mod request_tests {
    use super::RecallArgs;
    use crate::cli::Commands;
    use clap::Parser;

    fn args_of(argv: &[&str]) -> Commands {
        let mut v = vec!["uteke"];
        v.extend_from_slice(argv);
        crate::cli::Cli::parse_from(v).command
    }

    fn input(argv: &[&str]) -> uteke_core::RecallInput {
        let cmd = args_of(argv);
        RecallArgs::from_command(&cmd).unwrap().input(Some("ns"))
    }

    #[test]
    fn a_plain_recall_searches_memories_and_documents() {
        assert_eq!(input(&["recall", "q"]).search_type.as_deref(), Some("all"));
        assert_eq!(
            input(&["recall", "q", "--type", "all"])
                .search_type
                .as_deref(),
            Some("all")
        );
    }

    #[test]
    fn a_memory_only_flag_or_explain_keeps_the_memory_path() {
        for extra in [
            &["--at", "2026-06-01T12:00:00Z"][..],
            &["--entity", "e"],
            &["--category", "c"],
            &["--related"],
            &["--where", "k=v"],
            &["--explain"],
        ] {
            let mut argv = vec!["recall", "q"];
            argv.extend_from_slice(extra);
            assert_eq!(input(&argv).search_type, None, "{extra:?}");
        }
    }

    #[test]
    fn an_explicit_memory_or_doc_type_is_passed_on() {
        for t in ["memory", "doc", "bogus"] {
            assert_eq!(
                input(&["recall", "q", "--type", t, "--entity", "e"])
                    .search_type
                    .as_deref(),
                Some(t)
            );
        }
    }

    #[test]
    fn the_flags_reach_the_request() {
        let cmd = args_of(&[
            "recall",
            "q",
            "--strategy",
            "fts5",
            "--salience",
            "false",
            "--recency",
            "true",
            "--related",
            "--depth",
            "3",
            "--where",
            " k = v ",
            "--limit",
            "5000",
        ]);
        let args = RecallArgs::from_command(&cmd).unwrap();
        let (req, warning) = args.request(Some("ns")).unwrap();
        assert_eq!(req.limit, 100);
        assert!(warning.unwrap().contains("capped at 100"));
        assert_eq!(req.namespace.as_deref(), Some("ns"));
        assert_eq!((req.salience, req.recency), (Some(false), Some(true)));
        assert_eq!(req.related_depth, Some(3));
        assert_eq!(req.where_eq, Some(("k".into(), "v".into())));
        assert_eq!(
            req.strategy(None).unwrap(),
            uteke_core::RecallStrategy::Fts5
        );
    }

    #[test]
    fn a_blank_query_a_bad_at_and_a_bad_where_are_refused() {
        let cmd = args_of(&["recall", "  "]);
        assert_eq!(
            RecallArgs::from_command(&cmd)
                .unwrap()
                .request(None)
                .unwrap_err(),
            "Query must not be empty or whitespace-only"
        );
        let cmd = args_of(&["recall", "q", "--at", "yesterday"]);
        let e = RecallArgs::from_command(&cmd)
            .unwrap()
            .request(None)
            .unwrap_err();
        assert!(e.starts_with("Invalid 'at' timestamp: yesterday"), "{e}");
        let cmd = args_of(&["recall", "q", "--where", "bad"]);
        assert_eq!(
            RecallArgs::from_command(&cmd)
                .unwrap()
                .request(None)
                .unwrap_err(),
            "Invalid --where expression: 'bad'. Use key=value format."
        );
        let cmd = args_of(&["recall", "q", "--limit", "50"]);
        let (req, warning) = RecallArgs::from_command(&cmd)
            .unwrap()
            .request(None)
            .unwrap();
        assert_eq!((req.limit, warning), (50, None));
    }

    #[test]
    fn explain_and_context_do_not_mix() {
        let cmd = args_of(&["recall", "q", "--explain", "--context"]);
        assert_eq!(
            RecallArgs::from_command(&cmd)
                .unwrap()
                .check_rendering()
                .unwrap_err(),
            "--explain and --context cannot be used together"
        );
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
