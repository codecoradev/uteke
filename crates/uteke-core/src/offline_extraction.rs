//! Offline rule-based fact extraction — zero API calls, zero LLM dependencies,
//! zero regex dependencies.
//!
//! This module provides a lightweight, pattern-driven extractor that distills
//! raw text (chat transcripts, notes, documents) into atomic facts *without*
//! any network or API access. It is the default extraction mode, used when:
//!
//! - The user selects "offline" during onboarding
//! - `[extraction]` mode is set to `"offline"` in the config
//! - No API key is configured and `--extract` is requested
//!
//! ## How it works
//!
//! 1. **Sentence segmentation** — split text into candidate sentences.
//! 2. **Noise filtering** — drop greetings, fillers, tool output, URLs, code.
//! 3. **Keyword scoring** — rank candidates by information density.
//! 4. **Deduplication** — near-duplicate removal (case-insensitive).
//! 5. **Output** — return up to `max_facts` atomic facts.
//!
//! ## Limitations
//!
//! Offline extraction catches explicit, declarative facts well:
//! "My name is John", "I switched from Vim to Helix", "Deploy at 3pm".
//!
//! It does NOT handle nuanced context, multi-sentence reasoning, or pronoun
//! resolution across sentences. For those use cases, configure `mode = "llm"`.

use std::collections::HashSet;

/// Maximum facts to extract per document when not specified.
const DEFAULT_MAX_FACTS: usize = 20;

/// Minimum sentence length to be considered a fact candidate.
const MIN_FACT_LEN: usize = 10;

/// Maximum sentence length — longer sentences are likely paragraphs, not facts.
const MAX_FACT_LEN: usize = 300;

/// Keywords that indicate a factual, memorable statement.
///
/// Each tuple is (keyword(s), weight boost). Checked via case-insensitive
/// substring match — no regex needed.
static FACT_KEYWORDS: &[(&[&str], f32)] = &[
    // Identity & preferences (high signal)
    (&["my name is", "i'm ", "i am "], 1.5),
    (
        &[
            "i prefer",
            "i like",
            "i love",
            "i enjoy",
            "i hate",
            "i dislike",
        ],
        2.0,
    ),
    (&["i use", "using "], 1.0),
    (&["i work", "i live", "working at", "living in"], 1.5),
    (
        &["i have", "i've", "i had", "i need", "i want", "i decided"],
        1.0,
    ),
    // Decisions & plans
    (&["decided", "need to", "have to", "must", "should"], 1.5),
    (
        &["will", "going to", "plan to", "planning", "going forward"],
        1.0,
    ),
    (
        &[
            "sprint",
            "deadline",
            "milestone",
            "release",
            "deploy",
            "launch",
        ],
        1.0,
    ),
    (
        &[
            "switched",
            "migrated",
            "upgraded",
            "downgraded",
            "moved to",
            "moved from",
        ],
        2.0,
    ),
    // Technical facts
    (
        &["version", "endpoint", "url", "port", "address", "config"],
        0.5,
    ),
    // Relationships & roles
    (
        &[
            "team",
            "colleague",
            "manager",
            "report",
            "partner",
            "client",
        ],
        0.8,
    ),
    // Strong factual indicators
    (
        &["fact", "important", "note:", "remember", "key point", "fyi"],
        1.5,
    ),
    // Dates & times
    (
        &[
            "today",
            "tomorrow",
            "yesterday",
            "next week",
            "monday",
            "tuesday",
            "wednesday",
            "thursday",
            "friday",
            "saturday",
            "sunday",
        ],
        0.5,
    ),
    (
        &[
            "january",
            "february",
            "march",
            "april",
            "june",
            "july",
            "august",
            "september",
            "october",
            "november",
            "december",
        ],
        0.5,
    ),
];

/// Greetings and fillers — sentences that are ONLY these words are noise.
static NOISE_WORDS: &[&str] = &[
    "hi",
    "hey",
    "hello",
    "good morning",
    "good afternoon",
    "good evening",
    "thanks",
    "thank you",
    "ok",
    "okay",
    "sure",
    "right",
    "got it",
    "cool",
    "nice",
    "great",
    "awesome",
    "yes",
    "no",
    "yep",
    "nope",
    "yup",
    "sure thing",
    "lol",
    "haha",
];

/// Line prefixes that indicate tool output / system messages.
static NOISE_PREFIXES: &[&str] = &[
    "[",
    "{",
    "<",
    "running ",
    "executing ",
    "loading ",
    "error ",
    "warning ",
    "debug ",
    "trace ",
    "info ",
    "warn ",
    "http",
    "running `",
    "```",
];

/// Extract atomic facts from text using rule-based pattern matching.
///
/// No network calls, no API keys, no external dependencies.
/// Returns a list of factual statements worth remembering.
pub fn extract_facts(text: &str, max_facts: usize) -> Vec<String> {
    let max = if max_facts == 0 {
        DEFAULT_MAX_FACTS
    } else {
        max_facts
    };

    if text.trim().is_empty() {
        return Vec::new();
    }

    // 1. Segment into sentences
    let candidates = segment_sentences(text);

    // 2. Filter noise
    let filtered: Vec<&str> = candidates
        .iter()
        .map(|s| s.as_str())
        .filter(|s| !is_noise(s))
        .filter(|s| s.len() >= MIN_FACT_LEN && s.len() <= MAX_FACT_LEN)
        .collect();

    // 3. Score by salience
    let mut scored: Vec<(f32, &str)> = filtered.iter().map(|s| (score_salience(s), *s)).collect();

    // 4. Sort by score descending
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 5. Dedup (case-insensitive, first 50 chars as prefix key)
    let mut seen: HashSet<String> = HashSet::new();
    let mut facts: Vec<String> = Vec::new();

    for (_, sentence) in &scored {
        let normalized = normalize_for_dedup(sentence);
        // Use char-based prefix to avoid panicking on multibyte UTF-8 boundaries.
        let prefix: String = normalized.chars().take(50).collect();

        if seen.iter().any(|s| s.starts_with(&prefix)) {
            continue;
        }

        seen.insert(normalized);

        let fact = clean_fact(sentence);
        if !fact.is_empty() {
            facts.push(fact);
        }

        if facts.len() >= max {
            break;
        }
    }

    facts
}

/// Segment text into candidate sentences.
///
/// Handles common sentence boundaries: `.`, `!`, `?`, newlines.
/// Also handles bullet points and numbered lists.
fn segment_sentences(text: &str) -> Vec<String> {
    let mut sentences = Vec::new();

    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Skip markdown headers (#895: header-only facts are noise)
        if trimmed.starts_with('#') {
            continue;
        }

        // Skip markdown list/code/bold markers that are just formatting
        if trimmed.starts_with("```") || trimmed.starts_with("---") {
            continue;
        }

        // Split each line on sentence boundaries
        let mut current = String::new();
        let mut paren_depth: i32 = 0; // track () balance (#895)
        let chars: Vec<char> = trimmed.chars().collect();

        for (i, &ch) in chars.iter().enumerate() {
            // Track parenthesis depth — don't split inside parens
            if ch == '(' {
                paren_depth += 1;
            } else if ch == ')' {
                paren_depth = paren_depth.saturating_sub(1);
            }

            current.push(ch);

            if ch == '.' || ch == '!' || ch == '?' {
                // Don't split on decimal/version numbers: digit . digit (#895)
                if ch == '.' && paren_depth == 0 {
                    let prev_is_digit = i > 0 && chars[i - 1].is_ascii_digit();
                    let next_is_digit = i + 1 < chars.len() && chars[i + 1].is_ascii_digit();
                    if prev_is_digit && next_is_digit {
                        continue; // e.g. "1.23", "v0.12.0"
                    }
                }

                // Don't split inside parentheses (#895)
                if paren_depth > 0 {
                    continue;
                }

                let s = current.trim();
                if !s.is_empty() {
                    sentences.push(s.to_string());
                }
                current.clear();
                paren_depth = 0;
            }
        }

        // Remainder of line without terminal punctuation
        let s = current.trim();
        if !s.is_empty() {
            sentences.push(s.to_string());
        }
    }

    sentences
}

/// Check if a sentence is noise (greeting, question, tool output, etc.)
fn is_noise(sentence: &str) -> bool {
    let lower = sentence.to_lowercase();
    let lower_trimmed = lower.trim();

    // Check if it's ONLY a noise word
    for word in NOISE_WORDS {
        if lower_trimmed == *word || lower_trimmed == format!("{}.", word) {
            return true;
        }
    }

    // Check noise prefixes
    for prefix in NOISE_PREFIXES {
        if lower_trimmed.starts_with(prefix) {
            return true;
        }
    }

    // Questions (ends with ? and starts with question word)
    if lower_trimmed.ends_with('?') {
        let q_words = [
            "what",
            "why",
            "how",
            "when",
            "where",
            "who",
            "which",
            "can you",
            "could you",
            "would you",
            "do you",
            "are you",
            "is it",
            "should i",
            "will it",
        ];
        if q_words.iter().any(|w| lower_trimmed.starts_with(w)) {
            return true;
        }
    }

    // URLs alone
    if lower_trimmed.starts_with("http://") || lower_trimmed.starts_with("https://") {
        let parts: Vec<&str> = lower_trimmed.split_whitespace().collect();
        if parts.len() <= 1 {
            return true;
        }
    }

    // Code lines (4+ space indent)
    sentence.starts_with("    ")
}

/// Score a sentence's salience — how likely it contains a durable fact.
fn score_salience(sentence: &str) -> f32 {
    let lower = sentence.to_lowercase();
    let mut score: f32 = 0.1;

    // Keyword boosts
    for (keywords, boost) in FACT_KEYWORDS {
        if keywords.iter().any(|kw| lower.contains(kw)) {
            score += boost;
        }
    }

    // Length bonus — sentences between 20-150 chars are ideal
    let len = sentence.len();
    if (20..=150).contains(&len) {
        score += 0.3;
    } else if len > 150 {
        score -= 0.2;
    }

    // Number bonus — concrete data (dates, versions, counts)
    let digit_count = sentence.chars().filter(|c| c.is_ascii_digit()).count();
    if digit_count > 0 {
        score += 0.1 * digit_count.min(5) as f32;
    }

    // Proper noun bonus — capitalized words (names, places, products)
    let cap_words = sentence
        .split_whitespace()
        .filter(|w| w.chars().next().map(|c| c.is_uppercase()).unwrap_or(false))
        .count();
    if cap_words >= 2 {
        score += 0.2;
    }

    score
}

/// Normalize a sentence for deduplication purposes.
fn normalize_for_dedup(s: &str) -> String {
    s.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Clean up a fact for final output.
fn clean_fact(sentence: &str) -> String {
    let mut s = sentence.trim().to_string();

    // Strip leading bullet markers — use char-based slicing for UTF-8 safety.
    while s
        .chars()
        .next()
        .map(|c| matches!(c, '-' | '*' | '•' | '·' | '#'))
        .unwrap_or(false)
    {
        s = s
            .chars()
            .skip(1)
            .collect::<String>()
            .trim_start()
            .to_string();
    }

    // Strip leading numbering (1. 2. etc.)
    let chars: Vec<char> = s.chars().collect();
    if !chars.is_empty() && chars[0].is_ascii_digit() {
        let mut idx = 0;
        while idx < chars.len() && (chars[idx].is_ascii_digit() || chars[idx] == '.') {
            idx += 1;
        }
        if idx < chars.len() {
            s = chars[idx..]
                .iter()
                .collect::<String>()
                .trim_start()
                .to_string();
        }
    }

    // Strip markdown emphasis
    s = s.replace("**", "");
    s = s.replace("__", "");
    s = s.replace('*', "");
    s = s.replace('_', "");
    s = s.replace('`', "");

    // Strip leading/trailing quotes
    s = s
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '(' || c == ')')
        .trim()
        .to_string();

    // Ensure ends with punctuation
    if !s.is_empty() {
        let last = s.chars().last().unwrap();
        if last != '.' && last != '!' && last != '?' {
            s.push('.');
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_input() {
        assert!(extract_facts("", 10).is_empty());
        assert!(extract_facts("   \n\n  ", 10).is_empty());
    }

    #[test]
    fn test_preference_extraction() {
        let text = "Hi there! My name is John. I prefer Vim over Emacs. What time is it?";
        let facts = extract_facts(text, 10);
        assert!(!facts.is_empty());
        assert!(facts.iter().any(|f| f.to_lowercase().contains("prefer")));
    }

    #[test]
    fn test_noise_filtering() {
        let text = "Hi! Hey! Hello! Yes. No. Okay. Thanks!";
        let facts = extract_facts(text, 10);
        assert!(facts.is_empty(), "Should filter out all noise");
    }

    #[test]
    fn test_technical_fact() {
        let text = "I upgraded to Rust 1.75. The deploy is scheduled for Friday.";
        let facts = extract_facts(text, 10);
        assert!(!facts.is_empty());
    }

    #[test]
    fn test_dedup() {
        let text = "I use Vim. I use Vim. I use Vim for editing.";
        let facts = extract_facts(text, 10);
        assert!(facts.len() <= 2, "Should dedup near-identical sentences");
    }

    #[test]
    fn test_max_facts() {
        let text = "I prefer tea. I like coffee. I enjoy hiking. I love coding. I hate bugs.";
        let facts = extract_facts(text, 2);
        assert!(facts.len() <= 2);
    }

    #[test]
    fn test_clean_bullet() {
        assert_eq!(clean_fact("- My name is John"), "My name is John.");
        assert_eq!(clean_fact("1. Deploy at 3pm"), "Deploy at 3pm.");
    }

    #[test]
    fn test_chat_transcript() {
        let text = r"
User: Hi, can you help me?
Assistant: Sure, what do you need?
User: My name is Sarah and I switched from VS Code to Zed recently.
Assistant: Nice choice!
User: I work at Acme Corp as a backend engineer.
        ";
        let facts = extract_facts(text, 10);
        assert!(
            facts.iter().any(|f| f.to_lowercase().contains("sarah")),
            "Should extract name"
        );
        assert!(
            facts.iter().any(|f| f.to_lowercase().contains("zed")),
            "Should extract tool switch"
        );
    }

    #[test]
    fn test_url_filtered() {
        assert!(is_noise("https://example.com"));
        assert!(!is_noise("Check https://example.com for details"));
    }

    #[test]
    fn test_question_filtered() {
        assert!(is_noise("What time is the meeting?"));
        assert!(is_noise("Can you help me?"));
        assert!(!is_noise("The meeting is at 3pm."));
    }

    #[test]
    fn test_utf8_multibyte_safe() {
        // Text with multibyte characters (Indonesian, emoji) must not panic.
        let text = "Nama saya Budi. Saya suka nasi goreng. 🍜";
        let facts = extract_facts(text, 10);
        assert!(!facts.is_empty(), "Should handle UTF-8 text");

        // Bullet with multibyte
        assert_eq!(
            clean_fact("• Saya tinggal di Jakarta"),
            "Saya tinggal di Jakarta."
        );
    }

    // ── #895 regression tests ──────────────────────────────────────

    #[test]
    fn test_version_number_not_split() {
        // "Go 1.23" should stay as one sentence, not split into "Go 1." + "23"
        let sentences = segment_sentences("Backend uses Go 1.23 with Axum framework.");
        assert!(
            sentences.iter().any(|s| s.contains("1.23")),
            "Version number 1.23 should not be split, got: {:?}",
            sentences
        );
        assert!(
            !sentences
                .iter()
                .any(|s| s.trim() == "Go 1." || s.trim() == "23"),
            "Should not produce broken version fragments"
        );
    }

    #[test]
    fn test_multi_version_not_split() {
        // Multi-segment version like "v0.12.0" should stay intact
        let sentences = segment_sentences("Updated to v0.12.0 today.");
        assert!(
            sentences.iter().any(|s| s.contains("0.12.0")),
            "Version 0.12.0 should not be split, got: {:?}",
            sentences
        );
    }

    #[test]
    fn test_markdown_header_skipped() {
        // Headers like "## Decisions" should not become facts
        let sentences = segment_sentences("## Decisions\nWe chose Rust for performance.");
        assert!(
            !sentences
                .iter()
                .any(|s| s.contains("## Decisions") || s == "Decisions"),
            "Markdown header should be skipped, got: {:?}",
            sentences
        );
        assert!(
            sentences.iter().any(|s| s.contains("Rust")),
            "Content after header should still be extracted"
        );
    }

    #[test]
    fn test_paren_not_truncated() {
        // Sentence with parenthetical should not be split at period inside parens
        let sentences =
            segment_sentences("The deploy uses blue-green strategy (v2.1.0) for safety.");
        assert!(
            sentences
                .iter()
                .any(|s| s.contains("blue-green") && s.contains("v2.1.0")),
            "Parenthetical content should stay with sentence, got: {:?}",
            sentences
        );
    }

    #[test]
    fn test_decimal_in_sentence() {
        // Decimal numbers should not trigger sentence split
        let sentences = segment_sentences("The uptime is 99.9 percent this quarter.");
        assert!(
            sentences.iter().any(|s| s.contains("99.9")),
            "Decimal 99.9 should not be split, got: {:?}",
            sentences
        );
    }

    #[test]
    fn test_nested_parentheses() {
        // Nested parens should not cause premature splitting
        let sentences = segment_sentences("Used the new config (from PR #42 (hotfix)) yesterday.");
        assert!(
            sentences
                .iter()
                .any(|s| s.contains("hotfix") && s.contains("PR #42")),
            "Nested parens should stay together, got: {:?}",
            sentences
        );
    }
}
