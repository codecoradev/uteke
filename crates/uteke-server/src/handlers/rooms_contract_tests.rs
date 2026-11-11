//! The `rooms` field on memory hits is CORE (#1713, contract amendment 4
//! point 1, uteke-cloud #525). Its shape is pinned here on the wire, for every
//! CORE surface that carries it, with exact values:
//!
//! 1. `rooms: [{room_id, room_name}]`, a list on the hit object;
//! 2. sorted by `room_id` (not by insertion, not by name);
//! 3. `room_name` is the room title, or the id when the title is blank;
//! 4. omitted (no key: not `[]`, not `null`) when the memory is in no room;
//! 5. never on a document item, even for a document linked to a room;
//! 6. looked up after ranking and the cut to `limit` (order and scores are
//!    pinned in `uteke_core::recall_exec` tests; here: a hit cut by `limit`
//!    does not leak its rooms into the response).
//!
//! `the_surfaces_are_core_in_the_generated_contract` ties the surfaces to the
//! generated `docs/core-contract.json`, so a route dropped from CORE fails here.

use super::*;
use serde_json::{Value, json};
use tiny_http::TestRequest;

const Q: &str = "orchid ledger";

/// Three memories match [`Q`]: one in three rooms, one in no room, and one in
/// one room that is ranked last (for the `limit` check). One document matches
/// too, and is linked to a room.
struct Fixture {
    uteke: Mutex<Uteke>,
    roomed: String,
    roomless: String,
}

fn fixture() -> Fixture {
    let u = uteke_core::test_support::open_keyword();
    // Rooms created out of id order, with a title, no title and a blank title.
    u.create_room("team:beta", Some("Beta room"), "default")
        .unwrap();
    u.create_room("ops:gamma", Some("   "), "default").unwrap();
    let roomed = u
        .remember_in_room(
            &format!("{Q} kept in three rooms"),
            &[],
            None,
            None,
            "fact",
            "team:beta",
            "tester",
        )
        .unwrap()
        .id;
    for room in ["project:alpha", "ops:gamma"] {
        if u.get_room(room).unwrap().is_none() {
            u.create_room(room, None, "default").unwrap();
        }
        assert!(u.add_memory_to_room(&roomed, room, "tester").unwrap());
    }
    let roomless = u
        .remember(&format!("{Q} kept in no room"), &[], None, None)
        .unwrap();
    u.doc_upsert(
        "orchid-doc",
        "Orchid doc",
        &format!("{Q} as a document"),
        &[],
        None,
    )
    .unwrap();
    u.room_add_document("team:beta", "orchid-doc").unwrap();
    Fixture {
        uteke: Mutex::new(u),
        roomed,
        roomless,
    }
}

/// The exact `rooms` value of the memory in three rooms: sorted by `room_id`
/// (`ops:gamma` < `project:alpha` < `team:beta`, while insertion order was
/// beta, alpha, gamma and name order would put "Beta room" first); the blank
/// title and the missing title both fall back to the id.
fn expected_rooms() -> Value {
    json!([
        {"room_id": "ops:gamma", "room_name": "ops:gamma"},
        {"room_id": "project:alpha", "room_name": "project:alpha"},
        {"room_id": "team:beta", "room_name": "Beta room"},
    ])
}

impl Fixture {
    fn post(&self, url: &str, body: Value) -> Value {
        let leaked: &'static str = Box::leak(body.to_string().into_boxed_str());
        let mut req: Request = TestRequest::new()
            .with_method(Method::Post)
            .with_path(url)
            .with_body(leaked)
            .into();
        let ctx = ReqCtx {
            auth_token_hash: None,
            read_only_token_hash: None,
            cors_origins: Vec::new(),
            recall_config: None,
            extraction_config: None,
            host_guard: Default::default(),
            store_id: String::new(),
            embed_warmer: Default::default(),
        };
        let resp = route(&self.uteke, &ctx, &mut req);
        assert_eq!(resp.status_code().0, 200, "{url} {body}");
        serde_json::from_slice(&resp.into_reader().into_inner()).expect("JSON body")
    }

    /// Check rules 1-4 on one response: the roomed memory carries exactly
    /// [`expected_rooms`], the roomless one has no `rooms` key.
    fn assert_hits(&self, surface: &str, hits: &[Value]) {
        let mut seen = (false, false);
        for hit in hits {
            let id = hit["memory"]["id"]
                .as_str()
                .or_else(|| hit["memory_id"].as_str())
                .unwrap_or_else(|| panic!("{surface}: hit without a memory id: {hit}"));
            if id == self.roomed {
                assert_eq!(hit["rooms"], expected_rooms(), "{surface}: {hit}");
                seen.0 = true;
            } else {
                assert!(
                    hit.get("rooms").is_none(),
                    "{surface}: a memory in no room has no `rooms` key: {hit}"
                );
                seen.1 |= id == self.roomless;
            }
        }
        assert!(seen.0, "{surface}: the roomed memory was not returned");
        if surface != "/room/recall" {
            assert!(seen.1, "{surface}: the roomless memory was not returned");
        }
    }
}

fn array(v: &Value) -> &[Value] {
    v.as_array()
        .unwrap_or_else(|| panic!("array expected: {v}"))
}

#[test]
fn the_surfaces_are_core_in_the_generated_contract() {
    let contract: Value =
        serde_json::from_str(include_str!("../../../../docs/core-contract.json")).unwrap();
    let core: Vec<(String, String)> = array(&contract["core"])
        .iter()
        .map(|e| {
            (
                e["method"].as_str().unwrap().to_string(),
                e["path"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    for path in ["/recall", "/search", "/room/recall"] {
        assert!(
            core.contains(&("POST".to_string(), path.to_string())),
            "POST {path} carries the CORE `rooms` field and must be CORE"
        );
    }
}

#[test]
fn recall_plain_unified_pack_and_explain_carry_the_exact_rooms() {
    let f = fixture();
    for strategy in ["fts5", "fusion"] {
        let plain = f.post("/recall", json!({"query": Q, "strategy": strategy}));
        f.assert_hits(&format!("/recall {strategy}"), array(&plain));
    }
    let unified = f.post("/recall", json!({"query": Q, "search_type": "all"}));
    f.assert_hits("/recall unified", array(&unified));
    let pack = f.post("/recall", json!({"query": Q, "pack": true}));
    f.assert_hits("/recall pack", array(&pack["selected"]));
    let explain = f.post(
        "/recall",
        json!({"query": Q, "explain": true, "strategy": "fts5"}),
    );
    f.assert_hits("/recall explain", array(&explain));
}

#[test]
fn search_and_room_recall_with_a_query_carry_the_exact_rooms() {
    let f = fixture();
    let search = f.post("/search", json!({"query": Q}));
    f.assert_hits("/search", array(&search));
    let room = f.post(
        "/room/recall",
        json!({"room_id": "project:alpha", "query": Q}),
    );
    f.assert_hits("/room/recall", array(&room));
}

#[test]
fn a_document_item_never_carries_rooms() {
    let f = fixture();
    for search_type in ["doc", "all"] {
        let items = f.post("/recall", json!({"query": Q, "search_type": search_type}));
        for item in array(&items) {
            if item["result_type"] == "document" {
                assert_eq!(item["doc_slug"], "orchid-doc", "{item}");
                assert!(item.get("rooms").is_none(), "{search_type}: {item}");
            }
        }
    }
    let docs = f.post("/recall", json!({"query": Q, "search_type": "doc"}));
    assert!(
        array(&docs).iter().any(|i| i["result_type"] == "document"),
        "the document is returned, so the check is not vacuous: {docs}"
    );
}

#[test]
fn a_hit_cut_by_limit_leaks_no_rooms() {
    let f = fixture();
    let hits = f.post(
        "/recall",
        json!({"query": Q, "strategy": "fts5", "limit": 1}),
    );
    let hits = array(&hits);
    assert_eq!(hits.len(), 1, "{hits:?}");
    let id = hits[0]["memory"]["id"].as_str().unwrap();
    if id == f.roomed {
        assert_eq!(hits[0]["rooms"], expected_rooms());
    } else {
        assert!(hits[0].get("rooms").is_none(), "{}", hits[0]);
    }
}
