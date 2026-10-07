//! Report types for diagnostic commands.

use serde::{Deserialize, Serialize};

/// Status of a doctor check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DoctorStatus {
    Ok,
    Warn,
    Error,
}

/// A single check in the doctor report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorCheck {
    /// Name of the check.
    pub name: String,
    /// Status: ok, warn, error.
    pub status: DoctorStatus,
    /// Detail message.
    pub detail: String,
}

/// Result of `uteke doctor`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoctorReport {
    /// All checks performed.
    pub checks: Vec<DoctorCheck>,
}

/// Result of `uteke verify`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyReport {
    /// Count of memories in SQLite.
    pub db_count: usize,
    /// Count of document chunks in SQLite (chunk vectors share the index).
    #[serde(default)]
    pub chunk_count: usize,
    /// Count of vectors in usearch index.
    pub index_count: usize,
    /// Whether they match (db_count + chunk_count == index_count).
    pub consistent: bool,
}

/// Result of `uteke repair`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairReport {
    /// Count of memories in DB.
    pub db_count: usize,
    /// Index count before repair.
    pub index_before: usize,
    /// Index count after repair.
    pub index_after: usize,
    /// Document chunks included in the rebuilt index (#1110).
    /// Defaults to 0 when deserializing reports from older binaries.
    #[serde(default)]
    pub chunk_count: usize,
}

/// Result of compacting the vector index (#1324).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionReport {
    /// Physical rows in the index before compaction.
    pub rows_before: usize,
    /// Dead (tombstoned) rows among them.
    pub dead_before: usize,
    /// Physical rows after compaction (the live set; `rows_before` when dry-run).
    pub rows_after: usize,
    /// True when nothing was changed (`dry_run`).
    pub dry_run: bool,
}

/// Result of `uteke repair --reembed`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReembedReport {
    /// Total memories scanned.
    pub total_scanned: usize,
    /// Memories that had missing/empty embeddings.
    pub missing_count: usize,
    /// Memories successfully re-embedded.
    pub reembedded: usize,
    /// Memories that failed re-embedding.
    pub failed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_doctor_report_serialization() {
        let report = DoctorReport {
            checks: vec![
                DoctorCheck {
                    name: "DB".to_string(),
                    status: DoctorStatus::Ok,
                    detail: "ok".to_string(),
                },
                DoctorCheck {
                    name: "Index".to_string(),
                    status: DoctorStatus::Error,
                    detail: "mismatch".to_string(),
                },
            ],
        };
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"DB\""));
        assert!(json.contains("\"Error\""));
    }

    #[test]
    fn test_verify_report_serialization() {
        let report = VerifyReport {
            db_count: 10,
            chunk_count: 0,
            index_count: 10,
            consistent: true,
        };
        let json = serde_json::to_string(&report).unwrap();
        let restored: VerifyReport = serde_json::from_str(&json).unwrap();
        assert!(restored.consistent);
    }

    #[test]
    fn test_repair_report_serialization() {
        let report = RepairReport {
            db_count: 10,
            index_before: 5,
            index_after: 10,
            chunk_count: 2,
        };
        let json = serde_json::to_string(&report).unwrap();
        let restored: RepairReport = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.index_after, 10);
        assert_eq!(restored.chunk_count, 2);
    }
}
