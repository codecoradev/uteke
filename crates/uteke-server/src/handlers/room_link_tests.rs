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
