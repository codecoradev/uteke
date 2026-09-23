//! Budgeted context packing for recall results (#1281 Phase 1).
//!
//! Deterministic and LLM-free: greedily fill a character budget from the
//! ranked recall list, honouring caller-supplied `exclude_ids`. Scores and
//! rank order are untouched — this is a selection primitive, not a
//! re-ranker. (MMR diversity re-ranking is the Phase 2 experiment, gated
//! on LongMemEval + redundancy benchmarks before it can ship.)

use serde::{Deserialize, Serialize};

use crate::memory::types::UnifiedSearchResult;

/// Per-item overhead estimate (JSON envelope fields, separators), in chars.
const ITEM_OVERHEAD_CHARS: usize = 24;

/// Preview length for skipped items, so the report stays small.
const SKIPPED_PREVIEW_CHARS: usize = 160;

/// Result of packing recall results into a character budget (#1281).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPack {
    /// Items selected for the context window, in rank order.
    pub selected: Vec<UnifiedSearchResult>,
    /// Items not selected, in rank order, each with a reason.
    pub skipped: Vec<SkippedItem>,
    /// Estimated characters of selected content (incl. per-item overhead).
    pub budget_used: usize,
    /// The budget that was applied.
    pub budget_chars: usize,
}

/// A recall result left out of the pack, with the reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkippedItem {
    /// Content preview (truncated) of the skipped item.
    pub content: String,
    /// Why it was left out: `excluded` (caller exclude list) or `budget`.
    pub reason: String,
    /// Memory ID when the item is a memory.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_id: Option<String>,
}

/// Pack ranked recall results into `budget_chars` of content.
///
/// `exclude_ids` are memory IDs the caller already injected this turn;
/// they are dropped first (reason `excluded`). Remaining items are taken
/// in rank order while they fit the remaining budget; items that do not
/// fit are reported as `skipped` (reason `budget`) and scanning continues,
/// so a cheaper lower-ranked item can still fill residual space. Rank
/// order is preserved end-to-end — no knapsack shuffling.
pub fn pack_context(
    results: Vec<UnifiedSearchResult>,
    budget_chars: usize,
    exclude_ids: &[String],
) -> ContextPack {
    let mut selected = Vec::new();
    let mut skipped = Vec::new();
    let mut used = 0usize;

    for r in results {
        let id = r.memory_id.clone();
        if id
            .as_deref()
            .is_some_and(|id| exclude_ids.iter().any(|x| x == id))
        {
            skipped.push(SkippedItem {
                content: preview(&r.content),
                reason: "excluded".to_string(),
                memory_id: id,
            });
            continue;
        }
        let cost = r.content.chars().count() + ITEM_OVERHEAD_CHARS;
        if used + cost > budget_chars {
            skipped.push(SkippedItem {
                content: preview(&r.content),
                reason: "budget".to_string(),
                memory_id: id,
            });
            continue;
        }
        used += cost;
        selected.push(r);
    }

    ContextPack {
        selected,
        skipped,
        budget_used: used,
        budget_chars,
    }
}

fn preview(s: &str) -> String {
    if s.chars().count() <= SKIPPED_PREVIEW_CHARS {
        s.to_string()
    } else {
        let t: String = s.chars().take(SKIPPED_PREVIEW_CHARS).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ur(id: &str, content: &str, score: f32) -> UnifiedSearchResult {
        serde_json::from_value(serde_json::json!({
            "result_type": "memory",
            "score": score,
            "content": content,
            "memory_id": id,
        }))
        .unwrap()
    }

    #[test]
    fn fills_in_rank_order_until_budget() {
        let results = vec![
            ur("m1", &"a".repeat(50), 0.9),
            ur("m2", &"b".repeat(50), 0.8),
            ur("m3", &"c".repeat(50), 0.7),
        ];
        // Budget fits exactly two items (2 × (50 + 24) = 148).
        let pack = pack_context(results, 148, &[]);
        assert_eq!(pack.selected.len(), 2);
        assert_eq!(pack.selected[0].memory_id.as_deref(), Some("m1"));
        assert_eq!(pack.selected[1].memory_id.as_deref(), Some("m2"));
        assert_eq!(pack.skipped.len(), 1);
        assert_eq!(pack.skipped[0].reason, "budget");
        assert_eq!(pack.skipped[0].memory_id.as_deref(), Some("m3"));
        assert_eq!(pack.budget_used, 148);
        assert!(pack.budget_used <= pack.budget_chars);
    }

    #[test]
    fn excluded_ids_are_dropped_with_reason() {
        let results = vec![ur("m1", "alpha", 0.9), ur("m2", "beta", 0.8)];
        let pack = pack_context(results, 10_000, &["m1".to_string()]);
        assert_eq!(pack.selected.len(), 1);
        assert_eq!(pack.selected[0].memory_id.as_deref(), Some("m2"));
        assert_eq!(pack.skipped.len(), 1);
        assert_eq!(pack.skipped[0].reason, "excluded");
        assert_eq!(pack.skipped[0].memory_id.as_deref(), Some("m1"));
    }

    #[test]
    fn zero_budget_selects_nothing() {
        let results = vec![ur("m1", "alpha", 0.9)];
        let pack = pack_context(results, 0, &[]);
        assert!(pack.selected.is_empty());
        assert_eq!(pack.skipped.len(), 1);
        assert_eq!(pack.skipped[0].reason, "budget");
        assert_eq!(pack.budget_used, 0);
    }

    #[test]
    fn residual_budget_allows_cheaper_lower_ranked_item() {
        let results = vec![
            ur("big", &"x".repeat(90), 0.9),
            ur("small", &"y".repeat(10), 0.5),
        ];
        // Budget 120: big costs 114 and fits; small costs 34 → skipped.
        let pack = pack_context(results.clone(), 120, &[]);
        assert_eq!(pack.selected.len(), 1);
        // Budget 115: big would exceed (0 + 114 fits? 114 ≤ 115 fits) — use
        // 110 so big is skipped (114 > 110) and small (34) still fits.
        let pack2 = pack_context(results, 110, &[]);
        assert_eq!(pack2.selected.len(), 1);
        assert_eq!(pack2.selected[0].memory_id.as_deref(), Some("small"));
        assert_eq!(pack2.skipped[0].memory_id.as_deref(), Some("big"));
        let _ = pack; // first assertion set already checked
    }

    #[test]
    fn multibyte_content_counted_by_chars() {
        let results = vec![ur("m1", "é".repeat(30).as_str(), 0.9)];
        let pack = pack_context(results, 10_000, &[]);
        assert_eq!(pack.selected.len(), 1);
        assert_eq!(pack.budget_used, 30 + ITEM_OVERHEAD_CHARS);
    }
}
