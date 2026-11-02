//! `POST /import` options and counts (#1634, #1633).

use std::sync::Mutex;

use tiny_http::{Method, TestRequest};
use uteke_core::Uteke;

use super::route;
use crate::context::ReqCtx;

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

fn post(uteke: &Mutex<Uteke>, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
    let body: &'static str = Box::leak(body.to_string().into_boxed_str());
    let mut req: tiny_http::Request = TestRequest::new()
        .with_method(Method::Post)
        .with_path(path)
        .with_body(body)
        .into();
    let resp = route(uteke, &ctx(), &mut req);
    let status = resp.status_code().0;
    let bytes = resp.into_reader().into_inner();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn store() -> Mutex<Uteke> {
    Mutex::new(Uteke::open_with_backend(":memory:", None).expect("open without embedder"))
}

const FILE: &str = "{\"content\":\"the quokka is a small marsupial\"}\n{\"content\":\"rust has no garbage collector\"}";

#[test]
fn defer_embedding_stores_the_rows_and_reports_them() {
    let u = store();
    let (st, json) = post(
        &u,
        "/import",
        serde_json::json!({ "content": FILE, "defer_embedding": true }),
    );
    assert_eq!(st, 200, "{json}");
    assert_eq!(json["imported"], 2, "{json}");
    assert_eq!(json["deferred_embedding"], 2, "{json}");
    // keyword-searchable at once
    let (st, hits) = post(&u, "/search", serde_json::json!({ "query": "quokka" }));
    assert_eq!(st, 200, "{hits}");
    assert_eq!(hits.as_array().map(Vec::len), Some(1), "{hits}");

    // a deferred re-import skips the exact copies
    let (_, again) = post(
        &u,
        "/import",
        serde_json::json!({ "content": FILE, "defer_embedding": true }),
    );
    assert_eq!(again["imported"], 0, "{again}");
    assert_eq!(again["duplicates_skipped"], 2, "{again}");
    assert_eq!(again["deferred_embedding"], 0, "{again}");
}

#[test]
fn without_the_field_the_response_is_unchanged() {
    let u = store();
    let (st, json) = post(&u, "/import", serde_json::json!({ "content": FILE }));
    assert_eq!(st, 200, "{json}");
    assert_eq!(json["imported"], 2, "{json}");
    assert!(json.get("deferred_embedding").is_none(), "{json}");
}

#[test]
fn defer_embedding_must_be_a_boolean() {
    let u = store();
    let (st, json) = post(
        &u,
        "/import",
        serde_json::json!({ "content": FILE, "defer_embedding": "yes" }),
    );
    assert_eq!(st, 400, "{json}");
}
