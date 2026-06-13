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

/// Room summary result — topic clusters and discussion overview.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoomSummary {
    pub room_id: String,
    pub title: Option<String>,
    pub total_memories: usize,
    pub participants: Vec<String>,
    pub time_range: TimeRange,
    pub clusters: Vec<TopicCluster>,
    pub top_tags: Vec<crate::memory::types::TagInfo>,
    pub recent_decisions: Vec<String>,
    pub pinned_highlights: Vec<String>,
}

/// Time range of memories in a room.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TimeRange {
    pub earliest: String,
    pub latest: String,
}

/// A topic cluster derived from tag co-occurrence.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TopicCluster {
    pub topic: String,
    pub memory_count: usize,
    pub top_memories: Vec<String>,
    pub tags: Vec<String>,
    pub participants: Vec<String>,
    pub score: f32,
}

/// A structured document generated from a room's memories.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RoomDocument {
    pub room_id: String,
    pub title: Option<String>,
    pub generated_at: String,
    pub sections: Vec<DocumentSection>,
}

/// A section within a room document, grouping entries by memory type.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocumentSection {
    pub heading: String,
    pub icon: String,
    pub entries: Vec<DocumentEntry>,
}

/// A single entry within a document section.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DocumentEntry {
    pub content: String,
    pub author: String,
    pub tags: Vec<String>,
    pub created_at: String,
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
                 m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type, m.importance, m.pinned \
                 FROM memories m \
                 INNER JOIN room_memories rm ON m.id = rm.memory_id \
                 WHERE rm.room_id = ?1 AND rm.author = ?2 \
                 ORDER BY rm.joined_at ASC \
                 LIMIT ?3"
            }
            None => {
                "SELECT m.id, m.content, m.embedding, m.tags, m.metadata, \
                 m.created_at, m.updated_at, m.namespace, m.access_count, \
                 m.last_accessed, m.deprecated, m.valid_from, m.valid_until, m.memory_type, m.importance, m.pinned \
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

    /// Get memory IDs linked to a room.
    /// Returns just the IDs — much cheaper than full recall when only
    /// filtering is needed.
    pub fn get_room_memory_ids(
        &self,
        room_id: &str,
        author: Option<&str>,
    ) -> Result<Vec<String>, Error> {
        let sql = match author {
            Some(_) => "SELECT memory_id FROM room_memories WHERE room_id = ?1 AND author = ?2",
            None => "SELECT memory_id FROM room_memories WHERE room_id = ?1",
        };

        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| Error::db("get room memory ids", e))?;

        let ids: Vec<String> = match author {
            Some(a) => stmt
                .query_map(params![room_id, a], |row| row.get(0))
                .map_err(|e| Error::db("get room memory ids", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("get room memory ids", e))?,
            None => stmt
                .query_map(params![room_id], |row| row.get(0))
                .map_err(|e| Error::db("get room memory ids", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("get room memory ids", e))?,
        };

        Ok(ids)
    }

    /// Generate a room summary with LLM-free topic clustering.
    pub fn room_summary(&self, room_id: &str) -> Result<Option<RoomSummary>, Error> {
        let room = match self.get_room(room_id)? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Get ALL room memories (no limit)
        let memories = self.recall_room(room_id, None, i32::MAX as usize)?;

        if memories.is_empty() {
            return Ok(Some(RoomSummary {
                room_id: room.id,
                title: room.title,
                total_memories: 0,
                participants: vec![],
                time_range: TimeRange {
                    earliest: String::new(),
                    latest: String::new(),
                },
                clusters: vec![],
                top_tags: vec![],
                recent_decisions: vec![],
                pinned_highlights: vec![],
            }));
        }

        // Get author mapping from room_memories
        let mut author_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT memory_id, author FROM room_memories WHERE room_id = ?1")
                .map_err(|e| Error::db("room summary authors", e))?;
            let rows: Vec<(String, String)> = stmt
                .query_map(params![room_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| Error::db("room summary authors", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("room summary authors", e))?;
            for (mid, author) in rows {
                author_map.insert(mid, author);
            }
        }

        // Collect participants
        let participants: Vec<String> = {
            let mut ps: Vec<String> = author_map.values().cloned().collect();
            ps.sort();
            ps.dedup();
            ps
        };

        // Time range
        let earliest = memories
            .iter()
            .map(|m| m.created_at.to_rfc3339())
            .min()
            .unwrap_or_default();
        let latest = memories
            .iter()
            .map(|m| m.created_at.to_rfc3339())
            .max()
            .unwrap_or_default();

        let fmt_time = |s: &str| -> String { s.get(..19).unwrap_or(s).to_string() };

        // Tag frequency
        let mut tag_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for m in &memories {
            for tag in &m.tags {
                *tag_counts.entry(tag.clone()).or_insert(0) += 1;
            }
        }

        // Top tags (top 10)
        let mut tag_count_vec: Vec<(String, usize)> =
            tag_counts.iter().map(|(k, &v)| (k.clone(), v)).collect();
        tag_count_vec.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let top_tags: Vec<crate::memory::types::TagInfo> = tag_count_vec
            .iter()
            .take(10)
            .map(|(name, count)| crate::memory::types::TagInfo {
                name: name.clone(),
                count: *count,
            })
            .collect();

        // Build clusters from tag co-occurrence
        let mut tag_groups: std::collections::HashMap<String, Vec<&crate::memory::types::Memory>> =
            std::collections::HashMap::new();
        for m in &memories {
            for tag in &m.tags {
                tag_groups.entry(tag.clone()).or_default().push(m);
            }
        }

        // Convert to TopicClusters
        let mut clusters: Vec<TopicCluster> = tag_groups
            .iter()
            .map(|(tag, mems)| {
                let mem_count = mems.len();
                // Top 3 memories by importance (fallback recency)
                let mut sorted = mems.clone();
                sorted.sort_by(|a, b| {
                    b.importance
                        .partial_cmp(&a.importance)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then_with(|| b.created_at.cmp(&a.created_at))
                });
                let top_memories: Vec<String> = sorted
                    .iter()
                    .take(3)
                    .map(|m| {
                        let s = m.content.clone();
                        if s.len() > 100 {
                            format!("{}...", &s[..97])
                        } else {
                            s
                        }
                    })
                    .collect();

                // Cluster tags: collect all tags from cluster memories, sort by frequency
                let mut cluster_tags: std::collections::HashMap<String, usize> =
                    std::collections::HashMap::new();
                for m in mems {
                    for t in &m.tags {
                        *cluster_tags.entry(t.clone()).or_insert(0) += 1;
                    }
                }
                let mut cluster_tag_filtered: Vec<(&String, &usize)> =
                    cluster_tags.iter().filter(|(t, _)| *t != tag).collect();
                cluster_tag_filtered.sort_by(|a, b| b.1.cmp(a.1));
                let mut cluster_tag_vec: Vec<String> = cluster_tag_filtered
                    .into_iter()
                    .take(5)
                    .map(|(t, _)| t.clone())
                    .collect();
                cluster_tag_vec.insert(0, tag.clone());

                // Cluster participants
                let mut cluster_parts: Vec<String> = mems
                    .iter()
                    .filter_map(|m| author_map.get(&m.id).cloned())
                    .collect();
                cluster_parts.sort();
                cluster_parts.dedup();

                // Average importance score
                let score =
                    mems.iter().map(|m| m.importance as f32).sum::<f32>() / mems.len() as f32;

                TopicCluster {
                    topic: tag.clone(),
                    memory_count: mem_count,
                    top_memories,
                    tags: cluster_tag_vec,
                    participants: cluster_parts,
                    score,
                }
            })
            .collect();

        // Sort clusters by memory count descending
        clusters.sort_by(|a, b| {
            b.memory_count
                .cmp(&a.memory_count)
                .then_with(|| a.topic.cmp(&b.topic))
        });

        // Merge small clusters (< 2 memories) into "Other"
        let (small, big): (Vec<_>, Vec<_>) = clusters.into_iter().partition(|c| c.memory_count < 2);
        let mut final_clusters = big;
        if !small.is_empty() {
            let total_small: usize = small.iter().map(|c| c.memory_count).sum();
            let all_previews: Vec<String> = small
                .iter()
                .flat_map(|c| c.top_memories.iter())
                .take(3)
                .cloned()
                .collect();
            let mut all_tags: Vec<String> = small
                .iter()
                .flat_map(|c| c.tags.iter())
                .take(5)
                .cloned()
                .collect();
            all_tags.sort();
            all_tags.dedup();
            let mut all_parts: Vec<String> = small
                .iter()
                .flat_map(|c| c.participants.iter())
                .cloned()
                .collect();
            all_parts.sort();
            all_parts.dedup();
            let score: f32 = small
                .iter()
                .map(|c| c.score * c.memory_count as f32)
                .sum::<f32>()
                / total_small.max(1) as f32;

            final_clusters.push(TopicCluster {
                topic: "Other".to_string(),
                memory_count: total_small,
                top_memories: all_previews,
                tags: all_tags,
                participants: all_parts,
                score,
            });
        }

        // Recent decisions: memory_type == "decision", sorted by created_at desc, top 5
        let mut decisions: Vec<&crate::memory::types::Memory> = memories
            .iter()
            .filter(|m| m.memory_type == "decision")
            .collect();
        decisions.sort_by_key(|b| std::cmp::Reverse(b.created_at));
        let recent_decisions: Vec<String> = decisions
            .iter()
            .take(5)
            .map(|m| {
                let s = m.content.clone();
                if s.len() > 120 {
                    format!("{}...", &s[..117])
                } else {
                    s
                }
            })
            .collect();

        // Pinned highlights
        let pinned: Vec<String> = memories
            .iter()
            .filter(|m| m.pinned)
            .take(5)
            .map(|m| {
                let s = m.content.clone();
                if s.len() > 120 {
                    format!("{}...", &s[..117])
                } else {
                    s
                }
            })
            .collect();

        Ok(Some(RoomSummary {
            room_id: room.id,
            title: room.title,
            total_memories: memories.len(),
            participants,
            time_range: TimeRange {
                earliest: fmt_time(&earliest),
                latest: fmt_time(&latest),
            },
            clusters: final_clusters,
            top_tags,
            recent_decisions,
            pinned_highlights: pinned,
        }))
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

    /// Generate a structured document from room memories, grouped by memory_type.
    ///
    /// Returns `None` if the room does not exist.
    /// Sections: pinned first, then grouped by type (decision, fact, procedure, preference, context).
    /// Empty sections are omitted.
    pub fn room_document(&self, room_id: &str) -> Result<Option<RoomDocument>, Error> {
        let room = match self.get_room(room_id)? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Get ALL room memories
        let memories = self.recall_room(room_id, None, i32::MAX as usize)?;

        // Get author mapping from room_memories
        let mut author_map: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        {
            let mut stmt = self
                .conn
                .prepare("SELECT memory_id, author FROM room_memories WHERE room_id = ?1")
                .map_err(|e| Error::db("room document authors", e))?;
            let rows: Vec<(String, String)> = stmt
                .query_map(params![room_id], |row| Ok((row.get(0)?, row.get(1)?)))
                .map_err(|e| Error::db("room document authors", e))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| Error::db("room document authors", e))?;
            for (mid, author) in rows {
                author_map.insert(mid, author);
            }
        }

        let fmt_time = |s: &str| -> String { s.get(..19).unwrap_or(s).to_string() };

        let mut sections: Vec<DocumentSection> = Vec::new();

        // 1. Pinned section
        let pinned: Vec<&crate::memory::types::Memory> =
            memories.iter().filter(|m| m.pinned).collect();
        if !pinned.is_empty() {
            let mut entries: Vec<DocumentEntry> = pinned
                .into_iter()
                .map(|m| DocumentEntry {
                    content: m.content.clone(),
                    author: author_map.get(&m.id).cloned().unwrap_or_default(),
                    tags: m.tags.clone(),
                    created_at: fmt_time(&m.created_at.to_rfc3339()),
                })
                .collect();
            entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
            sections.push(DocumentSection {
                heading: "Pinned".to_string(),
                icon: "📌".to_string(),
                entries,
            });
        }

        // 2. Type-based sections
        struct TypeSection {
            key: &'static str,
            heading: &'static str,
            icon: &'static str,
        }
        let type_sections = [
            TypeSection {
                key: "decision",
                heading: "Decisions",
                icon: "📋",
            },
            TypeSection {
                key: "fact",
                heading: "Research & Facts",
                icon: "🔍",
            },
            TypeSection {
                key: "procedure",
                heading: "Procedures",
                icon: "⚙️",
            },
            TypeSection {
                key: "preference",
                heading: "Preferences",
                icon: "🎨",
            },
            TypeSection {
                key: "context",
                heading: "Context & Discussion",
                icon: "💬",
            },
        ];

        for ts in &type_sections {
            let mut matching: Vec<&crate::memory::types::Memory> = memories
                .iter()
                .filter(|m| m.memory_type == ts.key && !m.pinned)
                .collect();
            if matching.is_empty() {
                continue;
            }
            // Sort by importance desc, fallback recency
            matching.sort_by(|a, b| {
                b.importance
                    .partial_cmp(&a.importance)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.created_at.cmp(&a.created_at))
            });
            let entries: Vec<DocumentEntry> = matching
                .into_iter()
                .map(|m| DocumentEntry {
                    content: m.content.clone(),
                    author: author_map.get(&m.id).cloned().unwrap_or_default(),
                    tags: m.tags.clone(),
                    created_at: fmt_time(&m.created_at.to_rfc3339()),
                })
                .collect();
            sections.push(DocumentSection {
                heading: ts.heading.to_string(),
                icon: ts.icon.to_string(),
                entries,
            });
        }

        Ok(Some(RoomDocument {
            room_id: room.id,
            title: room.title,
            generated_at: fmt_time(&chrono::Utc::now().to_rfc3339()),
            sections,
        }))
    }
}
