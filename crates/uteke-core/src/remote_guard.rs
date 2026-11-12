//! Cost guard of the remote routes that embed a client-supplied string.
//!
//! A read-only token may call `POST /doc/similar` and `/skill/similar` with a
//! draft `text` and `POST /skill/search`, and their MCP tools, which is the
//! heaviest request it can make: the string is chunked and embedded. Two bounds
//! keep that cheap for the server:
//!
//! - a draft text is at most [`MAX_REMOTE_DRAFT_CHARS`] characters (the CLI
//!   keeps its own, larger limit);
//! - at most [`MAX_REMOTE_EMBEDS`] such requests embed at the same time per
//!   [`crate::Uteke`]; another one is refused at once with [`Error::Busy`]
//!   (HTTP 429, an MCP tool error) instead of queueing, so a burst cannot pile
//!   up threads and memory.
//!
//! A server that serialises requests on a store lock must take its slot
//! BEFORE the lock (a gate of its own, see [`EmbedGate`]): inside the lock
//! at most one request runs, so the per-[`crate::Uteke`] gate never trips.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::error::Error;

/// Longest draft text a remote caller may send to a similarity route, in
/// characters. A typical `SKILL.md` or note is well below it.
pub const MAX_REMOTE_DRAFT_CHARS: usize = 32_768;

/// Remote embedding requests that may run at the same time.
pub const MAX_REMOTE_EMBEDS: usize = 2;

/// Counts the remote embedding requests in flight; one per [`crate::Uteke`].
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
    /// A slot for one remote request that embeds a string (a skill search in
    /// `hybrid` or `semantic` mode). Hold it until the answer is built.
    ///
    /// # Errors
    ///
    /// `Busy` when [`MAX_REMOTE_EMBEDS`] such requests are already running.
    pub fn remote_embed_permit(&self) -> Result<EmbedPermit, Error> {
        self.remote_embeds.try_acquire()
    }

    /// Check a remote draft text and take a slot for embedding it. The length
    /// is checked first, without counting past the limit, so an oversized text
    /// is a 400 whether or not the server is busy.
    ///
    /// # Errors
    ///
    /// `Validation` over [`MAX_REMOTE_DRAFT_CHARS`] characters, `Busy` when
    /// the slots are taken.
    pub fn remote_draft_permit(&self, text: &str) -> Result<EmbedPermit, Error> {
        check_draft_len(text)?;
        self.remote_embed_permit()
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
    fn the_defaults_are_pinned() {
        assert_eq!(MAX_REMOTE_DRAFT_CHARS, 32_768);
        assert_eq!(MAX_REMOTE_EMBEDS, 2);
    }

    #[test]
    fn a_draft_is_bounded_at_the_cap_in_characters_before_the_gate() {
        let u = crate::test_support::open_keyword();
        let at = "é".repeat(MAX_REMOTE_DRAFT_CHARS);
        assert!(u.remote_draft_permit(&at).is_ok());
        let over = "é".repeat(MAX_REMOTE_DRAFT_CHARS + 1);
        assert_eq!(
            u.remote_draft_permit(&over).unwrap_err().to_string(),
            "Validation error: text is longer than 32768 characters"
        );
        // Oversized is a 400 even when every slot is taken.
        let _p1 = u.remote_embed_permit().unwrap();
        let _p2 = u.remote_embed_permit().unwrap();
        assert_eq!(
            u.remote_draft_permit("x").unwrap_err().class(),
            crate::ErrorClass::Busy
        );
        assert_eq!(
            u.remote_draft_permit(&over).unwrap_err().class(),
            crate::ErrorClass::BadRequest
        );
    }
}
