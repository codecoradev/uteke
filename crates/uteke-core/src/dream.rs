//! Dream cycle — coordinated maintenance pipeline (#353).
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
//! 4. **Orphans** — detect disconnected memories (#351, when available)
//! 5. **Compact** — aging cleanup + prune cold memories (existing)
//! 6. **Verify** — schema + index integrity check (existing doctor)
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
    pub fn dream(
        &self,
        namespace: Option<&str>,
        dry_run: bool,
        phases: &[DreamPhase],
    ) -> Result<DreamReport, Error> {
        let start = std::time::Instant::now();
        let selected: Vec<DreamPhase> = if phases.is_empty() {
            DreamPhase::all_in_order().to_vec()
        } else {
            phases.to_vec()
        };

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
            DreamPhase::Orphans => self.phase_orphans(namespace),
            DreamPhase::Compact => self.phase_compact(namespace, dry_run),
            DreamPhase::Verify => self.phase_verify(),
        }
    }

    fn phase_lint(&self, _namespace: Option<&str>) -> Result<PhaseResult, Error> {
        // Lightweight lint: count memories with unknown memory_type values.
        // Uses SQL COUNTs to avoid loading every memory into RAM (CodeCora
        // #390). Errors propagate — never silently mask DB failures.
        let total: i64 = self
            .store
            .conn
            .query_row("SELECT COUNT(*) FROM memories", [], |r| r.get(0))
            .map_err(|e| Error::db("dream lint total count", e))?;
        // Unknown types: anything not in the known set.
        let bad_count: i64 = self
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
        // Threshold 0.92 = very similar; tune down for more aggressive merges.
        let result = self.consolidate(namespace, 0.92, dry_run)?;
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
        Ok(PhaseResult {
            phase: DreamPhase::Dedup.as_str().to_string(),
            status: PhaseStatus::Ok,
            summary,
            changes: result.merged,
            warnings: if result.duplicates_found > 0 && result.merged == 0 {
                1
            } else {
                0
            },
        })
    }

    fn phase_orphans(&self, namespace: Option<&str>) -> Result<PhaseResult, Error> {
        // Detection only — never auto-delete. Inline SQL count of memories
        // with no edges (incoming or outgoing), access_count=0, not pinned,
        // and below the default threshold (#351 provides a richer API).
        const THRESHOLD: f64 = 0.3;
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
                   AND m.importance < ?1
                   AND m.namespace = ?2",
                rusqlite::params![THRESHOLD, ns],
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
                   AND m.importance < ?1",
                rusqlite::params![THRESHOLD],
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
        let summary = format!(
            "✓ {} memories pruned ({} deprecated)",
            result.pruned, result.deprecated
        );
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
        assert_eq!(order[5], DreamPhase::Verify);
        assert_eq!(order.len(), 6);
    }

    #[test]
    fn dream_runs_dry() {
        let uteke = crate::Uteke::open(":memory:").unwrap();
        let report = uteke.dream(None, true, &[]).unwrap();
        assert!(report.dry_run);
        assert_eq!(report.phases.len(), 6);
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
}
