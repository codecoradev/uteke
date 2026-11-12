//! `POST /skill/search|list|show|similar` (#1731): exact answers, bounds,
//! errors, auth, and that skill files never leak through the document routes.

use super::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tiny_http::TestRequest;
use uteke_core::test_support::{
    ISOLATION_QUERY as Q, open_keyword, open_with_embedder, seed_skill_library,
};

struct App {
    uteke: Mutex<Uteke>,
    deploy_hash: String,
}

impl App {
    fn new() -> Self {
        let u = open_keyword();
        let deploy_hash = seed_skill_library(&u);
        Self {
            uteke: Mutex::new(u),
            deploy_hash,
        }
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

    fn ok(&self, url: &str, body: Value) -> Value {
        let (status, v) = self.post(url, body);
        assert_eq!(status, 200, "{url}: {v}");
        v
    }

    fn refused(&self, url: &str, body: Value) -> (u16, String) {
        let (status, v) = self.post(url, body);
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

fn slugs(v: &Value, key: &str) -> Vec<String> {
    v[key]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h["slug"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn search_answers_with_exact_values() {
    let app = App::new();
    let v = app.ok(
        "/skill/search",
        json!({"query": "emberglow", "mode": "fts"}),
    );
    assert_eq!(slugs(&v, "hits"), ["deploy"]);
    assert_eq!(v["hits"][0]["file"], "references/rollback.md");
    assert_eq!(v["hits"][0]["status"], "approved");
    assert_eq!(v["hits"][0]["hash"], app.deploy_hash.as_str());
    assert_eq!(v["hits"][0]["description"], "Ship a release safely");
    assert_eq!(
        (v["limit"].as_u64(), v["count"].as_u64(), v["mode"].as_str()),
        (Some(5), Some(1), Some("fts"))
    );
    let v = app.ok("/skill/search", json!({"query": Q}));
    assert_eq!(v["mode"], "hybrid");
    assert_eq!(slugs(&v, "hits")[0..2], ["deploy", "deploy-copy"]);
    let v = app.ok("/skill/search", json!({"query": Q, "approved_only": true}));
    assert_eq!(slugs(&v, "hits"), ["deploy"]);
    let v = app.ok("/skill/search", json!({"query": Q, "limit": 1}));
    assert_eq!(v["hits"].as_array().unwrap().len(), 1);
}

#[test]
fn search_limits_and_query_are_bounded() {
    let app = App::new();
    let limit = |l: u64| app.ok("/skill/search", json!({"query": Q, "limit": l}))["limit"].as_u64();
    assert_eq!(
        (limit(0), limit(50), limit(51), limit(u64::MAX)),
        (Some(1), Some(50), Some(50), Some(50))
    );
    assert_eq!(
        app.ok(
            "/skill/search",
            json!({"query": "a".repeat(2000), "mode": "fts"})
        )["count"],
        0
    );
    assert_eq!(
        app.refused("/skill/search", json!({"query": "a".repeat(2001)})),
        (
            400,
            "Validation error: query is longer than 2000 characters".to_string()
        )
    );
    assert_eq!(
        app.refused("/skill/search", json!({"query": "  "})),
        (400, "Validation error: query must not be empty".to_string())
    );
    assert_eq!(
        app.refused("/skill/search", json!({"query": "x", "mode": "fuzzy"})),
        (
            400,
            "Validation error: mode must be hybrid, semantic or fts".to_string()
        )
    );
}

#[test]
fn list_show_and_similar_answer_with_exact_values() {
    let app = App::new();
    let v = app.ok("/skill/list", json!({}));
    assert_eq!(slugs(&v, "skills"), ["cooking", "deploy", "deploy-copy"]);
    assert_eq!(
        (
            v["limit"].as_u64(),
            v["total"].as_u64(),
            v["count"].as_u64()
        ),
        (Some(50), Some(3), Some(3))
    );
    let v = app.ok("/skill/list", json!({"status": "approved", "limit": 1}));
    assert_eq!(slugs(&v, "skills"), ["deploy"]);
    let v = app.ok("/skill/list", json!({"status": "draft", "limit": 1}));
    assert_eq!(
        (v["total"].as_u64(), v["count"].as_u64()),
        (Some(2), Some(1))
    );
    let limit = |l: u64| app.ok("/skill/list", json!({"limit": l}))["limit"].as_u64();
    assert_eq!(
        (limit(0), limit(200), limit(201)),
        (Some(1), Some(200), Some(200))
    );

    let v = app.ok("/skill/show", json!({"slug": "deploy"}));
    assert_eq!(v["approved_hash"], app.deploy_hash.as_str());
    assert_eq!(v["current"]["file_count"], 2);
    assert_eq!(v["owner"], "alice");

    let v = app.ok("/skill/similar", json!({"slug": "deploy-copy"}));
    assert_eq!(slugs(&v, "hits"), ["deploy"]);
    assert_eq!(v["hits"][0]["relation"], "possible_duplicate");
    let v = app.ok(
        "/skill/similar",
        json!({"text": Q, "limit": 1, "threshold": 0.0}),
    );
    assert_eq!(v["hits"].as_array().unwrap().len(), 1);
    assert!(v["source"].is_null());
}

#[test]
fn errors_are_400_404_and_503() {
    let app = App::new();
    assert_eq!(
        app.refused("/skill/show", json!({"slug": "ghost"})),
        (404, "skill not found: ghost".to_string())
    );
    assert_eq!(
        app.refused("/skill/similar", json!({"slug": "ghost"})),
        (404, "skill not found: ghost".to_string())
    );
    assert_eq!(
        app.refused("/skill/show", json!({"slug": "a".repeat(65)})),
        (
            400,
            "Validation error: slug is longer than 64 characters".to_string()
        )
    );
    assert_eq!(
        app.refused("/skill/similar", json!({})),
        (
            400,
            "Validation error: provide 'slug' or 'text'".to_string()
        )
    );
    assert_eq!(
        app.refused("/skill/similar", json!({"slug": "deploy", "text": "x"})),
        (
            400,
            "Validation error: provide either 'slug' or 'text', not both".to_string()
        )
    );
    assert_eq!(
        app.refused("/skill/similar", json!({"text": "x".repeat(32_769)})),
        (
            400,
            "Validation error: text is longer than 32768 characters".to_string()
        )
    );
    assert_eq!(
        app.post("/skill/similar", json!({"text": "x".repeat(32_768)}))
            .0,
        200
    );
    assert_eq!(
        app.refused(
            "/skill/similar",
            json!({"slug": "deploy", "threshold": 1.5})
        ),
        (
            400,
            "Validation error: threshold must be a number between 0 and 1, got 1.5".to_string()
        )
    );
    // Unknown and mistyped fields.
    for (url, body) in [
        ("/skill/search", json!({"query": "x", "namespace": "a"})),
        ("/skill/list", json!({"query": "x"})),
        ("/skill/show", json!({"slug": "deploy", "limit": 1})),
        ("/skill/similar", json!({"slug": "deploy", "mode": "fts"})),
        ("/skill/search", json!({})),
        ("/skill/show", json!({})),
        ("/skill/search", json!({"query": "x", "limit": -1})),
        ("/skill/list", json!({"limit": "ten"})),
    ] {
        let (status, error) = app.refused(url, body.clone());
        assert_eq!(status, 400, "{url} {body}: {error}");
    }
}

#[test]
fn a_draft_text_without_the_model_is_503_and_a_stored_slug_needs_none() {
    struct Down;
    impl uteke_core::Embedder for Down {
        fn embed(&self, _t: &str) -> Result<Vec<f32>, uteke_core::Error> {
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
    let app = App {
        uteke: Mutex::new(open_with_embedder(Box::new(Down))),
        deploy_hash: String::new(),
    };
    let (status, v) = app.post("/skill/similar", json!({"text": "anything"}));
    assert_eq!(status, 503, "{v}");
    assert_eq!(
        v["error"],
        "embedding model not available for this skill request"
    );
    let (status, v) = app.post("/skill/search", json!({"query": "x", "mode": "semantic"}));
    assert_eq!(status, 503, "{v}");
    // Keywords and listings need no model.
    app.ok("/skill/search", json!({"query": "x", "mode": "fts"}));
    app.ok("/skill/list", json!({}));
}

#[test]
fn read_only_token_ok_anonymous_401_and_wrong_token_401() {
    let app = App::new();
    for (url, body) in [
        ("/skill/search", json!({"query": Q})),
        ("/skill/list", json!({})),
        ("/skill/show", json!({"slug": "deploy"})),
        ("/skill/similar", json!({"slug": "deploy-copy"})),
    ] {
        for token in ["ro-secret", "rw-secret"] {
            let (status, v) = app.call(
                &ctx(true),
                Method::Post,
                url,
                Some(body.clone()),
                Some(token),
            );
            assert_eq!(status, 200, "{url} with {token}: {v}");
        }
        for token in [None, Some("wrong")] {
            let (status, _) = app.call(&ctx(true), Method::Post, url, Some(body.clone()), token);
            assert_eq!(status, 401, "{url} with {token:?}");
        }
    }
}

#[test]
fn the_write_side_is_not_routed() {
    let app = App::new();
    let before = app.ok("/skill/show", json!({"slug": "deploy"}));
    for path in [
        "/skill/index",
        "/skill/approve",
        "/skill/record",
        "/skill/lint",
    ] {
        // A read-only token is refused at the gate, a full one finds no route.
        for (token, want) in [(Some("ro-secret"), 403), (Some("rw-secret"), 404)] {
            let (status, _) = app.call(
                &ctx(true),
                Method::Post,
                path,
                Some(json!({"dir": "/tmp", "slug": "deploy"})),
                token,
            );
            assert_eq!(status, want, "{path} with {token:?}");
        }
        assert!(crate::api_registry::find("POST", path).is_none(), "{path}");
    }
    // GET is not routed either.
    for path in [
        "/skill/search",
        "/skill/list",
        "/skill/show",
        "/skill/similar",
    ] {
        assert!(crate::api_registry::find("GET", path).is_none(), "{path}");
    }
    assert_eq!(app.ok("/skill/show", json!({"slug": "deploy"})), before);
}

#[test]
fn skill_files_do_not_leak_through_the_document_routes() {
    let app = App::new();
    app.ok(
        "/doc/create",
        json!({"slug": "notes", "content": format!("{Q} plain document")}),
    );
    // By slug and by id: not found.
    let id = {
        let u = app.uteke.lock().unwrap();
        u.skill_show("deploy").unwrap();
        // The id of the SKILL.md document, read through the store of the test.
        u.doc_list_page(None, 100, 0)
            .unwrap()
            .into_iter()
            .find(|d| d.slug == "skill/deploy/SKILL.md")
            .expect("indexed")
            .id
    };
    for key in ["skill/deploy/SKILL.md", id.as_str()] {
        let (status, _) = app.post("/doc/get", json!({"slug": key}));
        assert_eq!(status, 404, "{key}");
        let (status, _) = app.post("/doc/get", json!({"id": key}));
        assert_eq!(status, 404, "{key}");
        let (status, _) = app.post("/doc/update", json!({"slug": key, "content": "x"}));
        assert_eq!(status, 404, "{key} cannot be edited as a document");
    }
    // Search, list and similarity see the plain document only.
    for mode in ["fts", "semantic", "hybrid"] {
        let v = app.ok("/doc/search", json!({"query": Q, "mode": mode}));
        let text = v.to_string();
        assert!(text.contains("notes"), "{mode}: {text}");
        assert!(!text.contains("skill/"), "{mode}: {text}");
    }
    let text = app.ok("/doc/list", json!({})).to_string();
    assert!(text.contains("notes") && !text.contains("skill/"), "{text}");
    let text = app
        .ok("/doc/similar", json!({"slug": "notes", "threshold": 0.0}))
        .to_string();
    assert!(
        !text.contains("skill/") && !text.contains("deploy"),
        "{text}"
    );
    let text = app
        .ok("/doc/similar", json!({"report": true, "threshold": 0.0}))
        .to_string();
    assert!(!text.contains("skill/"), "{text}");
    // And the skill answers carry no document id of the skill files.
    let text = app.ok("/skill/show", json!({"slug": "deploy"})).to_string();
    assert!(!text.contains(&id), "{text}");
}

#[test]
fn the_routes_are_lab_rows_that_do_not_write() {
    for (path, request) in [
        ("/skill/search", "SkillSearchRequest"),
        ("/skill/list", "SkillListRequest"),
        ("/skill/show", "SkillShowRequest"),
        ("/skill/similar", "SkillSimilarRequest"),
    ] {
        let row = crate::api_registry::find("POST", path).expect("registered");
        assert_eq!(row.tier, crate::api_registry::Tier::Lab, "{path}");
        assert!(!row.writes, "{path}");
        assert_eq!(row.request_type, Some(request), "{path}");
        assert_eq!(row.issues, &["1731"], "{path}");
    }
}

#[test]
fn list_and_show_never_carry_the_server_path() {
    let app = App::new();
    for (url, body) in [
        ("/skill/list", json!({})),
        ("/skill/show", json!({"slug": "deploy"})),
    ] {
        let text = app.ok(url, body).to_string();
        assert!(!text.contains("source_path"), "{url}: {text}");
        assert!(!text.contains("/tmp"), "{url}: {text}");
    }
    let v = app.ok("/skill/show", json!({"slug": "deploy"}));
    let mut keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "approved_hash",
            "created_at",
            "current",
            "ledger",
            "owner",
            "slug",
            "updated_at",
            "usage",
            "versions"
        ]
    );
}

#[test]
fn embedding_routes_answer_429_when_every_slot_is_taken() {
    let app = App::new();
    let (_a, _b) = {
        let u = app.uteke.lock().unwrap();
        (
            u.remote_embed_permit().unwrap(),
            u.remote_embed_permit().unwrap(),
        )
    };
    let busy = "busy: 2 embedding requests are already running; retry in a moment";
    for (url, body) in [
        ("/skill/similar", json!({"text": "x"})),
        ("/skill/search", json!({"query": "deploy"})),
        (
            "/skill/search",
            json!({"query": "deploy", "mode": "semantic"}),
        ),
    ] {
        assert_eq!(app.refused(url, body), (429, busy.to_string()), "{url}");
    }
    // Oversized is still a 400, and what does not embed still answers.
    assert_eq!(
        app.refused("/skill/similar", json!({"text": "x".repeat(32_769)}))
            .0,
        400
    );
    assert_eq!(
        app.post("/skill/search", json!({"query": "deploy", "mode": "fts"}))
            .0,
        200
    );
    assert_eq!(
        app.post("/skill/similar", json!({"slug": "deploy-copy"})).0,
        200
    );
    assert_eq!(app.post("/skill/list", json!({})).0, 200);
}
