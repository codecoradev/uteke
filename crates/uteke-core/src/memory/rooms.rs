//! Room operations — collaborative memory spaces for multi-agent discussions.

use crate::Error;
use rusqlite::params;
use rusqlite::OptionalExtension;

/// A shared collaboration context identified by an external ID.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Room {
    pub id: String,
    pub title: Option<String>,
    pub namespace: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Statistics about a room.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoomStats {
    pub room_id: String,
    pub title: Option<String>,
    pub memory_count: usize,
    pub participant_count: usize,
    pub participants: Vec<String>,
    pub created_at: String,
    pub last_activity: Option<String>,
}

/// A memory linked to a room with author attribution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoomMemory {
    pub memory_id: String,
    pub room_id: String,
    pub author: String,
    pub role: String,
    pub joined_at: String,
}

impl super::Store {
    /// Create a new room. Returns error if room already exists.
    pub fn create_room(
        &self,
        room_id: &str,
        title: Option<&str>,
        namespace: &str,
    ) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT INTO rooms (id, title, namespace, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![room_id, title, namespace, now, now],
            )
            .map_err(|e| {
                if e.to_string().contains("UNIQUE constraint") {
                    Error::db_msg(format!("Room already exists: {room_id}"))
                } else {
                    Error::db("create room", e)
                }
            })?;
        Ok(())
    }

    /// Get a room by ID.
    pub fn get_room(&self, room_id: &str) -> Result<Option<Room>, Error> {
        self.conn
            .query_row(
                "SELECT id, title, namespace, created_at, updated_at FROM rooms WHERE id = ?1",
                params![room_id],
                |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        namespace: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|e| Error::db("get room", e))
    }

    /// List rooms that a namespace has participated in.
    pub fn list_rooms(&self, namespace: Option<&str>) -> Result<Vec<Room>, Error> {
        let sql = match namespace {
            Some(_) => {
                "SELECT DISTINCT r.id, r.title, r.namespace, r.created_at, r.updated_at \
                 FROM rooms r \
                 INNER JOIN room_memories rm ON r.id = rm.room_id \
                 WHERE r.namespace = ?1 \
                 ORDER BY r.updated_at DESC"
            }
            None => {
                "SELECT id, title, namespace, created_at, updated_at FROM rooms \
                 ORDER BY updated_at DESC"
            }
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("list rooms", e))?;

        let rows = match namespace {
            Some(ns) => stmt
                .query_map(params![ns], |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        namespace: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                })
                .map_err(|e| Error::db("list rooms", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("list rooms", e))?,
            None => stmt
                .query_map([], |row| {
                    Ok(Room {
                        id: row.get(0)?,
                        title: row.get(1)?,
                        namespace: row.get(2)?,
                        created_at: row.get(3)?,
                        updated_at: row.get(4)?,
                    })
                })
                .map_err(|e| Error::db("list rooms", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("list rooms", e))?,
        };

        Ok(rows)
    }

    /// Get statistics about a room.
    pub fn room_stats(&self, room_id: &str) -> Result<Option<RoomStats>, Error> {
        let room = match self.get_room(room_id)? {
            Some(r) => r,
            None => return Ok(None),
        };

        let memory_count: usize = self
            .conn
            .query_row(
                "SELECT COUNT(DISTINCT memory_id) FROM room_memories WHERE room_id = ?1",
                params![room_id],
                |row| row.get(0),
            )
            .map_err(|e| Error::db("room memory count", e))?;

        // Get distinct authors as participants
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT author FROM room_memories WHERE room_id = ?1 ORDER BY author")
            .map_err(|e| Error::db("room participants", e))?;
        let participants: Vec<String> = stmt
            .query_map(params![room_id], |row| row.get(0))
            .map_err(|e| Error::db("room participants", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| Error::db("room participants", e))?;

        let last_activity: Option<String> = self
            .conn
            .query_row(
                "SELECT MAX(joined_at) FROM room_memories WHERE room_id = ?1",
                params![room_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| Error::db("room last activity", e))?
            .flatten();

        Ok(Some(RoomStats {
            room_id: room.id,
            title: room.title,
            memory_count,
            participant_count: participants.len(),
            participants,
            created_at: room.created_at,
            last_activity,
        }))
    }

    /// Link a memory to a room with author attribution.
    pub fn link_memory_to_room(
        &self,
        room_id: &str,
        memory_id: &str,
        author: &str,
        role: &str,
    ) -> Result<(), Error> {
        let now = chrono::Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO room_memories (room_id, memory_id, author, role, joined_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![room_id, memory_id, author, role, now],
            )
            .map_err(|e| Error::db("link memory to room", e))?;

        // Update room's updated_at timestamp
        self.conn
            .execute(
                "UPDATE rooms SET updated_at = ?1 WHERE id = ?2",
                params![now, room_id],
            )
            .map_err(|e| Error::db("update room timestamp", e))?;

        Ok(())
    }

    /// Recall all memories linked to a room, sorted by time.
    /// Cross-namespace: returns memories from ALL namespaces that contributed to the room.
    pub fn recall_room(
        &self,
        room_id: &str,
        author: Option<&str>,
        limit: usize,
    ) -> Result<Vec<crate::memory::types::Memory>, Error> {
        let sql = match author {
            Some(_) => {
                "SELECT m.id, m.content, m.embedding, m.tags, m.metadata, \
                 m.created_at, m.updated_at, m.namespace, m.access_count, \
                 m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type \
                 FROM memories m \
                 INNER JOIN room_memories rm ON m.id = rm.memory_id \
                 WHERE rm.room_id = ?1 AND rm.author = ?2 \
                 ORDER BY rm.joined_at ASC \
                 LIMIT ?3"
            }
            None => {
                "SELECT m.id, m.content, m.embedding, m.tags, m.metadata, \
                 m.created_at, m.updated_at, m.namespace, m.access_count, \
                 m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type \
                 FROM memories m \
                 INNER JOIN room_memories rm ON m.id = rm.memory_id \
                 WHERE rm.room_id = ?1 \
                 ORDER BY rm.joined_at ASC \
                 LIMIT ?2"
            }
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("recall room", e))?;

        use super::store::row_to_memory;

        let memories = match author {
            Some(a) => stmt
                .query_map(params![room_id, a, limit], row_to_memory)
                .map_err(|e| Error::db("recall room", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("recall room", e))?,
            None => stmt
                .query_map(params![room_id, limit], row_to_memory)
                .map_err(|e| Error::db("recall room", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("recall room", e))?,
        };

        Ok(memories)
    }

    /// Delete a room and all its memory links (CASCADE).
    pub fn delete_room(&self, room_id: &str) -> Result<(), Error> {
        let rows = self
            .conn
            .execute("DELETE FROM rooms WHERE id = ?1", params![room_id])
            .map_err(|e| Error::db("delete room", e))?;
        if rows == 0 {
            return Err(Error::db_msg(format!("Room not found: {room_id}")));
        }
        Ok(())
    }
}
