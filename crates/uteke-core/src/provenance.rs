//! Provenance trust boundaries for consolidated memories (#1089).
//!
//! Research backdrop: memory rewriting during consolidation can launder
//! untrusted observations into apparent user facts ("provenance laundering",
//! arXiv:2607.29167), upgrade hedged remarks into confident assertions
//! ("manufactured confidence", arXiv:2606.29279), and collapse authority at
//! the consolidation boundary (arXiv:2608.01679).
//!
//! This module defines the *policy* layer: trust tiers per source/author,
//! a non-amplification rule (a consolidated record never carries more
//! authority than its weakest source), and a hedging guard that rejects
//! confidence-upgrading rewrites. The LLM consolidation executor (follow-up
//! to the measure-only planner) must consult these rules before writing.

use crate::error::Error;
use crate::memory::types::Memory;
use serde::{Deserialize, Serialize};

/// Trust tier of a memory's provenance. Higher = more authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrustTier {
    /// Direct human statement (author_type=human, source_type=user).
    Human = 3,
    /// Agent-derived knowledge (author_type=agent, source_type=user/system).
    Agent = 2,
    /// Attested external observation (url, file, import).
    External = 1,
    /// Unknown or "derived" provenance — the floor.
    Untrusted = 0,
}

impl TrustTier {
    /// Classify a memory by its provenance fields.
    ///
    /// Non-amplification starts here: a memory recorded from an external
    /// observation never classifies above [`TrustTier::External`], no matter
    /// what its content claims.
    pub fn of(memory: &Memory) -> Self {
        match (memory.author_type.as_str(), memory.source_type.as_str()) {
            ("human", "user") => TrustTier::Human,
            ("agent", "user") | ("agent", "system") => TrustTier::Agent,
            (_, "url") | (_, "file") | (_, "import") => TrustTier::External,
            (_, "derived") | (_, "unknown") => TrustTier::Untrusted,
            // Anything else (future combos) is treated conservatively.
            _ => TrustTier::Untrusted,
        }
    }
}

/// Hedging markers whose *presence* in a source must survive consolidation.
/// A consolidated record that drops them while keeping the claim is a
/// confidence upgrade (manufactured confidence).
const HEDGE_MARKERS: &[&str] = &[
    "maybe",
    "perhaps",
    "possibly",
    "probably",
    "seems",
    "appears",
    "not sure",
    "unsure",
    "might",
    "could be",
    "unclear",
    "possibly",
    "mungkin",
    "sepertinya",
    "kayaknya",
    "kurang yakin",
    "belum pasti",
];

/// Provenance policy checks for consolidation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProvenancePolicy;

impl ProvenancePolicy {
    /// Non-amplification: the tier a consolidated record derived from
    /// `sources` is allowed to claim — always the weakest source tier.
    pub fn allowed_tier(sources: &[&Memory]) -> TrustTier {
        sources
            .iter()
            .map(|m| TrustTier::of(m))
            .min()
            .unwrap_or(TrustTier::Untrusted)
    }

    /// Detect a confidence upgrade: source memories hedge, but the
    /// consolidated output does not.
    ///
    /// Returns true when ANY source contains a hedge marker and NONE of the
    /// markers survive in `consolidated`. This is a coarse lexical check —
    /// the executor should regenerate (or downgrade) rather than store.
    pub fn confidence_upgraded(sources: &[&Memory], consolidated: &str) -> bool {
        let consolidated_lower = consolidated.to_lowercase();
        let any_hedged = sources
            .iter()
            .any(|m| contains_hedge(&m.content.to_lowercase()));
        let hedging_preserved = HEDGE_MARKERS.iter().any(|h| consolidated_lower.contains(h));
        any_hedged && !hedging_preserved
    }

    /// Validate a proposed consolidated record before it is written.
    ///
    /// Enforces, per #1089:
    /// 1. Provenance laundering — external/untrusted sources may not be
    ///    rewritten as human/user facts (`author_type=human, source_type=user`).
    /// 2. Manufactured confidence — hedged sources must stay hedged.
    pub fn validate_output(sources: &[&Memory], output: &Memory) -> Result<(), Error> {
        let allowed = Self::allowed_tier(sources);
        let claimed = TrustTier::of(output);
        if claimed > allowed {
            return Err(Error::Validation(format!(
                "provenance non-amplification violated: consolidated record claims \
                 tier {claimed:?} but weakest source tier is {allowed:?}"
            )));
        }
        if Self::confidence_upgraded(sources, &output.content) {
            return Err(Error::Validation(
                "manufactured confidence: hedged source consolidated into an \
                 unhedged assertion — preserve uncertainty markers"
                    .into(),
            ));
        }
        Ok(())
    }
}

fn contains_hedge(lower: &str) -> bool {
    HEDGE_MARKERS.iter().any(|h| lower.contains(h))
}

// ── Provenance chain query (#1172 Fase 1) ──────────────────────────────────

/// Full provenance view of a memory (#1172 Fase 1): identity + provenance
/// fields + trust tier + content hash + the auditable event chain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceReport {
    pub id: String,
    pub namespace: String,
    pub created_at: String,
    pub updated_at: String,
    pub author_type: String,
    pub source: Option<String>,
    pub source_type: String,
    /// SHA-256 of the content at the last write (schema v18+). `None` for
    /// memories written before the column existed.
    pub source_hash: Option<String>,
    /// Content currently stored — lets an auditor recompute the hash and
    /// verify tamper-evidence.
    pub content: String,
    /// Hash of the current content (computed live) — compare against
    /// `source_hash` to detect post-write modifications.
    pub content_hash_now: String,
    pub trust_tier: TrustTier,
    pub deprecated: bool,
    /// Event chain, newest first, with actor + evidence when recorded.
    pub events: Vec<crate::timeline::TimelineEvent>,
}

impl crate::Uteke {
    /// Build the full provenance report for a memory (#1172 Fase 1).
    ///
    /// Returns `Ok(None)` when the memory does not exist.
    pub fn provenance(&self, id: &str) -> Result<Option<ProvenanceReport>, crate::Error> {
        use sha2::Digest;

        let memory = match self.store.get_by_id(id)? {
            Some(m) => m,
            None => return Ok(None),
        };
        let source_hash = self.store.get_source_hash(id)?;
        let content_hash_now: String = sha2::Sha256::digest(memory.content.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let events = self.store.list_timeline_events(id, 0)?;
        let trust_tier = TrustTier::of(&memory);
        Ok(Some(ProvenanceReport {
            id: memory.id.clone(),
            namespace: memory.namespace.clone(),
            created_at: memory.created_at.to_rfc3339(),
            updated_at: memory.updated_at.to_rfc3339(),
            author_type: memory.author_type.clone(),
            source: memory.source.clone(),
            source_type: memory.source_type.clone(),
            source_hash,
            content: memory.content,
            content_hash_now,
            trust_tier,
            deprecated: memory.deprecated,
            events,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(author: &str, source_type: &str, content: &str) -> crate::Memory {
        crate::Memory {
            id: uuid_v4(),
            content: content.to_string(),
            embedding: vec![],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: 0,
            recall_count: 0,
            last_accessed: None,
            deprecated: false,
            deprecated_at: None,
            valid_from: None,
            valid_until: None,
            memory_type: "fact".to_string(),
            importance: 0.5,
            pinned: false,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: source_type.to_string(),
            author_type: author.to_string(),
        }
    }

    fn uuid_v4() -> String {
        // Deterministic-enough unique id for tests.
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        format!("test-{}", N.fetch_add(1, Ordering::SeqCst))
    }

    #[test]
    fn tier_classification() {
        assert_eq!(TrustTier::of(&mem("human", "user", "x")), TrustTier::Human);
        assert_eq!(TrustTier::of(&mem("agent", "user", "x")), TrustTier::Agent);
        assert_eq!(
            TrustTier::of(&mem("agent", "url", "x")),
            TrustTier::External
        );
        assert_eq!(
            TrustTier::of(&mem("agent", "derived", "x")),
            TrustTier::Untrusted
        );
    }

    #[test]
    fn non_amplification_takes_weakest() {
        let strong = mem("human", "user", "user said");
        let weak = mem("agent", "url", "from a blog");
        assert_eq!(
            ProvenancePolicy::allowed_tier(&[&strong, &weak]),
            TrustTier::External
        );
    }

    #[test]
    fn laundering_rejected() {
        let src = mem("agent", "url", "blog claims revenue grew");
        let mut out = mem("human", "user", "our revenue grew");
        assert!(ProvenancePolicy::validate_output(&[&src], &out).is_err());

        // Same claim kept at external tier is fine.
        out.author_type = "agent".into();
        out.source_type = "url".into();
        assert!(ProvenancePolicy::validate_output(&[&src], &out).is_ok());
    }

    #[test]
    fn confidence_upgrade_rejected() {
        let src = mem("human", "user", "mungkin rilis besok");
        let out = mem("human", "user", "rilis besok");
        let err = ProvenancePolicy::validate_output(&[&src], &out).unwrap_err();
        assert!(err.to_string().contains("confidence"));

        // Hedge preserved -> valid.
        let ok_out = mem("human", "user", "user said rilis mungkin besok");
        assert!(ProvenancePolicy::validate_output(&[&src], &ok_out).is_ok());
    }

    #[test]
    fn unhedged_sources_consolidate_freely() {
        let src = mem("human", "user", "uses bun");
        let out = mem("human", "user", "project uses bun");
        assert!(ProvenancePolicy::validate_output(&[&src], &out).is_ok());
    }
}
