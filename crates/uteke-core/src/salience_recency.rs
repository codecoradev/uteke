//! Salience + recency dual-axis recall ranking (#352).
//!
//! Two orthogonal boost functions that can be turned on per-query via
//! `--salience` / `--recency` CLI flags (or the equivalent API params).
//! Both are computed from existing memory fields — **zero LLM**.
//!
//! ## Salience (mattering)
//!
//! Combines `access_count`, `importance`, and `pinned` into a 0..=1 score.
//! Well-accessed, important, pinned memories drift upward; cold, low-signal
//! memories drift downward.
//!
//! ## Recency (freshness)
//!
//! Per-type exponential decay. Different memory types age at different rates
//! (Decisions and Preferences are evergreen, Events decay fast).
//!
//! Both axes are **additive boosts** capped to never dominate embedding
//! similarity — they act as tie-breakers between otherwise similar results.

use crate::memory::types::Memory;

/// Per-query salience/recency boost weights (#352).
///
/// All weights default to `0.0` so the feature is opt-in per query.
/// `salience_weight` and `recency_weight` are applied additively to the
/// final recall score after embedding similarity is computed.
#[derive(Debug, Clone, Copy)]
pub struct SalienceRecencyConfig {
    /// Weight for the salience boost (0.0 = off, 0.15 = default when enabled).
    pub salience_weight: f32,
    /// Weight for the recency boost (0.0 = off, 0.15 = default when enabled).
    pub recency_weight: f32,
}

impl Default for SalienceRecencyConfig {
    fn default() -> Self {
        Self {
            salience_weight: 0.0,
            recency_weight: 0.0,
        }
    }
}

impl SalienceRecencyConfig {
    /// Clamp weights to a sane [0.0, 1.0] range so misconfigured stores can't
    /// amplify the boost beyond the embedding-similarity signal.
    pub fn sanitized(self) -> Self {
        Self {
            salience_weight: self.salience_weight.clamp(0.0, 1.0),
            recency_weight: self.recency_weight.clamp(0.0, 1.0),
        }
    }

    /// True when neither axis is active — recall scoring is unchanged.
    pub fn is_noop(self) -> bool {
        self.salience_weight == 0.0 && self.recency_weight == 0.0
    }
}

/// Compute the salience score for a memory (0.0..=1.0).
///
/// Salience is "how much this memory matters": well-accessed, important,
/// pinned memories score higher. Composed from three signals:
///
/// - `access_freq` — log-normalized access count (0 at 0 accesses, ~1 at 1000).
/// - `importance` — the memory's stored importance score (0.0..=1.0).
/// - `pinned_bonus` — flat +0.2 for pinned memories (they never decay).
///
/// The three are weighted and clamped to 1.0.
pub fn salience_score(memory: &Memory) -> f32 {
    // access_count is u32; log10(0) is -inf, so guard with max(1).
    let access_freq = ((memory.access_count.max(1) as f32).log10() / 3.0).clamp(0.0, 1.0);
    let importance = memory.importance as f32;
    let pinned_bonus = if memory.pinned { 0.2 } else { 0.0 };

    // Weighted blend. Importance is the dominant signal (it's user-tuned);
    // access frequency is secondary; pinned is a flat lift.
    let blended = importance * 0.5 + access_freq * 0.3 + pinned_bonus;
    blended.clamp(0.0, 1.0)
}

/// Per-memory-type time constant (τ) in days for exponential decay (#352).
///
/// Evergreen types (Decision, Preference) decay slowly; time-bound types
/// (Event) decay fast. This is the exponential time constant τ at which
/// recency drops to 1/e (~0.37) of full freshness. (For a true half-life
/// — the age at which recency is exactly 0.5 — multiply by ln(2) ≈ 0.693.)
pub fn type_half_life_days(memory_type: &str) -> f32 {
    match memory_type {
        "decision" | "preference" => 365.0,
        "fact" | "reference" => 180.0,
        "event" => 30.0,
        "insight" => 240.0,
        // Default for note, procedure, context, and anything unknown.
        _ => 90.0,
    }
}

/// Compute the recency score for a memory (0.0..=1.0).
///
/// Exponential decay: `exp(-age_days / tau)` where tau is the per-type
/// time constant. A memory created today scores ~1.0; at one time
/// constant old it scores ~0.37 (1/e).
pub fn recency_score(memory: &Memory, now: chrono::DateTime<chrono::Utc>) -> f32 {
    let age_secs = (now - memory.created_at).num_seconds().max(0) as f32;
    let age_days = age_secs / 86_400.0;
    let half_life = type_half_life_days(&memory.memory_type);
    (-age_days / half_life).exp()
}

/// Apply salience + recency boosts to a base score (#352).
///
/// Used by recall_hybrid to fuse the dual-axis signals into the final
/// score. Returns the new score, clamped to [0.0, ∞) — callers may let
/// boosted scores exceed 1.0 (ranking is relative).
pub fn apply_boosts(
    base_score: f32,
    memory: &Memory,
    now: chrono::DateTime<chrono::Utc>,
    config: SalienceRecencyConfig,
) -> f32 {
    if config.is_noop() {
        return base_score;
    }
    let mut score = base_score;
    if config.salience_weight > 0.0 {
        score += salience_score(memory) * config.salience_weight;
    }
    if config.recency_weight > 0.0 {
        score += recency_score(memory, now) * config.recency_weight;
    }
    score.max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem(access: u32, importance: f64, pinned: bool, age_days: i64, type_: &str) -> Memory {
        Memory {
            id: "test".to_string(),
            content: "test".to_string(),
            embedding: vec![],
            tags: vec![],
            metadata: serde_json::Value::Null,
            created_at: chrono::Utc::now() - chrono::Duration::days(age_days),
            updated_at: chrono::Utc::now(),
            namespace: "default".to_string(),
            access_count: access,
            last_accessed: None,
            deprecated: false,
            valid_from: None,
            valid_until: None,
            memory_type: type_.to_string(),
            importance,
            pinned,
            content_type: "text".to_string(),
            slug: None,
            source: None,
            source_type: "user".to_string(),
        }
    }

    #[test]
    fn salience_zero_access_low_importance() {
        let m = mem(0, 0.0, false, 0, "fact");
        let s = salience_score(&m);
        assert!(s < 0.01, "low salience expected, got {s}");
    }

    #[test]
    fn salience_pinned_gets_bonus() {
        let m_unpinned = mem(0, 0.5, false, 0, "fact");
        let m_pinned = mem(0, 0.5, true, 0, "fact");
        assert!(salience_score(&m_pinned) > salience_score(&m_unpinned));
    }

    #[test]
    fn salience_high_access_climbs() {
        let cold = mem(0, 0.5, false, 0, "fact");
        let hot = mem(1000, 0.5, false, 0, "fact");
        assert!(salience_score(&hot) > salience_score(&cold));
    }

    #[test]
    fn salience_clamped_to_one() {
        let m = mem(u32::MAX, 1.0, true, 0, "fact");
        let s = salience_score(&m);
        assert!((0.0..=1.0).contains(&s), "salience out of range: {s}");
    }

    #[test]
    fn recency_brand_new_is_near_one() {
        let m = mem(0, 0.5, false, 0, "fact");
        let r = recency_score(&m, chrono::Utc::now());
        assert!(r > 0.99, "fresh memory recency should be ~1, got {r}");
    }

    #[test]
    fn recency_decays_with_age() {
        let fresh = mem(0, 0.5, false, 0, "fact");
        let old = mem(0, 0.5, false, 365, "fact");
        assert!(
            recency_score(&fresh, chrono::Utc::now()) > recency_score(&old, chrono::Utc::now())
        );
    }

    #[test]
    fn recency_per_type_half_life() {
        let now = chrono::Utc::now();
        // At 30 days old:
        let event_30d = mem(0, 0.5, false, 30, "event");
        let decision_30d = mem(0, 0.5, false, 30, "decision");
        let event_r = recency_score(&event_30d, now);
        let decision_r = recency_score(&decision_30d, now);
        // Event (half-life 30d) should be decayed more than Decision (half-life 365d).
        assert!(
            event_r < decision_r,
            "event should decay faster (event={event_r}, decision={decision_r})"
        );
    }

    #[test]
    fn type_half_life_known_types() {
        assert_eq!(type_half_life_days("decision"), 365.0);
        assert_eq!(type_half_life_days("preference"), 365.0);
        assert_eq!(type_half_life_days("fact"), 180.0);
        assert_eq!(type_half_life_days("reference"), 180.0);
        assert_eq!(type_half_life_days("event"), 30.0);
        assert_eq!(type_half_life_days("insight"), 240.0);
        // Unknown types fall back to the default.
        assert_eq!(type_half_life_days("note"), 90.0);
        assert_eq!(type_half_life_days("procedure"), 90.0);
        assert_eq!(type_half_life_days("unknown"), 90.0);
    }

    #[test]
    fn apply_boosts_noop_when_weights_zero() {
        let m = mem(0, 0.5, false, 0, "fact");
        let cfg = SalienceRecencyConfig::default();
        assert!(cfg.is_noop());
        assert_eq!(apply_boosts(0.7, &m, chrono::Utc::now(), cfg), 0.7);
    }

    #[test]
    fn apply_boosts_raises_score() {
        let m = mem(100, 0.9, true, 0, "decision");
        let cfg = SalienceRecencyConfig {
            salience_weight: 0.15,
            recency_weight: 0.15,
        };
        let boosted = apply_boosts(0.5, &m, chrono::Utc::now(), cfg);
        assert!(boosted > 0.5, "boost should raise score, got {boosted}");
    }

    #[test]
    fn config_sanitized_clamps() {
        let bad = SalienceRecencyConfig {
            salience_weight: 5.0,
            recency_weight: -1.0,
        };
        let s = bad.sanitized();
        assert_eq!(s.salience_weight, 1.0);
        assert_eq!(s.recency_weight, 0.0);
    }

    #[test]
    fn config_is_noop() {
        assert!(SalienceRecencyConfig::default().is_noop());
        assert!(!SalienceRecencyConfig {
            salience_weight: 0.1,
            recency_weight: 0.0,
        }
        .is_noop());
    }
}
