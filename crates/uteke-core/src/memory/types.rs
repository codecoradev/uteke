//! Core types for the Uteke memory engine.

use serde::{Deserialize, Serialize};

/// Default namespace for memories without explicit namespace.
pub const DEFAULT_NAMESPACE: &str = "default";

/// A stored memory with content, embedding, tags, and metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// The text content of the memory.
    pub content: String,
    /// 768-dimensional embedding vector.
    #[serde(skip_serializing, default)]
    pub embedding: Vec<f32>,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Arbitrary JSON metadata.
    pub metadata: serde_json::Value,
    /// When this memory was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When this memory was last updated.
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Namespace for multi-agent isolation.
    #[serde(default = "default_namespace")]
    pub namespace: String,
    /// How many times this memory has been accessed (recall, get).
    #[serde(default)]
    pub access_count: u32,
    /// When this memory was last accessed.
    #[serde(default)]
    pub last_accessed: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this memory has been superseded by a newer one.
    #[serde(default)]
    pub deprecated: bool,
    /// When this fact became valid (temporal metadata).
    #[serde(default)]
    pub valid_from: Option<chrono::DateTime<chrono::Utc>>,
    /// When this fact was invalidated (temporal metadata).
    #[serde(default)]
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Memory type: fact, procedure, preference, decision, context.
    #[serde(default = "default_memory_type")]
    pub memory_type: String,
    /// Composite importance score (0.0–1.0). Higher = more important.
    #[serde(default = "default_importance")]
    pub importance: f64,
    /// Whether this memory is pinned (never decays).
    #[serde(default)]
    pub pinned: bool,
    /// Content type: "text" (default) or "json".
    #[serde(default = "default_content_type")]
    pub content_type: String,
    /// Optional stable slug for `[[slug]]` auto-linking (v8, #346).
    /// Populated lazily when a memory is referenced by slug.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Provenance: free-form source identifier (URL, file path, "user", #348).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Provenance type: user, url, file, import, derived, system, unknown (#348).
    #[serde(
        default = "default_source_type",
        skip_serializing_if = "is_default_source_type"
    )]
    pub source_type: String,
}

fn default_namespace() -> String {
    DEFAULT_NAMESPACE.to_string()
}

/// Default source type for memories without explicit provenance (#348).
pub fn default_source_type() -> String {
    "user".to_string()
}

/// Serde predicate: skip if source_type equals the default.
fn is_default_source_type(s: &str) -> bool {
    s == "user"
}

fn default_importance() -> f64 {
    0.5
}

fn default_memory_type() -> String {
    "fact".to_string()
}

fn default_content_type() -> String {
    "text".to_string()
}

/// A search result with relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// The matched memory.
    pub memory: Memory,
    /// Cosine similarity score (0.0–1.0).
    pub score: f32,
}

/// Memory tier based on access recency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryTier {
    /// Accessed within last 7 days — boosted in recall.
    Hot,
    /// Accessed within last 30 days — normal recall.
    Warm,
    /// Not accessed in 30+ days — lower priority.
    Cold,
}

impl MemoryTier {
    /// Determine tier from last_accessed timestamp and configurable thresholds.
    ///
    /// `hot_days`: memories accessed within this many days are Hot.
    /// `warm_days`: memories accessed within this many days (but beyond hot) are Warm.
    /// Beyond `warm_days` (or never accessed) → Cold.
    pub fn from_last_accessed(
        last_accessed: Option<chrono::DateTime<chrono::Utc>>,
        hot_days: i64,
        warm_days: i64,
    ) -> Self {
        let Some(la) = last_accessed else {
            return MemoryTier::Cold;
        };
        let age = chrono::Utc::now() - la;
        if age.num_days() <= hot_days {
            MemoryTier::Hot
        } else if age.num_days() <= warm_days {
            MemoryTier::Warm
        } else {
            MemoryTier::Cold
        }
    }
}

/// Statistics about the memory store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreStats {
    /// Total number of memories.
    pub total_memories: usize,
    /// Number of unique tags.
    pub unique_tags: usize,
    /// Database file size in bytes.
    pub db_size_bytes: u64,
    /// Number of hot memories (accessed within 7 days).
    pub hot: usize,
    /// Number of warm memories (accessed within 30 days).
    pub warm: usize,
    /// Number of cold memories (not accessed in 30+ days).
    pub cold: usize,
    /// Number of recall cache hits.
    pub cache_hits: u64,
    /// Number of recall cache misses.
    pub cache_misses: u64,
}

/// Result of a bulk delete operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkDeleteResult {
    /// Number of memories deleted.
    pub deleted: usize,
    /// IDs of deleted memories.
    pub ids: Vec<String>,
}

/// Lightweight export format — no embedding vector (re-embedded on import).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportEntry {
    /// The text content.
    pub content: String,
    /// Tags for categorization.
    pub tags: Vec<String>,
    /// Arbitrary JSON metadata.
    pub metadata: serde_json::Value,
    /// When this memory was originally created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Optional source provenance (#348).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Result of an import operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    /// Number of memories imported.
    pub imported: usize,
    /// Number of entries skipped (duplicate or invalid).
    pub skipped: usize,
}

/// A tag with its usage count.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagInfo {
    /// Tag name.
    pub name: String,
    /// Number of memories using this tag.
    pub count: usize,
}

/// Aging status — breakdown of memories by access tier.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgingStatus {
    /// Total memories in namespace.
    pub total: usize,
    /// Hot memories (accessed within 7 days).
    pub hot: usize,
    /// Warm memories (accessed within 30 days but not hot).
    pub warm: usize,
    /// Cold memories (not accessed in 30+ days).
    pub cold: usize,
    /// Memories that have never been accessed.
    pub never_accessed: usize,
}

/// Result of a cleanup operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupResult {
    /// Number of memories deleted.
    pub deleted: usize,
}

/// Result of a prune operation (auto-forget with decay policy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PruneResult {
    /// Number of memories pruned.
    pub pruned: usize,
    /// IDs of pruned memories.
    pub ids: Vec<String>,
    /// Number of memories deprecated (contradicted).
    pub deprecated: usize,
    /// IDs of deprecated memories.
    pub deprecated_ids: Vec<String>,
}

/// Result of a contradiction check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContradictionResult {
    /// Whether a contradiction was detected.
    pub contradicted: bool,
    /// ID of the existing memory that was deprecated.
    pub deprecated_id: Option<String>,
    /// Similarity score that triggered the contradiction.
    pub similarity: f32,
}

/// Memory type classification.
///
/// The taxonomy is fixed (no user-defined types yet, see #349 non-goals).
/// Auto-inference is pattern-based and runs on every `remember()` when the
/// caller does not pass an explicit type — see [`MemoryType::infer_from_content`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryType {
    /// A factual statement (has temporal validity).
    Fact,
    /// A procedure or how-to (doesn't expire).
    Procedure,
    /// A user preference (doesn't expire).
    Preference,
    /// A design or architecture decision (may be superseded).
    Decision,
    /// Contextual information (session-scoped, may expire).
    Context,
    /// A general freeform note (no specific type signal detected).
    ///
    /// Added in #349. Distinguished from `Fact` (which claims verifiable
    /// truth) — `Note` is a generic capture bucket.
    Note,
    /// A realization or learning (contains "realized", "learned",
    /// "discovered", "turns out").
    ///
    /// Added in #349. High long-term value for memory-engine epics.
    Insight,
    /// Factual reference info (links, specs, docs — starts with URL or
    /// `ref:` / `see:` / `docs:`).
    ///
    /// Added in #349.
    Reference,
    /// A time-bound event (has an explicit ISO date + time word).
    ///
    /// Added in #349. Eligible for recency-boosted recall.
    Event,
}

impl MemoryType {
    /// Parse from string (case-insensitive).
    pub fn from_str_opt(s: &str) -> Option<Self> {
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "fact" => Some(Self::Fact),
            "procedure" => Some(Self::Procedure),
            "preference" => Some(Self::Preference),
            "decision" => Some(Self::Decision),
            "context" => Some(Self::Context),
            "note" => Some(Self::Note),
            "insight" => Some(Self::Insight),
            "reference" => Some(Self::Reference),
            "event" => Some(Self::Event),
            _ => None,
        }
    }

    /// Convert to string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fact => "fact",
            Self::Procedure => "procedure",
            Self::Preference => "preference",
            Self::Decision => "decision",
            Self::Context => "context",
            Self::Note => "note",
            Self::Insight => "insight",
            Self::Reference => "reference",
            Self::Event => "event",
        }
    }

    /// Whether this memory type has temporal validity.
    pub fn has_temporal_validity(&self) -> bool {
        matches!(
            self,
            Self::Fact | Self::Decision | Self::Context | Self::Event
        )
    }

    /// Recall score boost for this type (#349).
    ///
    /// Decisions and Preferences are high-signal, long-lived — they get a
    /// small boost so they drift upward in recall. Events get a tiny boost
    /// (recency is handled separately in #352). Notes get a slight penalty
    /// because they're undifferentiated captures.
    ///
    /// The boost is additive and small (±0.05) so it never dominates
    /// embedding similarity, only acts as a tie-breaker.
    pub fn recall_boost(&self) -> f32 {
        match self {
            Self::Decision | Self::Preference => 0.05,
            Self::Insight => 0.03,
            Self::Event => 0.02,
            Self::Note => -0.02,
            Self::Fact | Self::Procedure | Self::Context | Self::Reference => 0.0,
        }
    }

    /// Pattern-based type inference from content (zero LLM, #349).
    ///
    /// Order matters: the first pattern to match wins. Patterns are
    /// deliberately conservative — ambiguous content falls back to `Note`
    /// rather than guessing a stronger type.
    ///
    /// | Signal | Inferred type |
    /// |---|---|
    /// | Starts with URL / `ref:` / `see:` / `docs:` | `Reference` |
    /// | Contains `decided to`, `chose`, `will use`, `going with` | `Decision` |
    /// | Contains `realized`, `learned`, `discovered`, `turns out` | `Insight` |
    /// | Contains `step 1`, `how to`, or numbered list | `Procedure` |
    /// | Contains `always`, `never`, `prefer`, `hate` | `Preference` |
    /// | Contains ISO date + time word | `Event` |
    /// | _(none of the above)_ | `Note` |
    pub fn infer_from_content(content: &str) -> Self {
        // Look at the first non-empty line — reference markers are
        // line-start signals, not body signals. CodeCora #386: previous
        // code used lower.starts_with on the original content, which made
        // detection depend on leading whitespace. We now use first_line for
        // line-start signals.
        let trimmed = content.trim_start();
        let first_line = trimmed.lines().next().unwrap_or("").trim();
        let lower = content.to_ascii_lowercase();

        // Reference: starts with URL scheme or a ref marker on the first
        // non-empty line. Use first_line (not lower) so leading whitespace
        // doesn't matter.
        let first_lower = first_line.to_ascii_lowercase();
        if first_lower.starts_with("http://")
            || first_lower.starts_with("https://")
            || first_lower.starts_with("ref:")
            || first_lower.starts_with("see:")
            || first_lower.starts_with("docs:")
        {
            return Self::Reference;
        }

        // Decision: explicit commitment language.
        if contains_any(&lower, &["decided to", "chose ", "will use", "going with"])
            || contains_any(&lower, &["we decided", "decision:"])
        {
            return Self::Decision;
        }

        // Insight: realization language.
        if contains_any(&lower, &["realized", "learned", "discovered", "turns out"])
            || contains_any(&lower, &["insight:", "aha:"])
        {
            return Self::Insight;
        }

        // Procedure: how-to / steps.
        if contains_any(&lower, &["how to", "step 1", "steps:"]) || is_numbered_list(content) {
            return Self::Procedure;
        }

        // Preference: likes/dislikes.
        if contains_any(
            &lower,
            &["always ", "never ", "prefer", "hate", "i like", "i dislike"],
        ) {
            return Self::Preference;
        }

        // Event: ISO date + time word.
        if has_iso_date(&lower)
            && contains_any(
                &lower,
                &[
                    "at ",
                    " on ",
                    "meeting",
                    "deadline",
                    "standup",
                    "stand-up",
                    "scheduled",
                ],
            )
        {
            return Self::Event;
        }

        // Fallback: undifferentiated note.
        Self::Note
    }
}

/// Case-insensitive substring search against multiple needles.
fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Detect a leading numbered list pattern (e.g. "1. ...\n2. ...\n").
fn is_numbered_list(content: &str) -> bool {
    let mut consecutive = 0usize;
    let mut expected = 1u32;
    for line in content.lines() {
        let trimmed = line.trim_start();
        // Strip ALL leading digits (handles multi-digit like "10. ", "123) ").
        let num_str: String = trimmed.chars().take_while(|c| c.is_ascii_digit()).collect();
        if num_str.is_empty() {
            continue;
        }
        let rest = &trimmed[num_str.len()..];
        // "1. " / "1) " style
        if rest.starts_with('.') || rest.starts_with(')') {
            match num_str.parse::<u32>() {
                Ok(n) if n == expected => {
                    consecutive += 1;
                    expected += 1;
                    if consecutive >= 2 {
                        return true;
                    }
                }
                _ => {
                    // Parse failure (overflow) or out-of-order → reset.
                    consecutive = 0;
                    expected = 1;
                }
            }
            continue;
        }
        // Non-numbered line resets the counter.
        if !trimmed.is_empty() {
            consecutive = 0;
            expected = 1;
        }
    }
    false
}

/// Detect an ISO 8601 date pattern anywhere in the text (e.g. `2026-06-18`).
fn has_iso_date(text: &str) -> bool {
    // Cheap scan: 4 digits, dash, 2 digits, dash, 2 digits.
    let bytes = text.as_bytes();
    if bytes.len() < 10 {
        return false;
    }
    for i in 0..=(bytes.len().saturating_sub(10)) {
        if is_iso_date_bytes(&bytes[i..i + 10]) {
            return true;
        }
    }
    false
}

fn is_iso_date_bytes(b: &[u8]) -> bool {
    if b.len() != 10
        || !b[0].is_ascii_digit()
        || !b[1].is_ascii_digit()
        || !b[2].is_ascii_digit()
        || !b[3].is_ascii_digit()
        || b[4] != b'-'
        || !b[5].is_ascii_digit()
        || !b[6].is_ascii_digit()
        || b[7] != b'-'
        || !b[8].is_ascii_digit()
        || !b[9].is_ascii_digit()
    {
        return false;
    }
    // Validate month and day ranges (CodeCora #386 r4).
    // Per-month max day (non-leap year approximation — good enough for
    // pattern detection; we're not a date library).
    let month = (b[5] - b'0') * 10 + (b[6] - b'0');
    let day = (b[8] - b'0') * 10 + (b[9] - b'0');
    if !(1..=12).contains(&month) {
        return false;
    }
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => 29, // accept leap day; worst case false positive
        _ => return false,
    };
    day >= 1 && day <= max_day
}

/// Result of a consolidation (deduplication) operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsolidationResult {
    /// Number of duplicate pairs found.
    pub duplicates_found: usize,
    /// Number of memories merged (older duplicates removed).
    pub merged: usize,
    /// IDs of removed duplicate memories.
    pub removed_ids: Vec<String>,
    /// Kept memory IDs (one per duplicate group).
    pub kept_ids: Vec<String>,
}

/// A pair of similar memories (potential duplicate).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimilarPair {
    /// ID of the first (older) memory.
    pub id_a: String,
    /// Content preview of the first memory.
    pub content_a: String,
    /// ID of the second (newer) memory.
    pub id_b: String,
    /// Content preview of the second memory.
    pub content_b: String,
    /// Cosine similarity score.
    pub similarity: f32,
}

/// Recall strategy for hybrid search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RecallStrategy {
    /// Hybrid: vector + FTS5 merged via Reciprocal Rank Fusion.
    #[default]
    Hybrid,
    /// Vector similarity only (original behavior).
    Vector,
    /// FTS5 full-text search only.
    Fts5,
    /// Graph-augmented: hybrid RRF + graph-signal reranking (#378).
    /// Well-connected memories get a subtle log-scaled score boost.
    Graph,
}

impl RecallStrategy {
    /// Parse from string, with fallback to Hybrid for unknown values.
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s {
            "vector" => Some(Self::Vector),
            "fts5" => Some(Self::Fts5),
            "hybrid" => Some(Self::Hybrid),
            "graph" => Some(Self::Graph),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Vector => "vector",
            Self::Fts5 => "fts5",
            Self::Hybrid => "hybrid",
            Self::Graph => "graph",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_type_roundtrip() {
        for t in [
            MemoryType::Fact,
            MemoryType::Procedure,
            MemoryType::Preference,
            MemoryType::Decision,
            MemoryType::Context,
            MemoryType::Note,
            MemoryType::Insight,
            MemoryType::Reference,
            MemoryType::Event,
        ] {
            let s = t.as_str();
            assert_eq!(MemoryType::from_str_opt(s), Some(t), "roundtrip {s}");
        }
    }

    #[test]
    fn from_str_opt_case_insensitive() {
        assert_eq!(MemoryType::from_str_opt("FACT"), Some(MemoryType::Fact));
        assert_eq!(
            MemoryType::from_str_opt("Decision"),
            Some(MemoryType::Decision)
        );
        assert_eq!(MemoryType::from_str_opt("Note"), Some(MemoryType::Note));
        assert_eq!(
            MemoryType::from_str_opt("Insight"),
            Some(MemoryType::Insight)
        );
        assert_eq!(MemoryType::from_str_opt("unknown"), None);
    }

    #[test]
    fn infer_reference_url_prefix() {
        assert_eq!(
            MemoryType::infer_from_content("https://rust-lang.org/docs"),
            MemoryType::Reference
        );
        assert_eq!(
            MemoryType::infer_from_content("https://example.com/spec"),
            MemoryType::Reference
        );
        // Whitespace before ref marker should still match (CodeCora #386).
        assert_eq!(
            MemoryType::infer_from_content("  ref: RFC 1234 section 2"),
            MemoryType::Reference
        );
        assert_eq!(
            MemoryType::infer_from_content("ref: RFC 1234 section 2"),
            MemoryType::Reference
        );
        assert_eq!(
            MemoryType::infer_from_content("see: upstream docs"),
            MemoryType::Reference
        );
        assert_eq!(
            MemoryType::infer_from_content("docs: https://example.com"),
            MemoryType::Reference
        );
        // Uppercase URL schemes should also match (CodeCora #386 round 2).
        assert_eq!(
            MemoryType::infer_from_content("HTTPS://rust-lang.org/docs"),
            MemoryType::Reference
        );
        assert_eq!(
            MemoryType::infer_from_content("Http://example.com/spec"),
            MemoryType::Reference
        );
    }

    #[test]
    fn infer_decision_signals() {
        assert_eq!(
            MemoryType::infer_from_content("Decided to use SQLite for local storage"),
            MemoryType::Decision
        );
        assert_eq!(
            MemoryType::infer_from_content("We chose Rust for performance"),
            MemoryType::Decision
        );
        assert_eq!(
            MemoryType::infer_from_content("will use Redis for caching"),
            MemoryType::Decision
        );
        assert_eq!(
            MemoryType::infer_from_content("Going with the simpler approach"),
            MemoryType::Decision
        );
        assert_eq!(
            MemoryType::infer_from_content("decision: adopt feature flags"),
            MemoryType::Decision
        );
    }

    #[test]
    fn infer_insight_signals() {
        assert_eq!(
            MemoryType::infer_from_content("Realized the bug was in serialization"),
            MemoryType::Insight
        );
        assert_eq!(
            MemoryType::infer_from_content("learned that async drop is hard"),
            MemoryType::Insight
        );
        assert_eq!(
            MemoryType::infer_from_content("discovered a race condition"),
            MemoryType::Insight
        );
        assert_eq!(
            MemoryType::infer_from_content("turns out the cache was poisoned"),
            MemoryType::Insight
        );
    }

    #[test]
    fn infer_procedure_signals() {
        assert_eq!(
            MemoryType::infer_from_content("How to bake bread: mix flour and water"),
            MemoryType::Procedure
        );
        assert_eq!(
            MemoryType::infer_from_content("step 1: install Rust\nstep 2: run cargo"),
            MemoryType::Procedure
        );
        assert_eq!(
            MemoryType::infer_from_content("1. first do this\n2. then that\n3. finish"),
            MemoryType::Procedure
        );
        assert_eq!(
            MemoryType::infer_from_content("steps:\n1. foo\n2. bar"),
            MemoryType::Procedure
        );
        // Multi-digit numbered lists should also be detected (CodeCora #386 r2).
        assert_eq!(
            MemoryType::infer_from_content(
                "1. first step\n2. second step\n3. third step\n4. fourth step\n5. fifth step\n6. sixth step\n7. seventh step\n8. eighth step\n9. ninth step\n10. tenth step"
            ),
            MemoryType::Procedure
        );
        // Parenthesis style with multi-digit.
        assert_eq!(
            MemoryType::infer_from_content("1) alpha\n2) beta\n3) gamma"),
            MemoryType::Procedure
        );
    }

    #[test]
    fn infer_preference_signals() {
        assert_eq!(
            MemoryType::infer_from_content("always use tabs, never spaces"),
            MemoryType::Preference
        );
        assert_eq!(
            MemoryType::infer_from_content("I prefer dark mode"),
            MemoryType::Preference
        );
        assert_eq!(
            MemoryType::infer_from_content("I hate manual memory management"),
            MemoryType::Preference
        );
    }

    #[test]
    fn infer_event_signals() {
        // Debug what's inferred for each case.
        let cases = [
            ("Standup at 2026-06-18 09:00", MemoryType::Event),
            ("Deadline on 2026-07-01", MemoryType::Event),
            ("Meeting scheduled 2026-06-18", MemoryType::Event),
        ];
        for (content, expected) in cases {
            let got = MemoryType::infer_from_content(content);
            assert_eq!(got, expected, "content: {content:?}");
        }
    }

    #[test]
    fn infer_fallback_note() {
        assert_eq!(
            MemoryType::infer_from_content("just a random capture"),
            MemoryType::Note
        );
        assert_eq!(
            MemoryType::infer_from_content("lorem ipsum dolor sit amet"),
            MemoryType::Note
        );
    }

    #[test]
    fn recall_boost_signs() {
        // High-signal types get positive boost.
        assert!(MemoryType::Decision.recall_boost() > 0.0);
        assert!(MemoryType::Preference.recall_boost() > 0.0);
        assert!(MemoryType::Insight.recall_boost() > 0.0);
        // Notes get slight penalty.
        assert!(MemoryType::Note.recall_boost() < 0.0);
        // Neutral types.
        assert_eq!(MemoryType::Fact.recall_boost(), 0.0);
        assert_eq!(MemoryType::Procedure.recall_boost(), 0.0);
        assert_eq!(MemoryType::Context.recall_boost(), 0.0);
        assert_eq!(MemoryType::Reference.recall_boost(), 0.0);
    }

    #[test]
    fn has_temporal_validity_updated() {
        // Event (new) has temporal validity.
        assert!(MemoryType::Event.has_temporal_validity());
        // Note (new) does not.
        assert!(!MemoryType::Note.has_temporal_validity());
        assert!(!MemoryType::Insight.has_temporal_validity());
        assert!(!MemoryType::Reference.has_temporal_validity());
    }

    #[test]
    fn iso_date_detection() {
        assert!(has_iso_date("meeting on 2026-06-18 at noon"));
        assert!(has_iso_date("2026-12-31"));
        assert!(!has_iso_date("18/06/2026")); // not ISO
        assert!(!has_iso_date("no date here"));
        // Invalid dates should NOT match (CodeCora #386 r4).
        assert!(!has_iso_date("meeting on 2026-99-99"));
        assert!(!has_iso_date("date: 2026-13-40"));
        assert!(!has_iso_date("abc2026-00-00xyz"));
        // Per-month validation: Feb 30, Apr 31 are impossible.
        assert!(!has_iso_date("2026-02-31 deadline"));
        assert!(!has_iso_date("2026-04-31 event"));
        assert!(!has_iso_date("2026-02-30 meeting"));
        // But valid ones should.
        assert!(has_iso_date("meeting on 2026-12-31"));
        assert!(has_iso_date("2026-01-01"));
        assert!(has_iso_date("2026-02-29 leap")); // leap day accepted
    }

    #[test]
    fn numbered_list_detection() {
        assert!(is_numbered_list("1. first step\n2. second step\n3. third"));
        assert!(is_numbered_list("1) one\n2) two\n"));
        assert!(!is_numbered_list("just plain text"));
        assert!(!is_numbered_list("1. alone")); // only one item
    }
}
