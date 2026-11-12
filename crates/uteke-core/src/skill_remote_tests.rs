//! Tests of the read-only remote entry point of the skill library (#1731).

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::skill_library::*;
use crate::skill_remote::*;
use crate::test_support::open_keyword;
use crate::{Error, ErrorClass, Uteke};

fn skill_md(name: &str, desc: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n")
}

fn put(root: &Path, rel: &str, content: &str) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, content).unwrap();
}

const DEPLOY_BODY: &str = "# Deploy\n\nquartz meridian orchid ledger harbour deploy rollout";
const OTHER_BODY: &str = "# Cooking\n\nzebra violin tractor glacier lantern recipe";

/// `deploy`, `deploy-copy` (same text) and `cooking`; `deploy` is approved.
fn library() -> (Uteke, tempfile::TempDir) {
    let t = tempfile::tempdir().unwrap();
    for (slug, desc, body) in [
        ("deploy", "Ship a release safely", DEPLOY_BODY),
        ("deploy-copy", "Ship a release safely", DEPLOY_BODY),
        ("cooking", "Cook dinner", OTHER_BODY),
    ] {
        put(
            t.path(),
            &format!("{slug}/SKILL.md"),
            &skill_md(slug, desc, body),
        );
    }
    put(
        t.path(),
        "deploy/references/rollback.md",
        "# Rollback\n\nlantern harbour rollback procedure emberglow",
    );
    let u = open_keyword();
    u.skill_index(t.path(), Some("alice"), None).unwrap();
    let hash = crate::skill_package::read_package(&t.path().join("deploy"))
        .unwrap()
        .hash;
    u.skill_approve("deploy", &hash, Some("alice")).unwrap();
    (u, t)
}

fn args(v: Value) -> SkillRemoteArgs {
    SkillRemoteArgs::from_json(&v).unwrap()
}

fn run(u: &Uteke, op: SkillRemoteOp, v: Value) -> Value {
    serde_json::to_value(u.skill_remote(op, &args(v)).unwrap()).unwrap()
}

fn err(u: &Uteke, op: SkillRemoteOp, v: Value) -> Error {
    match SkillRemoteArgs::from_json(&v) {
        Err(e) => e,
        Ok(a) => u.skill_remote(op, &a).unwrap_err(),
    }
}

fn msg(u: &Uteke, op: SkillRemoteOp, v: Value) -> String {
    match err(u, op, v) {
        Error::Validation(m) => m,
        e => panic!("not a validation error: {e}"),
    }
}

fn slugs(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["slug"].as_str().unwrap().to_string())
        .collect()
}

use SkillRemoteOp::{List, Search, Show, Similar};

#[test]
fn search_answers_with_the_effective_parameters() {
    let (u, _t) = library();
    let v = run(&u, Search, json!({"query": "emberglow", "mode": "fts"}));
    assert_eq!(v["mode"], "fts");
    assert_eq!(v["approved_only"], false);
    assert_eq!(
        (v["limit"].as_u64(), v["count"].as_u64()),
        (Some(5), Some(1))
    );
    assert_eq!(slugs(&v, "hits"), ["deploy"]);
    assert_eq!(v["hits"][0]["file"], "references/rollback.md");
    assert_eq!(v["hits"][0]["status"], "approved");
    // Defaults: hybrid, five hits.
    let v = run(&u, Search, json!({"query": "quartz meridian orchid"}));
    assert_eq!(v["mode"], "hybrid");
    assert_eq!(slugs(&v, "hits")[0..1], ["deploy"]);
    // approved_only drops the drafts.
    let v = run(
        &u,
        Search,
        json!({"query": "quartz meridian orchid", "mode": "fts", "approved_only": true}),
    );
    assert_eq!(slugs(&v, "hits"), ["deploy"]);
    assert_eq!(v["approved_only"], true);
    // null is the default.
    let v = run(
        &u,
        Search,
        json!({"query": "emberglow", "mode": null, "limit": null, "approved_only": null}),
    );
    assert_eq!(
        (v["mode"].as_str(), v["limit"].as_u64()),
        (Some("hybrid"), Some(5))
    );
}

#[test]
fn search_limits_are_clamped_and_the_query_is_bounded() {
    let (u, _t) = library();
    let limit = |l: u64| run(&u, Search, json!({"query": "deploy", "limit": l}))["limit"].as_u64();
    assert_eq!(limit(0), Some(1));
    assert_eq!(limit(1), Some(1));
    assert_eq!(limit(50), Some(50));
    assert_eq!(limit(51), Some(50));
    assert_eq!(limit(u64::MAX), Some(50));
    let at = "a".repeat(MAX_QUERY_CHARS);
    assert_eq!(
        run(&u, Search, json!({"query": at, "mode": "fts"}))["count"],
        0
    );
    let over = "a".repeat(MAX_QUERY_CHARS + 1);
    assert_eq!(
        msg(&u, Search, json!({"query": over})),
        "query is longer than 2000 characters"
    );
    // Characters, not bytes.
    let multi = "é".repeat(MAX_QUERY_CHARS);
    assert!(
        u.skill_remote(Search, &args(json!({"query": multi, "mode": "fts"})))
            .is_ok()
    );
    assert_eq!(
        msg(&u, Search, json!({"query": "  "})),
        "query must not be empty"
    );
    assert_eq!(msg(&u, Search, json!({})), "'query' is required");
    assert_eq!(
        msg(&u, Search, json!({"query": "x", "mode": "fuzzy"})),
        "mode must be hybrid, semantic or fts"
    );
}

#[test]
fn list_counts_filters_and_clamps() {
    let (u, _t) = library();
    let v = run(&u, List, json!({}));
    assert_eq!(slugs(&v, "skills"), ["cooking", "deploy", "deploy-copy"]);
    assert_eq!(
        (
            v["limit"].as_u64(),
            v["total"].as_u64(),
            v["count"].as_u64()
        ),
        (Some(50), Some(3), Some(3))
    );
    assert_eq!(v["skills"][1]["owner"], "alice");
    let v = run(&u, List, json!({"status": "approved"}));
    assert_eq!(slugs(&v, "skills"), ["deploy"]);
    assert_eq!(
        (v["total"].as_u64(), v["count"].as_u64()),
        (Some(1), Some(1))
    );
    let v = run(&u, List, json!({"status": "draft", "limit": 1}));
    assert_eq!(slugs(&v, "skills"), ["cooking"]);
    assert_eq!(
        (v["total"].as_u64(), v["count"].as_u64()),
        (Some(2), Some(1))
    );
    let limit = |l: u64| run(&u, List, json!({"limit": l}))["limit"].as_u64();
    assert_eq!(
        (limit(0), limit(200), limit(201)),
        (Some(1), Some(200), Some(200))
    );
    assert_eq!(
        msg(&u, List, json!({"status": "stale"})),
        "status must be 'draft' or 'approved' (the status of a current version)"
    );
}

#[test]
fn show_returns_the_detail_and_bounds_the_slug() {
    let (u, _t) = library();
    let v = run(&u, Show, json!({"slug": "deploy"}));
    assert_eq!(v["slug"], "deploy");
    assert_eq!(v["current"]["status"], "approved");
    assert_eq!(v["approved_hash"], v["current"]["hash"]);
    assert_eq!(v["current"]["file_count"], 2);
    assert_eq!(v["ledger"].as_array().unwrap().len(), 2);
    assert!(matches!(
        err(&u, Show, json!({"slug": "nope"})),
        Error::NotFound(_)
    ));
    assert_eq!(msg(&u, Show, json!({})), "'slug' is required");
    assert_eq!(
        msg(&u, Show, json!({"slug": " "})),
        "slug must not be empty"
    );
    // Exactly at the bound is looked up (and not found), one over is refused.
    assert!(matches!(
        err(&u, Show, json!({"slug": "a".repeat(64)})),
        Error::NotFound(_)
    ));
    assert_eq!(
        msg(&u, Show, json!({"slug": "a".repeat(65)})),
        "slug is longer than 64 characters"
    );
}

#[test]
fn similar_by_slug_and_by_draft_text() {
    let (u, _t) = library();
    let v = run(&u, Similar, json!({"slug": "deploy-copy"}));
    assert_eq!(v["source"], "deploy-copy");
    assert_eq!(v["threshold"].as_f64().map(|t| t as f32), Some(0.8));
    assert_eq!(v["source_embedded"], true);
    assert_eq!(slugs(&v, "hits"), ["deploy"]);
    assert_eq!(v["hits"][0]["relation"], "possible_duplicate");
    let v = run(
        &u,
        Similar,
        json!({"slug": "deploy-copy", "threshold": 0.0, "limit": 1}),
    );
    assert_eq!(
        (v["matches"].as_u64(), v["hits"].as_array().unwrap().len()),
        (Some(2), 1)
    );
    let v = run(&u, Similar, json!({"text": DEPLOY_BODY}));
    assert!(v["source"].is_null());
    assert_eq!(slugs(&v, "hits"), ["deploy", "deploy-copy"]);
    assert!(matches!(
        err(&u, Similar, json!({"slug": "nope"})),
        Error::NotFound(_)
    ));
    assert_eq!(
        msg(&u, Similar, json!({"slug": "deploy", "threshold": 1.5})),
        msg(&u, Similar, json!({"text": "x", "threshold": 1.5}))
    );
}

#[test]
fn similar_validates_its_subject_and_bounds_the_text() {
    let (u, _t) = library();
    assert_eq!(msg(&u, Similar, json!({})), "provide 'slug' or 'text'");
    assert_eq!(
        msg(&u, Similar, json!({"slug": "deploy", "text": "x"})),
        "provide either 'slug' or 'text', not both"
    );
    assert_eq!(
        msg(&u, Similar, json!({"text": "  "})),
        "text must not be empty"
    );
    // The remote cap is lower than the CLI's (the size of an indexed file).
    assert_eq!(MAX_REMOTE_TEXT_CHARS, 32_768);
    let over = "a ".repeat(MAX_REMOTE_TEXT_CHARS / 2 + 1);
    assert_eq!(
        msg(&u, Similar, json!({"text": over})),
        "text is longer than 32768 characters"
    );
    let at = "a".repeat(MAX_REMOTE_TEXT_CHARS);
    assert!(u.skill_remote(Similar, &args(json!({"text": at}))).is_ok());
    let limit = |l: u64| {
        run(
            &u,
            Similar,
            json!({"slug": "deploy-copy", "threshold": 0.0, "limit": l}),
        )["hits"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!((limit(0), limit(1), limit(100)), (1, 1, 2));
    assert_eq!(
        msg(&u, Similar, json!({"slug": "a".repeat(65)})),
        "slug is longer than 64 characters"
    );
}

#[test]
fn fields_of_another_operation_and_unknown_fields_are_refused() {
    let (u, _t) = library();
    let cases: [(SkillRemoteOp, Value, &str); 8] = [
        (
            Search,
            json!({"query": "x", "slug": "a"}),
            "'slug' is not a parameter of skill search",
        ),
        (
            Search,
            json!({"query": "x", "status": "draft"}),
            "'status' is not a parameter of skill search",
        ),
        (
            List,
            json!({"query": "x"}),
            "'query' is not a parameter of skill list",
        ),
        (
            List,
            json!({"approved_only": true}),
            "'approved_only' is not a parameter of skill list",
        ),
        (
            Show,
            json!({"slug": "a", "limit": 1}),
            "'limit' is not a parameter of skill show",
        ),
        (
            Show,
            json!({"slug": "a", "text": "x"}),
            "'text' is not a parameter of skill show",
        ),
        (
            Similar,
            json!({"slug": "a", "mode": "fts"}),
            "'mode' is not a parameter of skill similar",
        ),
        (
            Similar,
            json!({"slug": "a", "query": "x"}),
            "'query' is not a parameter of skill similar",
        ),
    ];
    for (op, v, want) in cases {
        assert_eq!(msg(&u, op, v), want);
    }
    for v in [
        json!({"query": "x", "namespace": "a"}),
        json!({"query": "x", "limit": -1}),
        json!({"query": "x", "limit": 1.5}),
        json!({"query": 5}),
        json!({"query": "x", "approved_only": "yes"}),
        json!({"query": "x", "threshold": "high"}),
    ] {
        let e = SkillRemoteArgs::from_json(&v).unwrap_err();
        let Error::Validation(m) = e else {
            panic!("{v}: not a validation error");
        };
        assert!(m.starts_with("invalid arguments: "), "{m}");
    }
    assert!(SkillRemoteArgs::from_json(&json!("x")).is_err());
}

#[test]
fn an_empty_library_answers_empty() {
    let u = open_keyword();
    let v = run(&u, List, json!({}));
    assert_eq!(
        (
            v["total"].as_u64(),
            v["count"].as_u64(),
            v["skills"].clone()
        ),
        (Some(0), Some(0), json!([]))
    );
    assert_eq!(
        run(&u, Search, json!({"query": "anything"}))["hits"],
        json!([])
    );
    assert_eq!(
        run(&u, Similar, json!({"text": "anything"}))["hits"],
        json!([])
    );
}

#[test]
fn nothing_is_written() {
    let (u, _t) = library();
    let count = |table: &str| -> i64 {
        u.store
            .conn
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))
            .unwrap()
    };
    let before = (
        count("skill_ledger"),
        count("skill_usage"),
        count("documents"),
        count("skill_versions"),
    );
    run(&u, Search, json!({"query": "deploy"}));
    run(&u, List, json!({}));
    run(&u, Show, json!({"slug": "deploy"}));
    run(&u, Similar, json!({"slug": "deploy"}));
    run(&u, Similar, json!({"text": DEPLOY_BODY}));
    let after = (
        count("skill_ledger"),
        count("skill_usage"),
        count("documents"),
        count("skill_versions"),
    );
    assert_eq!(before, after);
    // And the usage counters stay zero: remote reads are not "recalled".
    let d = u.skill_show("deploy").unwrap();
    assert_eq!((d.usage.recalled, d.usage.loaded), (0, 0));
}

#[test]
fn list_and_show_never_carry_the_server_path() {
    let (u, t) = library();
    let path = t.path().display().to_string();
    let v = run(&u, List, json!({}));
    let mut keys: Vec<_> = v["skills"][0]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "description",
            "hash",
            "name",
            "owner",
            "slug",
            "status",
            "updated_at",
            "valid"
        ]
    );
    assert!(!v.to_string().contains(&path), "{v}");
    let v = run(&u, Show, json!({"slug": "deploy"}));
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "approved_hash",
            "created_at",
            "current",
            "ledger",
            "owner",
            "slug",
            "updated_at",
            "usage",
            "versions"
        ]
    );
    assert!(!v.to_string().contains(&path), "{v}");
    // The CLI shape keeps it.
    assert_eq!(
        u.skill_show("deploy").unwrap().source_path,
        t.path().join("deploy").display().to_string()
    );
}

#[test]
fn embedding_requests_are_refused_when_every_slot_is_taken() {
    let (u, _t) = library();
    let a = u.remote_embed_permit().unwrap();
    let _b = u.remote_embed_permit().unwrap();
    let busy = "busy: 2 embedding requests are already running; retry in a moment";
    for (op, v) in [
        (Similar, json!({"text": DEPLOY_BODY})),
        (Search, json!({"query": "deploy"})),
        (Search, json!({"query": "deploy", "mode": "semantic"})),
    ] {
        let e = u.skill_remote(op, &args(v)).unwrap_err();
        assert_eq!(
            (e.to_string().as_str(), e.class()),
            (busy, ErrorClass::Busy)
        );
    }
    // What does not embed still answers.
    assert!(
        u.skill_remote(Search, &args(json!({"query": "deploy", "mode": "fts"})))
            .is_ok()
    );
    assert!(
        u.skill_remote(Similar, &args(json!({"slug": "deploy-copy"})))
            .is_ok()
    );
    assert!(u.skill_remote(List, &args(json!({}))).is_ok());
    // An oversized text is a 400 even when busy.
    let over = "a".repeat(MAX_REMOTE_TEXT_CHARS + 1);
    assert_eq!(
        u.skill_remote(Similar, &args(json!({"text": over})))
            .unwrap_err()
            .class(),
        ErrorClass::BadRequest
    );
    // A freed slot admits the next request; a finished one frees its own.
    drop(a);
    assert!(
        u.skill_remote(Similar, &args(json!({"text": DEPLOY_BODY})))
            .is_ok()
    );
    assert!(
        u.skill_remote(Similar, &args(json!({"text": DEPLOY_BODY})))
            .is_ok()
    );
}
