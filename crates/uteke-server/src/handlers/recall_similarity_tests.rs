//! `POST /recall`: `similarity` on results and `strict` on it (#1650).

use std::sync::Mutex;

use tiny_http::{Method, TestRequest};
use uteke_core::Uteke;

use super::route;
use crate::context::ReqCtx;

fn store() -> Mutex<Uteke> {
    let u = uteke_core::test_support::open_keyword();
    u.remember(
        "quarterly revenue projections for the board meeting",
        &[],
        None,
        None,
    )
    .unwrap();
    for t in ["gardening tulips", "zebra savanna", "volcano basalt"] {
        u.remember(&format!("{t} notes"), &[], None, None).unwrap();
    }
    Mutex::new(u)
}

fn recall(app: &Mutex<Uteke>, body: serde_json::Value) -> serde_json::Value {
    let body: &'static str = Box::leak(body.to_string().into_boxed_str());
    let mut req: tiny_http::Request = TestRequest::new()
        .with_method(Method::Post)
        .with_path("/recall")
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
    let resp = route(app, &ctx, &mut req);
    assert_eq!(resp.status_code().0, 200);
    serde_json::from_slice(&resp.into_reader().into_inner()).unwrap()
}

#[test]
fn results_carry_similarity_and_strict_keeps_the_confident_one() {
    let app = store();
    let q = "quarterly revenue projections";
    let all = recall(&app, serde_json::json!({"query": q, "namespace": null}));
    let all = all.as_array().unwrap();
    assert!(all.len() > 1);
    assert!(all.iter().all(|r| r.get("similarity").is_some()));

    let strict = recall(
        &app,
        serde_json::json!({"query": q, "strict": true, "namespace": null}),
    );
    let strict = strict.as_array().unwrap();
    assert_eq!(strict.len(), 1, "{strict:?}");
    assert!(strict[0]["similarity"].as_f64().unwrap() >= 0.5);
    assert!(
        strict[0]["score"].as_f64().unwrap() < 0.5,
        "score is untouched"
    );
}

#[test]
fn a_full_text_only_hit_has_a_null_similarity_and_strict_drops_it() {
    let app = store();
    let body = |strict: bool| {
        serde_json::json!({
            "query": "quarterly revenue projections",
            "strategy": "fts5", "strict": strict, "namespace": null
        })
    };
    let plain = recall(&app, body(false));
    let plain = plain.as_array().unwrap();
    assert!(!plain.is_empty());
    assert!(plain.iter().all(|r| r["similarity"].is_null()));
    let strict = recall(&app, body(true));
    assert_eq!(strict.as_array().unwrap().len(), 0, "{strict}");
}

#[test]
fn pack_results_carry_the_similarity_too() {
    let app = store();
    let packed = recall(
        &app,
        serde_json::json!({"query": "quarterly revenue projections", "pack": true,
                           "search_type": "memory", "namespace": null}),
    );
    let first = &packed["selected"][0];
    assert!(first["similarity"].as_f64().is_some(), "{packed}");
}
