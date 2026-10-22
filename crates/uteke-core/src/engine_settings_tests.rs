//! The `[tier]`, `[dream]` and `[lifecycle]` sections of `uteke.toml` are read
//! straight into the core types (#1515): one schema for the CLI and the server.

use crate::{DreamConfig, LifecycleConfig, TierConfig};

fn parse<T: serde::de::DeserializeOwned>(toml_text: &str) -> Result<T, toml::de::Error> {
    toml::from_str(toml_text)
}

#[test]
fn an_empty_section_is_the_documented_default() {
    let t: TierConfig = parse("").unwrap();
    assert_eq!((t.hot_days, t.warm_days), (7, 30));
    assert!((t.hot_boost - 0.1).abs() < f64::EPSILON);

    let d: DreamConfig = parse("").unwrap();
    assert_eq!(d.contradict_max_memories, 200);
    assert!((d.contradict_similarity_threshold - 0.6).abs() < f32::EPSILON);
    assert!((d.contradict_tag_jaccard_min - 0.4).abs() < f32::EPSILON);
    assert!((d.dedup_threshold - 0.92).abs() < f32::EPSILON);
    assert!((d.orphan_importance_threshold - 0.15).abs() < f64::EPSILON);

    let l: LifecycleConfig = parse("").unwrap();
    assert!(l.soft_delete_only && l.auto_aging_enabled && l.auto_prune_enabled);
    assert!(l.dream_dedup_soft_delete && l.dream_compact_soft_delete);
    assert_eq!(
        (
            l.auto_aging_interval_hours,
            l.min_age_days,
            l.max_access_count
        ),
        (168, 90, 3)
    );
    assert_eq!(
        (
            l.min_deprecate_per_cycle,
            l.max_deprecate_per_cycle,
            l.deprecated_ttl_days
        ),
        (1, 50, 30)
    );
    assert!((l.max_deprecate_percent - 1.0).abs() < f64::EPSILON);
}

#[test]
fn a_partial_section_keeps_the_default_for_the_keys_it_omits() {
    let t: TierConfig = parse("hot_days = 3").unwrap();
    assert_eq!((t.hot_days, t.warm_days), (3, 30));
    let d: DreamConfig = parse("dedup_threshold = 0.8").unwrap();
    assert!((d.dedup_threshold - 0.8).abs() < f32::EPSILON);
    assert_eq!(d.contradict_max_memories, 200);
    let l: LifecycleConfig = parse("soft_delete_only = false\ndeprecated_ttl_days = 5").unwrap();
    assert!(!l.soft_delete_only);
    assert_eq!(l.deprecated_ttl_days, 5);
    assert_eq!(l.min_age_days, 90);
}

#[test]
fn negative_tier_days_are_rejected_as_before() {
    assert!(parse::<TierConfig>("hot_days = -1").is_err());
    assert!(parse::<TierConfig>("warm_days = -30").is_err());
    // Zero is a number of days.
    assert_eq!(parse::<TierConfig>("hot_days = 0").unwrap().hot_days, 0);
}

#[test]
fn a_wrong_type_is_an_error_not_a_silent_default() {
    assert!(parse::<LifecycleConfig>("min_age_days = \"ninety\"").is_err());
    assert!(parse::<DreamConfig>("contradict_max_memories = -5").is_err());
}

mod recall {
    use crate::RecallSettings;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn parse(toml_text: &str) -> RecallSettings {
        toml::from_str(toml_text).unwrap()
    }

    #[test]
    fn an_empty_section_is_the_documented_default() {
        let r = parse("");
        assert_eq!((r.min_score, r.min_score_strict), (0.0, 0.5));
        assert_eq!(r.default_strategy, "fusion");
        assert_eq!(
            (r.graph_density_weight, r.graph_authority_weight),
            (0.1, 0.1)
        );
        assert!(r.graph_rerank_enabled);
        assert_eq!(
            (r.salience_weight, r.recency_weight, r.jaccard_weight),
            (0.15, 0.15, 0.0)
        );
    }

    #[test]
    fn the_defaults_equal_the_built_in_request_defaults() {
        // The server leaves a request to the built-ins when [recall] is absent;
        // reading the default section must give the same thresholds.
        let r = RecallSettings::default();
        assert_eq!(r.min_score as f32, crate::RECALL_DEFAULT_MIN_SCORE);
        assert_eq!(r.min_score_strict as f32, crate::RECALL_STRICT_MIN_SCORE);
        assert!(crate::RecallStrategy::from_str_opt(&r.default_strategy).is_some());
    }

    #[test]
    fn a_partial_section_keeps_the_other_defaults() {
        let r = parse("min_score = 0.2\ndefault_strategy = \"vector\"");
        assert_eq!(r.min_score, 0.2);
        assert_eq!(r.default_strategy, "vector");
        assert_eq!(r.min_score_strict, 0.5);
        assert!(r.graph_rerank_enabled);
    }

    #[test]
    fn conversions_to_the_engine_types() {
        let r = parse(
            "min_score = 0.25\ngraph_density_weight = 0.3\ngraph_authority_weight = 0.2\ngraph_rerank_enabled = false",
        );
        let g = r.graph_rerank();
        assert_eq!(
            (g.density_weight, g.authority_weight, g.enabled),
            (0.3, 0.2, false)
        );
        assert_eq!(r.recall_config().min_score, 0.25_f32);
    }

    #[test]
    fn valid_overrides_win_over_the_file() {
        let r = parse("min_score = 0.2").with_overrides_from(env(&[
            ("UTEKE_RECALL_MIN_SCORE", "0.7"),
            ("UTEKE_RECALL_MIN_SCORE_STRICT", "0.9"),
            ("UTEKE_RECALL_STRATEGY", "graph"),
            ("UTEKE_GRAPH_DENSITY_WEIGHT", "0.4"),
            ("UTEKE_GRAPH_AUTHORITY_WEIGHT", "0.6"),
            ("UTEKE_GRAPH_RERANK_ENABLED", "OFF"),
        ]));
        assert_eq!((r.min_score, r.min_score_strict), (0.7, 0.9));
        assert_eq!(r.default_strategy, "graph");
        assert_eq!(
            (r.graph_density_weight, r.graph_authority_weight),
            (0.4, 0.6)
        );
        assert!(!r.graph_rerank_enabled);
    }

    #[test]
    fn invalid_or_out_of_range_overrides_are_ignored() {
        let base = parse("min_score = 0.2\ndefault_strategy = \"vector\"");
        let r = base.clone().with_overrides_from(env(&[
            ("UTEKE_RECALL_MIN_SCORE", "1.5"),
            ("UTEKE_RECALL_MIN_SCORE_STRICT", "high"),
            ("UTEKE_RECALL_STRATEGY", "nonsense"),
            ("UTEKE_GRAPH_DENSITY_WEIGHT", "-0.1"),
            ("UTEKE_GRAPH_AUTHORITY_WEIGHT", "NaN"),
            ("UTEKE_GRAPH_RERANK_ENABLED", "maybe"),
        ]));
        assert_eq!(r.min_score, 0.2);
        assert_eq!(r.min_score_strict, 0.5);
        assert_eq!(r.default_strategy, "vector");
        assert_eq!(r.graph_density_weight, 0.1);
        assert_eq!(r.graph_authority_weight, 0.1);
        assert!(r.graph_rerank_enabled);
    }
}
