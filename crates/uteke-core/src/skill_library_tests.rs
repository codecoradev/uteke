//! Tests of the personal skill library (#1731) on the keyword test embedder.

use std::fs;
use std::path::Path;

use crate::skill_library::*;
use crate::skill_package::{self as pkg, Severity};
use crate::test_support::open_keyword;
use crate::{Error, Uteke};

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

/// A skills directory: `deploy` (with a reference file) and `cooking`.
fn library_dir() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    put(
        t.path(),
        "deploy/SKILL.md",
        &skill_md("deploy", "Ship a release safely", DEPLOY_BODY),
    );
    put(
        t.path(),
        "deploy/references/rollback.md",
        "# Rollback\n\nlantern harbour rollback procedure emberglow",
    );
    put(t.path(), "deploy/scripts/run.sh", "echo deploy");
    put(
        t.path(),
        "cooking/SKILL.md",
        &skill_md("cooking", "Cook dinner", OTHER_BODY),
    );
    t
}

fn index(u: &Uteke, root: &Path) -> SkillIndexReport {
    u.skill_index(root, None, None).unwrap()
}

fn hash_of(dir: &Path) -> String {
    pkg::read_package(dir).unwrap().hash
}

fn ledger_actions(u: &Uteke, slug: &str) -> Vec<(String, Option<String>, Option<String>)> {
    let mut v: Vec<_> = u
        .skill_show(slug)
        .unwrap()
        .ledger
        .into_iter()
        .map(|l| (l.action, l.before_hash, l.after_hash))
        .collect();
    v.reverse();
    v
}

#[test]
fn index_records_skills_with_the_package_hash_and_changes_no_file() {
    let t = library_dir();
    let before: Vec<_> = ["deploy", "cooking"]
        .map(|s| hash_of(&t.path().join(s)))
        .into();
    let u = open_keyword();
    let r = index(&u, t.path());
    assert_eq!(
        (r.created, r.updated, r.reverted, r.unchanged),
        (2, 0, 0, 0)
    );
    assert!(r.rejected.is_empty() && !r.truncated);
    let slugs: Vec<_> = r.skills.iter().map(|s| s.slug.as_str()).collect();
    assert_eq!(slugs, ["cooking", "deploy"]);
    let deploy = r.skills.iter().find(|s| s.slug == "deploy").unwrap();
    assert_eq!(deploy.hash, before[0]);
    assert_eq!(deploy.status, "draft");
    assert_eq!((deploy.files, deploy.indexed_files), (3, 2));
    assert!(deploy.valid && deploy.issues.is_empty());
    // Read-only: the same bytes afterwards.
    assert_eq!(hash_of(&t.path().join("deploy")), before[0]);
    assert_eq!(hash_of(&t.path().join("cooking")), before[1]);

    let d = u.skill_show("deploy").unwrap();
    assert_eq!(d.current.hash, before[0]);
    assert_eq!(d.current.status, "draft");
    assert_eq!(d.approved_hash, None);
    assert_eq!(
        (d.current.file_count, d.current.pinned, d.current.locked),
        (3, false, false)
    );
    assert_eq!(
        d.current
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.indexed))
            .collect::<Vec<_>>(),
        [
            ("SKILL.md", true),
            ("references/rollback.md", true),
            ("scripts/run.sh", false)
        ]
    );
    assert_eq!(d.source_path, t.path().join("deploy").display().to_string());
    assert_eq!(
        ledger_actions(&u, "deploy"),
        [("created".into(), None, Some(before[0].clone()))]
    );
}

#[test]
fn skill_files_are_hidden_from_ordinary_documents() {
    let t = library_dir();
    let u = open_keyword();
    u.doc_upsert(
        "notes",
        "Notes",
        "quartz meridian orchid ledger harbour deploy",
        &[],
        None,
    )
    .unwrap();
    index(&u, t.path());

    let slugs =
        |v: Vec<crate::DocumentSummary>| -> Vec<String> { v.into_iter().map(|d| d.slug).collect() };
    assert_eq!(slugs(u.doc_list(None, 100).unwrap()), ["notes"]);
    for mode in ["fts", "semantic", "hybrid"] {
        let hits = u.doc_search("quartz meridian orchid", 10, mode).unwrap();
        assert_eq!(
            slugs(hits.into_iter().map(|h| h.document).collect()),
            ["notes"],
            "{mode}"
        );
    }
    // The export still sees every document.
    assert_eq!(u.doc_list_page(None, 100, 0).unwrap().len(), 4);
    // Ordinary similarity and the duplicate report do not see skills either.
    let r = u.doc_similar("notes", None, Some(0.0), 10).unwrap();
    assert!(r.hits.is_empty(), "{:?}", r.hits);
    let rep = u.doc_duplicate_report(None, None, Some(0.0), 10).unwrap();
    assert_eq!((rep.scanned, rep.total_pairs), (1, 0));
    // Not by slug or id either: skill files are read through the skill commands only.
    assert!(u.doc_get("skill/deploy/SKILL.md").unwrap().is_none());
    let id = u
        .store
        .get_document_by_slug("skill/deploy/SKILL.md")
        .unwrap()
        .unwrap()
        .id;
    assert!(u.doc_get(&id).unwrap().is_none());
    assert!(
        u.doc_list_children("skill/deploy/SKILL.md", 10)
            .unwrap()
            .is_empty()
    );
    assert!(u.doc_list_children(&id, 10).unwrap().is_empty());
    assert!(
        u.doc_update(&id, None, Some("x"), None, None)
            .unwrap()
            .is_none()
    );
    // The skill commands still read them.
    assert_eq!(u.skill_show("deploy").unwrap().current.file_count, 3);
}

#[test]
fn skill_search_sees_skills_and_not_plain_documents() {
    let t = library_dir();
    let u = open_keyword();
    u.doc_upsert(
        "notes",
        "Notes",
        "quartz meridian orchid ledger harbour deploy",
        &[],
        None,
    )
    .unwrap();
    index(&u, t.path());

    for mode in ["hybrid", "semantic", "fts"] {
        let hits = u
            .skill_search("quartz meridian orchid", 5, mode, false)
            .unwrap();
        assert_eq!(
            hits.first().map(|h| h.slug.as_str()),
            Some("deploy"),
            "{mode}"
        );
        assert!(hits.iter().all(|h| h.slug != "notes"), "{mode}");
        assert_eq!(
            hits[0].description.as_deref(),
            Some("Ship a release safely")
        );
        assert_eq!(hits[0].status, "draft");
    }
    // A reference file finds its skill and names the file.
    let hits = u.skill_search("emberglow", 5, "fts", false).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        (hits[0].slug.as_str(), hits[0].file.as_str()),
        ("deploy", "references/rollback.md")
    );
    // One hit per skill even when several files match, and the limit applies.
    let hits = u.skill_search("lantern harbour", 5, "fts", false).unwrap();
    assert_eq!(hits.iter().filter(|h| h.slug == "deploy").count(), 1);
    assert_eq!(
        u.skill_search("lantern harbour", 1, "fts", false)
            .unwrap()
            .len(),
        1
    );
    // approved_only
    assert!(
        u.skill_search("emberglow", 5, "fts", true)
            .unwrap()
            .is_empty()
    );
    let h = hash_of(&t.path().join("deploy"));
    u.skill_approve("deploy", &h, None).unwrap();
    let hits = u.skill_search("emberglow", 5, "fts", true).unwrap();
    assert_eq!(
        (hits[0].slug.as_str(), hits[0].status.as_str()),
        ("deploy", "approved")
    );
}

#[test]
fn skill_search_validates_its_input() {
    let u = open_keyword();
    let bad = |q: &str, mode: &str| u.skill_search(q, 5, mode, false).unwrap_err().to_string();
    assert!(bad("   ", "hybrid").contains("must not be empty"));
    assert!(bad(&"a".repeat(2001), "hybrid").contains("longer than 2000"));
    assert!(bad("ok", "fuzzy").contains("mode must be"));
    // Exactly at the bound is accepted; punctuation-only keyword queries are
    // not a syntax error.
    assert!(
        u.skill_search(&"a".repeat(2000), 5, "fts", false)
            .unwrap()
            .is_empty()
    );
    assert!(
        u.skill_search("\"(*) - OR NEAR", 5, "fts", false)
            .unwrap()
            .is_empty()
    );
    assert!(
        u.skill_search("!!!", 5, "hybrid", false)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unchanged_reindex_repairs_missing_documents_and_changes_nothing_else() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    let r = index(&u, t.path());
    assert_eq!((r.created, r.updated, r.unchanged), (0, 0, 2));
    assert!(r.near_duplicates.is_empty());
    assert_eq!(
        u.skill_show("deploy").unwrap().ledger.len(),
        1,
        "no ledger row for a no-op"
    );

    u.doc_delete("skill/deploy/references/rollback.md").unwrap();
    assert!(
        u.skill_search("emberglow", 5, "fts", false)
            .unwrap()
            .is_empty()
    );
    let r = index(&u, t.path());
    assert_eq!(r.unchanged, 2);
    assert_eq!(
        u.skill_search("emberglow", 5, "fts", false).unwrap().len(),
        1
    );
}

#[test]
fn changed_bytes_make_a_new_draft_and_stale_the_approval() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    let h1 = hash_of(&t.path().join("deploy"));
    u.skill_approve("deploy", &h1, Some("alice")).unwrap();

    put(
        t.path(),
        "deploy/references/rollback.md",
        "# Rollback\n\nnew text sapphire",
    );
    let h2 = hash_of(&t.path().join("deploy"));
    assert_ne!(h1, h2);
    let r = index(&u, t.path());
    assert_eq!((r.created, r.updated, r.unchanged), (0, 1, 1));
    let d = u.skill_show("deploy").unwrap();
    assert_eq!(
        (d.current.hash.as_str(), d.current.status.as_str()),
        (h2.as_str(), "draft")
    );
    assert_eq!(d.approved_hash, None);
    assert_eq!(
        d.versions
            .iter()
            .map(|v| (v.hash.clone(), v.status.clone(), v.current))
            .collect::<Vec<_>>(),
        [
            (h2.clone(), "draft".into(), true),
            (h1.clone(), "stale".into(), false)
        ]
    );
    // The old version's file text is gone from search, the new one is in.
    assert!(
        u.skill_search("emberglow", 5, "fts", false)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        u.skill_search("sapphire", 5, "fts", false).unwrap().len(),
        1
    );
    // The replaced hash cannot be approved.
    let e = u.skill_approve("deploy", &h1, None).unwrap_err();
    assert!(matches!(e, Error::Validation(_)), "{e}");
    assert!(e.to_string().contains("was replaced"));
    assert_eq!(
        ledger_actions(&u, "deploy"),
        [
            ("created".into(), None, Some(h1.clone())),
            ("approved".into(), None, Some(h1.clone())),
            ("updated".into(), Some(h1.clone()), Some(h2.clone())),
        ]
    );

    // A replaced draft is archived, not stale.
    put(
        t.path(),
        "deploy/references/rollback.md",
        "# Rollback\n\nthird text",
    );
    let h3 = hash_of(&t.path().join("deploy"));
    index(&u, t.path());
    let d = u.skill_show("deploy").unwrap();
    let status = |h: &str| {
        d.versions
            .iter()
            .find(|v| v.hash == h)
            .unwrap()
            .status
            .clone()
    };
    assert_eq!(
        (
            status(&h1).as_str(),
            status(&h2).as_str(),
            status(&h3).as_str()
        ),
        ("stale", "archived", "draft")
    );

    // Putting the approved bytes back makes that approval current again.
    put(
        t.path(),
        "deploy/references/rollback.md",
        "# Rollback\n\nlantern harbour rollback procedure emberglow",
    );
    assert_eq!(hash_of(&t.path().join("deploy")), h1);
    let r = index(&u, t.path());
    assert_eq!(r.reverted, 1);
    let d = u.skill_show("deploy").unwrap();
    assert_eq!(
        (d.current.hash.as_str(), d.current.status.as_str()),
        (h1.as_str(), "approved")
    );
    assert_eq!(d.approved_hash.as_deref(), Some(h1.as_str()));
    assert_eq!(
        u.skill_search("emberglow", 5, "fts", false).unwrap().len(),
        1
    );
    assert_eq!(u.skill_show("deploy").unwrap().ledger[0].action, "reverted");
}

#[test]
fn approve_checks_the_hash_and_is_idempotent() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    let h = hash_of(&t.path().join("cooking"));
    // The stored hash is tole's string form, `sha256:<64 hex>`.
    let hex = h.strip_prefix("sha256:").expect("sha256: prefix");
    assert_eq!(hex.len(), 64);
    assert!(hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));

    let err = |slug: &str, hash: &str| u.skill_approve(slug, hash, None).unwrap_err();
    assert!(matches!(err("nope", &h), Error::NotFound(_)));
    assert!(matches!(
        err("cooking", &"0".repeat(64)),
        Error::NotFound(_)
    ));
    assert!(matches!(
        err("cooking", &format!("sha256:{}", "0".repeat(64))),
        Error::NotFound(_)
    ));
    // A prefix needs at least 12 hex digits after the optional `sha256:`
    // (the length check is `parse_skill_ref`'s and `skill_approve`'s).
    assert!(parse_skill_ref(&format!("cooking@{}", &hex[..11])).is_err());
    assert!(parse_skill_ref(&format!("cooking@sha256:{}", &hex[..11])).is_err());
    assert!(parse_skill_ref(&format!("cooking@{}", &hex[..12])).is_ok());
    assert!(parse_skill_ref(&format!("cooking@sha256:{}", &hex[..12])).is_ok());
    assert!(err("cooking", &hex[..11]).to_string().contains("12 to 64"));
    assert!(
        parse_skill_ref(&format!("cooking@{hex}0")).is_err(),
        "65 characters"
    );
    assert!(parse_skill_ref(&format!("cooking@sha256:{hex}0")).is_err());
    assert!(parse_skill_ref(&format!("cooking@{}", "g".repeat(12))).is_err());
    assert!(parse_skill_ref(&format!("cooking@sha512:{hex}")).is_err());
    assert!(parse_skill_ref("cooking").is_err());
    assert!(parse_skill_ref(&format!("Cooking@{h}")).is_err());
    // Every accepted spelling names the same hex digits.
    for spelling in [
        hex.to_string(),
        h.clone(),
        hex.to_uppercase(),
        h.to_uppercase(),
    ] {
        assert_eq!(
            parse_skill_ref(&format!("cooking@{spelling}")).unwrap(),
            ("cooking", hex.to_string())
        );
    }

    // Prefix of the digits after `sha256:` (not of the string with the prefix).
    let a = u
        .skill_approve("cooking", &hex[..12], Some("alice"))
        .unwrap();
    assert_eq!(
        (a.hash.as_str(), a.status.as_str(), a.changed),
        (h.as_str(), "approved", true)
    );
    let d = u.skill_show("cooking").unwrap();
    assert_eq!(
        (d.current.approved_by.as_deref(), d.approved_hash.as_deref()),
        (Some("alice"), Some(h.as_str()))
    );
    assert!(d.current.approved_at.is_some());
    let again = u.skill_approve("cooking", &h, Some("bob")).unwrap();
    assert!(!again.changed);
    let again = u
        .skill_approve("cooking", &format!("sha256:{}", &hex[..20]), Some("bob"))
        .unwrap();
    assert!(!again.changed);
    let d = u.skill_show("cooking").unwrap();
    assert_eq!(
        d.ledger.len(),
        2,
        "created + approved, nothing for the repeat"
    );
    assert_eq!(d.current.approved_by.as_deref(), Some("alice"));
    // The actor is bounded and clean.
    assert!(
        u.skill_approve("cooking", &h, Some(&"a".repeat(129)))
            .is_err()
    );
    assert!(u.skill_approve("cooking", &h, Some("a\nb")).is_err());
}

#[test]
fn an_invalid_skill_is_indexed_but_cannot_be_approved() {
    let t = tempfile::tempdir().unwrap();
    put(
        t.path(),
        "broken/SKILL.md",
        "---\nname: other\n---\nSome body text here\n",
    );
    let u = open_keyword();
    let r = index(&u, t.path());
    let e = &r.skills[0];
    assert!(!e.valid);
    assert_eq!(
        e.issues
            .iter()
            .map(|i| (i.code.as_str(), i.severity))
            .collect::<Vec<_>>(),
        [
            ("name-mismatch", Severity::Error),
            ("missing-description", Severity::Error)
        ]
    );
    let h = hash_of(&t.path().join("broken"));
    let err = u.skill_approve("broken", &h, None).unwrap_err();
    assert!(err.to_string().contains("is not valid"), "{err}");
    assert_eq!(u.skill_show("broken").unwrap().current.status, "draft");
    let lint = u.skill_lint_library(10).unwrap();
    assert!(!lint.skills[0].valid);
    assert_eq!(lint.skills[0].issues.len(), 2);
}

#[test]
fn ledger_is_append_only() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    let c = &u.store.conn;
    let upd = c
        .execute("UPDATE skill_ledger SET actor = 'x'", [])
        .unwrap_err();
    assert!(upd.to_string().contains("append-only"), "{upd}");
    let del = c.execute("DELETE FROM skill_ledger", []).unwrap_err();
    assert!(del.to_string().contains("append-only"), "{del}");
    let n: i64 = c
        .query_row("SELECT COUNT(*) FROM skill_ledger", [], |r| r.get(0))
        .unwrap();
    assert_eq!(n, 2);
}

#[test]
fn similar_finds_duplicates_by_the_document_vector() {
    let t = library_dir();
    // `deploy-copy` has the text of `deploy`, `cooking` is unrelated.
    put(
        t.path(),
        "deploy-copy/SKILL.md",
        &skill_md("deploy-copy", "Ship a release safely", DEPLOY_BODY),
    );
    let u = open_keyword();
    let r = index(&u, t.path());
    // The index run reports the pair, once from each side.
    assert!(
        r.near_duplicates
            .iter()
            .any(|n| n.slug == "deploy-copy" && n.other == "deploy")
    );
    assert!(
        r.near_duplicates
            .iter()
            .all(|n| n.other != "cooking" && n.slug != "cooking")
    );

    let s = u.skill_similar("deploy-copy", None, 10).unwrap();
    assert!(s.source_embedded);
    assert_eq!(s.source.as_deref(), Some("deploy-copy"));
    assert_eq!(
        (s.threshold, s.similar_threshold, s.duplicate_threshold),
        (0.8, 0.8, 0.92)
    );
    assert_eq!(
        s.hits.iter().map(|h| h.slug.as_str()).collect::<Vec<_>>(),
        ["deploy"]
    );
    assert_eq!(s.hits[0].relation, "possible_duplicate");
    assert!(s.hits[0].score > 0.92);
    assert_eq!(s.matches, 1);
    // A lower threshold brings the unrelated one in as `related`; the skill
    // itself never appears.
    let s = u.skill_similar("deploy-copy", Some(0.0), 10).unwrap();
    assert!(s.hits.iter().all(|h| h.slug != "deploy-copy"));
    assert_eq!(s.hits.len(), 2);
    assert_eq!(s.hits.last().unwrap().slug, "cooking");
    assert_eq!(s.hits.last().unwrap().relation, "related");
    assert_eq!(
        u.skill_similar("deploy-copy", Some(0.0), 1)
            .unwrap()
            .hits
            .len(),
        1
    );
    assert_eq!(
        u.skill_similar("deploy-copy", Some(0.0), 1)
            .unwrap()
            .matches,
        2
    );
    // Errors.
    assert!(matches!(
        u.skill_similar("nope", None, 10).unwrap_err(),
        Error::NotFound(_)
    ));
    assert!(u.skill_similar("deploy", Some(1.5), 10).is_err());
    assert!(u.skill_similar("deploy", Some(f32::NAN), 10).is_err());

    // A draft that is not stored.
    let s = u.skill_similar_to_text(DEPLOY_BODY, None, 10).unwrap();
    assert_eq!(s.source, None);
    assert_eq!(s.hits.len(), 2);
    // The bare body scores below the duplicate threshold against SKILL.md
    // (which also holds the frontmatter), and above the similar one.
    assert!(s.hits.iter().all(|h| h.score >= 0.8
        && h.relation
            == if h.score >= 0.92 {
                "possible_duplicate"
            } else {
                "similar_to"
            }));
    assert!(u.skill_similar_to_text("  ", None, 10).is_err());
}

#[test]
fn index_rejects_what_it_cannot_read_and_keeps_going() {
    let t = library_dir();
    put(
        t.path(),
        "Bad_Name/SKILL.md",
        &skill_md("x", "d", "body text"),
    );
    // Two directories with the same slug.
    put(
        t.path(),
        "group/cooking/SKILL.md",
        &skill_md("cooking", "again", "other body"),
    );
    let u = open_keyword();
    let r = index(&u, t.path());
    assert_eq!(r.created, 2);
    let reasons: Vec<_> = r.rejected.iter().map(|x| x.reason.as_str()).collect();
    assert!(
        reasons.iter().any(|x| x.contains("not a valid skill slug")),
        "{reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|x| x.contains("slug 'cooking' is already used")),
        "{reasons:?}"
    );
    assert!(
        u.skill_index(&t.path().join("missing"), None, None)
            .is_err()
    );
}

#[test]
fn an_ordinary_document_with_a_skill_slug_blocks_the_index() {
    let t = library_dir();
    let u = open_keyword();
    u.doc_upsert("skill/deploy/SKILL.md", "Mine", "my own note", &[], None)
        .unwrap();
    let err = u.skill_index(t.path(), None, None).unwrap_err();
    assert!(
        err.to_string().contains("a document already has the slug"),
        "{err}"
    );
    // The note is untouched, and no skill row was written for it.
    let d = u.doc_get("skill/deploy/SKILL.md").unwrap().unwrap();
    assert_eq!(
        (d.content_type.as_str(), d.content.as_str()),
        ("markdown", "my own note")
    );
    assert!(u.skill_show("deploy").is_err());
}

#[test]
fn the_library_is_bounded() {
    let t = library_dir();
    let u = open_keyword();
    {
        let tx = u.store.conn.unchecked_transaction().unwrap();
        for i in 0..MAX_SKILLS {
            tx.execute(
                "INSERT INTO skills (id, slug, source_path, created_at, updated_at) \
                 VALUES (?1, ?2, '/x', 't', 't')",
                rusqlite::params![format!("id{i}"), format!("filler-{i}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
    }
    let err = u.skill_index(t.path(), None, None).unwrap_err();
    assert!(err.to_string().contains("2000 skills"), "{err}");
    // Re-indexing an existing skill does not need room: not applicable here, the
    // two are new. List and lint are bounded.
    assert_eq!(
        u.skill_list(None, 1_000_000).unwrap().len(),
        0,
        "fillers have no version"
    );
    assert!(u.skill_list(Some("stale"), 10).is_err());
}

#[test]
fn usage_counters_are_exact() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    assert_eq!(u.skill_show("deploy").unwrap().usage, SkillUsage::default());
    u.skill_record_usage("deploy", UsageEvent::Recalled)
        .unwrap();
    u.skill_record_usage("deploy", UsageEvent::Recalled)
        .unwrap();
    u.skill_record_usage("deploy", UsageEvent::Loaded).unwrap();
    let usage = u.skill_record_usage("deploy", UsageEvent::Failed).unwrap();
    assert_eq!(
        (usage.recalled, usage.loaded, usage.succeeded, usage.failed),
        (2, 1, 0, 1)
    );
    assert!(usage.last_used_at.is_some());
    assert!(u.skill_record_usage("nope", UsageEvent::Loaded).is_err());
    assert_eq!(UsageEvent::parse("succeeded"), Some(UsageEvent::Succeeded));
    assert_eq!(UsageEvent::parse("x"), None);
    // New bytes start from zero: the counters belong to a version.
    put(t.path(), "deploy/references/rollback.md", "changed");
    index(&u, t.path());
    assert_eq!(u.skill_show("deploy").unwrap().usage, SkillUsage::default());
}

#[test]
fn list_filters_by_status_and_orders_by_slug() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    let h = hash_of(&t.path().join("cooking"));
    u.skill_approve("cooking", &h, None).unwrap();
    let all = u.skill_list(None, 50).unwrap();
    assert_eq!(
        all.iter()
            .map(|s| (s.slug.as_str(), s.status.as_str()))
            .collect::<Vec<_>>(),
        [("cooking", "approved"), ("deploy", "draft")]
    );
    assert_eq!(u.skill_list(Some("draft"), 50).unwrap().len(), 1);
    assert_eq!(
        u.skill_list(Some("approved"), 50).unwrap()[0].slug,
        "cooking"
    );
    assert_eq!(u.skill_list(None, 1).unwrap().len(), 1);
    assert_eq!(
        u.skill_list(None, 0).unwrap().len(),
        1,
        "limit 0 is clamped to 1"
    );
}

#[test]
fn lint_directory_reports_without_a_store() {
    let t = library_dir();
    put(
        t.path(),
        "nodesc/SKILL.md",
        "---\nname: nodesc\n---\nbody here\n",
    );
    fs::create_dir_all(t.path().join("empty-dir")).unwrap();
    let r = lint_directory(t.path()).unwrap();
    let v: Vec<_> = r
        .skills
        .iter()
        .map(|s| (s.slug.as_str(), s.valid))
        .collect();
    assert_eq!(v, [("cooking", true), ("deploy", true), ("nodesc", false)]);
    assert!(r.rejected.is_empty());
    assert!(lint_directory(&t.path().join("nope")).is_err());
}

#[test]
fn fresh_store_has_the_skill_tables_at_the_current_schema() {
    let u = open_keyword();
    assert_eq!(u.store.schema_version().unwrap(), 27);
    for table in ["skills", "skill_versions", "skill_usage", "skill_ledger"] {
        let n: i64 = u
            .store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1, "{table}");
    }
    // One current version per skill is enforced by the schema.
    let t = library_dir();
    index(&u, t.path());
    let id: String = u
        .store
        .conn
        .query_row("SELECT id FROM skills WHERE slug = 'deploy'", [], |r| {
            r.get(0)
        })
        .unwrap();
    let dup = u.store.conn.execute(
        "INSERT INTO skill_versions (id, skill_id, content_hash, status, is_current, created_at, updated_at) \
         VALUES ('v2', ?1, 'other', 'draft', 1, 't', 't')",
        [id],
    );
    assert!(dup.is_err());
}

#[test]
fn index_records_an_absolute_source_path_for_a_relative_root() {
    let t = library_dir();
    // The temp dir spelled relative to the working directory.
    let cwd = std::env::current_dir().unwrap();
    let up = "../".repeat(cwd.components().count().saturating_sub(1));
    let rel = format!("{up}{}", t.path().to_string_lossy().trim_start_matches('/'));
    assert!(Path::new(&rel).is_relative());
    let u = open_keyword();
    let r = u.skill_index(Path::new(&rel), None, None).unwrap();
    assert_eq!(r.created, 2, "{r:?}");
    let shown = u.skill_show("deploy").unwrap();
    assert!(
        Path::new(&shown.source_path).is_absolute(),
        "{}",
        shown.source_path
    );
    assert!(
        Path::new(&shown.source_path).is_dir(),
        "{}",
        shown.source_path
    );
}

/// A store written by an earlier develop build holds bare-hex hashes of the old
/// (big-endian) encoding. They are not tole hashes and cannot be converted, so
/// there is no migration: the next index sees a different hash and makes a new
/// draft version, the old approval goes stale, and the old row cannot be
/// approved.
#[test]
fn a_legacy_bare_hash_is_superseded_by_the_next_index() {
    let t = library_dir();
    let u = open_keyword();
    index(&u, t.path());
    let h = hash_of(&t.path().join("cooking"));
    u.skill_approve("cooking", &h, Some("alice")).unwrap();
    let legacy = "ab".repeat(32);
    u.store
        .conn
        .execute(
            "UPDATE skill_versions SET content_hash = ?1 WHERE content_hash = ?2",
            [&legacy, &h],
        )
        .unwrap();
    assert!(matches!(
        u.skill_approve("cooking", &legacy[..12], None).unwrap_err(),
        Error::NotFound(_)
    ));
    let r = index(&u, t.path());
    let cooking = r.skills.iter().find(|s| s.slug == "cooking").unwrap();
    assert_eq!(cooking.action, IndexAction::Updated, "{r:?}");
    assert_eq!(cooking.hash, h);
    let d = u.skill_show("cooking").unwrap();
    assert_eq!(d.current.hash, h);
    assert_eq!(d.current.status, "draft");
    assert!(d.approved_hash.is_none());
    assert!(
        d.versions
            .iter()
            .any(|v| v.hash == legacy && v.status == "stale")
    );
}
