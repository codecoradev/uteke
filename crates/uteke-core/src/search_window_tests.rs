//! Killing tests for the document and skill search windows and rank formulas
//! that the v0.24.0 mutation gate left open (#1766). Each test names what it
//! pins; the fixtures run on the keyword test embedder, no ONNX model needed.
//!
//! The keyword embedder hashes words into 64 buckets, so a chunk's cosine to
//! the query `alpha` is `1 / sqrt(distinct words in the chunk)` when the chunk
//! holds `alpha` once and every other word once: more filler words, lower
//! rank. A chunk's text starts with its heading line (`# S1`), which counts as
//! a word of the chunk.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::embed::Embedder;
use crate::test_support::{KeywordEmbedder, open_keyword, open_with_embedder};
use crate::{DocKind, Error, Uteke};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-6
}

fn slugs(results: &[crate::memory::documents::DocumentSearchResult]) -> Vec<String> {
    results.iter().map(|r| r.document.slug.clone()).collect()
}

/// Mark the documents whose slug matches `like` as files of a skill, which
/// the plain document search leaves out and the skill search takes.
fn make_skill_files(u: &Uteke, like: &str) {
    u.store
        .conn
        .execute(
            "UPDATE documents SET content_type = 'skill' WHERE slug LIKE ?1",
            [like],
        )
        .unwrap();
}

fn chunk_count(u: &Uteke, slug: &str) -> usize {
    let id = u.store.get_document_by_slug(slug).unwrap().unwrap().id;
    u.store
        .get_chunk_ids_for_documents(std::slice::from_ref(&id))
        .unwrap()
        .len()
}

// --- hybrid document search -------------------------------------------------

/// Three documents for the query `alpha`:
/// semantic order `x-doc`, `a-doc`, `b-doc` (cosine 1, 0.71, 0.5);
/// FTS order `b-doc`, `a-doc`, `x-doc` (titles that hold `alpha` rank first).
fn three_documents_ranked_both_ways() -> Uteke {
    let u = open_keyword();
    u.doc_upsert("x-doc", "Doc X", "alpha", &[], None).unwrap();
    u.doc_upsert("a-doc", "alpha", "alpha f1", &[], None)
        .unwrap();
    u.doc_upsert("b-doc", "alpha alpha alpha", "alpha f1 f2 f3", &[], None)
        .unwrap();
    u
}

/// With `limit` 1 each list is asked for 2 documents (`limit * 2`). `a-doc` is
/// second in both lists, so it scores 1/62 + 1/62; `x-doc` is first in the
/// semantic list only and `b-doc` first in the FTS list only (1/61 each). A
/// third place in either list would lift `x-doc` or `b-doc` over `a-doc`
/// (1/61 + 1/63), so a window wider than 2 changes the winner, and a window of
/// 0 changes it too.
#[test]
fn hybrid_search_asks_each_list_for_twice_the_limit_and_fuses_their_ranks() {
    let u = three_documents_ranked_both_ways();
    assert_eq!(
        slugs(&u.doc_search("alpha", 3, "semantic").unwrap()),
        ["x-doc", "a-doc", "b-doc"]
    );
    assert_eq!(
        slugs(&u.doc_search("alpha", 3, "fts").unwrap()),
        ["b-doc", "a-doc", "x-doc"]
    );

    let top = u.doc_search("alpha", 1, "hybrid").unwrap();
    assert_eq!(slugs(&top), ["a-doc"]);
    assert!(close(top[0].score, 2.0 / 62.0), "{}", top[0].score);
    assert_eq!(top[0].mode, "hybrid");
}

/// With `limit` 3 every document is in both lists: a score is the sum of
/// 1 / (60 + rank) over the lists, rank counted from 1.
#[test]
fn hybrid_search_scores_a_document_with_the_sum_of_its_reciprocal_ranks() {
    let u = three_documents_ranked_both_ways();
    let all = u.doc_search("alpha", 3, "hybrid").unwrap();
    assert_eq!(all.len(), 3);
    let score = |slug: &str| {
        all.iter()
            .find(|r| r.document.slug == slug)
            .unwrap_or_else(|| panic!("{slug} missing from {all:?}"))
            .score
    };
    // x-doc: semantic 1st, FTS 3rd. a-doc: 2nd and 2nd. b-doc: 3rd and 1st.
    assert!(close(score("x-doc"), 1.0 / 61.0 + 1.0 / 63.0));
    assert!(close(score("a-doc"), 1.0 / 62.0 + 1.0 / 62.0));
    assert!(close(score("b-doc"), 1.0 / 63.0 + 1.0 / 61.0));
    // Best score first (x-doc and b-doc tie, a-doc is last).
    assert_eq!(all[2].document.slug, "a-doc");
    assert!(all[0].score >= all[1].score && all[1].score > all[2].score);
}

// --- semantic document search -----------------------------------------------

/// The search reads `limit * 10` chunks from the index, and a chunk of the
/// other kind does not count against `limit * 3`: with `limit` 2 a plain
/// document 15th by similarity, behind 14 files of skills, is still found
/// (a window of `limit + 10` = 12 would not reach it).
#[test]
fn semantic_search_reads_ten_chunks_per_result_past_the_other_kind() {
    let u = open_keyword();
    for i in 0..14 {
        u.doc_upsert(&format!("crowd-{i}"), "Crowd", "alpha", &[], None)
            .unwrap();
    }
    make_skill_files(&u, "crowd-%");
    u.doc_upsert("plain", "Plain", "alpha f1 f2", &[], None)
        .unwrap();

    let found = u.doc_search_semantic("alpha", 2, DocKind::Plain).unwrap();
    assert_eq!(slugs(&found), ["plain"]);
    assert!(close(found[0].score, 1.0 / 3.0_f32.sqrt()), "{found:?}");
    // The same window from the other side: 14 skill files, 2 asked for.
    let skills = u.doc_search_semantic("alpha", 2, DocKind::Skill).unwrap();
    assert_eq!(skills.len(), 2);
}

/// A document `a-doc` with six chunks that all beat the other two documents.
fn six_chunks_then_two_more_documents() -> Uteke {
    let u = open_keyword();
    let sections = (1..=6)
        .map(|i| format!("# S{i}\nalpha"))
        .collect::<Vec<_>>()
        .join("\n\n");
    u.doc_upsert("a-doc", "A", &sections, &[], None).unwrap();
    u.doc_upsert("b-doc", "B", "alpha f1 f2", &[], None)
        .unwrap();
    u.doc_upsert("c-doc", "C", "alpha f1 f2 f3 f4", &[], None)
        .unwrap();
    assert_eq!(chunk_count(&u, "a-doc"), 6, "the fixture needs six chunks");
    u
}

/// The window counts chunks, three per result: with `limit` 2 the first six
/// chunks are read, all of `a-doc`, so `b-doc` (the 7th) is not reached and the
/// search returns fewer than `limit` documents. A cap that is never reached
/// (`kept_chunks` that does not count) would return `b-doc` too.
#[test]
fn semantic_search_stops_after_three_chunks_per_result() {
    let u = six_chunks_then_two_more_documents();
    let found = u.doc_search_semantic("alpha", 2, DocKind::Plain).unwrap();
    assert_eq!(slugs(&found), ["a-doc"]);
}

/// With `limit` 3 the cap is nine chunks, which covers all eight: a cap of
/// `limit + 3` = 6 would stop after `a-doc`.
#[test]
fn semantic_search_reaches_the_documents_behind_a_long_one_within_the_window() {
    let u = six_chunks_then_two_more_documents();
    let found = u.doc_search_semantic("alpha", 3, DocKind::Plain).unwrap();
    assert_eq!(slugs(&found), ["a-doc", "b-doc", "c-doc"]);
    assert!(close(found[0].score, 1.0 / 2.0_f32.sqrt()));
    assert!(close(found[1].score, 1.0 / 3.0_f32.sqrt()));
    assert!(close(found[2].score, 1.0 / 5.0_f32.sqrt()));
}

/// A snippet longer than 200 bytes is cut at 200 and gets `...`; one of 200 or
/// less is returned whole.
#[test]
fn a_chunk_snippet_is_cut_only_above_two_hundred_bytes() {
    let u = open_keyword();
    for len in [199usize, 200, 201] {
        let content = format!("alpha {}", "x".repeat(len - 6));
        assert_eq!(content.len(), len);
        u.doc_upsert(&format!("snip-{len}"), "Snippet", &content, &[], None)
            .unwrap();
    }
    let found = u.doc_search_semantic("alpha", 10, DocKind::Plain).unwrap();
    let snippet = |len: usize| {
        found
            .iter()
            .find(|r| r.document.slug == format!("snip-{len}"))
            .unwrap()
            .chunk_snippet
            .clone()
    };
    let whole = |len: usize| format!("alpha {}", "x".repeat(len - 6));
    assert_eq!(snippet(199), whole(199));
    assert_eq!(snippet(200), whole(200));
    assert_eq!(snippet(201), format!("{}...", &whole(201)[..200]));
    assert_eq!(snippet(201).len(), 203);
}

/// Two chunks of one document with exactly the same vector (the same words in
/// another order): the document keeps the chunk the index returned first, and
/// a later chunk of equal score does not replace it. The expectation is read
/// from the index, so it does not depend on how ties are ordered.
#[test]
fn a_document_keeps_the_first_of_two_equally_close_chunks() {
    let u = open_keyword();
    u.doc_upsert(
        "tie",
        "Tie",
        "# Aa\nalpha bb\n\n# Bb\nalpha aa\n",
        &[],
        None,
    )
    .unwrap();
    assert_eq!(chunk_count(&u, "tie"), 2);

    let query = u.embed("alpha").unwrap();
    let hits = u.index.read().unwrap().search(&query, 10, 40);
    let ids: Vec<String> = hits
        .iter()
        .filter_map(|(key, _)| key.strip_prefix("chunk:").map(str::to_string))
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(close(hits[0].1, hits[1].1), "the chunks must tie: {hits:?}");
    let rows = u.store.get_chunks_by_ids_ordered(&ids).unwrap();
    let heading_of = |id: &str| {
        rows.iter()
            .find(|row| row.0 == id)
            .map(|row| row.2.clone())
            .unwrap()
    };
    let (first, second) = (heading_of(&ids[0]), heading_of(&ids[1]));
    assert_ne!(first, second);

    let found = u.doc_search_semantic("alpha", 5, DocKind::Plain).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].chunk_heading, first);
}

// --- documents of a skill ---------------------------------------------------

/// The children of a skill's files are not listed, whether the skill is named
/// by slug or by id; a plain document lists its children.
#[test]
fn a_skill_lists_no_children_by_slug_or_by_id() {
    let u = open_keyword();
    u.doc_upsert("parent", "Parent", "alpha", &[], None)
        .unwrap();
    u.doc_upsert_with_parent("child", "Child", "alpha", &[], None, Some("parent"))
        .unwrap();
    let id = u.store.get_document_by_slug("parent").unwrap().unwrap().id;
    let children = |key: &str| {
        u.doc_list_children(key, 10)
            .unwrap()
            .into_iter()
            .map(|d| d.slug)
            .collect::<Vec<_>>()
    };
    assert_eq!(children("parent"), ["child"]);
    assert_eq!(children(&id), ["child"]);

    make_skill_files(&u, "parent");
    assert!(children("parent").is_empty());
    assert!(children(&id).is_empty());
}

// --- skill search -----------------------------------------------------------

fn skill_md(name: &str, desc: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {desc}\n---\n{body}\n")
}

fn put(root: &Path, rel: &str, content: &str) {
    let path = root.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A skill `s1` with nine reference files that all beat one file of `s2` and
/// one of `s3`.
fn one_skill_with_many_matching_files() -> tempfile::TempDir {
    let t = tempfile::tempdir().unwrap();
    put(
        t.path(),
        "s1/SKILL.md",
        &skill_md("s1", "First skill", "# One\n\nunrelated body one"),
    );
    for i in 1..=9 {
        put(t.path(), &format!("s1/references/r{i}.md"), "alpha");
    }
    put(
        t.path(),
        "s2/SKILL.md",
        &skill_md("s2", "Second skill", "# Two\n\nunrelated body two"),
    );
    put(t.path(), "s2/references/r.md", "alpha f1");
    put(
        t.path(),
        "s3/SKILL.md",
        &skill_md("s3", "Third skill", "# Three\n\nunrelated body three"),
    );
    put(t.path(), "s3/references/r.md", "alpha f1 f2");
    t
}

/// Several files of one skill can match, so the search asks for `limit * 5`
/// files: with `limit` 3, 15 files. `s1` takes the first nine; `s2` and `s3`
/// are the 10th and 11th. A window of `limit + 5` = 8 files holds `s1` only.
#[test]
fn skill_search_asks_for_five_files_per_skill_wanted() {
    let t = one_skill_with_many_matching_files();
    let u = open_keyword();
    u.skill_index(t.path(), None, None).unwrap();

    let hits = u.skill_search("alpha", 3, "semantic", false).unwrap();
    let ranked: Vec<(&str, &str)> = hits
        .iter()
        .map(|h| (h.slug.as_str(), h.file.as_str()))
        .collect();
    assert_eq!(
        ranked,
        [
            ("s1", ranked[0].1),
            ("s2", "references/r.md"),
            ("s3", "references/r.md")
        ]
    );
    assert!(ranked[0].1.starts_with("references/r"), "{ranked:?}");
    assert!(close(hits[0].score, 1.0));
    assert!(close(hits[1].score, 1.0 / 2.0_f32.sqrt()));
    assert!(close(hits[2].score, 1.0 / 3.0_f32.sqrt()));
}

/// An embedder that works until `down` is set.
struct SwitchableEmbedder {
    down: Arc<AtomicBool>,
}

impl Embedder for SwitchableEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
        if self.down.load(Ordering::SeqCst) {
            return Err(Error::embed_msg("embedder is down"));
        }
        KeywordEmbedder.embed(text)
    }
    fn dims(&self) -> usize {
        KeywordEmbedder.dims()
    }
    fn max_seq_len(&self) -> usize {
        KeywordEmbedder.max_seq_len()
    }
    fn name(&self) -> &str {
        "switchable-test-embedder"
    }
}

/// `semantic` mode is a promise of vector search: an embedding failure is an
/// error, where `hybrid` answers from the FTS list alone.
#[test]
fn skill_search_in_semantic_mode_fails_when_the_query_cannot_be_embedded() {
    let t = one_skill_with_many_matching_files();
    let down = Arc::new(AtomicBool::new(false));
    let u = open_with_embedder(Box::new(SwitchableEmbedder { down: down.clone() }));
    u.skill_index(t.path(), None, None).unwrap();
    assert!(
        !u.skill_search("alpha", 3, "semantic", false)
            .unwrap()
            .is_empty()
    );

    down.store(true, Ordering::SeqCst);
    assert!(u.skill_search("alpha", 3, "semantic", false).is_err());
    let hybrid = u.skill_search("alpha", 3, "hybrid", false).unwrap();
    assert!(!hybrid.is_empty(), "hybrid falls back on the FTS list");
}
