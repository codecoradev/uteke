//! Cost guard of the remote routes that embed a client-supplied string.
//!
//! A read-only token may call `POST /doc/similar` and `/skill/similar` with a
//! draft `text` and `POST /skill/search`, and their MCP tools, which is the
//! heaviest request it can make: the string is chunked and embedded. Two bounds
//! keep that cheap for the server:
//!
//! - a draft text is at most [`MAX_REMOTE_DRAFT_CHARS`] characters (the CLI
//!   keeps its own, larger limit);
//! - at most [`MAX_REMOTE_EMBEDS`] such requests embed at the same time; another
//!   one is refused at once with [`Error::Busy`] (HTTP 429, an MCP tool error)
//!   instead of queueing, so a burst cannot pile up threads and memory.
//!
//! The core answers "does this request embed a client string?" itself, from
//! the validation that decides the request: [`EmbedNeed`] is returned by
//! [`crate::skill_remote::SkillRemoteArgs::embed_need`],
//! [`crate::doc_similarity::DocSimilarQuery::embed_need`] and
//! [`EmbedNeed::for_search_mode`]. A transport asks once, then takes its slot
//! with [`EmbedGate::admit`]. There is ONE gate per [`crate::Uteke`]
//! ([`crate::Uteke::embed_gate`]); the gate is a cheap clone, so a server that
//! serialises requests on a store lock hands a clone to its router and takes
//! the slot BEFORE the lock (inside it at most one request runs, so a gate
//! checked there could never refuse a burst). The core entry points
//! (`skill_remote`, `doc_similarity_query`) take no slot themselves.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Error;

/// Longest draft text a remote caller may send to a similarity route, in
/// characters. A typical `SKILL.md` or note is well below it.
pub const MAX_REMOTE_DRAFT_CHARS: usize = 32_768;

/// Remote embedding requests that may run at the same time.
pub const MAX_REMOTE_EMBEDS: usize = 2;

/// What a request embeds: the one answer the core gives to "does this request
/// embed a client string?".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedNeed<'a> {
    /// A search query (bounded by the query limit of its route:
    /// [`EmbedNeed::for_doc_search`], or the skill search validation).
    Query,
    /// A draft text, at most [`MAX_REMOTE_DRAFT_CHARS`] characters.
    Draft(&'a str),
}

impl EmbedNeed<'_> {
    /// A document or skill search in `mode`: every mode but `fts` embeds the
    /// query (an unknown mode falls back to `hybrid`, which does).
    pub fn for_search_mode(mode: &str) -> Option<Self> {
        (mode != "fts").then_some(Self::Query)
    }

    /// A remote document search: the query is at most
    /// [`crate::skill_library::MAX_QUERY_CHARS`] characters in every mode (the
    /// same bound as a remote skill search), then [`Self::for_search_mode`].
    /// Without the bound a 10 MB `query` was embedded (or matched) under a slot.
    ///
    /// # Errors
    ///
    /// `Validation` when the query is longer than the limit.
    pub fn for_doc_search(query: &str, mode: &str) -> Result<Option<Self>, Error> {
        let max = crate::skill_library::MAX_QUERY_CHARS;
        if query.chars().take(max + 1).count() > max {
            return Err(Error::validation(format!(
                "query is longer than {max} characters"
            )));
        }
        Ok(Self::for_search_mode(mode))
    }
}

/// Counts the remote embedding requests in flight; one per [`crate::Uteke`],
/// shared by its clones.
#[derive(Debug, Clone)]
pub struct EmbedGate {
    in_flight: Arc<AtomicUsize>,
    limit: usize,
}

/// A running remote embedding request; dropping it frees its slot, also when
/// the request fails or panics.
#[derive(Debug)]
pub struct EmbedPermit(Arc<AtomicUsize>);

impl Drop for EmbedPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

impl EmbedGate {
    pub fn new(limit: usize) -> Self {
        Self {
            in_flight: Arc::new(AtomicUsize::new(0)),
            limit,
        }
    }

    /// Requests holding a slot right now.
    pub fn in_flight(&self) -> usize {
        self.in_flight.load(Ordering::Acquire)
    }

    /// Take a slot, or `Busy` when `limit` requests already embed.
    pub fn try_acquire(&self) -> Result<EmbedPermit, Error> {
        let mut current = self.in_flight.load(Ordering::Acquire);
        loop {
            if current >= self.limit {
                return Err(Error::Busy(format!(
                    "busy: {} embedding requests are already running; retry in a moment",
                    self.limit
                )));
            }
            match self.in_flight.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return Ok(EmbedPermit(Arc::clone(&self.in_flight))),
                Err(seen) => current = seen,
            }
        }
    }
}

impl EmbedGate {
    /// Bound the request and take its slot: `None` for a request that embeds
    /// nothing. A draft over the cap is `Validation` (a 400) whether or not
    /// the slots are taken, then `Busy` when they are.
    ///
    /// # Errors
    ///
    /// `Validation` for an oversized draft, `Busy` when the slots are taken.
    pub fn admit(&self, need: Option<EmbedNeed<'_>>) -> Result<Option<EmbedPermit>, Error> {
        let Some(need) = need else {
            return Ok(None);
        };
        if let EmbedNeed::Draft(text) = need {
            check_draft_len(text)?;
        }
        self.try_acquire().map(Some)
    }
}

impl Default for EmbedGate {
    fn default() -> Self {
        Self::new(MAX_REMOTE_EMBEDS)
    }
}

/// Refuse a remote draft text over [`MAX_REMOTE_DRAFT_CHARS`] characters,
/// counting no further than the limit.
///
/// # Errors
///
/// `Validation` when the text is longer than the limit.
pub fn check_draft_len(text: &str) -> Result<(), Error> {
    if text.chars().take(MAX_REMOTE_DRAFT_CHARS + 1).count() > MAX_REMOTE_DRAFT_CHARS {
        return Err(Error::validation(format!(
            "text is longer than {MAX_REMOTE_DRAFT_CHARS} characters"
        )));
    }
    Ok(())
}

impl crate::Uteke {
    /// The gate of the remote requests that embed a client string. One per
    /// [`crate::Uteke`]; clone it to take slots from another thread or before
    /// a lock around the `Uteke`.
    pub fn embed_gate(&self) -> &EmbedGate {
        &self.remote_embeds
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_admits_exactly_its_limit_and_frees_a_slot_on_drop() {
        let gate = EmbedGate::new(2);
        let a = gate.try_acquire().unwrap();
        let _b = gate.try_acquire().unwrap();
        let e = gate.try_acquire().unwrap_err();
        assert_eq!(
            e.to_string(),
            "busy: 2 embedding requests are already running; retry in a moment"
        );
        assert_eq!(e.class(), crate::ErrorClass::Busy);
        drop(a);
        assert!(gate.try_acquire().is_ok());
    }

    #[test]
    fn in_flight_counts_the_permits_held_right_now() {
        let gate = EmbedGate::new(2);
        assert_eq!(gate.in_flight(), 0);
        let a = gate.try_acquire().unwrap();
        assert_eq!(gate.in_flight(), 1);
        let b = gate.try_acquire().unwrap();
        assert_eq!(gate.in_flight(), 2);
        drop(a);
        assert_eq!(gate.in_flight(), 1);
        drop(b);
        assert_eq!(gate.in_flight(), 0);
    }

    #[test]
    fn the_defaults_are_pinned() {
        assert_eq!(MAX_REMOTE_DRAFT_CHARS, 32_768);
        assert_eq!(MAX_REMOTE_EMBEDS, 2);
    }

    #[test]
    fn a_draft_is_bounded_at_the_cap_in_characters_before_the_gate() {
        let gate = EmbedGate::default();
        let at = "é".repeat(MAX_REMOTE_DRAFT_CHARS);
        assert!(gate.admit(Some(EmbedNeed::Draft(&at))).is_ok());
        let over = "é".repeat(MAX_REMOTE_DRAFT_CHARS + 1);
        assert_eq!(
            gate.admit(Some(EmbedNeed::Draft(&over)))
                .unwrap_err()
                .to_string(),
            "Validation error: text is longer than 32768 characters"
        );
        // Oversized is a 400 even when every slot is taken.
        let _p1 = gate.try_acquire().unwrap();
        let _p2 = gate.try_acquire().unwrap();
        assert_eq!(
            gate.admit(Some(EmbedNeed::Draft("x"))).unwrap_err().class(),
            crate::ErrorClass::Busy
        );
        assert_eq!(
            gate.admit(Some(EmbedNeed::Draft(&over)))
                .unwrap_err()
                .class(),
            crate::ErrorClass::BadRequest
        );
        // A request that embeds nothing needs no slot.
        assert!(gate.admit(None).unwrap().is_none());
    }

    #[test]
    fn a_clone_of_the_gate_counts_in_the_same_slots() {
        let u = crate::test_support::open_keyword();
        let clone = u.embed_gate().clone();
        let _a = clone.admit(Some(EmbedNeed::Query)).unwrap();
        let _b = u.embed_gate().admit(Some(EmbedNeed::Query)).unwrap();
        assert_eq!(
            clone.admit(Some(EmbedNeed::Query)).unwrap_err().class(),
            crate::ErrorClass::Busy
        );
    }

    #[test]
    fn a_remote_doc_search_query_is_bounded_in_every_mode() {
        let max = crate::skill_library::MAX_QUERY_CHARS;
        let at = "a".repeat(max);
        let over = "a".repeat(max + 1);
        let multi = "\u{e9}".repeat(max);
        for mode in ["fts", "hybrid", "semantic"] {
            assert!(EmbedNeed::for_doc_search(&at, mode).is_ok(), "{mode}");
            assert!(EmbedNeed::for_doc_search(&multi, mode).is_ok(), "{mode}");
            let err = EmbedNeed::for_doc_search(&over, mode).unwrap_err();
            assert_eq!(err.class(), crate::ErrorClass::BadRequest, "{mode}");
            assert_eq!(
                err.to_string(),
                format!("Validation error: query is longer than {max} characters")
            );
        }
        assert_eq!(EmbedNeed::for_doc_search("q", "fts").unwrap(), None);
        assert_eq!(
            EmbedNeed::for_doc_search("q", "hybrid").unwrap(),
            Some(EmbedNeed::Query)
        );
    }

    #[test]
    fn only_a_search_mode_other_than_fts_embeds() {
        assert_eq!(EmbedNeed::for_search_mode("fts"), None);
        for m in ["hybrid", "semantic", "anything"] {
            assert_eq!(EmbedNeed::for_search_mode(m), Some(EmbedNeed::Query));
        }
    }
}
