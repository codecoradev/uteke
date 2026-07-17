//! Jaccard token similarity reranking signal (#719).
//!
//! Pure set-overlap signal that catches different cases from FTS5 BM25
//! (which uses IDF-weighted scoring). Jaccard measures the ratio of shared
//! tokens between query and content, independent of term rarity.
//!
//! Adopted from Hermes holographic memory analysis — Hermes uses Jaccard as
//! a 0.3-weighted reranking signal in its hybrid fusion.

use std::collections::HashSet;

/// Compute Jaccard similarity between two token sets.
///
/// Returns `|A ∩ B| / |A ∪ B|` in `[0.0, 1.0]`.
/// Returns 0.0 if both sets are empty.
pub fn jaccard_similarity(query: &HashSet<String>, content: &HashSet<String>) -> f32 {
    if query.is_empty() && content.is_empty() {
        return 0.0;
    }
    let intersection = query.intersection(content).count();
    let union = query.union(content).count();
    if union == 0 {
        return 0.0;
    }
    intersection as f32 / union as f32
}

/// Tokenize text into a lowercase HashSet.
///
/// Splits on whitespace and strips punctuation from each token.
/// Empty tokens are discarded.
pub fn tokenize(text: &str) -> HashSet<String> {
    text.split_whitespace()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_alphanumeric())
                .to_ascii_lowercase()
        })
        .filter(|t| !t.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jaccard_identical() {
        let a: HashSet<_> = ["hello", "world"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard_similarity(&a, &a) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn jaccard_disjoint() {
        let a: HashSet<_> = ["hello", "world"].iter().map(|s| s.to_string()).collect();
        let b: HashSet<_> = ["foo", "bar"].iter().map(|s| s.to_string()).collect();
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn jaccard_partial() {
        let a: HashSet<_> = ["hello", "world", "foo"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let b: HashSet<_> = ["hello", "bar"].iter().map(|s| s.to_string()).collect();
        // intersection = {"hello"} = 1, union = {"hello","world","foo","bar"} = 4
        let j = jaccard_similarity(&a, &b);
        assert!((j - 0.25).abs() < 0.01, "expected ~0.25, got {j}");
    }

    #[test]
    fn jaccard_empty_both() {
        let a: HashSet<String> = HashSet::new();
        let b: HashSet<String> = HashSet::new();
        assert!((jaccard_similarity(&a, &b) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn tokenize_basic() {
        let tokens = tokenize("Hello World foo");
        assert!(tokens.contains("hello"));
        assert!(tokens.contains("world"));
        assert!(tokens.contains("foo"));
        assert_eq!(tokens.len(), 3);
    }

    #[test]
    fn tokenize_strips_punctuation() {
        let tokens = tokenize("hello, world! (test)");
        assert!(tokens.contains("hello"));
        assert!(tokens.contains("world"));
        assert!(tokens.contains("test"));
    }

    #[test]
    fn tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_whitespace_only() {
        let tokens = tokenize("   \t\n  ");
        assert!(tokens.is_empty());
    }

    #[test]
    fn jaccard_with_tokenize() {
        let q = tokenize("rust memory engine");
        let c = tokenize("rust is a fast memory engine for AI");
        let j = jaccard_similarity(&q, &c);
        // intersection = {"rust", "memory", "engine"} = 3
        // union = {"rust", "memory", "engine", "is", "a", "fast", "for", "ai"} = 8
        assert!((j - 0.375).abs() < 0.01, "expected ~0.375, got {j}");
    }
}
