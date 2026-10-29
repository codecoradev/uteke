//! `POST /room/memory` adds an existing memory to a room (#1589).

use std::sync::Mutex;

use tiny_http::{Method, TestRequest};
use uteke_core::Uteke;

use super::route;
use crate::context::ReqCtx;

struct App {
    uteke: Mutex<Uteke>,
}

impl App {
    fn new() -> Self {
        let u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        u.create_room("team", None, "default").unwrap();
        Self {
            uteke: Mutex::new(u),
        }
    }

    fn post(&self, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        let body: &'static str = Box::leak(body.to_string().into_boxed_str());
        let mut req: tiny_http::Request = TestRequest::new()
            .with_method(Method::Post)
            .with_path(path)
            .with_body(body)
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
        let status = resp.status_code().0;
        let bytes = resp.into_reader().into_inner();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    fn remember(&self, content: &str) -> String {
        let (st, body) = self.post("/remember", serde_json::json!({ "content": content }));
        assert_eq!(st, 200, "{body}");
        body["id"].as_str().unwrap().to_string()
    }

    fn members(&self, room: &str) -> usize {
        self.uteke
            .lock()
            .unwrap()
            .recall_room(room, None, 0)
            .unwrap()
            .len()
    }
}

#[test]
fn an_existing_memory_joins_a_room_once_and_is_not_copied() {
    let app = App::new();
    let id = app.remember("shared by two rooms");
    let total = app.uteke.lock().unwrap().count(None).unwrap();

    let (st, body) = app.post(
        "/room/memory",
        serde_json::json!({"memory_id": id, "room": "team", "author": "alice"}),
    );
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["linked"], true);
    assert_eq!(body["already_linked"], false);

    // A prefix works, and linking twice is not an error.
    let (st, body) = app.post(
        "/room/memory",
        serde_json::json!({"memory_id": &id[..8], "room": "team"}),
    );
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["already_linked"], true);
    assert_eq!(body["memory_id"], id.as_str());

    assert_eq!(app.members("team"), 1);
    assert_eq!(app.uteke.lock().unwrap().count(None).unwrap(), total);
}

#[test]
fn a_missing_room_or_memory_is_a_404_and_a_bad_body_a_400() {
    let app = App::new();
    let id = app.remember("exists");
    let (st, body) = app.post(
        "/room/memory",
        serde_json::json!({"memory_id": id, "room": "nope"}),
    );
    assert_eq!(st, 404, "{body}");
    let (st, body) = app.post(
        "/room/memory",
        serde_json::json!({"memory_id": "ffffffff", "room": "team"}),
    );
    assert_eq!(st, 404, "{body}");
    let (st, _) = app.post(
        "/room/memory",
        serde_json::json!({"memory_id": "%", "room": "team"}),
    );
    assert_eq!(st, 400);
    let (st, _) = app.post("/room/memory", serde_json::json!({"memory_id": id}));
    assert_eq!(st, 400, "room is required");
    assert_eq!(app.members("team"), 0);
}

/// `/export` carries each memory's rooms and `/import` restores the links and
/// names the rooms the target lacks (#1588).
#[test]
fn export_and_import_round_trip_the_room_links() {
    use tiny_http::Method;
    let get = |app: &Mutex<Uteke>, path: &str| -> String {
        let mut req: tiny_http::Request = TestRequest::new()
            .with_method(Method::Get)
            .with_path(path)
            .into();
        let resp = route(app, &ctx(), &mut req);
        String::from_utf8(resp.into_reader().into_inner()).unwrap()
    };
    let source = Mutex::new(uteke_core::test_support::open_keyword());
    {
        let u = source.lock().unwrap();
        u.create_room("team", None, "default").unwrap();
        u.create_room("only-here", None, "default").unwrap();
        let id = u
            .remember(
                "alpha beta gamma delta epsilon zeta eta theta iota",
                &[],
                None,
                None,
            )
            .unwrap();
        u.add_memory_to_room(&id, "team", "alice").unwrap();
        u.add_memory_to_room(&id, "only-here", "alice").unwrap();
    }
    let export = get(&source, "/export");
    assert!(
        export.contains("\"rooms\":[\"only-here\",\"team\"]"),
        "{export}"
    );

    let target = Mutex::new(uteke_core::test_support::open_keyword());
    target
        .lock()
        .unwrap()
        .create_room("team", None, "default")
        .unwrap();
    let export_json: String = serde_json::from_str::<serde_json::Value>(&export)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or(export.clone());
    let body: &'static str = Box::leak(
        serde_json::json!({ "content": export_json })
            .to_string()
            .into_boxed_str(),
    );
    let mut req: tiny_http::Request = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/import")
        .with_body(body)
        .into();
    let resp = route(&target, &ctx(), &mut req);
    assert_eq!(resp.status_code().0, 200);
    let json: serde_json::Value = serde_json::from_slice(&resp.into_reader().into_inner()).unwrap();
    assert_eq!(json["imported"], 1, "{json}");
    assert_eq!(json["room_links"], 1, "{json}");
    assert_eq!(
        json["rooms_missing"],
        serde_json::json!(["only-here"]),
        "{json}"
    );
    assert_eq!(
        target
            .lock()
            .unwrap()
            .recall_room("team", None, 0)
            .unwrap()
            .len(),
        1
    );
}

fn ctx() -> ReqCtx {
    ReqCtx {
        auth_token_hash: None,
        read_only_token_hash: None,
        cors_origins: Vec::new(),
        recall_config: None,
        extraction_config: None,
        host_guard: Default::default(),
        store_id: String::new(),
        embed_warmer: Default::default(),
    }
}
