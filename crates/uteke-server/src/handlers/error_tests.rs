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

    // A missing memory on update stays a 400 until the contract moves it.
    let (status, _) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": "no-such-memory", "content": "x"})),
    );
    assert_eq!(status, 400);

    // Consolidate pair with unknown ids: Validation, 400.
    let (status, _) = app.call(
        Method::Post,
        "/consolidate/pair",
        Some(json!({"id_keep": "nope-a", "id_remove": "nope-b"})),
    );
    assert_eq!(status, 400);
}
