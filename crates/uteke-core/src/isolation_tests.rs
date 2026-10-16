//! Scope-isolation regression gate (#1467).
//!
//! A store with two namespaces (`alpha`, `beta`) and two project tags whose
//! memories share the SAME vocabulary, so every retrieval surface has hits in
//! both scopes for the same query. Each surface is then called scoped to one
//! namespace and must return nothing from the other. The server, MCP and CLI
//! suites repeat the check over their transports; `docs/isolation-suite.md`
//! lists which surface each one covers.
//!
//! Documents are global by design (NULL namespace, #1271): a namespace-scoped
//! unified recall never returns them.

use crate::Uteke;
use crate::memory::types::{RecallStrategy, SearchType};

use crate::test_support::{ISOLATION_QUERY as QUERY, ISOLATION_SCOPES as SCOPES, seed_two_scopes};

struct Seeded {
    u: Uteke,
}

fn other(scope: &str) -> &'static str {
    if scope == "alpha" { "beta" } else { "alpha" }
}

fn seed() -> Seeded {
    Seeded {
        u: seed_two_scopes(),
    }
}

/// Every returned namespace must be `scope`; `what` names the surface.
fn only_scope<'a>(what: &str, scope: &str, namespaces: impl Iterator<Item = &'a str>) -> usize {
    let mut n = 0;
    for ns in namespaces {
        assert_eq!(ns, scope, "{what} scoped to {scope} returned a {ns} record");
        n += 1;
    }
    n
}

const STRATEGIES: [RecallStrategy; 5] = [
    RecallStrategy::Hybrid,
    RecallStrategy::Vector,
    RecallStrategy::Fts5,
    RecallStrategy::Graph,
    RecallStrategy::Fusion,
];

#[test]
fn recall_never_crosses_namespaces() {
    let s = seed();
    for scope in SCOPES {
        let hits =
            s.u.recall(QUERY, 20, None, Some(scope), 0.0, None, None)
                .unwrap();
        let n = only_scope(
            "recall",
            scope,
            hits.iter().map(|h| h.memory.namespace.as_str()),
        );
        assert!(
            n > 0,
            "recall scoped to {scope} found nothing: the check would be vacuous"
        );
        for strategy in STRATEGIES {
            let hits =
                s.u.recall_hybrid(QUERY, 20, None, Some(scope), strategy, 0.0)
                    .unwrap();
            let what = format!("recall_hybrid({strategy:?})");
            let n = only_scope(
                &what,
                scope,
                hits.iter().map(|h| h.memory.namespace.as_str()),
            );
            assert!(n > 0, "{what} scoped to {scope} found nothing");
        }
    }
}

#[test]
fn tag_filters_do_not_widen_the_scope() {
    let s = seed();
    for scope in SCOPES {
        // `shared` exists in both namespaces; the other scope's project tag
        // matches nothing in this one.
        let hits =
            s.u.recall(QUERY, 20, Some(&["shared"]), Some(scope), 0.0, None, None)
                .unwrap();
        let n = only_scope(
            "recall+tag",
            scope,
            hits.iter().map(|h| h.memory.namespace.as_str()),
        );
        assert!(n > 0);
        let foreign = format!("project:{}", other(scope));
        let hits =
            s.u.recall(
                QUERY,
                20,
                Some(&[foreign.as_str()]),
                Some(scope),
                0.0,
                None,
                None,
            )
            .unwrap();
        assert!(
            hits.is_empty(),
            "{foreign} tag leaked into {scope}: {}",
            hits.len()
        );
    }
}

#[test]
fn unified_recall_never_crosses_namespaces() {
    let s = seed();
    for scope in SCOPES {
        for strategy in STRATEGIES {
            // Mixed and memory-only recall: memories stay in scope and the
            // global documents are not returned (#1271).
            for search_type in [SearchType::All, SearchType::Memory] {
                let hits =
                    s.u.recall_unified(
                        QUERY,
                        20,
                        None,
                        Some(scope),
                        0.0,
                        search_type,
                        None,
                        None,
                        false,
                        strategy,
                    )
                    .unwrap();
                let what = format!("recall_unified({strategy:?},{search_type:?}) in {scope}");
                for h in &hits {
                    assert!(
                        h.doc_slug.is_none(),
                        "{what} returned the global document {:?}",
                        h.doc_slug
                    );
                    assert_eq!(h.namespace.as_deref(), Some(scope), "{what}: {h:?}");
                }
            }
        }
    }
}

/// `SearchType::Document` is the global document view by design (documents
/// have no namespace, #1271): it returns documents only, never a memory of
/// any namespace. Pinned so a change to that design is a deliberate one.
#[test]
fn document_only_recall_is_the_global_document_view_and_returns_no_memories() {
    let s = seed();
    for scope in SCOPES {
        let hits =
            s.u.recall_unified(
                QUERY,
                20,
                None,
                Some(scope),
                0.0,
                SearchType::Document,
                None,
                None,
                false,
                RecallStrategy::Hybrid,
            )
            .unwrap();
        assert!(
            !hits.is_empty(),
            "the document view found nothing: the check would be vacuous"
        );
        for h in &hits {
            assert!(
                h.memory_id.is_none(),
                "document-only recall returned a memory in {scope}"
            );
            assert_eq!(h.doc_slug.as_deref(), Some("shared-doc"));
            assert_eq!(h.namespace, None, "documents carry no namespace");
        }
    }
}

#[test]
fn keyword_search_list_tags_and_export_stay_in_scope() {
    let s = seed();
    for scope in SCOPES {
        let hits = s.u.search(QUERY, 20, None, Some(scope)).unwrap();
        let n = only_scope(
            "search",
            scope,
            hits.iter().map(|h| h.memory.namespace.as_str()),
        );
        assert!(n > 0, "search scoped to {scope} found nothing");

        let listed = s.u.list(None, 50, 0, Some(scope)).unwrap();
        let n = only_scope("list", scope, listed.iter().map(|m| m.namespace.as_str()));
        assert_eq!(n, 4, "list in {scope}");

        let by_tag = s.u.list(Some("shared"), 50, 0, Some(scope)).unwrap();
        only_scope(
            "list(tag)",
            scope,
            by_tag.iter().map(|m| m.namespace.as_str()),
        );

        let tags = s.u.tags_with_counts(Some(scope)).unwrap();
        let foreign = format!("project:{}", other(scope));
        assert!(
            tags.iter().all(|t| t.name != foreign),
            "tags in {scope} list the {foreign} tag"
        );
        let shared = tags.iter().find(|t| t.name == "shared").unwrap();
        assert_eq!(
            shared.count, 4,
            "tag count must not include the other namespace"
        );

        let exported = s.u.export(Some(scope)).unwrap();
        assert!(exported.contains(&format!("of {scope}")));
        assert!(
            !exported.contains(&format!("of {}", other(scope))),
            "export in {scope} contains {} records",
            other(scope)
        );
        assert_eq!(s.u.count(Some(scope)).unwrap(), 4);
    }
}

#[test]
fn room_surfaces_stay_in_scope() {
    let s = seed();
    for scope in SCOPES {
        let own = format!("room-{scope}");
        let foreign = format!("room-{}", other(scope));
        // The room's own memories come back.
        let mine = s.u.recall_room_scoped(&own, None, Some(scope), 20).unwrap();
        assert_eq!(
            only_scope(
                "room memories",
                scope,
                mine.iter().map(|m| m.namespace.as_str())
            ),
            1
        );
        // Another namespace's room asked for under this scope yields nothing.
        let theirs =
            s.u.recall_room_scoped(&foreign, None, Some(scope), 20)
                .unwrap();
        assert!(
            theirs.is_empty(),
            "{foreign} leaked into {scope}: {}",
            theirs.len()
        );
        // Room listing is namespace-scoped too.
        let rooms = s.u.list_rooms(Some(scope)).unwrap();
        assert!(
            rooms.iter().all(|r| r.id != foreign),
            "{foreign} listed in {scope}"
        );
    }
}

#[test]
fn graph_views_and_listings_of_namespaces_stay_in_scope() {
    let s = seed();
    for scope in SCOPES {
        let data = s.u.graph_data(Some(scope)).unwrap();
        let foreign_ids: Vec<String> =
            s.u.list(None, 50, 0, Some(other(scope)))
                .unwrap()
                .into_iter()
                .map(|m| m.id)
                .collect();
        for node in &data.nodes {
            assert!(
                !foreign_ids.contains(&node.id),
                "graph_data in {scope} contains {} node {}",
                other(scope),
                node.id
            );
        }
        let view = s.u.graph_view(Some(scope), None, 100).unwrap();
        for node in &view.nodes {
            assert!(
                !foreign_ids.contains(&node.id),
                "graph_view in {scope} contains {} node {}",
                other(scope),
                node.id
            );
        }
    }
}
