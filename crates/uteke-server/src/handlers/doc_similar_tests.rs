//! `POST /doc/similar` (#1730): similar documents of a stored document or a
//! draft text, and the near-duplicate report, with exact scores.
//!
//! The embedder maps a marker word to an exact vector (`R43` is `[4, 3, 0]`,
//! whose cosine with `AXIS` `[1, 0, 0]` is 4/5), so every expected score is an
//! exact value and no ONNX runtime is needed.

use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tiny_http::TestRequest;
use uteke_core::test_support::{
    doc_upsert_in_namespace, doc_upsert_unembedded, open_with_embedder,
};

struct MarkerEmbedder;

const MARKERS: [(&str, [f32; 3]); 6] = [
    ("AXIS", [1.0, 0.0, 0.0]),
    ("R43", [4.0, 3.0, 0.0]),
    ("R34", [3.0, 4.0, 0.0]),
    ("R247", [24.0, 7.0, 0.0]),
    ("R724", [7.0, 24.0, 0.0]),
    ("R01", [0.0, 1.0, 0.0]),
];

impl uteke_core::Embedder for MarkerEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, uteke_core::Error> {
        MARKERS
            .iter()
            .find(|(m, _)| text.contains(m))
            .map(|(_, v)| v.to_vec())
            .ok_or_else(|| uteke_core::Error::validation(format!("no marker in {text:?}")))
    }
    fn dims(&self) -> usize {
        3
    }
    fn max_seq_len(&self) -> usize {
        128
    }
    fn name(&self) -> &str {
        "marker-test-embedder"
    }
}

/// An embedder that is down.
struct DownEmbedder;

impl uteke_core::Embedder for DownEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, uteke_core::Error> {
        Err(uteke_core::Error::embed_msg("model unavailable"))
    }
    fn dims(&self) -> usize {
        3
    }
    fn max_seq_len(&self) -> usize {
        128
    }
    fn name(&self) -> &str {
        "down-test-embedder"
    }
}

struct App {
    uteke: Mutex<Uteke>,
}

impl App {
    fn new() -> Self {
        Self {
            uteke: Mutex::new(open_with_embedder(Box::new(MarkerEmbedder))),
        }
    }

    /// `src` and neighbours at the cosines 1.0, 0.96, 0.8, 0.6, 0.28 and 0.
    fn library() -> Self {
        let app = Self::new();
        for (slug, marker) in [
            ("src", "AXIS"),
            ("dup-same", "AXIS"),
            ("dup-near", "R247"),
            ("sim-edge", "R43"),
            ("low", "R34"),
            ("far", "R724"),
            ("orth", "R01"),
        ] {
            app.create(slug, marker, None);
        }
        app
    }

    fn create(&self, slug: &str, marker: &str, parent: Option<&str>) {
        let (status, body) = self.post(
            "/doc/create",
            json!({"slug": slug, "content": format!("# {slug}\n\n{marker}"), "parent": parent}),
        );
        assert_eq!(status, 200, "{body}");
    }

    fn call(
        &self,
        ctx: &ReqCtx,
        method: Method,
        url: &str,
        body: Option<Value>,
        auth: Option<&str>,
    ) -> (u16, Value) {
        let mut builder = TestRequest::new().with_method(method).with_path(url);
        if let Some(b) = body {
            let leaked: &'static str = Box::leak(b.to_string().into_boxed_str());
            builder = builder.with_body(leaked);
        }
        if let Some(token) = auth {
            let h: &'static str = Box::leak(format!("Bearer {token}").into_boxed_str());
            builder =
                builder.with_header(tiny_http::Header::from_bytes("Authorization", h).unwrap());
        }
        let mut req: Request = builder.into();
        let resp = route(&self.uteke, ctx, &mut req);
        let status = resp.status_code().0;
        let text = String::from_utf8(resp.into_reader().into_inner()).unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn post(&self, url: &str, body: Value) -> (u16, Value) {
        self.call(&ctx(false), Method::Post, url, Some(body), None)
    }

    /// 200 and the body.
    fn ok(&self, body: Value) -> Value {
        let (status, v) = self.post("/doc/similar", body);
        assert_eq!(status, 200, "{v}");
        v
    }

    /// The status and error message.
    fn refused(&self, body: Value) -> (u16, String) {
        let (status, v) = self.post("/doc/similar", body);
        (status, v["error"].as_str().unwrap_or_default().to_string())
    }
}

fn ctx(with_tokens: bool) -> ReqCtx {
    ReqCtx {
        auth_token_hash: with_tokens.then(|| Sha256::digest(b"rw-secret").into()),
        read_only_token_hash: with_tokens.then(|| Sha256::digest(b"ro-secret").into()),
        cors_origins: Vec::new(),
        recall_config: None,
        extraction_config: None,
        host_guard: Default::default(),
        store_id: String::new(),
        embed_warmer: Default::default(),
    }
}

fn hits(v: &Value) -> Vec<(String, f64, String)> {
    v["hits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| {
            (
                h["document"]["slug"].as_str().unwrap().to_string(),
                h["score"].as_f64().unwrap(),
                h["relation"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn row(slug: &str, score: f64, relation: &str) -> (String, f64, String) {
    (slug.to_string(), score, relation.to_string())
}

fn pairs(v: &Value) -> Vec<(String, String, f64)> {
    v["pairs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            (
                p["a"]["slug"].as_str().unwrap().to_string(),
                p["b"]["slug"].as_str().unwrap().to_string(),
                p["score"].as_f64().unwrap(),
            )
        })
        .collect()
}

fn pair(a: &str, b: &str, score: f64) -> (String, String, f64) {
    (a.to_string(), b.to_string(), score)
}

// ── similar documents ─────────────────────────────────────────────────────

#[test]
fn a_slug_returns_the_similar_documents_with_exact_scores() {
    let app = App::library();
    let v = app.ok(json!({"slug": "src"}));
    assert_eq!(v["source"]["slug"], "src");
    assert_eq!(v["threshold"], 0.8);
    assert_eq!(v["similar_threshold"], 0.8);
    assert_eq!(v["duplicate_threshold"], 0.92);
    assert_eq!(v["source_embedded"], true);
    assert_eq!(v["documents_without_vector"], 0);
    assert_eq!(v["matches"], 3);
    assert_eq!(
        hits(&v),
        vec![
            row("dup-same", 1.0, "possible_duplicate"),
            row("dup-near", 0.96, "possible_duplicate"),
            row("sim-edge", 0.8, "similar_to"),
        ]
    );
    // An id works as well, and `id` wins over `slug` like on the other routes.
    let id = v["source"]["id"].as_str().unwrap();
    assert_eq!(app.ok(json!({"id": id})), v);
    assert_eq!(app.ok(json!({"id": id, "slug": "ignored"})), v);
}

#[test]
fn threshold_boundaries_are_inclusive_at_the_value() {
    let app = App::library();
    let at = app.ok(json!({"slug": "src", "threshold": 0.8}));
    assert_eq!(at["matches"], 3, "exactly at the threshold is a hit");
    let above = app.ok(json!({"slug": "src", "threshold": 0.8000001}));
    assert_eq!(above["matches"], 2, "just above drops the 0.8 document");
    let below = app.ok(json!({"slug": "src", "threshold": 0.7999999}));
    assert_eq!(below["matches"], 3);
    let at = app.ok(json!({"slug": "src", "threshold": 0.96}));
    assert_eq!(hits(&at).len(), 2);
    let above = app.ok(json!({"slug": "src", "threshold": 0.9600001}));
    assert_eq!(
        hits(&above),
        vec![row("dup-same", 1.0, "possible_duplicate")]
    );
    let low = app.ok(json!({"slug": "src", "threshold": 0.6}));
    assert_eq!(low["threshold"], 0.6);
    assert_eq!(
        hits(&low).last().unwrap(),
        &row("low", 0.6, "related"),
        "below the similar_to threshold a hit is only related"
    );
}

#[test]
fn limit_cuts_the_hits_and_matches_stays_the_count() {
    let app = App::library();
    let v = app.ok(json!({"slug": "src", "limit": 2}));
    assert_eq!(v["matches"], 3);
    assert_eq!(
        hits(&v),
        vec![
            row("dup-same", 1.0, "possible_duplicate"),
            row("dup-near", 0.96, "possible_duplicate"),
        ]
    );
    let v = app.ok(json!({"slug": "src", "limit": 0}));
    assert_eq!(hits(&v).len(), 1, "a limit of 0 is raised to 1");
    let v = app.ok(json!({"slug": "src", "limit": 100000, "threshold": 0}));
    assert_eq!(
        hits(&v).len(),
        6,
        "an oversized limit is clamped, not refused"
    );
}

#[test]
fn a_draft_text_is_compared_and_nothing_is_stored() {
    let app = App::library();
    let v = app.ok(json!({"text": "# Draft\n\nAXIS"}));
    assert!(v.get("source").is_none(), "a draft has no source document");
    assert_eq!(v["source_embedded"], true);
    assert_eq!(v["matches"], 4);
    assert_eq!(
        hits(&v),
        vec![
            row("dup-same", 1.0, "possible_duplicate"),
            row("src", 1.0, "possible_duplicate"),
            row("dup-near", 0.96, "possible_duplicate"),
            row("sim-edge", 0.8, "similar_to"),
        ]
    );
    let (status, list) = app.post("/doc/list", json!({"limit": 50}));
    assert_eq!(status, 200);
    assert_eq!(list.as_array().unwrap().len(), 7, "{list}");
    let other = app.ok(json!({"text": "R01 nothing close", "threshold": 0.9}));
    assert_eq!(
        hits(&other),
        vec![
            row("orth", 1.0, "possible_duplicate"),
            row("far", 0.96, "possible_duplicate"),
        ]
    );
}

#[test]
fn deleted_documents_namespaces_and_documents_without_vectors() {
    let app = App::library();
    {
        let u = app.uteke.lock().unwrap();
        doc_upsert_in_namespace(&u, "ns-copy", "# ns-copy\n\nAXIS", "team-a");
        doc_upsert_unembedded(&u, "pending", "# pending\n\nAXIS");
    }
    let v = app.ok(json!({"slug": "src", "namespace": "team-a"}));
    assert_eq!(hits(&v), vec![row("ns-copy", 1.0, "possible_duplicate")]);
    assert_eq!(
        v["documents_without_vector"], 0,
        "pending is in no namespace"
    );
    let v = app.ok(json!({"slug": "src", "namespace": "team-b"}));
    assert_eq!(v["matches"], 0);
    let v = app.ok(json!({"slug": "src"}));
    assert_eq!(v["matches"], 4);
    assert_eq!(v["documents_without_vector"], 1);
    assert!(hits(&v).iter().all(|h| h.0 != "pending"));

    let (status, _) = app.call(
        &ctx(false),
        Method::Delete,
        "/doc/delete?slug=dup-near",
        None,
        None,
    );
    assert_eq!(status, 200);
    let v = app.ok(json!({"slug": "src"}));
    assert!(hits(&v).iter().all(|h| h.0 != "dup-near"));
    assert_eq!(v["matches"], 3);

    let v = app.ok(json!({"slug": "pending"}));
    assert_eq!(v["source"]["slug"], "pending");
    assert_eq!(v["source_embedded"], false);
    assert_eq!(v["matches"], 0);
    assert_eq!(v["hits"], json!([]));
}

#[test]
fn an_empty_library_has_no_hits() {
    let app = App::new();
    let v = app.ok(json!({"text": "AXIS"}));
    assert_eq!(
        (
            v["matches"].as_u64(),
            v["documents_without_vector"].as_u64()
        ),
        (Some(0), Some(0))
    );
    assert_eq!(v["hits"], json!([]));
    assert_eq!(
        app.refused(json!({"slug": "ghost"})),
        (404, "document not found: ghost".to_string())
    );
    let v = app.ok(json!({"report": true}));
    assert_eq!(v["scanned"], 0);
    assert_eq!(v["pairs"], json!([]));
    assert_eq!(v["groups"], json!([]));
}

// ── duplicate report ──────────────────────────────────────────────────────

fn report_app() -> App {
    let app = App::new();
    app.create("a", "AXIS", None);
    app.create("b", "AXIS", Some("a"));
    app.create("c", "R247", Some("b"));
    app.create("d", "R01", None);
    app.create("e", "R01", None);
    app.create("f", "R724", None);
    app
}

#[test]
fn the_report_lists_pairs_and_groups() {
    let app = report_app();
    let v = app.ok(json!({"report": true}));
    assert_eq!(v["threshold"], 0.92);
    assert_eq!(v["scanned"], 6);
    assert_eq!(v["documents_without_vector"], 0);
    assert_eq!(v["total_pairs"], 6);
    assert_eq!(
        pairs(&v),
        vec![
            pair("a", "b", 1.0),
            pair("d", "e", 1.0),
            pair("a", "c", 0.96),
            pair("b", "c", 0.96),
            pair("d", "f", 0.96),
            pair("e", "f", 0.96),
        ]
    );
    assert_eq!(v["groups"], json!([["a", "b", "c"], ["d", "e", "f"]]));
}

#[test]
fn the_report_threshold_limit_subtree_and_namespace() {
    let app = report_app();
    let v = app.ok(json!({"report": true, "threshold": 0.96}));
    assert_eq!(v["total_pairs"], 6, "exactly at the score is a pair");
    let v = app.ok(json!({"report": true, "threshold": 0.9600001}));
    assert_eq!(pairs(&v), vec![pair("a", "b", 1.0), pair("d", "e", 1.0)]);
    assert_eq!(v["groups"], json!([["a", "b"], ["d", "e"]]));
    let v = app.ok(json!({"report": true, "limit": 2}));
    assert_eq!(v["total_pairs"], 6);
    assert_eq!(pairs(&v), vec![pair("a", "b", 1.0), pair("d", "e", 1.0)]);
    assert_eq!(v["groups"], json!([["a", "b", "c"], ["d", "e", "f"]]));
    let v = app.ok(json!({"report": true, "subtree": "b"}));
    assert_eq!(v["scanned"], 2);
    assert_eq!(pairs(&v), vec![pair("b", "c", 0.96)]);
    let v = app.ok(json!({"report": true, "subtree": "a"}));
    assert_eq!(v["scanned"], 3);
    assert_eq!(v["total_pairs"], 3);
    {
        let u = app.uteke.lock().unwrap();
        doc_upsert_in_namespace(&u, "n1", "# n1\n\nR43", "team-a");
        doc_upsert_in_namespace(&u, "n2", "# n2\n\nR43", "team-a");
    }
    let v = app.ok(json!({"report": true, "namespace": "team-a"}));
    assert_eq!(v["scanned"], 2);
    assert_eq!(pairs(&v), vec![pair("n1", "n2", 1.0)]);
    let v = app.ok(json!({"report": true, "namespace": "team-a", "subtree": "a"}));
    assert_eq!(
        (v["scanned"].as_u64(), v["total_pairs"].as_u64()),
        (Some(0), Some(0))
    );
}

// ── errors and auth ───────────────────────────────────────────────────────

#[test]
fn bad_requests_are_400_with_a_reason() {
    let app = App::library();
    let cases = [
        (
            json!({}),
            "provide 'slug' (or 'id'), 'text', or \"report\": true",
        ),
        (
            json!({"slug": "src", "text": "AXIS"}),
            "provide either 'slug' (or 'id') or 'text', not both",
        ),
        (
            json!({"report": true, "slug": "src"}),
            "a report takes no 'slug', 'id' or 'text'; use 'subtree' and 'namespace'",
        ),
        (
            json!({"report": true, "text": "AXIS"}),
            "a report takes no 'slug', 'id' or 'text'; use 'subtree' and 'namespace'",
        ),
        (
            json!({"slug": "src", "subtree": "src"}),
            "'subtree' needs \"report\": true",
        ),
        (
            json!({"slug": "src", "threshold": 1.5}),
            "Validation error: threshold must be a number between 0 and 1, got 1.5",
        ),
        (
            json!({"slug": "src", "threshold": -0.1}),
            "Validation error: threshold must be a number between 0 and 1, got -0.1",
        ),
        (
            json!({"report": true, "threshold": 2}),
            "Validation error: threshold must be a number between 0 and 1, got 2",
        ),
        (
            json!({"text": "   "}),
            "Validation error: text must not be empty",
        ),
    ];
    for (body, message) in cases {
        let (status, error) = app.refused(body.clone());
        assert_eq!((status, error.as_str()), (400, message), "{body}");
    }
    let (status, error) = app.refused(json!({"slug": "src", "treshold": 0.5}));
    assert_eq!(status, 400);
    assert!(error.contains("unknown field `treshold`"), "{error}");
    let (status, error) = app.refused(json!({"slug": 7}));
    assert_eq!(status, 400);
    assert!(error.starts_with("Invalid JSON"), "{error}");
}

#[test]
fn unknown_documents_are_404() {
    let app = App::library();
    assert_eq!(
        app.refused(json!({"slug": "ghost"})),
        (404, "document not found: ghost".to_string())
    );
    assert_eq!(
        app.refused(json!({"report": true, "subtree": "ghost"})),
        (404, "document not found: ghost".to_string())
    );
}

#[test]
fn a_draft_text_without_the_model_is_503_and_a_stored_slug_needs_none() {
    let app = App {
        uteke: Mutex::new(open_with_embedder(Box::new(DownEmbedder))),
    };
    let (status, v) = app.post("/doc/similar", json!({"text": "AXIS"}));
    assert_eq!(status, 503, "{v}");
    assert_eq!(
        v["error"],
        "embedding model not available for similarity search"
    );
    // A report and a stored document read vectors from SQLite: no model needed.
    let v = app.ok(json!({"report": true}));
    assert_eq!(v["scanned"], 0);
}

#[test]
fn the_read_only_token_may_call_it_and_anonymous_may_not() {
    let app = App::library();
    let call = |token: Option<&str>| {
        app.call(
            &ctx(true),
            Method::Post,
            "/doc/similar",
            Some(json!({"slug": "src", "threshold": 0.9})),
            token,
        )
    };
    let (status, v) = call(Some("ro-secret"));
    assert_eq!(status, 200, "{v}");
    assert_eq!(v["matches"], 2);
    let (status, v) = call(Some("rw-secret"));
    assert_eq!(status, 200, "{v}");
    let (status, _) = call(None);
    assert_eq!(status, 401);
    let (status, _) = call(Some("wrong"));
    assert_eq!(status, 401);
    let (status, _) = app.call(
        &ctx(true),
        Method::Post,
        "/doc/similar",
        Some(json!({"report": true})),
        Some("ro-secret"),
    );
    assert_eq!(status, 200, "a report is read-only too");
}

#[test]
fn the_route_is_a_lab_row_that_does_not_write() {
    let row = crate::api_registry::find("POST", "/doc/similar").expect("registered");
    assert_eq!(row.tier, crate::api_registry::Tier::Lab);
    assert!(!row.writes);
    assert_eq!(row.request_type, Some("DocSimilarRequest"));
    // Only POST is routed.
    assert!(crate::api_registry::find("GET", "/doc/similar").is_none());
}
