//! The status and body each handler answers for each class of core error
//! (#1517). The statuses are the ones the API had before the classification.

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
        };
        let resp = route(&self.uteke, &ctx, &mut req);
        let status = resp.status_code().0;
        let text = String::from_utf8(resp.into_reader().into_inner()).unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }
}

#[test]
fn a_missing_room_and_a_missing_document_are_404_without_text_matching() {
    let app = App::new();
    let (status, body) = app.call(Method::Delete, "/room/delete?room_id=ghost", None);
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], "Room not found: ghost");

    let (status, body) = app.call(
        Method::Post,
        "/doc/move",
        Some(json!({"id": "no-such-doc", "new_parent": null})),
    );
    assert_eq!(status, 404, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("not found"),
        "{body}"
    );
}

#[test]
fn a_validation_error_is_400_through_the_shared_mapping() {
    let app = App::new();
    let (_, ok) = app.call(Method::Post, "/remember", Some(json!({"content": "plain"})));
    let id = ok["id"].as_str().unwrap().to_string();

    // Update with an invalid importance: Validation, 400 (update path).
    let (status, body) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": id, "importance": 7.0})),
    );
    assert_eq!(status, 400, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("Importance"),
        "{body}"
    );

    let absent = "0190a1b2-c3d4-7e5f-8a9b-0c1d2e3f4a5b";
    // A missing memory on update is a 404 with the bare message (uteke-cloud#244).
    let (status, body) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": absent, "content": "x"})),
    );
    assert_eq!(status, 404, "{body}");
    assert_eq!(body["error"], format!("Memory not found: {absent}"));
}

#[test]
fn every_named_but_missing_target_is_404_with_the_bare_message() {
    let app = App::new();
    let cases = [
        (
            Method::Post,
            "/consolidate/pair",
            json!({"id_keep": "nope-a", "id_remove": "nope-b"}),
            "memory nope-a not found",
        ),
        (
            Method::Post,
            "/room/rename",
            json!({"from": "ghost", "to": "other"}),
            "Room not found: ghost",
        ),
        (
            Method::Post,
            "/namespaces/rename",
            json!({"from": "ghost-ns", "to": "other-ns"}),
            "Namespace not found: ghost-ns",
        ),
    ];
    for (method, url, body, message) in cases {
        let (status, got) = app.call(method, url, Some(body));
        assert_eq!(status, 404, "{url}: {got}");
        assert_eq!(got["error"], message, "{url}");
    }

    // Moving into a room that does not exist: the target room is missing.
    let (_, room) = app.call(Method::Post, "/room/create", Some(json!({"title": "src"})));
    let src = room["id"]
        .as_str()
        .or(room["room_id"].as_str())
        .unwrap_or("src");
    let (status, got) = app.call(
        Method::Post,
        "/room/memory/move",
        Some(json!({"memory_id": "m", "from_room": src, "to_room": "ghost-room"})),
    );
    assert_eq!(status, 404, "{got}");
    assert_eq!(got["error"], "Room not found: ghost-room");
}

#[test]
fn a_malformed_id_is_still_a_400_not_a_404() {
    let app = App::new();
    let (status, body) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": "not-a-uuid", "content": "x"})),
    );
    assert_eq!(status, 400, "{body}");
}

fn answer(e: uteke_core::Error) -> (u16, String) {
    let req: Request = TestRequest::new()
        .with_method(Method::Get)
        .with_path("/x")
        .into();
    let ctx = ReqCtx {
        auth_token_hash: None,
        read_only_token_hash: None,
        cors_origins: Vec::new(),
        recall_config: None,
        extraction_config: None,
        host_guard: Default::default(),
        store_id: String::new(),
    };
    let resp = ctx.core_error(&req, &e);
    let status = resp.status_code().0;
    let body: Value =
        serde_json::from_str(&String::from_utf8(resp.into_reader().into_inner()).unwrap()).unwrap();
    (status, body["error"].as_str().unwrap().to_string())
}

#[test]
fn a_500_hides_the_detail_except_for_an_embedding_failure() {
    use uteke_core::Error;
    let (status, msg) = answer(Error::generic("secret path /var/lib/x failed"));
    assert_eq!((status, msg.as_str()), (500, "Internal server error"));
    let (status, msg) = answer(Error::Io(std::io::Error::other("/home/u/store.db denied")));
    assert_eq!((status, msg.as_str()), (500, "Internal server error"));
    let (status, msg) = answer(Error::embed_msg("embedder unavailable; run `uteke repair`"));
    assert_eq!(status, 500);
    assert!(msg.contains("uteke repair"), "{msg}");
}
