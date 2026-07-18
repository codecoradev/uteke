//! Dream cycle — coordinated maintenance pipeline (#353, #720).
//!
//! A single command (`uteke dream`) that runs all maintenance phases in
//! dependency order, all local, zero LLM. Inspired by GBrain's overnight
//! dream cycle.
//!
//! ## Phases
//!
//! 1. **Lint** — type validation + broken-ref detection
//! 2. **Backlinks** — rebuild `referenced_by` edges (#350)
//! 3. **Dedup** — find & merge near-duplicates (existing consolidate)
//! 4. **Contradict** — detect contradictory memories via tag overlap + embedding divergence (#720)
//! 5. **Orphans** — detect disconnected memories (#351, when available)
//! 6. **Compact** — aging cleanup + prune cold memories (existing)
//! 7. **Verify** — schema + index integrity check (existing doctor)
//!
//! All phases are idempotent and safe to re-run.

use crate::error::Error;
use serde::{Deserialize, Serialize};

/// A single phase of the dream cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DreamPhase {
    Lint,
    Backlinks,
    Dedup,
    Contradict,
    Orphans,
    Compact,
    Verify,
}

impl DreamPhase {
    pub fn all_in_order() -> &'static [DreamPhase] {
        &[
            DreamPhase::Lint,
            DreamPhase::Backlinks,
            DreamPhase::Dedup,
            DreamPhase::Contradict,
            DreamPhase::Orphans,
            DreamPhase::Compact,
            DreamPhase::Verify,
        ]
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Lint => "lint",
            Self::Backlinks => "backlinks",
            Self::Dedup => "dedup",
            Self::Contradict => "contradict",
            Self::Orphans => "orphans",
            Self::Compact => "compact",
            Self::Verify => "verify",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "lint" => Some(Self::Lint),
            "backlinks" => Some(Self::Backlinks),
            "dedup" => Some(Self::Dedup),
            "contradict" => Some(Self::Contradict),
            "orphans" => Some(Self::Orphans),
            "compact" => Some(Self::Compact),
            "verify" => Some(Self::Verify),
            _ => None,
        }
    }
}

/// Result of a single phase execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseResult {
    pub phase: String,
    pub status: PhaseStatus,
    /// Human-readable summary line.
    pub summary: String,
    /// Number of items changed (0 for read-only / verify phases).
    pub changes: usize,
    /// Number of warnings emitted.
    pub warnings: usize,
}

/// Per-phase outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhaseStatus {
    /// Phase completed without issues.
    Ok,
    /// Phase completed but flagged items need attention.
    Warning,
    /// Phase failed; subsequent phases may still run.
    Error,
}

/// Full dream cycle result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DreamReport {
    pub phases: Vec<PhaseResult>,
    pub total_changes: usize,
    pub total_warnings: usize,
    pub total_errors: usize,
    pub dry_run: bool,
    pub duration_ms: u128,
}

impl crate::Uteke {
    /// Run the dream cycle pipeline (#353).
    ///
    /// Phases run in dependency order: lint → backlinks → dedup → orphans →
    /// compact → verify. Pass `dry_run = true` to report without changes.
    /// `phases` filters which phases to run (empty slice = all).
    ///
    /// **Namespace scope**: lint, dedup, orphans, and compact honor the
    /// namespace filter. Backlinks and verify are always global (they
    /// operate on the edge graph and schema, which are not
    /// namespace-partitioned).
    ///
    /// **Dry-run**: phases report against the *current* database state.
    /// Orphan counts in a dry run do not reflect projected post-maintenance
    /// state (because backlinks and dedup skip mutations). This is by
    /// design — dry-run answers "what would happen?" not "what will the
    /// state look like after?".
    pub fn dream(
        &self,
        namespace: Option<&str>,
        dry_run: bool,
        phases: &[DreamPhase],
    ) -> Result<DreamReport, Error> {
        let start = std::time::Instant::now();
        let mut selected: Vec<DreamPhase> = if phases.is_empty() {
            DreamPhase::all_in_order().to_vec()
        } else {
            phases.to_vec()
        };
        // Always execute in canonical dependency order, regardless of the
        // order the user passed --phases in (CodeCora #390 r4).
        let canonical = DreamPhase::all_in_order();
        selected.sort_by_key(|p| canonical.iter().position(|c| c == p).unwrap_or(usize::MAX));
        // Dedup: if the user passed the same phase twice, run it once.
        selected.dedup();

        let mut results = Vec::with_capacity(selected.len());
        for phase in &selected {
            // Each phase runs independently — a failure in one phase is
            // recorded as an error result but does NOT abort the whole
            // pipeline (CodeCora #390). Subsequent phases still run.
            let r = match self.run_phase(*phase, namespace, dry_run) {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("dream phase {:?} failed: {e}", phase);
                    PhaseResult {
                        phase: phase.as_str().to_string(),
                        status: PhaseStatus::Error,
                        summary: format!("✗ phase failed: {e}"),
                        changes: 0,
                        warnings: 1,
                    }
                }
            };
            results.push(r);
        }

        let total_changes = results.iter().map(|r| r.changes).sum();
        let total_warnings = results.iter().map(|r| r.warnings).sum();
        let total_errors = results
            .iter()
            .filter(|r| r.status == PhaseStatus::Error)
            .count();

        Ok(DreamReport {
            phases: results,
            total_changes,
            total_warnings,
            total_errors,
            dry_run,
            duration_ms: start.elapsed().as_millis(),
        })
    }

    fn run_phase(
        &self,
        phase: DreamPhase,
        namespace: Option<&str>,
        dry_run: bool,
    ) -> Result<PhaseResult, Error> {
        match phase {
            DreamPhase::Lint => self.phase_lint(namespace),
            DreamPhase::Backlinks => self.phase_backlinks(dry_run),
            DreamPhase::Dedup => self.phase_dedup(namespace, dry_run),
            DreamPhase::Contradict => self.phase_contradict(namespace, dry_run),
            DreamPhase::Orphans => self.phase_orphans(namespace),
            DreamPhase::Compact => self.phase_compact(namespace, dry_run),
            DreamPhase::Verify => self.phase_verify(),
        }
    }

    fn phase_lint(&self, namespace: Option<&str>) -> Result<PhaseResult, Error> {
        // Lightweight lint: count memories with unknown memory_type values.
        // Uses SQL COUNTs to avoid loading every memory into RAM (CodeCora
        // #390). Namespace-aware when a namespace is provided.
        // Errors propagate — never silently mask DB failures.
        let (total, bad_count): (i64, i64) = if let Some(ns) = namespace {
            let t: i64 = self
                .store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM memories WHERE namespace = ?1",
                    rusqlite::params![ns],
                    |r| r.get(0),
                )
                .map_err(|e| Error::db("dream lint total count", e))?;
            let b: i64 = self
                .store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM memories
                     WHERE namespace = ?1
                       AND memory_type NOT IN
                       ('fact','procedure','preference','decision','context',
                        'note','insight','reference','event')",
                    rusqlite::params![ns],
                    |r| r.get(0),
                )
                .map_err(|e| Error::db("dream lint bad type count", e))?;
            (t, b)
        } else {
            let t: i64 = self
                .store
                .conn
                .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
                .map_err(|e| Error::db("dream lint total count", e))?;
            let b: i64 = self
                .store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM memories
                     WHERE memory_type NOT IN
                       ('fact','procedure','preference','decision','context',
                        'note','insight','reference','event')",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| Error::db("dream lint bad type count", e))?;
            (t, b)
        };
        let warnings = bad_count as usize;
        let total = total as usize;
        let summary = if warnings == 0 {
            format!("✓ {total} memories, all types valid")
        } else {
            format!(
                "⚠ {warnings} memories have unknown types (out of {total}) — consider running uteke repair"
            )
        };
        Ok(PhaseResult {
            phase: DreamPhase::Lint.as_str().to_string(),
            status: if warnings == 0 {
                PhaseStatus::Ok
            } else {
                PhaseStatus::Warning
            },
            summary,
            changes: 0,
            warnings,
        })
    }

    fn phase_backlinks(&self, dry_run: bool) -> Result<PhaseResult, Error> {
        let edges_before = self
            .count_edges()
            .map_err(|e| Error::db("dream count edges", e))?;
        if dry_run {
            return Ok(PhaseResult {
                phase: DreamPhase::Backlinks.as_str().to_string(),
                status: PhaseStatus::Ok,
                summary: format!("✓ {edges_before} edges (dry-run, no rebuild)"),
                changes: 0,
                warnings: 0,
            });
        }
        let created = self.rebuild_backlinks()?;
        let edges_after = self
            .count_edges()
            .map_err(|e| Error::db("dream count edges after rebuild", e))?;
        let summary = if created == 0 {
            format!("✓ {edges_after} edges verified, no backlinks needed")
        } else {
            format!("✓ {edges_after} edges verified, +{created} backlinks created")
        };
        Ok(PhaseResult {
            phase: DreamPhase::Backlinks.as_str().to_string(),
            status: PhaseStatus::Ok,
            summary,
            changes: created,
            warnings: 0,
        })
    }

    fn phase_dedup(&self, namespace: Option<&str>, dry_run: bool) -> Result<PhaseResult, Error> {
        // Threshold from config (#731) — very similar; tune down for more aggressive merges.
        let result = self.consolidate(namespace, self.dream_config.dedup_threshold as f64, dry_run)?;
        let summary = if result.merged == 0 {
            format!(
                "✓ {} duplicate pairs found, 0 merged{}",
                result.duplicates_found,
                if dry_run { " (dry-run)" } else { "" }
            )
        } else {
            format!(
                "✓ {} duplicates found, {} merged",
                result.duplicates_found, result.merged
            )
        };
        let has_warnings = result.duplicates_found > 0 && result.merged == 0;
        Ok(PhaseResult {
            phase: DreamPhase::Dedup.as_str().to_string(),
            status: if has_warnings {
                PhaseStatus::Warning
            } else {
                PhaseStatus::Ok
            },
            summary,
            changes: result.merged,
            warnings: if has_warnings { 1 } else { 0 },
        })
    }

    /// Contradiction detection phase (#720).
    ///
    /// Scans top-N most recently updated memories for pairs that:
    /// 1. Share at least one tag (topic overlap)
    /// 2. Have high tag Jaccard overlap (≥ 0.3)
    /// 3. Have low embedding cosine similarity (≤ threshold, default 0.6)
    ///
    /// Flagged pairs get a "contradicts" graph edge (older → newer).
    fn phase_contradict(
        &self,
        namespace: Option<&str>,
        dry_run: bool,
    ) -> Result<PhaseResult, Error> {
        let similarity_threshold = self.dream_config.contradict_similarity_threshold;
        let tag_overlap_min: usize = 1;
        let tag_jaccard_min = self.dream_config.contradict_tag_jaccard_min;
        let max_memories = self.dream_config.contradict_max_memories;

        // Load top-N memories ordered by updated_at DESC
        let memories = self.load_recent_memories(namespace, max_memories)?;

        if memories.len() < 2 {
            return Ok(PhaseResult {
                phase: DreamPhase::Contradict.as_str().to_string(),
                status: PhaseStatus::Ok,
                summary: "✓ fewer than 2 memories, nothing to scan".to_string(),
                changes: 0,
                warnings: 0,
            });
        }

        // Build tag sets for each memory
        let tag_sets: Vec<std::collections::HashSet<&str>> = memories
            .iter()
            .map(|m| m.tags.iter().map(|t| t.as_str()).collect())
            .collect();

        // O(n²) pair scan
        let mut contradiction_count = 0usize;
        let mut edges_created = 0usize;

        for i in 0..memories.len() {
            for j in (i + 1)..memories.len() {
                let m1 = &memories[i];
                let m2 = &memories[j];

                // Skip deprecated memories
                if m1.deprecated || m2.deprecated {
                    continue;
                }

                let tags1 = &tag_sets[i];
                let tags2 = &tag_sets[j];

                // Must share at least one tag
                if tags1.is_empty() || tags2.is_empty() {
                    continue;
                }
                let intersection = tags1.intersection(tags2).count();
                if intersection < tag_overlap_min {
                    continue;
                }

                // Tag Jaccard must be above minimum
                let union = tags1.union(tags2).count();
                let tag_jaccard = intersection as f32 / union as f32;
                if tag_jaccard < tag_jaccard_min {
                    continue;
                }

                // Both must have embeddings for cosine comparison
                if m1.embedding.is_empty() || m2.embedding.is_empty() {
                    continue;
                }

                // Cosine similarity
                let cosine = crate::consolidate::cosine_similarity(&m1.embedding, &m2.embedding);

                // Low similarity = potential contradiction
                if cosine > similarity_threshold {
                    continue;
                }

                contradiction_count += 1;

                if dry_run {
                    tracing::info!(
                        "contradiction (dry-run): sim={:.3} tag_jaccard={:.2} | '{}' ↔ '{}'",
                        cosine,
                        tag_jaccard,
                        &m1.content.chars().take(50).collect::<String>(),
                        &m2.content.chars().take(50).collect::<String>(),
                    );
                    continue;
                }

                // Determine older → newer ordering
                let (older, newer) = if m1.updated_at <= m2.updated_at {
                    (&m1.id, &m2.id)
                } else {
                    (&m2.id, &m1.id)
                };

                // Create "contradicts" graph edge
                let gs = crate::GraphStore::new(&self.store.conn);
                match gs.add_edge(older, newer, "contradicts", cosine as f64) {
                    Ok(()) => {
                        edges_created += 1;
                        tracing::info!(
                            "contradiction detected: sim={:.3} tag_j={:.2} {} -[contradicts]-> {}",
                            cosine,
                            tag_jaccard,
                            &older[..8.min(older.len())],
                            &newer[..8.min(newer.len())],
                        );
                    }
                    Err(e) => {
                        tracing::warn!("failed to create contradiction edge: {e}");
                    }
                }
            }
        }

        let summary = if contradiction_count == 0 {
            "✓ no contradictions detected".to_string()
        } else if dry_run {
            format!(
                "⚠ {contradiction_count} potential contradiction{} (dry-run, no edges created)",
                if contradiction_count == 1 { "" } else { "s" }
            )
        } else {
            format!(
                "⚠ {contradiction_count} contradiction{} detected, {edges_created} edge{} created",
                if contradiction_count == 1 { "" } else { "s" },
                if edges_created == 1 { "" } else { "s" },
            )
        };

        Ok(PhaseResult {
            phase: DreamPhase::Contradict.as_str().to_string(),
            status: if contradiction_count == 0 {
                PhaseStatus::Ok
            } else {
                PhaseStatus::Warning
            },
            summary,
            changes: edges_created,
            warnings: contradiction_count,
        })
    }

    /// Load top-N most recently updated memories with embeddings.
    fn load_recent_memories(
        &self,
        namespace: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::memory::Memory>, Error> {
        let sql = match namespace {
            Some(_ns) => {
                "SELECT id, content, embedding, tags, metadata, \
                 created_at, updated_at, namespace, access_count, \
                 last_accessed, deprecated, valid_from, valid_until, \
                 memory_type, importance, pinned, content_type, slug \
                 FROM memories WHERE namespace = ?1 AND deprecated = 0 \
                 ORDER BY updated_at DESC LIMIT ?2"
            }
            None => {
                "SELECT id, content, embedding, tags, metadata, \
                 created_at, updated_at, namespace, access_count, \
                 last_accessed, deprecated, valid_from, valid_until, \
                 memory_type, importance, pinned, content_type, slug \
                 FROM memories WHERE deprecated = 0 \
                 ORDER BY updated_at DESC LIMIT ?1"
            }
        };

        let mut stmt = self
            .store
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("dream contradict prepare", e))?;

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(
                    rusqlite::params![ns, limit as i64],
                    crate::memory::store::row_to_memory,
                )
                .map_err(|e| Error::db("dream contradict query", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("dream contradict fetch", e))?,
            None => stmt
                .query_map(
                    rusqlite::params![limit as i64],
                    crate::memory::store::row_to_memory,
                )
                .map_err(|e| Error::db("dream contradict query", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("dream contradict fetch", e))?,
        };

        Ok(rows)
    }

    fn phase_orphans(&self, namespace: Option<&str>) -> Result<PhaseResult, Error> {
        // Detection only — never auto-delete. Inline SQL count of memories
        // with no edges (incoming or outgoing), access_count=0, not pinned,
        // and below the configured threshold (#731).
        let threshold = self.dream_config.orphan_importance_threshold;
        // Namespace-aware query: when a namespace is specified, only count
        // orphans in that namespace (CodeCora #390).
        let count: i64 = if let Some(ns) = namespace {
            self.store.conn.query_row(
                "SELECT COUNT(DISTINCT m.id)
                 FROM memories m
                 LEFT JOIN memory_edges out_e ON out_e.source_id = m.id
                 LEFT JOIN memory_edges in_e  ON in_e.target_id  = m.id
                 WHERE out_e.id IS NULL
                   AND in_e.id IS NULL
                   AND m.access_count = 0
                   AND m.pinned = 0
                   AND m.deprecated = 0
                   AND m.importance < ?1
                   AND m.namespace = ?2",
                rusqlite::params![threshold, ns],
                |r| r.get(0),
            )
        } else {
            self.store.conn.query_row(
                "SELECT COUNT(DISTINCT m.id)
                 FROM memories m
                 LEFT JOIN memory_edges out_e ON out_e.source_id = m.id
                 LEFT JOIN memory_edges in_e  ON in_e.target_id  = m.id
                 WHERE out_e.id IS NULL
                   AND in_e.id IS NULL
                   AND m.access_count = 0
                   AND m.pinned = 0
                   AND m.deprecated = 0
                   AND m.importance < ?1",
                rusqlite::params![threshold],
                |r| r.get(0),
            )
        }
        .map_err(|e| Error::db("dream orphan count", e))?;
        let count = count as usize;
        let summary = if count == 0 {
            "✓ no orphan memories detected".to_string()
        } else {
            format!(
                "⚠ {count} orphan memor{} detected (run: uteke orphans)",
                if count == 1 { "y" } else { "ies" }
            )
        };
        Ok(PhaseResult {
            phase: DreamPhase::Orphans.as_str().to_string(),
            status: if count == 0 {
                PhaseStatus::Ok
            } else {
                PhaseStatus::Warning
            },
            summary,
            changes: 0,
            warnings: count,
        })
    }

    fn phase_compact(&self, namespace: Option<&str>, dry_run: bool) -> Result<PhaseResult, Error> {
        // Prune deprecated memories older than 30 days.
        const TTL_DAYS: u32 = 30;
        let result = self.prune(TTL_DAYS, namespace, dry_run)?;
        let summary = if dry_run {
            format!(
                "✓ {} memories would be pruned ({} deprecated)",
                result.pruned, result.deprecated
            )
        } else {
            format!(
                "✓ {} memories pruned ({} deprecated)",
                result.pruned, result.deprecated
            )
        };
        Ok(PhaseResult {
            phase: DreamPhase::Compact.as_str().to_string(),
            status: PhaseStatus::Ok,
            summary,
            changes: result.pruned,
            warnings: 0,
        })
    }

    fn phase_verify(&self) -> Result<PhaseResult, Error> {
        let report = self.verify()?;
        let consistent = report.consistent;
        let status = if consistent {
            PhaseStatus::Ok
        } else {
            PhaseStatus::Error
        };
        let summary = format!(
            "{} db={} index={} (run: uteke doctor for details)",
            if status == PhaseStatus::Ok {
                "✓"
            } else {
                "✗"
            },
            report.db_count,
            report.index_count,
        );
        Ok(PhaseResult {
            phase: DreamPhase::Verify.as_str().to_string(),
            status,
            summary,
            changes: 0,
            warnings: if status == PhaseStatus::Error { 1 } else { 0 },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_roundtrip() {
        for p in DreamPhase::all_in_order() {
            assert_eq!(DreamPhase::from_str_opt(p.as_str()), Some(*p));
        }
        assert_eq!(DreamPhase::from_str_opt("unknown"), None);
        // Case-insensitive.
        assert_eq!(DreamPhase::from_str_opt("DEDUP"), Some(DreamPhase::Dedup));
    }

    #[test]
    fn phase_order() {
        let order = DreamPhase::all_in_order();
        assert_eq!(order[0], DreamPhase::Lint);
        assert_eq!(order[3], DreamPhase::Contradict);
        assert_eq!(order[6], DreamPhase::Verify);
        assert_eq!(order.len(), 7);
    }

    #[test]
    fn dream_runs_dry() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let report = uteke.dream(None, true, &[]).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.phases.len(), 7);
        // Dry run should make no changes.
        assert_eq!(report.total_changes, 0);
    }

    #[test]
    fn dream_phase_filter() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let report = uteke
            .dream(None, true, &[DreamPhase::Lint, DreamPhase::Verify])
            .unwrap();
        assert_eq!(report.phases.len(), 2);
        assert_eq!(report.phases[0].phase, "lint");
        assert_eq!(report.phases[1].phase, "verify");
    }

    #[test]
    fn dream_phase_order_reversed() {
        // CodeCora #390 r4: even if user passes phases in non-canonical
        // order, they must execute in dependency order.
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let report = uteke
            .dream(
                None,
                true,
                &[DreamPhase::Verify, DreamPhase::Lint, DreamPhase::Backlinks],
            )
            .unwrap();
        assert_eq!(report.phases.len(), 3);
        // Must be in canonical order: lint → backlinks → verify.
        assert_eq!(report.phases[0].phase, "lint");
        assert_eq!(report.phases[1].phase, "backlinks");
        assert_eq!(report.phases[2].phase, "verify");
    }

    #[test]
    fn dream_phase_dedup() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let report = uteke
            .dream(
                None,
                true,
                &[DreamPhase::Lint, DreamPhase::Lint, DreamPhase::Lint],
            )
            .unwrap();
        assert_eq!(report.phases.len(), 1, "duplicate phases should be deduped");
    }

    #[test]
    fn contradict_phase_dry_run_no_memories() {
        // No memories = early exit with "nothing to scan"
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let report = uteke.dream(None, true, &[DreamPhase::Contradict]).unwrap();
        assert_eq!(report.phases.len(), 1);
        assert_eq!(report.phases[0].phase, "contradict");
        assert_eq!(report.total_changes, 0);
        assert!(report.phases[0].summary.contains("nothing to scan"));
    }

    #[test]
    fn contradict_phase_in_all_order() {
        // Contradict should appear at index 3 in all_in_order
        let order = DreamPhase::all_in_order();
        assert!(order.contains(&DreamPhase::Contradict));
        // Contradict comes after Dedup and before Orphans
        let dedup_idx = order
            .iter()
            .position(|p| matches!(p, DreamPhase::Dedup))
            .unwrap();
        let contradict_idx = order
            .iter()
            .position(|p| matches!(p, DreamPhase::Contradict))
            .unwrap();
        let orphans_idx = order
            .iter()
            .position(|p| matches!(p, DreamPhase::Orphans))
            .unwrap();
        assert!(
            dedup_idx < contradict_idx,
            "Contradict must come after Dedup"
        );
        assert!(
            contradict_idx < orphans_idx,
            "Contradict must come before Orphans"
        );
    }

    #[test]
    fn contradict_from_str_roundtrip() {
        assert_eq!(
            DreamPhase::from_str_opt("contradict"),
            Some(DreamPhase::Contradict)
        );
        assert_eq!(
            DreamPhase::from_str_opt("CONTRADICT"),
            Some(DreamPhase::Contradict)
        );
        assert_eq!(DreamPhase::Contradict.as_str(), "contradict");
    }
}
