//! The privacy filter on the fields next to the content (#1464): tags, the
//! strings inside metadata and the `source`, and what an update reports.

use crate::privacy::{PrivacyConfig, PrivacyMode};
use crate::test_support::open_keyword;
use crate::{Error, Uteke};

fn gh() -> String {
    format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
}

fn store(mode: PrivacyMode) -> Uteke {
    let mut u = open_keyword();
    u.set_privacy_config(PrivacyConfig {
        mode,
        allowlist: Vec::new(),
    });
    u
}

#[test]
fn remember_masks_secrets_in_tags_and_metadata_strings() {
    let u = store(PrivacyMode::Redact);
    let tag = format!("key-{}", gh());
    let meta = serde_json::json!({
        "note": format!("token {}", gh()),
        "nested": {"list": [format!("x {}", gh()), 3]},
        "count": 7,
    });
    let o = u
        .remember_detailed("clean content", &[tag.as_str(), "ops"], Some(meta), None)
        .unwrap();
    assert!(o.redacted());
    assert_eq!(o.redactions.iter().map(|r| r.count).sum::<usize>(), 3);

    let m = u.get_by_id(&o.id).unwrap().unwrap();
    assert_eq!(m.content, "clean content");
    let dump = format!("{:?} {}", m.tags, m.metadata);
    assert!(!dump.contains("aB3dE5gH7jK9"), "{dump}");
    assert!(dump.contains("[REDACTED:github-token]"));
    assert_eq!(m.metadata["count"], 7, "other values are untouched");
    assert_eq!(m.metadata["redacted"], true);
    assert!(m.tags.contains(&"ops".to_string()));
}

#[test]
fn strict_rejects_a_secret_in_a_tag_or_in_metadata() {
    let u = store(PrivacyMode::Strict);
    let tag = gh();
    let err = u
        .remember_detailed("clean", &[tag.as_str()], None, None)
        .unwrap_err();
    assert!(matches!(err, Error::SecretDetected { .. }), "{err}");
    let err = u
        .remember_detailed("clean", &[], Some(serde_json::json!({"k": gh()})), None)
        .unwrap_err();
    assert!(matches!(err, Error::SecretDetected { .. }), "{err}");
    assert!(u.list(None, 10, 0, None).unwrap().is_empty());
}

#[test]
fn an_update_filters_tags_and_metadata_and_reports_it() {
    let u = store(PrivacyMode::Redact);
    let id = u.remember_detailed("plain", &[], None, None).unwrap().id;

    // Metadata only: no new content, a secret in a metadata string.
    let out = u
        .update_memory_detailed(
            &id,
            None,
            None,
            Some(&serde_json::json!({"note": format!("k {}", gh())})),
            None,
            None,
            None,
        )
        .unwrap();
    assert!(out.updated && out.redacted());
    assert_eq!(out.redaction_summary().as_deref(), Some("github-token x1"));
    let m = u.get_by_id(&id).unwrap().unwrap();
    assert!(
        !m.metadata.to_string().contains("aB3dE5gH7jK9"),
        "{}",
        m.metadata
    );
    assert_eq!(m.metadata["redacted"], true);

    // Tags.
    let out = u
        .update_memory_detailed(&id, None, Some(&[gh()]), None, None, None, None)
        .unwrap();
    assert!(out.redacted());
    let m = u.get_by_id(&id).unwrap().unwrap();
    assert!(
        !format!("{:?}", m.tags).contains("aB3dE5gH7jK9"),
        "{:?}",
        m.tags
    );

    // Nothing to mask, and the plain bool API still works.
    let out = u
        .update_memory_detailed(&id, Some("still plain"), None, None, None, None, None)
        .unwrap();
    assert!(out.updated && !out.redacted() && out.redaction_summary().is_none());
    assert!(
        u.update_memory(&id, None, None, None, Some(0.9), None, None)
            .unwrap()
    );
}

#[test]
fn strict_rejects_an_update_with_a_secret_in_metadata_or_tags() {
    let u = store(PrivacyMode::Strict);
    let id = u.remember_detailed("plain", &[], None, None).unwrap().id;
    let err = u
        .update_memory(
            &id,
            None,
            None,
            Some(&serde_json::json!({"k": gh()})),
            None,
            None,
            None,
        )
        .unwrap_err();
    assert!(matches!(err, Error::SecretDetected { .. }), "{err}");
    let err = u
        .update_memory(&id, None, Some(&[gh()]), None, None, None, None)
        .unwrap_err();
    assert!(matches!(err, Error::SecretDetected { .. }), "{err}");
}

#[test]
fn the_source_is_masked_or_refused() {
    let u = store(PrivacyMode::Redact);
    let id = u.remember_detailed("plain", &[], None, None).unwrap().id;
    assert!(
        u.set_source(&id, Some(&format!("https://x.example/?t={}", gh())), "user")
            .unwrap()
    );
    let m = u.get_by_id(&id).unwrap().unwrap();
    let source = m.source.unwrap();
    assert!(
        source.contains("[REDACTED:github-token]") && !source.contains("aB3dE5gH7jK9"),
        "{source}"
    );

    let s = store(PrivacyMode::Strict);
    let id = s.remember_detailed("plain", &[], None, None).unwrap().id;
    let err = s.set_source(&id, Some(&gh()), "user").unwrap_err();
    assert!(matches!(err, Error::SecretDetected { .. }), "{err}");
    assert!(s.get_by_id(&id).unwrap().unwrap().source.is_none());
}

#[test]
fn import_masks_tags_and_source() {
    let u = store(PrivacyMode::Redact);
    let row = serde_json::json!({
        "content": "imported row",
        "tags": [gh(), "ops"],
        "source": format!("file with {}", gh()),
    })
    .to_string();
    let r = u.import(&row, None).unwrap();
    assert_eq!((r.imported, r.redacted), (1, 1));
    let m = &u.list(None, 10, 0, None).unwrap()[0];
    let dump = format!("{:?} {:?}", m.tags, m.source);
    assert!(!dump.contains("aB3dE5gH7jK9"), "{dump}");
    assert!(m.tags.contains(&"ops".to_string()));
}
