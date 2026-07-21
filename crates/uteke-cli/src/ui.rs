//! `uteke ui` — a lightweight local web UI that streams live indexing progress
//! and renders the resulting knowledge graph.
//!
//! Design goals mirror the rest of this repo: single Rust binary, zero npm,
//! offline. The frontend is one self-contained HTML page (vanilla JS + a
//! canvas force-directed graph) embedded at compile time. A background thread
//! runs `index_tree_cb`, buffering [`IndexProgress`] events into shared state;
//! the browser polls `/api/progress` for the live chunking feed and
//! `/api/graph` for the node/edge data once indexing settles.

use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tiny_http::{Header, Response, Server};
use uteke_core::{IndexProgress, Uteke};

use crate::cli::Cli;
use crate::config::Config;

/// Embedded single-page frontend (vanilla JS canvas graph + live log).
const INDEX_HTML: &str = include_str!("ui/index.html");

/// One buffered progress line, shaped for JSON consumption by the browser.
#[derive(Clone, serde::Serialize)]
struct ProgressLine {
    /// Event kind: "discovered" | "file" | "skip" | "prune" | "done" | "error".
    kind: String,
    /// Human-readable message.
    msg: String,
    /// Optional file path (file/skip events).
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    /// 1-based position in the discovered set (file events).
    #[serde(skip_serializing_if = "Option::is_none")]
    index: Option<usize>,
    /// Total discovered files (discovered/file events).
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    /// Chunks stored for this file (file events).
    #[serde(skip_serializing_if = "Option::is_none")]
    chunks: Option<usize>,
}

/// Shared indexing state polled by the browser.
struct UiState {
    /// Append-only progress log; browser polls with `?since=N`.
    log: Mutex<Vec<ProgressLine>>,
    /// True once the background indexing thread has finished (success or error).
    done: AtomicBool,
    /// Running chunk total, surfaced in the header.
    chunks: AtomicUsize,
    /// Namespace being served.
    namespace: String,
    /// The shared store, reused for graph queries after indexing.
    uteke: Arc<Mutex<Uteke>>,
}

impl UiState {
    fn push(&self, line: ProgressLine) {
        if let Ok(mut log) = self.log.lock() {
            log.push(line);
        }
    }
}

/// Entry point for `uteke ui`. Opens the (project-scoped) store, kicks off a
/// background index, and serves the UI until Ctrl-C.
pub(crate) fn run(
    cli: &Cli,
    path: Option<&str>,
    port: u16,
    force: bool,
    no_index: bool,
    no_open: bool,
) -> Result<(), String> {
    let config = Config::load();

    // Resolve the repo root + store path the same way the main store-open path
    // does, so the UI serves the project store when run inside a repo.
    let root: PathBuf = match path {
        Some(p) => PathBuf::from(p),
        None => crate::config::find_project_root()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "cannot determine project root".to_string())?,
    };

    let store_path = cli
        .store
        .as_deref()
        .map(|s| s.to_string())
        .unwrap_or_else(|| Config::expand_tilde(&config.store.path));

    let namespace = crate::resolve_namespace(cli, &config);

    let uteke = Uteke::open_with_embedding_and_graph(
        &store_path,
        &config.embedding.backend,
        uteke_core::EmbeddingSettings {
            api_key: config.embedding.api_key.clone(),
            base_url: config.embedding.base_url.clone(),
            endpoint_path: config.embedding.endpoint_path.clone(),
            model: config.embedding.model.clone(),
            dims: config.embedding.dims,
        },
        uteke_core::TierConfig {
            hot_days: config.tier.hot_days as i64,
            warm_days: config.tier.warm_days as i64,
            hot_boost: config.tier.hot_boost,
        },
        uteke_core::RecallConfig {
            min_score: config.recall.min_score as f32,
        },
        uteke_core::GraphRerankConfig {
            density_weight: config.recall.graph_density_weight,
            authority_weight: config.recall.graph_authority_weight,
            enabled: config.recall.graph_rerank_enabled,
        },
    )
    .map_err(|e| format!("failed to open store: {e}"))?;

    let uteke = Arc::new(Mutex::new(uteke));
    let state = Arc::new(UiState {
        log: Mutex::new(Vec::new()),
        done: AtomicBool::new(false),
        chunks: AtomicUsize::new(0),
        namespace: namespace.clone(),
        uteke: Arc::clone(&uteke),
    });

    // Kick off indexing in the background unless suppressed.
    if no_index {
        state.push(ProgressLine {
            kind: "done".into(),
            msg: "Serving existing graph (no re-index)".into(),
            path: None,
            index: None,
            total: None,
            chunks: None,
        });
        state.done.store(true, Ordering::SeqCst);
    } else {
        spawn_indexer(Arc::clone(&state), Arc::clone(&uteke), root, namespace, force);
    }

    // Bind the HTTP server.
    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| format!("failed to bind {addr}: {e}"))?;
    let url = format!("http://{addr}");
    println!("uteke ui serving at {url}  (Ctrl-C to stop)");
    if !no_open {
        open_browser(&url);
    }

    for req in server.incoming_requests() {
        let resp = handle(&state, req.url());
        if let Err(e) = req.respond(resp) {
            tracing::warn!("ui respond failed: {e}");
        }
    }
    Ok(())
}

/// Spawn the background indexing thread. Streams [`IndexProgress`] into the
/// shared log so the browser can render live chunking progress.
fn spawn_indexer(
    state: Arc<UiState>,
    uteke: Arc<Mutex<Uteke>>,
    root: PathBuf,
    namespace: String,
    force: bool,
) {
    std::thread::spawn(move || {
        // Hold the lock for the whole index — the graph endpoint will contend
        // briefly, but indexing is the point of this screen.
        let guard = match uteke.lock() {
            Ok(g) => g,
            Err(_) => {
                state.push(err_line("store lock poisoned"));
                state.done.store(true, Ordering::SeqCst);
                return;
            }
        };

        let st = Arc::clone(&state);
        let result = guard.index_tree_cb(&namespace, &root, force, false, |ev| match ev {
            IndexProgress::Discovered { files } => st.push(ProgressLine {
                kind: "discovered".into(),
                msg: format!("Discovered {files} source file(s)"),
                path: None,
                index: None,
                total: Some(files),
                chunks: None,
            }),
            IndexProgress::FileStarted { .. } => {
                // The browser renders motion from FileIndexed/FileSkipped; a
                // started event per file would just double the log volume.
            }
            IndexProgress::FileIndexed { path, chunks } => {
                st.chunks.fetch_add(chunks, Ordering::SeqCst);
                st.push(ProgressLine {
                    kind: "file".into(),
                    msg: format!("{path} — {chunks} chunk(s)"),
                    path: Some(path.to_string()),
                    index: None,
                    total: None,
                    chunks: Some(chunks),
                });
            }
            IndexProgress::FileSkipped { path } => st.push(ProgressLine {
                kind: "skip".into(),
                msg: format!("{path} — unchanged"),
                path: Some(path.to_string()),
                index: None,
                total: None,
                chunks: None,
            }),
            IndexProgress::Pruned { files } => {
                if files > 0 {
                    st.push(ProgressLine {
                        kind: "prune".into(),
                        msg: format!("Pruned {files} deleted file(s)"),
                        path: None,
                        index: None,
                        total: None,
                        chunks: None,
                    });
                }
            }
        });

        match result {
            Ok(summary) => state.push(ProgressLine {
                kind: "done".into(),
                msg: format!(
                    "Indexed {} file(s), {} chunk(s); {} unchanged; {} pruned",
                    summary.indexed, summary.chunks, summary.skipped, summary.pruned
                ),
                path: None,
                index: None,
                total: None,
                chunks: Some(summary.chunks),
            }),
            Err(e) => state.push(err_line(&format!("index failed: {e}"))),
        }
        state.done.store(true, Ordering::SeqCst);
    });
}

fn err_line(msg: &str) -> ProgressLine {
    ProgressLine {
        kind: "error".into(),
        msg: msg.to_string(),
        path: None,
        index: None,
        total: None,
        chunks: None,
    }
}

/// Route a single request. Only three endpoints: the page, the progress feed,
/// and the graph data.
fn handle(state: &UiState, url: &str) -> Response<Cursor<Vec<u8>>> {
    let (path, query) = url.split_once('?').unwrap_or((url, ""));
    match path {
        "/" | "/index.html" => html_response(INDEX_HTML),
        "/api/progress" => progress_response(state, query),
        "/api/graph" => graph_response(state),
        _ => Response::from_string("not found").with_status_code(404),
    }
}

/// `GET /api/progress?since=N` → `{ lines: [...], done, chunks, next }`.
fn progress_response(state: &UiState, query: &str) -> Response<Cursor<Vec<u8>>> {
    let since = query
        .split('&')
        .find_map(|kv| kv.strip_prefix("since="))
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);

    let log = state.log.lock().expect("log lock");
    let total = log.len();
    let slice: Vec<ProgressLine> = if since < total {
        log[since..].to_vec()
    } else {
        Vec::new()
    };
    drop(log);

    let body = serde_json::json!({
        "lines": slice,
        "next": total,
        "done": state.done.load(Ordering::SeqCst),
        "chunks": state.chunks.load(Ordering::SeqCst),
        "namespace": state.namespace,
    });
    json_response(&body)
}

/// `GET /api/graph` → a graph synthesized from `code`-tagged memories.
///
/// Code chunks are stored as flat memories (not knowledge-graph nodes), so we
/// build a structural view here: one node per file, one per symbol, and a
/// `contains` edge from each file to its symbols. This is what "see the code
/// graph" means for an indexed repo — the existing entity graph
/// (`graph_data`) stays available for user-authored relationships.
fn graph_response(state: &UiState) -> Response<Cursor<Vec<u8>>> {
    let uteke = match state.uteke.lock() {
        Ok(g) => g,
        Err(_) => return json_error("store lock poisoned"),
    };
    match build_code_graph(&uteke, &state.namespace) {
        Ok(data) => json_response(&data),
        Err(e) => json_error(&format!("graph error: {e}")),
    }
}

/// A lightweight node/edge payload for the frontend canvas.
#[derive(serde::Serialize)]
struct UiNode {
    id: String,
    label: String,
    entity_type: String,
    properties: serde_json::Value,
}

#[derive(serde::Serialize)]
struct UiEdge {
    source_id: String,
    target_id: String,
    relation: String,
}

#[derive(serde::Serialize)]
struct UiGraph {
    nodes: Vec<UiNode>,
    edges: Vec<UiEdge>,
}

/// Walk all `code` memories in the namespace and fold them into a file/symbol
/// graph. Files become container nodes; each chunk becomes a symbol node with
/// a `contains` edge from its file.
fn build_code_graph(uteke: &Uteke, namespace: &str) -> Result<UiGraph, uteke_core::Error> {
    use std::collections::HashSet;

    let mut nodes: Vec<UiNode> = Vec::new();
    let mut edges: Vec<UiEdge> = Vec::new();
    let mut seen_files: HashSet<String> = HashSet::new();

    // Page through code memories so large repos don't allocate one giant Vec.
    let mut offset = 0usize;
    let page = 500usize;
    loop {
        let batch = uteke.list(Some("code"), page, offset, Some(namespace))?;
        if batch.is_empty() {
            break;
        }
        for m in &batch {
            let meta = &m.metadata;
            let file = meta
                .get("file")
                .and_then(|v| v.as_str())
                .unwrap_or("(unknown)");
            let sym_name = meta
                .get("symbol_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let sym_type = meta
                .get("symbol_type")
                .and_then(|v| v.as_str())
                .unwrap_or("chunk");
            let line_start = meta.get("line_start").and_then(|v| v.as_u64());
            let line_end = meta.get("line_end").and_then(|v| v.as_u64());

            let file_id = format!("file:{file}");
            if seen_files.insert(file_id.clone()) {
                let short = file.rsplit('/').next().unwrap_or(file);
                nodes.push(UiNode {
                    id: file_id.clone(),
                    label: short.to_string(),
                    entity_type: "file".into(),
                    properties: serde_json::json!({ "file": file }),
                });
            }

            // Symbol node keyed by memory id (stable, unique per chunk).
            let label = if sym_name.is_empty() {
                sym_type.to_string()
            } else {
                sym_name.to_string()
            };
            nodes.push(UiNode {
                id: m.id.clone(),
                label,
                entity_type: sym_type.to_string(),
                properties: serde_json::json!({
                    "file": file,
                    "symbol_type": sym_type,
                    "symbol_name": sym_name,
                    "line_start": line_start,
                    "line_end": line_end,
                }),
            });
            edges.push(UiEdge {
                source_id: file_id,
                target_id: m.id.clone(),
                relation: "contains".into(),
            });
        }
        if batch.len() < page {
            break;
        }
        offset += page;
    }

    Ok(UiGraph { nodes, edges })
}

fn html_response(body: &str) -> Response<Cursor<Vec<u8>>> {
    let header = Header::from_bytes(&b"Content-Type"[..], &b"text/html; charset=utf-8"[..])
        .expect("valid header");
    Response::from_string(body).with_header(header)
}

fn json_response<T: serde::Serialize>(body: &T) -> Response<Cursor<Vec<u8>>> {
    let s = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("valid header");
    Response::from_string(s).with_header(header)
}

fn json_error(msg: &str) -> Response<Cursor<Vec<u8>>> {
    let s = serde_json::json!({ "error": msg }).to_string();
    let header =
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).expect("valid header");
    Response::from_string(s)
        .with_status_code(500)
        .with_header(header)
}

/// Best-effort browser launch; failure is non-fatal (URL is already printed).
fn open_browser(url: &str) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let cmd = "";

    if cmd.is_empty() {
        return;
    }
    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
