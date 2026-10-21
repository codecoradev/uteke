//! The privacy filter on the write paths (#1464, part 2b-1): `remember`,
//! `remember_with_contradiction`, room remember, `update_memory` and `import`.
//! A masked secret must be absent from the content, the FTS index, the
//! metadata, the timeline and the database file itself.

use crate::memory::types::DEFAULT_NAMESPACE;
use crate::privacy::{PrivacyConfig, PrivacyMode, compile_allowlist};
use crate::test_support::{open_keyword, open_keyword_at};
use crate::{Error, Uteke};

/// The fake credential, assembled at run time so the repository holds no
/// string a scanner would flag.
fn gh() -> String {
    format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
}

fn aws() -> String {
    format!("{}{}", "AKIA", "IOSFODNN7EXAMPLQ")
}

fn store(mode: PrivacyMode) -> Uteke {
    let mut u = open_keyword();
    u.set_privacy_config(PrivacyConfig {
        mode,
        allowlist: Vec::new(),
    });
    u
}

fn kinds(o: &crate::operations::RememberOutcome) -> Vec<(&'static str, usize)> {
    o.redactions.iter().map(|r| (r.kind, r.count)).collect()
}

#[test]
fn redact_mode_masks_the_secret_and_flags_the_memory() {
    let u = store(PrivacyMode::Redact);
    let text = format!("deploy with {} then {}", gh(), aws());
    let o = u
        .remember_typed_detailed(&text, &["ops"], None, None, "fact")
        .unwrap();
    assert!(o.redacted());
    assert_eq!(kinds(&o), vec![("aws-access-key", 1), ("github-token", 1)]);

    let m = u.get_by_id(&o.id).unwrap().unwrap();
    assert_eq!(
        m.content,
        "deploy with ghp_[REDACTED:github-token] then AKIA[REDACTED:aws-access-key]"
    );
    assert_eq!(m.metadata["redacted"], true);
    assert_eq!(
        m.metadata["redactions"],
        serde_json::json!([
            {"kind": "aws-access-key", "count": 1},
            {"kind": "github-token", "count": 1}
        ])
    );
    // Never the value, in the metadata or in the audit event.
    let flags = m.metadata.to_string();
    assert!(!flags.contains(&gh()) && !flags.contains(&aws()), "{flags}");

    // The audit entry names the kinds and nothing else.
    let events = u.timeline(&o.id, 10).unwrap();
    let redacted: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "redacted")
        .collect();
    assert_eq!(redacted.len(), 1, "{events:?}");
    let data = redacted[0].event_data.clone().unwrap();
    assert!(
        data.contains("github-token") && !data.contains(&gh()),
        "{data}"
    );

    // Recall and keyword search see the masked form only.
    let hits = u.search("deploy", 10, None, None).unwrap();
    assert_eq!(hits[0].memory.content, m.content);
    assert!(u.search(&gh(), 10, None, None).unwrap().is_empty());
}

#[test]
fn clean_content_and_caller_metadata_are_untouched_and_merged() {
    let u = store(PrivacyMode::Redact);
    let clean = u
        .remember_typed_detailed(
            "nothing secret here",
            &[],
            Some(serde_json::json!({"k": "v"})),
            None,
            "fact",
        )
        .unwrap();
    assert!(!clean.redacted());
    let m = u.get_by_id(&clean.id).unwrap().unwrap();
    assert_eq!(
        m.metadata,
        serde_json::json!({"k": "v"}),
        "no flags on clean content"
    );
    assert!(
        u.timeline(&clean.id, 10)
            .unwrap()
            .iter()
            .all(|e| e.event_type != "redacted")
    );

    // With a secret, the caller's keys stay next to the flags.
    let o = u
        .remember_typed_detailed(
            &format!("key {}", gh()),
            &[],
            Some(serde_json::json!({"k": "v"})),
            None,
            "fact",
        )
        .unwrap();
    let m = u.get_by_id(&o.id).unwrap().unwrap();
    assert_eq!(m.metadata["k"], "v");
    assert_eq!(m.metadata["redacted"], true);
}

#[test]
fn strict_mode_refuses_the_write_and_stores_nothing() {
    let u = store(PrivacyMode::Strict);
    let err = u
        .remember_typed_detailed(&format!("key {}", gh()), &[], None, None, "fact")
        .unwrap_err();
    match &err {
        Error::SecretDetected { kinds } => assert_eq!(kinds, &vec!["github-token".to_string()]),
        other => panic!("expected SecretDetected, got {other:?}"),
    }
    assert!(err.to_string().contains("github-token"), "{err}");
    assert!(
        !err.to_string().contains(&gh()),
        "the error never carries the value"
    );
    assert_eq!(u.count(None).unwrap(), 0);
    // Clean content is still accepted in strict mode.
    u.remember("plain note", &[], None, None).unwrap();
    assert_eq!(u.count(None).unwrap(), 1);
}

#[test]
fn off_mode_stores_the_content_as_given() {
    let u = store(PrivacyMode::Off);
    let o = u
        .remember_typed_detailed(&format!("key {}", gh()), &[], None, None, "fact")
        .unwrap();
    assert!(!o.redacted());
    let m = u.get_by_id(&o.id).unwrap().unwrap();
    assert!(m.content.contains(&gh()));
    assert!(m.metadata.get("redacted").is_none());
}

#[test]
fn the_allowlist_keeps_a_matching_value() {
    let mut u = open_keyword();
    u.set_privacy_config(PrivacyConfig {
        mode: PrivacyMode::Redact,
        allowlist: compile_allowlist(&["^ghp_aB3".to_string()]).unwrap(),
    });
    let o = u
        .remember_typed_detailed(&format!("key {}", gh()), &[], None, None, "fact")
        .unwrap();
    assert!(!o.redacted());
    assert!(u.get_by_id(&o.id).unwrap().unwrap().content.contains(&gh()));
}

#[test]
fn the_contradiction_and_room_paths_are_filtered_too() {
    let u = store(PrivacyMode::Redact);
    let (o, _) = u
        .remember_with_contradiction(&format!("key {}", gh()), &[], None, None, None, true, 0.9)
        .unwrap();
    assert_eq!(kinds(&o), vec![("github-token", 1)]);
    assert!(!u.get_by_id(&o.id).unwrap().unwrap().content.contains(&gh()));

    let room = u
        .remember_in_room(
            &format!("room {}", aws()),
            &[],
            None,
            None,
            "fact",
            "r1",
            "alice",
        )
        .unwrap();
    assert_eq!(kinds(&room), vec![("aws-access-key", 1)]);
    assert!(
        !u.get_by_id(&room.id)
            .unwrap()
            .unwrap()
            .content
            .contains(&aws())
    );

    let strict = store(PrivacyMode::Strict);
    assert!(matches!(
        strict.remember_with_contradiction(
            &format!("key {}", gh()),
            &[],
            None,
            None,
            None,
            true,
            0.9
        ),
        Err(Error::SecretDetected { .. })
    ));
    assert!(matches!(
        strict.remember_in_room(
            &format!("room {}", aws()),
            &[],
            None,
            None,
            "fact",
            "r1",
            "alice"
        ),
        Err(Error::SecretDetected { .. })
    ));
    assert_eq!(strict.count(None).unwrap(), 0);
}

#[test]
fn update_memory_masks_new_content_and_flags_the_metadata() {
    let u = store(PrivacyMode::Redact);
    let id = u
        .remember_typed_detailed(
            "first version",
            &[],
            Some(serde_json::json!({"k": "v"})),
            None,
            "fact",
        )
        .unwrap()
        .id;
    assert!(
        u.update_memory(
            &id,
            Some(&format!("now with {}", gh())),
            None,
            None,
            None,
            None,
            None
        )
        .unwrap()
    );
    let m = u.get_by_id(&id).unwrap().unwrap();
    assert_eq!(m.content, "now with ghp_[REDACTED:github-token]");
    assert_eq!(m.metadata["k"], "v", "the existing metadata is kept");
    assert_eq!(m.metadata["redacted"], true);
    assert_eq!(
        u.timeline(&id, 20)
            .unwrap()
            .iter()
            .filter(|e| e.event_type == "redacted")
            .count(),
        1
    );
    // An update without content leaves the flags alone.
    u.update_memory(&id, None, None, None, Some(0.9), None, None)
        .unwrap();
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().metadata["redacted"],
        true
    );

    // Strict: the update is refused and the memory keeps its old content.
    let s = store(PrivacyMode::Strict);
    let sid = s.remember("original", &[], None, None).unwrap();
    let err = s
        .update_memory(
            &sid,
            Some(&format!("now {}", gh())),
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
    assert!(matches!(err, Error::SecretDetected { .. }), "{err:?}");
    assert_eq!(s.get_by_id(&sid).unwrap().unwrap().content, "original");
}

/// The flags describe the current content: an update to clean content drops
/// the ones an earlier version left (the timeline keeps the history), and a
/// caller who supplies metadata owns it.
#[test]
fn a_clean_update_drops_the_stale_flags_and_keeps_the_other_metadata() {
    let u = store(PrivacyMode::Redact);
    let id = u
        .remember_typed_detailed(
            &format!("key {}", gh()),
            &[],
            Some(serde_json::json!({"k": "v"})),
            None,
            "fact",
        )
        .unwrap()
        .id;
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().metadata["redacted"],
        true
    );

    u.update_memory(
        &id,
        Some("rotated, nothing secret now"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let m = u.get_by_id(&id).unwrap().unwrap();
    assert_eq!(m.content, "rotated, nothing secret now");
    assert_eq!(
        m.metadata,
        serde_json::json!({"k": "v"}),
        "flags dropped, k kept"
    );
    let events = u.timeline(&id, 20).unwrap();
    assert_eq!(
        events.iter().filter(|e| e.event_type == "redacted").count(),
        1,
        "the history of the first masking stays"
    );

    // An update that does not touch the content keeps whatever flags exist.
    let id2 = u
        .remember_typed_detailed(&format!("key {}", gh()), &[], None, None, "fact")
        .unwrap()
        .id;
    u.update_memory(&id2, None, Some(&["t".to_string()]), None, None, None, None)
        .unwrap();
    assert_eq!(
        u.get_by_id(&id2).unwrap().unwrap().metadata["redacted"],
        true
    );

    // Metadata supplied by the caller replaces it as before, untouched.
    u.update_memory(
        &id2,
        Some("clean again"),
        None,
        Some(&serde_json::json!({"mine": 1})),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        u.get_by_id(&id2).unwrap().unwrap().metadata,
        serde_json::json!({"mine": 1})
    );
}

/// Flags follow the content: a read-edit-write round trip of masked text and a
/// metadata-only update both keep them; null import metadata still gets them.
#[test]
fn flags_survive_a_masked_round_trip_a_metadata_only_update_and_null_metadata() {
    let u = store(PrivacyMode::Redact);
    let id = u
        .remember_typed_detailed(
            &format!("key {}", gh()),
            &[],
            Some(serde_json::json!({"k": "v"})),
            None,
            "fact",
        )
        .unwrap()
        .id;

    // An agent reads the masked memory, fixes a word and writes it back: the
    // content still carries the mask marker, so the flags stay.
    u.update_memory(
        &id,
        Some("key ghp_[REDACTED:github-token] (rotated)"),
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().metadata["redacted"],
        true
    );

    // A metadata-only update replaces the metadata but keeps the flags that
    // describe the unchanged content.
    u.update_memory(
        &id,
        None,
        None,
        Some(&serde_json::json!({"new": 1})),
        None,
        None,
        None,
    )
    .unwrap();
    let m = u.get_by_id(&id).unwrap().unwrap();
    assert_eq!(m.metadata["new"], 1);
    assert_eq!(m.metadata["redacted"], true);
    assert!(
        m.metadata.get("k").is_none(),
        "the caller's metadata replaced the old keys"
    );
    // Metadata that sets its own flags is left as given.
    u.update_memory(
        &id,
        None,
        None,
        Some(&serde_json::json!({"redacted": false})),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().metadata["redacted"],
        false
    );

    // An import row with `"metadata": null` is still flagged.
    let file = format!("{{\"content\":\"export {}\",\"metadata\":null}}", gh());
    let imp = store(PrivacyMode::Redact);
    imp.import(&file, None).unwrap();
    let row = &imp.list(None, 10, 0, None).unwrap()[0];
    assert_eq!(row.metadata["redacted"], true, "{row:?}");
}

#[test]
fn import_masks_rows_and_strict_skips_them() {
    let file = format!(
        "{{\"content\":\"export {}\"}}\n{{\"content\":\"plain row\"}}",
        gh()
    );
    let u = store(PrivacyMode::Redact);
    let r = u.import(&file, None).unwrap();
    assert_eq!(
        (r.imported, r.redacted, r.rejected_secret),
        (2, 1, 0),
        "one of the two rows had a secret masked"
    );
    let rows = u.list(None, 10, 0, None).unwrap();
    assert!(rows.iter().all(|m| !m.content.contains(&gh())), "{rows:?}");
    let masked = rows
        .iter()
        .find(|m| m.content.contains("[REDACTED:github-token]"))
        .unwrap();
    assert_eq!(masked.metadata["redacted"], true);
    assert!(
        rows.iter()
            .find(|m| m.content == "plain row")
            .unwrap()
            .metadata
            .get("redacted")
            .is_none()
    );

    let s = store(PrivacyMode::Strict);
    let r = s.import(&file, None).unwrap();
    assert_eq!(
        (r.imported, r.skipped, r.rejected_secret, r.redacted),
        (1, 1, 1, 0),
        "the row with a secret is skipped and counted"
    );
    assert_eq!(s.list(None, 10, 0, None).unwrap()[0].content, "plain row");
}

/// The strongest check: after a masked write, the secret is nowhere in the
/// database files: not in the row, the FTS index, the metadata or the timeline.
#[test]
fn the_secret_is_not_in_the_database_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("uteke.db");
    {
        let u = open_keyword_at(&path);
        let o = u
            .remember_typed_detailed(
                &format!("deploy {} now", gh()),
                &["ops"],
                None,
                Some(DEFAULT_NAMESPACE),
                "fact",
            )
            .unwrap();
        assert!(o.redacted());
        assert!(
            u.update_memory(
                &o.id,
                Some(&format!("again {}", aws())),
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap()
        );
        u.import(&format!("{{\"content\":\"imported {}\"}}", gh()), None)
            .unwrap();
    }
    let mut scanned = 0;
    for entry in std::fs::read_dir(dir.path()).unwrap().flatten() {
        let bytes = std::fs::read(entry.path()).unwrap();
        scanned += bytes.len();
        for secret in [gh(), aws()] {
            let found = bytes.windows(secret.len()).any(|w| w == secret.as_bytes());
            assert!(!found, "{secret} found in {:?}", entry.file_name());
        }
    }
    assert!(scanned > 0, "the database files were scanned");
}

#[test]
fn the_mode_parses_and_defaults_to_redact() {
    assert_eq!(PrivacyMode::default(), PrivacyMode::Redact);
    assert_eq!(PrivacyMode::parse(" Strict ").unwrap(), PrivacyMode::Strict);
    assert_eq!(PrivacyMode::parse("OFF").unwrap(), PrivacyMode::Off);
    assert!(PrivacyMode::parse("paranoid").is_err());
    // A fresh store masks by default.
    let u = open_keyword();
    assert_eq!(u.privacy_config().mode, PrivacyMode::Redact);
}

/// New content together with new metadata: the flags follow the content, not
/// the caller's metadata and not the previous version.
#[test]
fn an_update_with_content_and_metadata_flags_by_the_content() {
    let u = store(PrivacyMode::Redact);
    let id = u
        .remember_typed_detailed(&format!("key {}", gh()), &[], None, None, "fact")
        .unwrap()
        .id;

    // Clean content: a flag the caller put in the metadata is dropped.
    u.update_memory(
        &id,
        Some("nothing secret"),
        None,
        Some(&serde_json::json!({"k": "v", "redacted": true})),
        None,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        u.get_by_id(&id).unwrap().unwrap().metadata,
        serde_json::json!({"k": "v"})
    );

    // Content that still carries a mask marker keeps the earlier flags.
    let id2 = u
        .remember_typed_detailed(&format!("key {}", gh()), &[], None, None, "fact")
        .unwrap()
        .id;
    u.update_memory(
        &id2,
        Some("edited, still [REDACTED:github-token]"),
        None,
        Some(&serde_json::json!({"k": "v"})),
        None,
        None,
        None,
    )
    .unwrap();
    let m = u.get_by_id(&id2).unwrap().unwrap();
    assert_eq!(m.metadata["k"], "v");
    assert_eq!(m.metadata["redacted"], true);
}

#[test]
fn the_redaction_summary_names_kinds_and_counts_not_values() {
    let u = store(PrivacyMode::Redact);
    let masked = u
        .remember_detailed(&format!("a {} b {}", gh(), aws()), &[], None, None)
        .unwrap();
    let summary = masked.redaction_summary().expect("two kinds masked");
    assert!(summary.contains("github-token x1"), "{summary}");
    assert!(summary.contains("aws-access-key x1"), "{summary}");
    assert!(!summary.contains(&gh()));

    let clean = u
        .remember_detailed("nothing secret", &[], None, None)
        .unwrap();
    assert_eq!(clean.redaction_summary(), None);
}
