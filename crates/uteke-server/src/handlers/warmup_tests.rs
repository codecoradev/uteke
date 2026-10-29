//! The embedding of a recall or a remember happens before the store lock
//! (#1518). These tests drive `route` with an OpenAI-compatible endpoint that
//! answers after a fixed delay: with the old behaviour N concurrent requests
//! took N delays, now they overlap.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tiny_http::{Method, TestRequest};
use uteke_core::Uteke;

use super::route;
use crate::context::ReqCtx;

const DELAY: Duration = Duration::from_millis(150);
const DIMS: usize = 64;

/// A mock `/embeddings` endpoint: every request is answered after `DELAY`.
fn mock_embedding_endpoint(seen: Arc<Mutex<Vec<String>>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read the headers, then the body by Content-Length.
                loop {
                    let n = stream.read(&mut chunk).unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_lowercase();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let len = text[..end]
                            .lines()
                            .find_map(|l| l.strip_prefix("content-length:"))
                            .and_then(|v| v.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break;
                        }
                    }
                }
                seen.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&buf).into_owned());
                std::thread::sleep(DELAY);
                // A vector that depends on the text: identical vectors would be
                // stored as one memory by the duplicate check.
                let hash = buf.iter().fold(0xcbf29ce484222325u64, |h, b| {
                    (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
                });
                let vector: Vec<f32> = (0..DIMS)
                    .map(|i| {
                        if (hash >> (i % 64)) & 1 == 1 {
                            1.0
                        } else {
                            0.0
                        }
                    })
                    .collect();
                let body = serde_json::json!({
                    "data": [{"index": 0, "embedding": vector}],
                    "model": "m",
                })
                .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    format!("http://127.0.0.1:{port}")
}

struct App {
    /// Every request body the embedding endpoint received.
    seen: Arc<Mutex<Vec<String>>>,
    uteke: Mutex<Uteke>,
    ctx: ReqCtx,
    _dir: std::path::PathBuf,
}

impl App {
    fn new(name: &str) -> Self {
        Self::with_privacy(name, Default::default())
    }

    fn with_privacy(name: &str, privacy: uteke_core::privacy::PrivacyConfig) -> Self {
        let dir = std::env::temp_dir().join(format!("warmup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let settings = uteke_core::EmbeddingSettings {
            api_key: "k".into(),
            base_url: mock_embedding_endpoint(Arc::clone(&seen)),
            model: "m".into(),
            dims: DIMS,
            ..Default::default()
        };
        let mut uteke = Uteke::open_with_embedding_and_graph(
            &dir,
            "openai",
            settings,
            Default::default(),
            Default::default(),
            Default::default(),
            None,
        )
        .expect("open with an openai backend");
        uteke.set_privacy_config(privacy);
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
        Self {
            seen,
            uteke: Mutex::new(uteke),
            ctx,
            _dir: dir,
        }
    }

    fn post(&self, path: &str, body: serde_json::Value) -> (u16, serde_json::Value) {
        let body: &'static str = Box::leak(body.to_string().into_boxed_str());
        let mut req: tiny_http::Request = TestRequest::new()
            .with_method(Method::Post)
            .with_path(path)
            .with_body(body)
            .into();
        let resp = route(&self.uteke, &self.ctx, &mut req);
        let status = resp.status_code().0;
        let bytes = resp.into_reader().into_inner();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._dir);
    }
}

#[test]
fn concurrent_requests_overlap_their_embedding() {
    let app = App::new("overlap");
    // The first request builds the embedder and hands out the warm-up handle.
    let (st, first) = app.post("/remember", serde_json::json!({"content": "first memory"}));
    assert_eq!(st, 200, "{first}");
    assert!(
        app.ctx.embed_warmer.lock().unwrap().is_some(),
        "no warm-up handle"
    );

    let n = 6;
    let started = Instant::now();
    std::thread::scope(|s| {
        for i in 0..n {
            let app = &app;
            s.spawn(move || {
                let (st, body) = if i % 2 == 0 {
                    app.post(
                        "/remember",
                        serde_json::json!({"content": format!("concurrent memory {i}")}),
                    )
                } else {
                    app.post(
                        "/recall",
                        serde_json::json!({"query": format!("concurrent {i}")}),
                    )
                };
                assert_eq!(st, 200, "{body}");
            });
        }
    });
    let elapsed = started.elapsed();
    // Serialised, six requests take at least six delays (900 ms).
    assert!(
        elapsed < DELAY * (n as u32) * 6 / 10,
        "the requests were serialised: {elapsed:?}"
    );
}

#[test]
fn a_write_is_visible_to_the_next_read() {
    let app = App::new("ryw");
    let (st, _) = app.post(
        "/remember",
        serde_json::json!({"content": "warm up the embedder"}),
    );
    assert_eq!(st, 200);
    let (st, stored) = app.post(
        "/remember",
        serde_json::json!({"content": "the quokka is a small marsupial"}),
    );
    assert_eq!(st, 200, "{stored}");
    let (st, listed) = app.post("/list", serde_json::json!({}));
    assert_eq!(st, 200, "{listed}");
    assert!(listed.to_string().contains("quokka"), "{listed}");
    let id = stored["id"].as_str().unwrap();
    assert!(listed.to_string().contains(id), "{listed}");
}

#[test]
fn a_poisoned_store_lock_does_not_fail_later_requests() {
    let app = App::new("poison");
    std::thread::scope(|s| {
        let h = s.spawn(|| {
            let _guard = app.uteke.lock().unwrap();
            panic!("a handler panicked while holding the store lock");
        });
        assert!(h.join().is_err());
    });
    assert!(app.uteke.is_poisoned());
    let (st, body) = app.post(
        "/remember",
        serde_json::json!({"content": "after the panic"}),
    );
    assert_eq!(st, 200, "{body}");
}

fn secret() -> String {
    format!("{}{}", "ghp_", "aB3dE5gH7jK9mN1pQ3rS5tU7vW9xY1zA3bC5")
}

#[test]
fn a_secret_is_masked_before_the_warm_up_embeds_it() {
    let app = App::new("mask");
    app.post(
        "/remember",
        serde_json::json!({"content": "build the embedder"}),
    );
    let content = format!("the deploy token is {}", secret());
    let (st, body) = app.post("/remember", serde_json::json!({ "content": content }));
    assert_eq!(st, 200, "{body}");
    let seen = app.seen.lock().unwrap().join("\n");
    assert!(!seen.contains(&secret()), "the embedder received a secret");
    assert!(
        seen.contains("the deploy token is"),
        "the warm-up did not run"
    );
}

#[test]
fn strict_mode_sends_nothing_to_the_embedder() {
    let strict = uteke_core::privacy::PrivacyConfig {
        mode: uteke_core::privacy::PrivacyMode::Strict,
        ..Default::default()
    };
    let app = App::with_privacy("strict", strict);
    app.post(
        "/remember",
        serde_json::json!({"content": "build the embedder"}),
    );
    let content = format!("the deploy token is {}", secret());
    let (st, _) = app.post("/remember", serde_json::json!({ "content": content }));
    assert_eq!(st, 400, "strict mode refuses the write");
    let seen = app.seen.lock().unwrap().join("\n");
    assert!(!seen.contains(&secret()), "the embedder received a secret");
    assert!(
        !seen.contains("deploy token"),
        "a refused write was embedded"
    );
}
