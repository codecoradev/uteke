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

/// #1632: `/import` takes bodies above the 10 MB limit of the other routes.
#[test]
fn an_import_body_above_the_general_limit_is_accepted() {
    let u = store();
    let padding = " ".repeat(uteke_core::MAX_PAYLOAD_SIZE + 1024);
    let (st, json) = post(
        &u,
        "/import",
        serde_json::json!({ "content": format!("{padding}{{\"content\":\"one row\"}}") }),
    );
    assert_eq!(st, 200, "{json}");
    assert_eq!(json["imported"], 1, "{json}");
}

/// #1632: a body over the import limit is refused before it is read, with a
/// message that names the limit and the ways around it.
#[test]
fn an_import_over_its_limit_is_413_with_the_way_out() {
    let u = store();
    let declared = (crate::types::max_import_size() + 1).to_string();
    let mut req: tiny_http::Request = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/import")
        .with_header(tiny_http::Header::from_bytes("Content-Length", declared.as_bytes()).unwrap())
        .with_body("{}")
        .into();
    let resp = route(&u, &ctx(), &mut req);
    assert_eq!(resp.status_code().0, 413);
    let body: serde_json::Value = serde_json::from_slice(&resp.into_reader().into_inner()).unwrap();
    let msg = body.to_string();
    for needle in ["UTEKE_MAX_IMPORT_SIZE", "defer_embedding", "uteke import"] {
        assert!(msg.contains(needle), "{needle}: {msg}");
    }
}

#[test]
fn the_import_limit_reads_its_variable() {
    use crate::types::{DEFAULT_MAX_IMPORT_SIZE, import_limit_from};
    assert_eq!(import_limit_from(None), DEFAULT_MAX_IMPORT_SIZE);
    assert_eq!(import_limit_from(Some("33554432")), 33_554_432);
    assert_eq!(import_limit_from(Some(" 1048576 ")), 1_048_576);
    for bad in ["", "0", "-5", "lots"] {
        assert_eq!(
            import_limit_from(Some(bad)),
            DEFAULT_MAX_IMPORT_SIZE,
            "{bad}"
        );
    }
}

/// #1632: a Cloud export over HTTP reports the documents and rooms.
#[test]
fn a_cloud_export_reports_documents_and_rooms() {
    let u = store();
    let file = [
        r#"{"format_version":1,"exported_at":"2026-11-02T00:00:00Z"}"#,
        r#"{"content":"a memory in the room","rooms":["r1"]}"#,
        r#"{"type":"document","slug":"guide","title":"Guide","content":"the guide","parent":null}"#,
        r#"{"type":"room","id":"r1","title":"Team","doc_slugs":["guide"]}"#,
    ]
    .join("\n");
    let (st, json) = post(&u, "/import", serde_json::json!({ "content": file }));
    assert_eq!(st, 200, "{json}");
    assert_eq!(json["imported"], 1, "{json}");
    assert_eq!(json["skipped"], 0, "{json}");
    assert_eq!(json["documents_imported"], 1, "{json}");
    assert_eq!(json["rooms_imported"], 1, "{json}");
    assert_eq!(json["room_documents_linked"], 1, "{json}");
    assert_eq!(json["room_links"], 1, "{json}");
}
