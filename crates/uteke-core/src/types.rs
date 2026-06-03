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
    /// Count of vectors in usearch index.
    pub index_count: usize,
    /// Whether they match.
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
        };
        let json = serde_json::to_string(&report).unwrap();
        let restored: RepairReport = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.index_after, 10);
    }
}
