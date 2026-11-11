//! Room search (#1704): find a room from a topic.
//!
//! `room list` returns every room with no filter or ranking, which is not a
//! discovery tool on a store with hundreds of rooms (many of them untitled and
//! undescribed). Two signals are fused with the reciprocal-rank fusion the
//! hybrid recall already uses:
//!
//! 1. **Metadata**: the query terms against the room id, title and
//!    description (a term in the id or title counts double).
//! 2. **Content**: the memories a recall of the query finds, grouped by the
//!    rooms they belong to (#1703); a room with more and better hits ranks
//!    higher. This finds a room whose memories match even when its title and
//!    description say nothing.
//!
//! No new storage and no schema change: the rooms are scanned in memory (a
//! store has hundreds, not millions) and the content arm reuses the existing
//! recall indexes.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::memory::types::RecallStrategy;

/// Results returned when the caller gives no limit (or 0).
pub const ROOM_SEARCH_DEFAULT_LIMIT: usize = 10;
/// The cap on `limit`.
pub const ROOM_SEARCH_MAX_LIMIT: usize = 100;

/// Reciprocal-rank-fusion constant, the same as hybrid recall.
const RRF_K: f64 = 60.0;
/// Memory hits the content arm looks at.
const CONTENT_HITS: usize = 50;
/// The vector similarity a memory needs to count as a content match: the
/// default of `recall --strict` (`[recall] min_score_strict`).
const SIMILARITY_FLOOR: f32 = 0.5;
/// Matching memories shown per room.
const MATCHES_PER_ROOM: usize = 2;
/// Characters of a matching memory shown.
const SNIPPET_CHARS: usize = 160;

/// A memory of a room that matched the query, shown so the caller can check
/// the match before opening the room.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomMatch {
    /// The memory id.
    pub memory_id: String,
    /// The start of its content.
    pub snippet: String,
}

/// A room found by [`crate::Uteke::search_rooms`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoomSearchHit {
    /// The room id.
    pub room_id: String,
    /// The room title, or the id when it has none.
    pub room_name: String,
    /// The room description, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Live (not deprecated) memories in the room.
    pub memory_count: usize,
    /// When the room was last updated (RFC 3339).
    pub updated_at: String,
    /// Fused rank score; only its order is meaningful.
    pub score: f32,
    /// Up to two memories of the room that matched, best first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub matches: Vec<RoomMatch>,
}

/// Lowercased alphanumeric terms of `text` (`disc:tole-naik` gives `disc`,
/// `tole`, `naik`), without duplicates and in order of appearance.
fn terms(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in text
        .to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        if !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Metadata score of a room for the query terms: 2 per term found in the id
/// or title, 1 per term found only in the description.
fn metadata_score(query_terms: &[String], room: &crate::Room) -> u32 {
    let named = format!("{} {}", room.id, room.title.as_deref().unwrap_or("")).to_lowercase();
    let described = room.description.as_deref().unwrap_or("").to_lowercase();
    query_terms
        .iter()
        .map(|t| {
            if named.contains(t.as_str()) {
                2
            } else if described.contains(t.as_str()) {
                1
            } else {
                0
            }
        })
        .sum()
}

fn snippet(content: &str) -> String {
    let flat: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= SNIPPET_CHARS {
        flat
    } else {
        let cut: String = flat.chars().take(SNIPPET_CHARS).collect();
        format!("{cut}…")
    }
}

impl crate::Uteke {
    /// Find rooms for `query` (#1704).
    ///
    /// `namespace = None` searches every room; with a namespace only the
    /// rooms of that namespace can be returned and the content arm only looks
    /// at memories of it. `limit` 0 means [`ROOM_SEARCH_DEFAULT_LIMIT`]; it is
    /// capped at [`ROOM_SEARCH_MAX_LIMIT`]. A blank query is an error. The
    /// result is deterministic for the same store: ties break by room id.
    pub fn search_rooms(
        &self,
        query: &str,
        limit: usize,
        namespace: Option<&str>,
    ) -> Result<Vec<RoomSearchHit>, Error> {
        let query = query.trim();
        if query.is_empty() {
            return Err(Error::Validation("query must not be blank".to_string()));
        }
        let limit = if limit == 0 {
            ROOM_SEARCH_DEFAULT_LIMIT
        } else {
            limit.min(ROOM_SEARCH_MAX_LIMIT)
        };

        let rooms: BTreeMap<String, crate::Room> = self
            .list_rooms(namespace)?
            .into_iter()
            .map(|r| (r.id.clone(), r))
            .collect();
        if rooms.is_empty() {
            return Ok(Vec::new());
        }

        // Arm 1: metadata. Best score first, ties by room id (the map order).
        let query_terms = terms(query);
        let mut by_metadata: Vec<(&String, u32)> = rooms
            .iter()
            .map(|(id, room)| (id, metadata_score(&query_terms, room)))
            .filter(|(_, score)| *score > 0)
            .collect();
        by_metadata.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));

        // Arm 2: content. Group the hits of a recall by the rooms of their
        // memories, summing the reciprocal ranks of the hits.
        let mut hits = self.recall_hybrid(
            query,
            CONTENT_HITS,
            None,
            namespace,
            RecallStrategy::Fusion,
            0.0,
        )?;
        // Recall returns the nearest memories even when nothing is close, so
        // keep only the confident ones: a vector similarity at the
        // `recall --strict` floor, or an exact keyword match (the full-text
        // leg, which has no similarity).
        let keyword_ids: std::collections::HashSet<String> = self
            .recall_hybrid(
                query,
                CONTENT_HITS,
                None,
                namespace,
                RecallStrategy::Fts5,
                0.0,
            )?
            .into_iter()
            .map(|h| h.memory.id)
            .collect();
        hits.retain(|h| {
            h.similarity.is_some_and(|s| s >= SIMILARITY_FLOOR)
                || keyword_ids.contains(&h.memory.id)
        });
        self.attach_rooms(&mut hits)?;
        let mut content_score: HashMap<String, f64> = HashMap::new();
        let mut matches: HashMap<String, Vec<RoomMatch>> = HashMap::new();
        for (rank, hit) in hits.iter().enumerate() {
            let rr = 1.0 / (RRF_K + rank as f64 + 1.0);
            for room in &hit.rooms {
                if !rooms.contains_key(&room.room_id) {
                    continue; // another scope's room
                }
                *content_score.entry(room.room_id.clone()).or_default() += rr;
                let shown = matches.entry(room.room_id.clone()).or_default();
                if shown.len() < MATCHES_PER_ROOM {
                    shown.push(RoomMatch {
                        memory_id: hit.memory.id.clone(),
                        snippet: snippet(&hit.memory.content),
                    });
                }
            }
        }
        let mut by_content: Vec<(&String, f64)> = rooms
            .keys()
            .filter_map(|id| content_score.get(id).map(|s| (id, *s)))
            .collect();
        by_content.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(b.0))
        });

        // Fuse the two rankings.
        let mut fused: HashMap<&String, f64> = HashMap::new();
        for (rank, (id, _)) in by_metadata.iter().enumerate() {
            *fused.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
        for (rank, (id, _)) in by_content.iter().enumerate() {
            *fused.entry(*id).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        }
        let mut ranked: Vec<(&String, f64)> = fused.into_iter().collect();
        ranked.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(b.0))
        });
        ranked.truncate(limit);

        ranked
            .into_iter()
            .map(|(id, score)| {
                let room = &rooms[id];
                let name = room
                    .title
                    .as_deref()
                    .map(str::trim)
                    .filter(|t| !t.is_empty())
                    .unwrap_or(&room.id)
                    .to_string();
                Ok(RoomSearchHit {
                    room_id: room.id.clone(),
                    room_name: name,
                    description: room.description.clone().filter(|d| !d.trim().is_empty()),
                    memory_count: self.store.get_room_memory_ids(&room.id, None)?.len(),
                    updated_at: room.updated_at.clone(),
                    score: score as f32,
                    matches: matches.remove(id).unwrap_or_default(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::open_keyword;

    fn store() -> crate::Uteke {
        let u = open_keyword();
        // Matches by title only: its memories say nothing about the topic.
        u.create_room("planning", Some("Quarterly budget planning"), "default")
            .unwrap();
        u.remember_in_room(
            "unrelated note about lunch",
            &[],
            None,
            None,
            "fact",
            "planning",
            "a",
        )
        .unwrap();
        // Matches by content only: no title, no description.
        u.remember_in_room(
            "the budget was approved for the new office",
            &[],
            None,
            None,
            "fact",
            "misc-notes",
            "a",
        )
        .unwrap();
        // Matches nothing.
        u.remember_in_room(
            "tulips bloom in spring",
            &[],
            None,
            None,
            "fact",
            "garden",
            "a",
        )
        .unwrap();
        u
    }

    #[test]
    fn a_title_match_and_a_content_only_match_are_both_found() {
        let u = store();
        let hits = u.search_rooms("budget", 10, None).unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.room_id.as_str()).collect();
        assert!(ids.contains(&"planning"), "title match: {ids:?}");
        assert!(
            ids.contains(&"misc-notes"),
            "a room with no title or description is found by its memories: {ids:?}"
        );
        assert!(!ids.contains(&"garden"), "no match: {ids:?}");
    }

    #[test]
    fn a_hit_names_the_room_and_shows_the_matching_memory() {
        let u = store();
        let hits = u.search_rooms("budget", 10, None).unwrap();
        let notes = hits.iter().find(|h| h.room_id == "misc-notes").unwrap();
        assert_eq!(
            notes.room_name, "misc-notes",
            "an untitled room shows its id"
        );
        assert_eq!(notes.memory_count, 1);
        assert_eq!(notes.matches.len(), 1);
        assert!(notes.matches[0].snippet.contains("budget"));
        let planning = hits.iter().find(|h| h.room_id == "planning").unwrap();
        assert_eq!(planning.room_name, "Quarterly budget planning");
        assert!(planning.matches.is_empty(), "no memory of it matched");
    }

    #[test]
    fn a_room_matching_on_both_signals_ranks_first() {
        let u = open_keyword();
        u.create_room("budget-2026", Some("Budget 2026"), "default")
            .unwrap();
        u.remember_in_room(
            "budget limits agreed for 2026",
            &[],
            None,
            None,
            "fact",
            "budget-2026",
            "a",
        )
        .unwrap();
        u.remember_in_room(
            "the budget was mentioned once",
            &[],
            None,
            None,
            "fact",
            "other",
            "a",
        )
        .unwrap();
        let hits = u.search_rooms("budget", 10, None).unwrap();
        assert_eq!(hits[0].room_id, "budget-2026");
        assert!(hits[0].score > hits[1].score);
    }

    #[test]
    fn the_description_counts_and_the_limit_and_blank_query_are_enforced() {
        let u = open_keyword();
        for i in 0..5 {
            let id = format!("r{i}");
            u.create_room(&id, None, "default").unwrap();
            u.update_room(&id, None, Some("decisions about the migration"))
                .unwrap();
        }
        let hits = u.search_rooms("migration", 3, None).unwrap();
        assert_eq!(hits.len(), 3);
        assert_eq!(
            hits[0].description.as_deref(),
            Some("decisions about the migration")
        );
        // Deterministic: the same call, the same order; ties by room id.
        let again = u.search_rooms("migration", 3, None).unwrap();
        assert_eq!(hits, again);
        assert_eq!(hits[0].room_id, "r0");
        assert!(matches!(
            u.search_rooms("   ", 5, None),
            Err(Error::Validation(_))
        ));
        // 0 means the default, not "none".
        assert_eq!(u.search_rooms("migration", 0, None).unwrap().len(), 5);
    }

    #[test]
    fn deprecated_memories_are_not_counted() {
        let u = open_keyword();
        let id = u
            .remember_in_room("quokka facts", &[], None, None, "fact", "wild", "a")
            .unwrap()
            .id;
        u.remember_in_room("more quokka facts", &[], None, None, "fact", "wild", "a")
            .unwrap();
        u.forget(&id).unwrap();
        let hits = u.search_rooms("quokka", 5, None).unwrap();
        assert_eq!(hits[0].room_id, "wild");
        assert_eq!(hits[0].memory_count, 1);
    }
}

/// The query terms (#1729).
#[cfg(test)]
mod terms_tests {
    use super::terms;

    #[test]
    fn terms_are_lowercase_alphanumeric_without_duplicates_in_order() {
        assert_eq!(
            terms("Disc:tole-naik DISC naik extra"),
            vec!["disc", "tole", "naik", "extra"]
        );
        assert!(terms("  --  ").is_empty());
    }
}
