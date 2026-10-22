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
