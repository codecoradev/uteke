//! `POST /capture` and `GET /capture/status` (#1466, LAB).

use super::*;
use serde_json::{Value, json};
use tiny_http::TestRequest;

fn app(enabled: bool) -> Mutex<Uteke> {
    let mut u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
    let mut e = uteke_core::EngineSettings::default();
    e.capture.enabled = enabled;
    u.apply_settings(&e);
    Mutex::new(u)
}

fn call(uteke: &Mutex<Uteke>, method: Method, url: &str, body: Option<Value>) -> (u16, Value) {
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
    let resp = route(uteke, &ctx, &mut req);
    let status = resp.status_code().0;
    let text = String::from_utf8(resp.into_reader().into_inner()).unwrap();
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn turn(content: &str) -> Value {
    json!({"session_id": "s1", "role": "user", "content": content})
}

#[test]
fn capture_is_refused_while_it_is_off() {
    let uteke = app(false);
    let (status, body) = call(&uteke, Method::Post, "/capture", Some(turn("hello")));
    assert_eq!(status, 409, "{body}");
    assert!(
        body["error"].as_str().unwrap().contains("[capture]"),
        "{body}"
    );
    let (_, status_body) = call(&uteke, Method::Get, "/capture/status", None);
    assert_eq!(status_body["pending"], 0, "nothing was queued");
}

#[test]
fn a_captured_turn_is_queued_once() {
    let uteke = app(true);
    let (status, first) = call(
        &uteke,
        Method::Post,
        "/capture",
        Some(turn("I prefer tabs over spaces")),
    );
    assert_eq!(status, 200, "{first}");
    assert_eq!(first["duplicate"], false);
    let (_, again) = call(
        &uteke,
        Method::Post,
        "/capture",
        Some(turn("I prefer tabs over spaces")),
    );
    assert_eq!(again["duplicate"], true);
    assert_eq!(again["id"], first["id"]);

    let (status, s) = call(&uteke, Method::Get, "/capture/status", None);
    assert_eq!(status, 200, "{s}");
    assert_eq!(s["pending"], 1);
    assert_eq!(s["sessions_pending"], 1);
}

#[test]
fn an_unknown_role_and_a_missing_field_are_400() {
    let uteke = app(true);
    let (status, body) = call(
        &uteke,
        Method::Post,
        "/capture",
        Some(json!({"session_id": "s1", "role": "system", "content": "x"})),
    );
    assert_eq!(status, 400, "{body}");
    let (status, body) = call(
        &uteke,
        Method::Post,
        "/capture",
        Some(json!({"session_id": "s1", "content": "x"})),
    );
    assert_eq!(status, 400, "{body}");
}

#[test]
fn a_secret_is_masked_before_it_is_queued() {
    let uteke = app(true);
    let (status, body) = call(
        &uteke,
        Method::Post,
        "/capture",
        Some(turn(
            "my key is sk-ant-api03-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["redacted"], true, "{body}");
}
