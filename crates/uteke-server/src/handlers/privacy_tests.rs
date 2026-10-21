//! The privacy filter over HTTP (#1464): `redacted` on the remember responses,
//! and a strict-mode rejection as `400 secret_detected` with the kinds and
//! never the value.

use super::*;
use serde_json::{Value, json};
use tiny_http::TestRequest;
use uteke_core::privacy::{PrivacyConfig, PrivacyMode};

struct App {
    uteke: Mutex<Uteke>,
}

impl App {
    fn new(mode: PrivacyMode) -> Self {
        let mut u = Uteke::open_with_backend(":memory:", None).expect("open without embedder");
        u.set_privacy_config(PrivacyConfig {
            mode,
            allowlist: Vec::new(),
        });
        Self {
            uteke: Mutex::new(u),
        }
    }

    fn call(&self, method: Method, url: &str, body: Option<Value>) -> (u16, Value, String) {
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
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::Null),
            text,
        )
    }

    fn count(&self) -> usize {
        let (_, listed, _) = self.call(Method::Post, "/list", Some(json!({"limit": 50})));
        listed.as_array().map_or(0, Vec::len)
    }
}

fn token() -> String {
    format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
}

#[test]
fn remember_reports_redacted_and_stores_the_masked_text() {
    let app = App::new(PrivacyMode::Redact);
    let (status, body, _) = app.call(
        Method::Post,
        "/remember",
        Some(json!({"content": format!("deploy key {}", token())})),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["redacted"], true);
    let id = body["id"].as_str().unwrap();
    let (_, memory, raw) = app.call(Method::Get, &format!("/memory?id={id}"), None);
    assert!(
        memory["content"]
            .as_str()
            .unwrap()
            .contains("[REDACTED:github-token]")
    );
    assert!(
        !raw.contains("aB3dE5gH7jK9"),
        "the secret must not come back: {raw}"
    );
    assert_eq!(memory["metadata"]["redacted"], true);

    let (_, clean, _) = app.call(
        Method::Post,
        "/remember",
        Some(json!({"content": "nothing secret here"})),
    );
    assert_eq!(clean["redacted"], false, "{clean}");
}

#[test]
fn strict_mode_rejects_with_400_secret_detected_and_stores_nothing() {
    let app = App::new(PrivacyMode::Strict);
    let (status, body, raw) = app.call(
        Method::Post,
        "/remember",
        Some(json!({"content": format!("deploy key {}", token())})),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "secret_detected");
    assert_eq!(body["kinds"], json!(["github-token"]));
    assert!(body["message"].as_str().unwrap().contains("github-token"));
    assert!(
        !raw.contains("aB3dE5gH7jK9"),
        "the value must not be echoed: {raw}"
    );
    assert_eq!(app.count(), 0);

    // Room remember and update answer the same way.
    let (status, body, _) = app.call(
        Method::Post,
        "/room/remember",
        Some(json!({"room_id": "r1", "content": format!("key {}", token())})),
    );
    assert_eq!((status, &body["error"]), (400, &json!("secret_detected")));
    let (_, ok, _) = app.call(Method::Post, "/remember", Some(json!({"content": "plain"})));
    let id = ok["id"].as_str().unwrap();
    let (status, body, _) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": id, "content": format!("now {}", token())})),
    );
    assert_eq!((status, &body["error"]), (400, &json!("secret_detected")));
    assert_eq!(app.count(), 1, "only the plain memory exists");
}

#[test]
fn room_remember_reports_redacted() {
    let app = App::new(PrivacyMode::Redact);
    let (status, body, _) = app.call(
        Method::Post,
        "/room/remember",
        Some(json!({"room_id": "r1", "content": format!("key {}", token())})),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["redacted"], true);
}

#[test]
fn put_memory_reports_redacted_and_strict_covers_source_and_metadata() {
    let app = App::new(PrivacyMode::Redact);
    let (_, ok, _) = app.call(Method::Post, "/remember", Some(json!({"content": "plain"})));
    let id = ok["id"].as_str().unwrap().to_string();
    let (status, body, _) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": id, "metadata": {"note": format!("k {}", token())}})),
    );
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["redacted"], true);
    let (_, body, _) = app.call(
        Method::Put,
        "/memory",
        Some(json!({"id": id, "importance": 0.8})),
    );
    assert_eq!(body["redacted"], false, "{body}");

    let strict = App::new(PrivacyMode::Strict);
    let (status, body, raw) = strict.call(
        Method::Post,
        "/remember",
        Some(json!({"content": "plain", "source": token()})),
    );
    assert_eq!((status, &body["error"]), (400, &json!("secret_detected")));
    assert!(!raw.contains("aB3dE5gH7jK9"), "{raw}");
    assert_eq!(
        strict.count(),
        0,
        "nothing is stored before the source check"
    );
    let (status, body, _) = strict.call(
        Method::Post,
        "/remember",
        Some(json!({"content": "plain", "metadata": {"k": token()}})),
    );
    assert_eq!((status, &body["error"]), (400, &json!("secret_detected")));
}
