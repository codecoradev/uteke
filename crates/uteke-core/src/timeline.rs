//! Timeline event tracking — per-memory audit log (#347).
//!
//! Append-only log of state changes per memory. Each event has a type
//! (`created`, `updated`, `recalled`, `consolidated`, `tagged`, `forgot`)
//! and optional JSON `event_data`. Stored in the `timeline_events` table
//! (schema v9).
//!
//! ## Usage
//!
//! Timeline events are emitted automatically by the memory lifecycle hooks
//! (`remember_precomputed`, recall access tracking, consolidate, forget).
//! They can also be queried explicitly via `Uteke::timeline(memory_id)` or
//! the `uteke timeline <id>` CLI command.

use crate::error::Error;
use crate::memory::store::Store;
use rusqlite::params;
use serde::{Deserialize, Serialize};

/// Event types tracked by the timeline.
///
/// Stored as a TEXT column; new variants are append-only (existing entries
/// must never be renamed or removed or they'll become unqueryable).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TimelineEventType {
    /// Memory first written.
    Created,
    /// Content/metadata changed.
    Updated,
    /// Memory was retrieved via search/recall.
    Recalled,
    /// Merged with another memory during consolidation.
    Consolidated,
    /// Tag added or removed.
    Tagged,
    /// Memory deleted.
    Forgot,
}

impl TimelineEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Updated => "updated",
            Self::Recalled => "recalled",
            Self::Consolidated => "consolidated",
            Self::Tagged => "tagged",
            Self::Forgot => "forgot",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "created" => Some(Self::Created),
            "updated" => Some(Self::Updated),
            "recalled" => Some(Self::Recalled),
            "consolidated" => Some(Self::Consolidated),
            "tagged" => Some(Self::Tagged),
            "forgot" => Some(Self::Forgot),
            _ => None,
        }
    }
}

/// A single timeline event row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEvent {
    pub id: i64,
    pub memory_id: String,
    pub event_type: String,
    /// Optional JSON payload describing what changed.
    pub event_data: Option<String>,
    pub created_at: String,
}

impl Store {
    /// Append a timeline event for a memory (#347). Best-effort — failures
    /// are logged and never fail the caller's primary operation.
    pub fn add_timeline_event(
        &self,
        memory_id: &str,
        event_type: TimelineEventType,
        event_data: Option<&serde_json::Value>,
    ) -> Result<(), Error> {
        let data_str = event_data.map(|v| match serde_json::to_string(v) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("timeline: failed to serialize event data: {e}");
                // Store null sentinel instead of empty string to make the
                // failure detectable downstream (CodeCora #388).
                "null".to_string()
            }
        });
        self.conn
            .execute(
                "INSERT INTO timeline_events (memory_id, event_type, event_data, created_at)
                 VALUES (?1, ?2, ?3, ?4)",
                params![
                    memory_id,
                    event_type.as_str(),
                    data_str,
                    chrono::Utc::now().to_rfc3339(),
                ],
            )
            .map_err(|e| Error::db("add timeline event", e))?;
        Ok(())
    }

    /// List timeline events for a memory, newest first.
    ///
    /// `limit=0` returns all events.
    pub fn list_timeline_events(
        &self,
        memory_id: &str,
        limit: usize,
    ) -> Result<Vec<TimelineEvent>, Error> {
        let sql = if limit == 0 {
            "SELECT id, memory_id, event_type, event_data, created_at FROM timeline_events
             WHERE memory_id = ?1 ORDER BY created_at DESC, id DESC"
        } else {
            "SELECT id, memory_id, event_type, event_data, created_at FROM timeline_events
             WHERE memory_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2"
        };
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("prepare timeline events", e))?;
        let rows = if limit == 0 {
            stmt.query_map(params![memory_id], map_timeline_row)
                .map_err(|e| Error::db("query timeline events", e))?
        } else {
            stmt.query_map(params![memory_id, limit as i64], map_timeline_row)
                .map_err(|e| Error::db("query timeline events", e))?
        };
        let mut events = Vec::new();
        for row in rows {
            events.push(row.map_err(|e| Error::db("timeline event row", e))?);
        }
        Ok(events)
    }

    /// Count total timeline events for a memory.
    pub fn count_timeline_events(&self, memory_id: &str) -> Result<usize, Error> {
        let n: i64 = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM timeline_events WHERE memory_id = ?1",
                params![memory_id],
                |r| r.get(0),
            )
            .map_err(|e| Error::db("count timeline events", e))?;
        Ok(n as usize)
    }
}

fn map_timeline_row(r: &rusqlite::Row) -> rusqlite::Result<TimelineEvent> {
    Ok(TimelineEvent {
        id: r.get(0)?,
        memory_id: r.get(1)?,
        event_type: r.get(2)?,
        event_data: r.get(3)?,
        created_at: r.get(4)?,
    })
}

impl crate::Uteke {
    /// Query the timeline for a memory (#347).
    pub fn timeline(&self, memory_id: &str, limit: usize) -> Result<Vec<TimelineEvent>, Error> {
        self.store.list_timeline_events(memory_id, limit)
    }

    /// Total timeline event count for a memory.
    pub fn count_timeline_events(&self, memory_id: &str) -> Result<usize, Error> {
        self.store.count_timeline_events(memory_id)
    }

    /// Best-effort timeline event append. Failures are logged and swallowed
    /// — timeline tracking must never break the primary operation.
    pub(crate) fn try_timeline_event(
        &self,
        memory_id: &str,
        event_type: TimelineEventType,
        event_data: Option<&serde_json::Value>,
    ) {
        if let Err(e) = self
            .store
            .add_timeline_event(memory_id, event_type, event_data)
        {
            tracing::warn!("timeline event failed for {memory_id}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::types::Memory;

    fn mem() -> Memory {
        Memory {
            id: uuid::Uuid::new_v4().to_string(),
            content: "test".to_string(),
            embedding: vec![],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: 0,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
        }
    }

    #[test]
    fn timeline_event_type_roundtrip() {
        for t in [
            TimelineEventType::Created,
            TimelineEventType::Updated,
            TimelineEventType::Recalled,
            TimelineEventType::Consolidated,
            TimelineEventType::Tagged,
            TimelineEventType::Forgot,
        ] {
            assert_eq!(TimelineEventType::from_str_opt(t.as_str()), Some(t));
        }
        assert_eq!(TimelineEventType::from_str_opt("unknown"), None);
    }

    #[test]
    fn timeline_add_and_list() {
        let store = Store::open(":memory:").unwrap();
        let m = mem();
        store.insert(&m).unwrap();

        store
            .add_timeline_event(&m.id, TimelineEventType::Created, None)
            .unwrap();
        store
            .add_timeline_event(
                &m.id,
                TimelineEventType::Updated,
                Some(&serde_json::json!({"field": "content"})),
            )
            .unwrap();

        let events = store.list_timeline_events(&m.id, 0).unwrap();
        assert_eq!(events.len(), 2);
        // Newest first.
        assert_eq!(events[0].event_type, "updated");
        assert_eq!(events[1].event_type, "created");
        assert!(events[0].event_data.is_some());
        assert!(events[1].event_data.is_none());
    }

    #[test]
    fn timeline_limit() {
        let store = Store::open(":memory:").unwrap();
        let m = mem();
        store.insert(&m).unwrap();
        for _ in 0..10 {
            store
                .add_timeline_event(&m.id, TimelineEventType::Recalled, None)
                .unwrap();
        }
        assert_eq!(store.count_timeline_events(&m.id).unwrap(), 10);
        let limited = store.list_timeline_events(&m.id, 3).unwrap();
        assert_eq!(limited.len(), 3);
    }

    #[test]
    fn timeline_empty_for_missing_memory() {
        let store = Store::open(":memory:").unwrap();
        let events = store
            .list_timeline_events("00000000-0000-0000-0000-000000000000", 0)
            .unwrap();
        assert!(events.is_empty());
        assert_eq!(
            store
                .count_timeline_events("00000000-0000-0000-0000-000000000000")
                .unwrap(),
            0
        );
    }

    #[test]
    fn migration_dispatcher_reaches_v9() {
        let store = Store::open(":memory:").unwrap();
        let v = store.schema_version().unwrap();
        assert_eq!(v, 10, "fresh store must reach CURRENT_SCHEMA_VERSION=10");
        // timeline_events table must exist.
        let m = mem();
        store.insert(&m).unwrap();
        store
            .add_timeline_event(&m.id, TimelineEventType::Created, None)
            .unwrap();
        assert_eq!(store.count_timeline_events(&m.id).unwrap(), 1);
    }
}
