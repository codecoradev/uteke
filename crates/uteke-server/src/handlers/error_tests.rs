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
            embed_warmer: Default::default(),
            remote_embeds: Default::default(),
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
        embed_warmer: Default::default(),
        remote_embeds: Default::default(),
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

/// #1751: `PUT /memory` and `uteke_update` run the same core execution, so
/// the same update leaves the same stored state and is accepted or refused
/// alike (refused fields leave the validity alone, a missing memory is not
/// found). Each case runs against two fresh stores, one per transport.
#[test]
fn http_and_mcp_updates_agree_on_outcome_and_stored_state() {
    let cases: Vec<(&str, Value)> = vec![
        (
            "fields+validity",
            json!({"importance": 0.9, "valid_until": "2099-01-01T00:00:00Z"}),
        ),
        (
            "validity only",
            json!({"valid_until": "2099-01-01T00:00:00Z"}),
        ),
        (
            "refused field + validity",
            json!({"importance": 7.0, "valid_until": "2099-01-01T00:00:00Z"}),
        ),
        (
            "bad type + validity",
            json!({"memory_type": "nope", "valid_until": "2099-01-01T00:00:00Z"}),
        ),
        ("bad interval", json!({"valid_from": "soon"})),
        ("nothing", json!({})),
    ];
    for (name, fields) in cases {
        let http = App::new();
        let (_, ok) = http.call(Method::Post, "/remember", Some(json!({"content": "probe"})));
        let id = ok["id"].as_str().unwrap().to_string();
        let mut body = fields.clone();
        body["id"] = json!(id);
        let (status, _) = http.call(Method::Put, "/memory", Some(body));

        let mcp_store = Uteke::open_with_backend(":memory:", None).unwrap();
        let mid = mcp_store.remember("probe", &[], None, None).unwrap();
        let mut args = fields.clone();
        args["id"] = json!(mid);
        let raw = json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": {"name": "uteke_update", "arguments": args}})
        .to_string();
        let out: Value =
            serde_json::from_str(&uteke_mcp::handle_jsonrpc(&mcp_store, &raw).unwrap()).unwrap();
        let mcp_ok = out["result"]["isError"] != json!(true) && out.get("error").is_none();

        assert_eq!(status == 200, mcp_ok, "{name}: http {status}, mcp {out}");
        let h = http.uteke.lock().unwrap().get_by_id(&id).unwrap().unwrap();
        let m = mcp_store.get_by_id(&mid).unwrap().unwrap();
        assert_eq!(h.importance, m.importance, "{name}");
        assert_eq!(h.valid_until, m.valid_until, "{name}");
        assert_eq!(h.valid_from.is_some(), m.valid_from.is_some(), "{name}");
    }
}

/// #1752: every error variant has one HTTP shape, whichever handler answers
/// it. `Error::samples()` lists every variant (a new one fails the core test
/// until it is listed), and this table fails until its class is placed.
#[test]
fn every_error_variant_has_one_http_wire_shape() {
    use crate::context::http_wire;
    use uteke_core::{Error, ErrorClass};
    let mut classes = Vec::new();
    for e in Error::samples() {
        let wire = http_wire(&e, false);
        let (status, error) = match e.class() {
            ErrorClass::BadRequest => (400, e.to_string()),
            ErrorClass::NotFound => (404, e.to_string()),
            ErrorClass::Busy => (429, e.to_string()),
            ErrorClass::SecretDetected => (400, "secret_detected".to_string()),
            ErrorClass::RejectedValue => (409, "rejected_value".to_string()),
            ErrorClass::InvalidRoomId => (400, "invalid_room_id".to_string()),
            ErrorClass::Embedding => (500, e.to_string()),
            ErrorClass::Internal => (500, "Internal server error".to_string()),
        };
        assert_eq!(wire.status, status, "{e:?}");
        assert_eq!(wire.body["error"], json!(error), "{e:?}");
        // The detail of an internal failure never reaches the client.
        if e.class() == ErrorClass::Internal {
            assert!(!wire.body.to_string().contains("detail"), "{e:?}");
        }
        classes.push(e.class());
    }
    // Every class is exercised by at least one variant.
    for class in [
        ErrorClass::BadRequest,
        ErrorClass::NotFound,
        ErrorClass::Busy,
        ErrorClass::SecretDetected,
        ErrorClass::RejectedValue,
        ErrorClass::InvalidRoomId,
        ErrorClass::Embedding,
        ErrorClass::Internal,
    ] {
        assert!(classes.contains(&class), "{class:?}");
    }
    // The rich bodies carry their extra fields, never a value.
    let secret = http_wire(
        &Error::SecretDetected {
            kinds: vec!["jwt".into()],
        },
        false,
    );
    assert_eq!(secret.body["kinds"], json!(["jwt"]));
    let rejected = http_wire(
        &Error::RejectedValue {
            reason: "wrong".into(),
        },
        false,
    );
    assert_eq!(rejected.body["reason"], "wrong");
    let room = http_wire(&Error::InvalidRoomId { id: "plain".into() }, false);
    assert_eq!(room.body["room_id"], "plain");
    // The bare variant drops the `Validation error: ` prefix only.
    assert_eq!(
        http_wire(&Error::validation("bad"), true).body["error"],
        "bad"
    );
    assert_eq!(
        http_wire(&Error::validation("bad"), false).body["error"],
        "Validation error: bad"
    );
    assert_eq!(
        http_wire(&Error::not_found("gone"), true).body["error"],
        "gone"
    );
}

/// `POST /recall` answers a busy or not-found core error by its class, not
/// 500 (#1752): the status comes from the same mapping as every other route.
#[test]
fn a_recall_failure_is_answered_by_its_class() {
    let (status, body) = answer(uteke_core::Error::Busy("too many".into()));
    assert_eq!((status, body.as_str()), (429, "too many"));
    let req: Request = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/recall")
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
        remote_embeds: Default::default(),
    };
    let resp = ctx.core_error_bare(&req, &uteke_core::Error::validation("bad"));
    assert_eq!(resp.status_code().0, 400);
    let text = String::from_utf8(resp.into_reader().into_inner()).unwrap();
    assert_eq!(text, r#"{"error":"bad"}"#);
    let resp = ctx.core_error_embedding_503(&req, &uteke_core::Error::embed_msg("x"), "no model");
    assert_eq!(resp.status_code().0, 503);
}
