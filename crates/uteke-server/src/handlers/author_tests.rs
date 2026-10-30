//! `author` on every write path over HTTP (#1427 phase 2): `POST /remember`
//! takes it without a room, `POST /room/remember` stamps it on the memory too,
//! and it comes back on the memory object.

use super::*;
use serde_json::{Value, json};
use tiny_http::TestRequest;

struct App {
    uteke: Mutex<Uteke>,
}

impl App {
    fn new() -> Self {
        Self {
            uteke: Mutex::new(
                Uteke::open_with_backend(":memory:", None).expect("open without embedder"),
            ),
        }
    }

    fn call(&self, method: Method, url: &str, body: Option<Value>) -> (u16, Value) {
        let mut req: Request = match body {
            Some(b) => {
                let leaked: &'static str = Box::leak(b.to_string().into_boxed_str());
                TestRequest::new()
                    .with_method(method)
                    .with_path(url)
                    .with_body(leaked)
                    .into()
            }
            None => TestRequest::new().with_method(method).with_path(url).into(),
        };
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
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    fn author_of(&self, id: &str) -> Value {
        let (status, memory) = self.call(Method::Get, &format!("/memory?id={id}"), None);
        assert_eq!(status, 200, "{memory}");
        memory["author"].clone()
    }
}

#[test]
fn remember_takes_an_author_without_a_room_and_returns_it() {
    let app = App::new();
    let (status, body) = app.call(
        Method::Post,
        "/remember",
        Some(json!({"content": "attributed note", "author": "  claude-code "})),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(
        body["author"], "claude-code",
        "the response echoes the trimmed author"
    );
    let id = body["id"].as_str().unwrap();
    assert_eq!(app.author_of(id), "claude-code", "GET /memory returns it");

    // No author: nothing recorded, nothing echoed, nothing serialized.
    let (_, plain) = app.call(
        Method::Post,
        "/remember",
        Some(json!({"content": "plain note"})),
    );
    assert!(plain.get("author").is_none(), "{plain}");
    assert_eq!(app.author_of(plain["id"].as_str().unwrap()), Value::Null);

    // It also reaches /list.
    let (_, listed) = app.call(Method::Post, "/list", Some(json!({"limit": 10})));
    let authors: Vec<Value> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["author"].clone())
        .collect();
    assert_eq!(
        authors.iter().filter(|a| *a == "claude-code").count(),
        1,
        "{listed}"
    );
}

#[test]
fn an_invalid_author_is_a_400_and_stores_nothing() {
    let app = App::new();
    for bad in [
        json!(""),
        json!("   "),
        json!("tab\there"),
        json!("y".repeat(129)),
    ] {
        let (status, body) = app.call(
            Method::Post,
            "/remember",
            Some(json!({"content": "must not be stored", "author": bad})),
        );
        assert_eq!(status, 400, "{bad}: {body}");
        assert!(body.to_string().contains("author"), "{bad}: {body}");
    }
    let (_, listed) = app.call(Method::Post, "/list", Some(json!({"limit": 10})));
    assert_eq!(
        listed.as_array().unwrap().len(),
        0,
        "an invalid author stores nothing"
    );
}

#[test]
fn room_remember_stamps_the_writer_on_the_memory() {
    let app = App::new();
    let (status, named) = app.call(
        Method::Post,
        "/room/remember",
        Some(json!({"room_id": "r1", "content": "from alice", "author": "alice"})),
    );
    assert_eq!(status, 200, "{named}");
    assert_eq!(app.author_of(named["id"].as_str().unwrap()), "alice");

    // No author named: the room link falls back to "user", the memory has none.
    let (_, unnamed) = app.call(
        Method::Post,
        "/room/remember",
        Some(json!({"room_id": "r1", "content": "from nobody"})),
    );
    assert_eq!(app.author_of(unnamed["id"].as_str().unwrap()), Value::Null);

    // An author the room link accepts but the memory field rejects is not an
    // error: the write succeeded, the memory stays unattributed.
    let (status, long) = app.call(
        Method::Post,
        "/room/remember",
        Some(json!({"room_id": "r1", "content": "long author", "author": "z".repeat(200)})),
    );
    assert_eq!(status, 200, "{long}");
    assert_eq!(app.author_of(long["id"].as_str().unwrap()), Value::Null);
}
