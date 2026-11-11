//! `uteke namespace migrate` (#1427 phase 3, task I).

use crate::Uteke;
use crate::namespace_migrate::{NamespaceMigrationMap, NamespaceMigrationReport};
use crate::remember_request::RememberRequest;
use crate::test_support::{open_keyword, open_keyword_at};

fn remember(
    u: &Uteke,
    ns: &str,
    content: &str,
    author: Option<&str>,
    source: Option<&str>,
) -> String {
    let req = RememberRequest {
        content: content.to_string(),
        tags: vec!["seed".to_string()],
        namespace: Some(ns.to_string()),
        memory_type: Some("fact".to_string()),
        metadata: None,
        detect_contradiction: false,
        source: source.map(str::to_string),
        source_type: None,
        author_type: None,
        room: None,
        author: author.map(str::to_string),
        validity: None,
    };
    u.run_remember(&req).unwrap().outcome.id
}

/// Every row the migration may touch, in a stable order, so a before/after
/// comparison shows any write.
fn snapshot(u: &Uteke) -> String {
    let conn = u.graph_store();
    let mut out = String::new();
    for sql in [
        "SELECT id, namespace, tags, author, source, deprecated, updated_at FROM memories ORDER BY id",
        "SELECT memory_id, tag FROM memory_tags ORDER BY memory_id, tag",
        "SELECT id, namespace, updated_at FROM rooms ORDER BY id",
        "SELECT room_id, memory_id, author FROM room_memories ORDER BY room_id, memory_id",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let cols = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                let mut line = Vec::new();
                for i in 0..cols {
                    let v: rusqlite::types::Value = row.get(i)?;
                    line.push(format!("{v:?}"));
                }
                Ok(line.join("|"))
            })
            .unwrap();
        for r in rows {
            out.push_str(&r.unwrap());
            out.push('\n');
        }
    }
    out
}

fn map(toml: &str) -> NamespaceMigrationMap {
    NamespaceMigrationMap::parse(toml).unwrap()
}

fn entry<'a>(
    report: &'a NamespaceMigrationReport,
    ns: &str,
) -> &'a crate::namespace_migrate::NamespaceMigration {
    report
        .namespaces
        .iter()
        .find(|e| e.namespace == ns)
        .unwrap_or_else(|| panic!("no entry for {ns}: {report:?}"))
}

fn export_path(dir: &tempfile::TempDir) -> std::path::PathBuf {
    dir.path().join("before-migrate.ndjson")
}

// --- the map file ---------------------------------------------------------

#[test]
fn the_map_parses_every_target_kind() {
    let m = map(r#"
        [map."ns-room"]
        room = "r1"
        [map."ns-tag"]
        tag = "x"
        [map."ns-author"]
        author = "agent-a"
        [map."ns-source"]
        source = "host-1"
        [map."ns-drop"]
        drop = true
        [map."ns-both"]
        room = "r2"
        author = "agent-b"
    "#);
    assert_eq!(m.map.len(), 6);
    assert_eq!(m.map["ns-room"].room.as_deref(), Some("r1"));
    assert!(m.map["ns-drop"].drop);
    assert_eq!(m.map["ns-both"].author.as_deref(), Some("agent-b"));
}

#[test]
fn the_map_never_names_default() {
    let err = NamespaceMigrationMap::parse("[map.default]\ntag = \"x\"").unwrap_err();
    assert!(err.to_string().contains("default"), "{err}");
}

#[test]
fn the_map_rejects_an_entry_without_target_a_drop_with_targets_and_unknown_keys() {
    for bad in [
        "[map.a]\n",
        "[map.a]\ndrop = true\ntag = \"x\"",
        "[map.a]\nnamespace = \"b\"",
        "[map.a]\ntag = \"  \"",
        "[map.a]\nroom = \"\"",
        "[map.a]\nauthor = \"\"",
        "[map.a]\nsource = \"\"",
        "[other]\n",
    ] {
        assert!(
            NamespaceMigrationMap::parse(bad).is_err(),
            "accepted a bad map: {bad:?}"
        );
    }
}

#[test]
fn the_namespace_name_is_one_to_128_characters() {
    // blank names: empty and whitespace only
    for blank in ["", "   "] {
        let toml = format!("[map.\"{blank}\"]\ntag = \"x\"");
        assert!(
            NamespaceMigrationMap::parse(&toml).is_err(),
            "accepted a blank name {blank:?}"
        );
    }
    let at_limit = "a".repeat(128);
    let over_limit = "a".repeat(129);
    let ok = NamespaceMigrationMap::parse(&format!("[map.\"{at_limit}\"]\ntag = \"x\""))
        .expect("128 characters is the longest allowed name");
    assert!(ok.map.contains_key(&at_limit));
    let err =
        NamespaceMigrationMap::parse(&format!("[map.\"{over_limit}\"]\ntag = \"x\"")).unwrap_err();
    assert!(err.to_string().contains("1-128 characters"), "{err}");
}

// --- dry run --------------------------------------------------------------

#[test]
fn a_dry_run_writes_nothing_and_reports_what_would_move() {
    let u = open_keyword();
    remember(&u, "ns-a", "alpha one", None, None);
    remember(&u, "ns-a", "alpha two", None, None);
    remember(&u, "ns-b", "beta one", None, None);
    remember(&u, "default", "default one", None, None);
    let before = snapshot(&u);

    let report = u
        .plan_namespace_migration(&map(
            "[map.ns-a]\nroom = \"new-room\"\n[map.ns-b]\ndrop = true",
        ))
        .unwrap();

    assert_eq!(snapshot(&u), before, "a dry run wrote to the store");
    assert!(!report.applied);
    assert!(report.export_path.is_none());
    let a = entry(&report, "ns-a");
    assert!(a.found);
    assert_eq!(a.memories, 2);
    assert!(a.room_created, "a missing room is reported as created");
    let b = entry(&report, "ns-b");
    assert_eq!(b.memories, 1);
    assert!(b.target.drop);
    assert!(u.get_room("new-room").unwrap().is_none());
}

#[test]
fn unmapped_namespaces_are_reported_and_left_alone() {
    let u = open_keyword();
    remember(&u, "ns-a", "alpha", None, None);
    remember(&u, "ns-left", "left one", None, None);
    remember(&u, "default", "default one", None, None);
    let dir = tempfile::tempdir().unwrap();

    let report = u
        .apply_namespace_migration(&map("[map.ns-a]\ntag = \"a\""), &export_path(&dir), false)
        .unwrap();

    assert_eq!(report.unmapped.len(), 1, "{report:?}");
    assert_eq!(report.unmapped[0].namespace, "ns-left");
    assert_eq!(report.unmapped[0].memories, 1);
    assert_eq!(u.count(Some("ns-left")).unwrap(), 1);
}

// --- each target kind -----------------------------------------------------

#[test]
fn a_room_target_that_does_not_exist_is_created_and_linked() {
    let u = open_keyword();
    let id1 = remember(&u, "research-x", "room memory one", Some("agent-a"), None);
    let id2 = remember(&u, "research-x", "room memory two", None, None);
    let dir = tempfile::tempdir().unwrap();

    let report = u
        .apply_namespace_migration(
            &map("[map.research-x]\nroom = \"research-x\""),
            &export_path(&dir),
            false,
        )
        .unwrap();

    assert!(report.applied);
    let e = entry(&report, "research-x");
    assert_eq!(e.memories, 2);
    assert!(e.room_created);
    let room = u.get_room("research-x").unwrap().expect("room created");
    assert_eq!(room.namespace, "default");
    let mut linked = u.store.get_room_memory_ids("research-x", None).unwrap();
    linked.sort();
    let mut want = vec![id1.clone(), id2.clone()];
    want.sort();
    assert_eq!(linked, want);
    for id in [&id1, &id2] {
        assert_eq!(u.get(id).unwrap().namespace, "default");
    }
    assert_eq!(u.count(Some("research-x")).unwrap(), 0);
}

#[test]
fn a_room_target_that_exists_is_reused() {
    let u = open_keyword();
    u.create_room("shared", Some("Shared"), "default").unwrap();
    let id = remember(&u, "ns-a", "into an existing room", None, None);
    let dir = tempfile::tempdir().unwrap();

    let report = u
        .apply_namespace_migration(
            &map("[map.ns-a]\nroom = \"shared\""),
            &export_path(&dir),
            false,
        )
        .unwrap();

    assert!(!entry(&report, "ns-a").room_created);
    assert_eq!(
        u.get_room("shared").unwrap().unwrap().title.as_deref(),
        Some("Shared")
    );
    assert_eq!(
        u.store.get_room_memory_ids("shared", None).unwrap(),
        vec![id]
    );
}

#[test]
fn a_tag_target_lands_in_the_tag_column_and_the_tag_index() {
    let u = open_keyword();
    let id = remember(&u, "ns-a", "tagged memory", None, None);
    let dir = tempfile::tempdir().unwrap();

    u.apply_namespace_migration(
        &map("[map.ns-a]\ntag = \"proj:a\""),
        &export_path(&dir),
        false,
    )
    .unwrap();

    let m = u.get(&id).unwrap();
    assert_eq!(m.namespace, "default");
    assert_eq!(m.tags, vec!["seed".to_string(), "proj:a".to_string()]);
    assert_eq!(u.store.count_by_tag("proj:a", None).unwrap(), 1);
}

#[test]
fn an_author_target_fills_the_author_and_keeps_a_different_one_as_a_conflict() {
    let u = open_keyword();
    let plain = remember(&u, "ns-a", "no author yet", None, None);
    let owned = remember(&u, "ns-a", "already owned", Some("agent-z"), None);
    let same = remember(&u, "ns-a", "same author", Some("agent-a"), None);
    let dir = tempfile::tempdir().unwrap();

    let dry = u
        .plan_namespace_migration(&map("[map.ns-a]\nauthor = \"agent-a\""))
        .unwrap();
    assert_eq!(entry(&dry, "ns-a").author_conflicts, 1);

    let report = u
        .apply_namespace_migration(
            &map("[map.ns-a]\nauthor = \"agent-a\""),
            &export_path(&dir),
            false,
        )
        .unwrap();

    assert_eq!(entry(&report, "ns-a").author_conflicts, 1);
    assert_eq!(u.get(&plain).unwrap().author.as_deref(), Some("agent-a"));
    assert_eq!(u.get(&owned).unwrap().author.as_deref(), Some("agent-z"));
    assert_eq!(u.get(&same).unwrap().author.as_deref(), Some("agent-a"));
}

#[test]
fn a_source_target_fills_the_source_and_keeps_a_different_one_as_a_conflict() {
    let u = open_keyword();
    let plain = remember(&u, "ns-a", "no source yet", None, None);
    let owned = remember(&u, "ns-a", "already sourced", None, Some("host-9"));
    let dir = tempfile::tempdir().unwrap();

    let report = u
        .apply_namespace_migration(
            &map("[map.ns-a]\nsource = \"host-1\""),
            &export_path(&dir),
            false,
        )
        .unwrap();

    assert_eq!(entry(&report, "ns-a").source_conflicts, 1);
    assert_eq!(u.get(&plain).unwrap().source.as_deref(), Some("host-1"));
    assert_eq!(u.get(&owned).unwrap().source.as_deref(), Some("host-9"));
}

#[test]
fn targets_compose_room_and_author() {
    let u = open_keyword();
    let id = remember(&u, "ns-a", "composed", None, None);
    let dir = tempfile::tempdir().unwrap();

    u.apply_namespace_migration(
        &map("[map.ns-a]\nroom = \"r\"\nauthor = \"agent-a\""),
        &export_path(&dir),
        false,
    )
    .unwrap();

    assert_eq!(u.get(&id).unwrap().author.as_deref(), Some("agent-a"));
    let author: String = u
        .graph_store()
        .query_row(
            "SELECT author FROM room_memories WHERE room_id = 'r' AND memory_id = ?1",
            [&id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(author, "agent-a");
}

#[test]
fn a_drop_needs_allow_drop_and_then_soft_deletes() {
    let u = open_keyword();
    let id = remember(&u, "ns-junk", "throw me away", None, None);
    remember(&u, "ns-keep", "keep me", None, None);
    let dir = tempfile::tempdir().unwrap();
    let before = snapshot(&u);
    let m = map("[map.ns-junk]\ndrop = true\n[map.ns-keep]\ntag = \"k\"");

    let err = u
        .apply_namespace_migration(&m, &export_path(&dir), false)
        .unwrap_err();
    assert!(err.to_string().contains("--allow-drop"), "{err}");
    assert_eq!(snapshot(&u), before, "a refused run wrote to the store");
    assert!(!export_path(&dir).exists(), "a refused run wrote an export");

    let report = u
        .apply_namespace_migration(&m, &export_path(&dir), true)
        .unwrap();
    assert_eq!(entry(&report, "ns-junk").memories, 1);
    let dropped = u.store.get_by_id(&id).unwrap().unwrap();
    assert!(
        dropped.deprecated,
        "a drop soft-deletes, it never hard-deletes"
    );
    assert_eq!(u.count(Some("ns-junk")).unwrap(), 0);
}

// --- safety: export, idempotency, default, unknown ------------------------

#[test]
fn apply_exports_first_and_the_export_restores_the_original() {
    let dir = tempfile::tempdir().unwrap();
    let u = open_keyword_at(dir.path().join("live.db"));
    remember(&u, "ns-a", "restorable alpha", Some("agent-a"), None);
    remember(&u, "ns-b", "restorable beta", None, Some("host-1"));
    remember(&u, "default", "restorable default", None, None);
    let mut original = u.list_namespaces_with_counts().unwrap();
    original.sort();

    let path = export_path(&dir);
    let report = u
        .apply_namespace_migration(
            &map("[map.ns-a]\ntag = \"a\"\n[map.ns-b]\nroom = \"b\""),
            &path,
            false,
        )
        .unwrap();
    assert_eq!(
        report.export_path.as_deref(),
        Some(path.to_string_lossy().as_ref())
    );
    assert_eq!(
        u.list_namespaces_with_counts().unwrap(),
        vec![("default".to_string(), 3)]
    );

    let restored = open_keyword_at(dir.path().join("restored.db"));
    restored
        .import_full(&std::fs::read_to_string(&path).unwrap())
        .unwrap();
    let mut back = restored.list_namespaces_with_counts().unwrap();
    back.sort();
    assert_eq!(back, original);
}

#[test]
fn apply_refuses_to_overwrite_an_existing_export() {
    let u = open_keyword();
    remember(&u, "ns-a", "alpha", None, None);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(export_path(&dir), "keep me").unwrap();
    let before = snapshot(&u);

    let err = u
        .apply_namespace_migration(&map("[map.ns-a]\ntag = \"a\""), &export_path(&dir), false)
        .unwrap_err();

    assert!(err.to_string().contains("exists"), "{err}");
    assert_eq!(
        std::fs::read_to_string(export_path(&dir)).unwrap(),
        "keep me"
    );
    assert_eq!(snapshot(&u), before);
}

#[test]
fn a_rerun_is_a_no_op() {
    let u = open_keyword();
    remember(&u, "ns-a", "alpha", None, None);
    remember(&u, "ns-b", "beta", None, None);
    let m = map("[map.ns-a]\nroom = \"r\"\nauthor = \"agent-a\"\n[map.ns-b]\ntag = \"b\"");
    let dir = tempfile::tempdir().unwrap();
    u.apply_namespace_migration(&m, &export_path(&dir), false)
        .unwrap();
    let after_first = snapshot(&u);

    let second_path = dir.path().join("second.ndjson");
    let report = u
        .apply_namespace_migration(&m, &second_path, false)
        .unwrap();

    assert_eq!(snapshot(&u), after_first, "a re-run wrote to the store");
    assert!(
        report.export_path.is_none(),
        "nothing to do needs no export"
    );
    assert!(!second_path.exists());
    for ns in ["ns-a", "ns-b"] {
        assert_eq!(entry(&report, ns).memories, 0);
        assert!(!entry(&report, ns).found);
    }
}

#[test]
fn an_unknown_namespace_in_the_map_is_reported_not_fatal() {
    let u = open_keyword();
    remember(&u, "ns-a", "alpha", None, None);
    let dir = tempfile::tempdir().unwrap();

    let report = u
        .apply_namespace_migration(
            &map("[map.ns-a]\ntag = \"a\"\n[map.no-such-ns]\nroom = \"r\""),
            &export_path(&dir),
            false,
        )
        .unwrap();

    let ghost = entry(&report, "no-such-ns");
    assert!(!ghost.found);
    assert_eq!(ghost.memories, 0);
    assert!(
        !ghost.room_created,
        "no room is created for a namespace with nothing to move"
    );
    assert!(u.get_room("r").unwrap().is_none());
    assert_eq!(u.count(Some("ns-a")).unwrap(), 0);
}

#[test]
fn default_is_never_touched() {
    let u = open_keyword();
    let id = remember(&u, "default", "default stays", None, None);
    remember(&u, "ns-a", "alpha", None, None);
    let before = u.get(&id).unwrap();
    let dir = tempfile::tempdir().unwrap();

    u.apply_namespace_migration(
        &map("[map.ns-a]\ntag = \"seed\"\nauthor = \"x\""),
        &export_path(&dir),
        false,
    )
    .unwrap();

    let after = u.get(&id).unwrap();
    assert_eq!(after.tags, before.tags);
    assert_eq!(after.author, before.author);
    assert_eq!(after.updated_at, before.updated_at);
}

#[test]
fn soft_deleted_rows_stay_where_they_are() {
    let u = open_keyword();
    remember(&u, "ns-a", "live alpha", None, None);
    let gone = remember(&u, "ns-a", "deprecated alpha", None, None);
    u.forget(&gone).unwrap();
    let dir = tempfile::tempdir().unwrap();

    let dry = u
        .plan_namespace_migration(&map("[map.ns-a]\ntag = \"a\""))
        .unwrap();
    assert_eq!(entry(&dry, "ns-a").memories, 1);
    assert_eq!(entry(&dry, "ns-a").soft_deleted, 1);

    u.apply_namespace_migration(&map("[map.ns-a]\ntag = \"a\""), &export_path(&dir), false)
        .unwrap();
    let row = u.store.get_by_id(&gone).unwrap().unwrap();
    assert_eq!(row.namespace, "ns-a");
    assert!(!row.tags.contains(&"a".to_string()));
}

/// #1668: the pre-migration export holds every memory, so it is created
/// owner-only, not with the umask default.
#[cfg(unix)]
#[test]
fn the_export_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let u = open_keyword();
    remember(&u, "ns-a", "alpha", None, None);
    let dir = tempfile::tempdir().unwrap();
    let path = export_path(&dir);

    u.apply_namespace_migration(&map("[map.ns-a]\ntag = \"a\""), &path, false)
        .unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "export mode {mode:o}");
}

#[test]
fn a_namespace_with_only_deprecated_memories_is_not_unmapped() {
    let u = open_keyword();
    remember(&u, "ns-live", "still active", None, None);
    let gone = remember(&u, "ns-gone", "soft deleted", None, None);
    u.soft_forget(&gone, "no longer true").unwrap();

    let report = u.plan_namespace_migration(&map("")).unwrap();
    let unmapped: Vec<(&str, usize)> = report
        .unmapped
        .iter()
        .map(|n| (n.namespace.as_str(), n.memories))
        .collect();
    assert_eq!(
        unmapped,
        vec![("ns-live", 1)],
        "a namespace with no active memory has nothing to migrate"
    );
}
