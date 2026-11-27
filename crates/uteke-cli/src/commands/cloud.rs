//! Cloud mode (#1791): route memory/room/doc commands to a remote endpoint.
//!
//! `UTEKE_BASE_URL` set (with `UTEKE_CLOUD_TOKEN`) makes the CLI talk the same
//! wire the `uteke-serve` API serves — the CORE subset uteke-cloud guarantees
//! (`kontrak-core-cloud/02-daftar-core`) — instead of opening the local store.
//! The endpoint speaks the same shapes (the contract pins them), so the local
//! server client in [`super::server`] and this module share its helpers.
//!
//! Precedence: `--store` stays local-only — when it is passed, cloud mode is
//! off for that invocation (explicit user intent, not a silent fallback).
//! Env unset → behavior unchanged. Env set but the token missing → warn and
//! use the local store; `UTEKE_STRICT=1` turns that into a hard error.

use crate::cli::{Cli, Commands, DocCommands, RoomCommands};
use crate::output;
use std::time::Duration;

/// Where the remote endpoint lives. When set (and `--store` is not), memory,
/// room and document commands target it instead of the local store.
pub(crate) const BASE_URL_ENV: &str = "UTEKE_BASE_URL";
/// Bearer token for the remote endpoint. The cloud REST API already accepts
/// it (#1791, verified live 2026-11-17).
pub(crate) const TOKEN_ENV: &str = "UTEKE_CLOUD_TOKEN";
/// When `1`, an incomplete cloud setup (`UTEKE_BASE_URL` without a token)
/// fails the command instead of warning and falling back to the local store.
pub(crate) const STRICT_ENV: &str = "UTEKE_STRICT";

/// The resolved cloud target, or `None` when cloud mode is off.
pub(crate) enum CloudDecision {
    /// Route to this endpoint.
    Cloud(CloudTarget),
    /// Run locally, unchanged (env unset, or `--store` passed).
    Local,
    /// Incomplete setup: run locally, print this warning first. Only with
    /// strict mode off.
    LocalWithWarning(String),
    /// Incomplete setup under strict mode: fail with this message.
    StrictError(String),
}

pub(crate) struct CloudTarget {
    pub(crate) base_url: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Decide the target for this invocation. `store_override` is the explicit
/// `--store` value, which keeps the run local (#1791: `--store` stays
/// local-only).
pub(crate) fn resolve(store_override: Option<&str>) -> CloudDecision {
    let Some(base_url) = env(BASE_URL_ENV) else {
        return CloudDecision::Local;
    };
    if store_override.is_some() {
        return CloudDecision::Local;
    }
    // One trailing slash would double up in `{base}/{path}` joins.
    let base_url = base_url.trim_end_matches('/').to_string();
    if env(TOKEN_ENV).is_none() {
        let msg = format!(
            "{TOKEN_ENV} is not set: the command would run against {base_url} without \
             credentials. Set the token, or unset {BASE_URL_ENV} for the local store."
        );
        let strict = env(STRICT_ENV).is_some_and(|v| v == "1");
        return if strict {
            CloudDecision::StrictError(msg)
        } else {
            CloudDecision::LocalWithWarning(format!(
                "Warning: {msg} Falling back to the local store."
            ))
        };
    }
    CloudDecision::Cloud(CloudTarget { base_url })
}

/// Commands this module can route. Everything else is refused in cloud mode:
/// silently running it against the local store would split the data between
/// two databases (#1439 lesson, applied to the cloud target).
fn routed(command: &Commands) -> bool {
    matches!(
        command,
        Commands::Remember { .. }
            | Commands::Recall { .. }
            | Commands::Search { .. }
            | Commands::List { .. }
            | Commands::Get { .. }
            | Commands::Stats
            | Commands::Forget { .. }
            | Commands::Room { .. }
            | Commands::Doc { .. }
    )
}

/// Short name for error messages.
fn command_name(command: &Commands) -> &'static str {
    match command {
        Commands::Remember { .. } => "remember",
        Commands::Recall { .. } => "recall",
        Commands::Search { .. } => "search",
        Commands::List { .. } => "list",
        Commands::Get { .. } => "get",
        Commands::Stats => "stats",
        Commands::Forget { .. } => "forget",
        Commands::Room { .. } => "room",
        Commands::Doc { .. } => "doc",
        _ => "this command",
    }
}

/// The HTTP client for the remote endpoint: bearer token on every request,
/// a bounded wait so an unreachable remote fails instead of hanging.
///
/// A token that cannot be a header value (a trailing newline from a secrets
/// file, for example) is an error, not a silent unauthenticated run: an
/// unauthenticated request would fail remotely with a confusing 401 (or, on a
/// misconfigured server, write into someone else's workspace). The timeouts
/// are part of the contract with the caller — a build that loses them would
/// hang on an unreachable remote — so a client build failure errors too.
fn build_client() -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10));
    if let Some(token) = env(TOKEN_ENV) {
        let value =
            reqwest::header::HeaderValue::from_str(&format!("Bearer {token}")).map_err(|_| {
                format!(
                    "{TOKEN_ENV} cannot be used as an Authorization header value (control \
                    characters or whitespace outside the header alphabet?). Trim it or \
                    re-export it."
                )
            })?;
        let mut value = value;
        value.set_sensitive(true);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }
    builder
        .build()
        .map_err(|e| format!("Failed to build the HTTP client for the remote: {e}"))
}

fn parse_json_value(resp: reqwest::blocking::Response) -> Result<serde_json::Value, String> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!("Cloud returned {status}: {body}"));
    }
    resp.json().map_err(|e| format!("Parse error: {e}"))
}

fn parse_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::blocking::Response,
) -> Result<T, String> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(format!("Cloud returned {status}: {body}"));
    }
    resp.json().map_err(|e| format!("Parse error: {e}"))
}

fn strip_deprecation(data: &mut serde_json::Value) {
    if let Some(w) = data["warning"].as_str() {
        data["warning"] = match uteke_core::namespace_deprecation::strip_deprecation(w) {
            Some(rest) => serde_json::Value::String(rest),
            None => serde_json::Value::Null,
        };
    }
}

/// Route the command to the cloud endpoint. Only [`routed`] commands reach
/// this; every other variant is refused by [`run`]'s guard.
pub(crate) fn run(cli: &Cli, target: &CloudTarget) -> Result<(), String> {
    let command = &cli.command;
    if !routed(command) {
        return Err(format!(
            "'{}' is not supported in cloud mode ({} set): the remote API has no wire for it. \
             Unset {} to run it against the local store.",
            command_name(command),
            BASE_URL_ENV,
            BASE_URL_ENV,
        ));
    }
    let base_url = &target.base_url;
    let client = build_client()?;
    let ns = cli.namespace.as_deref().unwrap_or("default");

    match command {
        Commands::Remember {
            content,
            tags,
            r#type,
            entity,
            category,
            meta,
            room,
            author,
            timestamp,
            ..
        } => {
            // The timestamp anchor is a prefix of the content (#1232), applied
            // before the request exactly as on a local store.
            let content = match timestamp {
                Some(ts) => super::anchor::apply_timestamp_anchor(content, ts)?,
                None => content.clone(),
            };
            let resp = if let Some(room_id) = room {
                // A room write is `POST /room/remember` on the wire; the room
                // is created when missing.
                client
                    .post(format!("{base_url}/room/remember"))
                    .json(&serde_json::json!({
                        "room_id": room_id,
                        "content": content,
                        "tags": tags,
                        "type": r#type,
                        "author": author,
                        "metadata": uteke_core::metadata_from_cli_flags(
                            entity.as_deref(),
                            category.as_deref(),
                            meta,
                        ),
                        "namespace": ns,
                    }))
                    .send()
                    .map_err(|e| format!("Cloud error: {e}"))?
            } else {
                // `POST /remember` takes no `room` (#1439); the plain write
                // carries the typed request shared with HTTP and MCP (#1343).
                let body = uteke_core::RememberInput {
                    content,
                    tags: tags.clone(),
                    namespace: Some(ns.to_string()),
                    memory_type: r#type.clone(),
                    metadata: uteke_core::metadata_from_cli_flags(
                        entity.as_deref(),
                        category.as_deref(),
                        meta,
                    ),
                    author: author.clone(),
                    ..uteke_core::RememberInput::default()
                }
                .to_http_body();
                client
                    .post(format!("{base_url}/remember"))
                    .json(&body)
                    .send()
                    .map_err(|e| format!("Cloud error: {e}"))?
            };
            let mut data = parse_json_value(resp)?;
            strip_deprecation(&mut data);
            if cli.json {
                println!("{data}");
            } else if let Some(room_id) = room {
                println!(
                    "\u{2713} Memory stored in room '{room_id}'\n  ID: {}",
                    data["id"]
                );
            } else {
                println!("\u{2713} Memory stored\n  ID: {}", data["id"]);
                if data["embedding_written"] == serde_json::Value::Bool(false) {
                    println!(
                        "  \u{26a0} Embedding NOT written by the remote; it is keyword-searchable only."
                    );
                }
            }
        }
        Commands::Recall { .. } => {
            let args = super::recall::RecallArgs::from_command(command)
                .expect("a recall command carries recall flags");
            // The wire has no field for some flags; dropping them silently
            // would return a differently-ranked result than local (#1332).
            let unsupported = super::server::recall_flags_unsupported_by_server(&args);
            if !unsupported.is_empty() {
                return Err(format!(
                    "recall flag(s) {} have no wire field on the remote API; unset {} for the local store",
                    unsupported.join(", "),
                    BASE_URL_ENV,
                ));
            }
            args.check_rendering()?;
            let input = args.input(Some(ns));
            let resp = client
                .post(format!("{base_url}/recall"))
                .json(&input.to_http_body())
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let data = parse_json_value(resp)?;
            let output = super::server::recall_output_from_server(data, &input)?;
            let min_score = match &output {
                uteke_core::recall_exec::RecallOutput::Memories { min_score, .. } => *min_score,
                _ => args.min.unwrap_or(0.0),
            };
            super::recall::render(cli, &args, output, min_score);
        }
        Commands::Search {
            query, limit, tags, ..
        } => {
            let resp = client
                .post(format!("{base_url}/search"))
                .json(&serde_json::json!({
                    "query": query,
                    "limit": limit,
                    "tags": tags,
                    "namespace": ns,
                }))
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let results = parse_response::<Vec<uteke_core::SearchResult>>(resp)?;
            if cli.json {
                output::print_json(&results);
            } else {
                output::print_search_human(&results);
            }
        }
        Commands::List {
            entity: Some(_), ..
        }
        | Commands::List {
            category: Some(_), ..
        } => {
            return Err(format!(
                "--entity/--category have no wire field on the remote list; unset {} for the local store",
                BASE_URL_ENV,
            ));
        }
        Commands::List {
            tag,
            limit,
            offset,
            at,
            ..
        } => {
            let body = uteke_core::ListInput {
                tag: tag.clone(),
                limit: Some(*limit),
                offset: Some(*offset),
                namespace: Some(ns.to_string()),
                at: at.clone(),
                ..uteke_core::ListInput::default()
            }
            .to_http_body();
            let resp = client
                .post(format!("{base_url}/list"))
                .json(&body)
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let memories = parse_response::<Vec<uteke_core::Memory>>(resp)?;
            if cli.json {
                output::print_json(&memories);
            } else {
                output::print_list_human(&memories);
            }
        }
        Commands::Get { id } => {
            // GET /memory takes a full UUID on the wire; a prefix gets a 400.
            let resp = client
                .get(format!("{base_url}/memory"))
                .query(&[("id", id)])
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let memory = parse_response::<uteke_core::Memory>(resp)?;
            if cli.json {
                output::print_json(&memory);
            } else {
                output::print_get_human(&memory);
            }
        }
        Commands::Stats => {
            let resp = client
                .post(format!("{base_url}/stats"))
                .json(&serde_json::json!({ "namespace": ns }))
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let stats = parse_response::<uteke_core::StoreStats>(resp)?;
            if cli.json {
                output::print_json(&stats);
            } else {
                output::print_stats_human(&stats);
            }
        }
        Commands::Forget {
            id,
            tag,
            cold,
            all,
            confirm,
            reason,
        } => {
            if id.is_none() && tag.is_none() && (*cold || *all) {
                return Err(format!(
                    "--cold and --all are local operations; unset {} for the local store",
                    BASE_URL_ENV,
                ));
            }
            if let Some(id) = id
                && !super::forget::confirm_single(id, *confirm)
            {
                println!("Cancelled.");
                return Ok(());
            }
            let input = uteke_core::ForgetInput {
                id: id.clone(),
                tag: tag.clone(),
                namespace: Some(ns.to_string()),
                reason: reason.clone(),
            };
            let Some(path) = input.to_http_path(|v| urlencoding::encode(v).into_owned()) else {
                return Err("Provide an ID or --tag".into());
            };
            let resp = client
                .delete(format!("{base_url}{path}"))
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let mut data = parse_json_value(resp)?;
            strip_deprecation(&mut data);
            if cli.json {
                println!("{data}");
            } else if let Some(id) = id {
                println!("\u{2713} Memory forgotten: {id}");
            } else if let Some(tag) = tag {
                println!(
                    "\u{2713} Deleted {} memories with tag '{tag}'",
                    data["deleted"]
                );
            }
        }
        Commands::Room { command } => run_room(cli, &client, base_url, ns, command)?,
        Commands::Doc { command } => run_doc(cli, &client, base_url, command)?,
        _ => unreachable!("routed() gates every variant handled above"),
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn run_room(
    cli: &Cli,
    client: &reqwest::blocking::Client,
    base_url: &str,
    ns: &str,
    command: &RoomCommands,
) -> Result<(), String> {
    let post = |path: &str, body: serde_json::Value| -> Result<serde_json::Value, String> {
        let resp = client
            .post(format!("{base_url}{path}"))
            .json(&body)
            .send()
            .map_err(|e| format!("Cloud error: {e}"))?;
        parse_json_value(resp)
    };
    match command {
        RoomCommands::Create { room_id, title } => {
            let data = post(
                "/room/create",
                serde_json::json!({ "room_id": room_id, "title": title, "namespace": ns }),
            )?;
            let warning = data["warnings"][0].as_str().map(str::to_string);
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"created": room_id, "namespace": ns})
                );
            } else {
                println!("\u{2713} Room '{room_id}' created");
            }
            if let Some(w) = warning {
                eprintln!("Warning: {w}");
            }
        }
        RoomCommands::Rename { old_id, new_id } => {
            let data = post(
                "/room/rename",
                serde_json::json!({ "from": old_id, "to": new_id }),
            )?;
            let warning = data["warnings"][0].as_str().map(str::to_string);
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({"renamed": old_id, "to": new_id, "room": data["room"]})
                );
            } else {
                println!(
                    "\u{2713} Room '{old_id}' renamed to '{new_id}' ('{old_id}' still works as an alias)"
                );
            }
            if let Some(w) = warning {
                eprintln!("Warning: {w}");
            }
        }
        RoomCommands::Update {
            room_id,
            title,
            description,
        } => {
            let data = post(
                "/room/update",
                serde_json::json!({ "room_id": room_id, "title": title, "description": description }),
            )?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&data["room"]).unwrap());
            } else {
                println!("\u{2713} Room '{room_id}' updated");
            }
        }
        RoomCommands::MoveMemory {
            memory_id,
            from_room,
            to_room,
        } => {
            // A 404 means no link in the source room; surface it like local.
            let data = post(
                "/room/memory/move",
                serde_json::json!({
                    "memory_id": memory_id,
                    "from_room": from_room,
                    "to_room": to_room,
                }),
            )?;
            if data["moved"].is_null() && data["error"].is_null() {
                return Err(format!(
                    "Memory '{memory_id}' has no link in room '{from_room}'"
                ));
            }
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "moved": memory_id,
                        "from_room": from_room,
                        "to_room": to_room,
                    })
                );
            } else {
                println!("\u{2713} Memory '{memory_id}' moved from '{from_room}' to '{to_room}'");
            }
        }
        RoomCommands::Link {
            memory_id,
            room,
            author,
        } => {
            let data = post(
                "/room/memory",
                serde_json::json!({
                    "memory_id": memory_id,
                    "room": room,
                    "author": author,
                }),
            )?;
            let id = data["memory_id"].as_str().unwrap_or(memory_id);
            let linked = data["linked"].as_bool().unwrap_or(false);
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "memory_id": id,
                        "room": room,
                        "linked": linked,
                        "already_linked": !linked,
                    })
                );
            } else if linked {
                println!("\u{2713} Memory '{id}' added to room '{room}'");
            } else {
                println!("\u{2713} Memory '{id}' was already in room '{room}'");
            }
        }
        RoomCommands::List { namespace } => {
            let resp = client
                .get(format!("{base_url}/room/list"))
                .query(&[("namespace", namespace)])
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let rooms = parse_response::<Vec<uteke_core::Room>>(resp)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&rooms).unwrap());
            } else if rooms.is_empty() {
                println!("No rooms found.");
            } else {
                println!("Found {} room(s):\n", rooms.len());
                for room in &rooms {
                    let title = room.title.as_deref().unwrap_or("(untitled)");
                    println!("  {}  {}", room.id, title);
                    println!(
                        "    namespace: {}  created: {}",
                        room.namespace,
                        room.created_at.get(..19).unwrap_or(&room.created_at)
                    );
                }
            }
        }
        RoomCommands::Stats { room_id } => {
            let resp = client
                .post(format!("{base_url}/room/stats"))
                .json(&serde_json::json!({ "room_id": room_id }))
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let stats = parse_response::<uteke_core::RoomStats>(resp)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&stats).unwrap());
            } else {
                println!("Room: {}", stats.room_id);
                if let Some(title) = &stats.title {
                    println!("  Title: {title}");
                }
                println!("  Memories: {}", stats.memory_count);
                println!("  Participants: {}", stats.participant_count);
                if !stats.participants.is_empty() {
                    println!("    {}", stats.participants.join(", "));
                }
                println!(
                    "  Created: {}",
                    stats.created_at.get(..19).unwrap_or(&stats.created_at)
                );
                if let Some(last) = &stats.last_activity {
                    println!("  Last activity: {}", last.get(..19).unwrap_or(last));
                }
            }
        }
        RoomCommands::Search {
            query,
            limit,
            namespace,
        } => {
            let hits = post(
                "/room/search",
                serde_json::json!({ "query": query, "limit": limit, "namespace": namespace }),
            )?;
            let hits: Vec<uteke_core::RoomSearchHit> =
                serde_json::from_value(hits).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&hits);
            } else {
                output::print_room_search_human(&hits);
            }
        }
        RoomCommands::Recall {
            room_id,
            query,
            query_flag,
            author,
            limit,
            min,
        } => {
            let effective_query = query.as_ref().or(query_flag.as_ref());
            let mut body = serde_json::json!({
                "room_id": room_id,
                "author": author,
                "limit": limit,
            });
            if let Some(q) = effective_query {
                body["query"] = serde_json::json!(q);
                if let Some(m) = min {
                    body["min_score"] = serde_json::json!(m);
                }
            }
            let data = post("/room/recall", body)?;
            if effective_query.is_some() {
                let results: Vec<uteke_core::SearchResult> =
                    serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
                if cli.json {
                    output::print_json(&results);
                } else if results.is_empty() {
                    println!("No matching memories found in room {room_id}.");
                } else {
                    output::print_room_semantic_human(room_id, &results);
                }
            } else {
                let memories: Vec<uteke_core::Memory> =
                    serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&memories).unwrap());
                } else if memories.is_empty() {
                    println!("No memories found in room {room_id}.");
                } else {
                    println!(
                        "Found {} memory/memories in room {room_id}:\n",
                        memories.len()
                    );
                    for (i, m) in memories.iter().enumerate() {
                        let preview = uteke_core::safe_truncate(&m.content, 80);
                        let tags = if m.tags.is_empty() {
                            String::new()
                        } else {
                            format!(" [{}]", m.tags.join(", "))
                        };
                        println!(
                            "  {}. {} (ns: {}){}\n     ID: {}",
                            i + 1,
                            preview,
                            m.namespace,
                            tags,
                            &m.id[..8.min(m.id.len())],
                        );
                    }
                }
            }
        }
        RoomCommands::Delete { room_id, confirm } => {
            if !confirm {
                eprintln!("This will delete room {room_id} and all memory links.");
                eprintln!("Memories themselves are NOT deleted. Use --confirm to proceed.");
                return Err("Operation not confirmed".to_string());
            }
            let resp = client
                .delete(format!("{base_url}/room/delete"))
                .query(&[("room_id", room_id)])
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let data = parse_json_value(resp)?;
            let unlinked = data["unlinked_memories"].as_u64().unwrap_or(0);
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "deleted": room_id,
                        "unlinked_memories": unlinked,
                    })
                );
            } else {
                println!(
                    "Room {room_id} unregistered. {unlinked} memory link(s) removed; the memories are retained. Delete a memory with `uteke forget <id>`."
                );
            }
        }
        RoomCommands::Summary { room_id } => {
            let data = post("/room/summary", serde_json::json!({ "room_id": room_id }))?;
            if data["room_id"].is_null() {
                return Err(format!("Room not found: {room_id}"));
            }
            let summary: uteke_core::RoomSummary =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&summary).unwrap());
            } else {
                println!("Room: {}", summary.room_id);
                if let Some(title) = &summary.title {
                    println!("  Title: {title}");
                }
                println!("  Memories: {}", summary.total_memories);
                println!("  Participants: {}", summary.participants.join(", "));
                println!(
                    "  Time: {} → {}",
                    summary.time_range.earliest, summary.time_range.latest
                );
            }
        }
        RoomCommands::Document { room_id } => {
            let data = post(
                "/room/summary-document",
                serde_json::json!({ "room_id": room_id }),
            )?;
            if data["room_id"].is_null() {
                return Err(format!("Room not found: {room_id}"));
            }
            let doc: uteke_core::RoomDocument =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&doc).unwrap());
            } else {
                output::print_room_document_human(&doc);
            }
        }
        RoomCommands::AddDocument { room_id, doc_slug } => {
            let resp = client
                .put(format!("{base_url}/room/document/add"))
                .json(&serde_json::json!({ "room_id": room_id, "doc_slug": doc_slug }))
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            parse_json_value(resp)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "room_id": room_id,
                        "doc_slug": doc_slug,
                        "linked": true,
                    })
                );
            } else {
                println!("Linked document '{doc_slug}' to room '{room_id}'.");
            }
        }
        RoomCommands::RemoveDocument { room_id, doc_slug } => {
            let resp = client
                .delete(format!("{base_url}/room/document/remove"))
                .json(&serde_json::json!({ "room_id": room_id, "doc_slug": doc_slug }))
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            parse_json_value(resp)?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "room_id": room_id,
                        "doc_slug": doc_slug,
                        "linked": false,
                    })
                );
            } else {
                println!("Unlinked document '{doc_slug}' from room '{room_id}'.");
            }
        }
        RoomCommands::ListDocuments { room_id } => {
            let data = post(
                "/room/document/list",
                serde_json::json!({ "room_id": room_id }),
            )?;
            let docs: Vec<String> = serde_json::from_value(data["doc_slugs"].clone())
                .map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "room_id": room_id, "documents": docs })
                );
            } else if docs.is_empty() {
                println!("No documents linked to room '{room_id}'.");
            } else {
                println!("Documents linked to room '{room_id}':");
                for slug in &docs {
                    println!("  \u{2022} {slug}");
                }
            }
        }
        RoomCommands::ListRooms { doc_slug } => {
            let data = post(
                "/doc/room/list",
                serde_json::json!({ "doc_slug": doc_slug }),
            )?;
            let rooms: Vec<String> = serde_json::from_value(data["room_ids"].clone())
                .map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "doc_slug": doc_slug, "rooms": rooms })
                );
            } else if rooms.is_empty() {
                println!("No rooms reference document '{doc_slug}'.");
            } else {
                println!("Rooms referencing document '{doc_slug}':");
                for room_id in &rooms {
                    println!("  \u{2022} {room_id}");
                }
            }
        }
        RoomCommands::Consolidate { .. } => {
            return Err(format!(
                "'room consolidate' is not supported in cloud mode ({} set); unset {} for the local store",
                BASE_URL_ENV, BASE_URL_ENV,
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn run_doc(
    cli: &Cli,
    client: &reqwest::blocking::Client,
    base_url: &str,
    command: &DocCommands,
) -> Result<(), String> {
    let post = |path: &str, body: serde_json::Value| -> Result<serde_json::Value, String> {
        let resp = client
            .post(format!("{base_url}{path}"))
            .json(&body)
            .send()
            .map_err(|e| format!("Cloud error: {e}"))?;
        parse_json_value(resp)
    };
    match command {
        DocCommands::Create {
            slug,
            title,
            file,
            content,
            tags,
            parent,
        } => {
            let doc_content = if let Some(c) = content {
                c.clone()
            } else if let Some(f) = file {
                if f == "-" {
                    use std::io::Read;
                    let mut buf = String::new();
                    std::io::stdin()
                        .read_to_string(&mut buf)
                        .map_err(|e| format!("Failed to read stdin: {e}"))?;
                    buf
                } else {
                    std::fs::read_to_string(f).map_err(|e| format!("Failed to read file: {e}"))?
                }
            } else {
                return Err("Provide --content <text> or --file <path>".into());
            };
            let doc_title = title.clone().unwrap_or_else(|| {
                doc_content
                    .lines()
                    .find(|l| l.starts_with("# "))
                    .map(|l| l.trim_start_matches("# ").to_string())
                    .unwrap_or_else(|| slug.replace('-', " "))
            });
            let data = post(
                "/doc/create",
                serde_json::json!({
                    "slug": slug,
                    "title": doc_title,
                    "content": doc_content,
                    "tags": tags,
                    "parent": parent,
                }),
            )?;
            let id = data["id"].as_str().unwrap_or_default().to_string();
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "id": id,
                        "slug": slug,
                        "parent": parent,
                        "status": "created",
                    })
                );
            } else {
                println!("\u{2713} Document '{slug}' created (id: {id})");
                println!("  Title: {doc_title}");
                println!("  Size:  {} chars", doc_content.len());
                if let Some(p) = parent {
                    println!("  Parent: {p}");
                }
            }
        }
        DocCommands::Get { id_or_slug } => {
            // The wire takes `id` or `slug` (id wins); a CLI argument that is
            // not a UUID travels as the slug. `uuid` is already in the tree via
            // uteke-core, but the CLI does not depend on it: a full UUID is
            // 36 characters with four dashes in the right places, which a slug
            // never is (slugs are lowercase alphanumeric plus dashes).
            let field_name = if looks_like_uuid(id_or_slug) {
                "id"
            } else {
                "slug"
            };
            let data = post("/doc/get", serde_json::json!({ field_name: id_or_slug }))?;
            if data.get("slug").is_none() {
                return Err(format!("Document '{id_or_slug}' not found"));
            }
            let doc: uteke_core::Document =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&doc);
            } else {
                println!("Title: {} (depth: {})", doc.title, doc.depth);
                println!("Slug:  {}", doc.slug);
                println!(
                    "v{} | {} | {}",
                    doc.version, doc.content_type, doc.updated_at
                );
                if let Some(ref pid) = doc.parent_id {
                    println!("Parent: {pid}");
                }
                if !doc.tags.is_empty() {
                    println!("Tags:  {}", doc.tags.join(", "));
                }
                println!();
                println!("{}", doc.content);
            }
        }
        DocCommands::List {
            limit,
            tree,
            namespace,
        } => {
            if *tree {
                return Err(format!(
                    "--tree needs per-document children walks; unset {} for the local store",
                    BASE_URL_ENV,
                ));
            }
            let data = post(
                "/doc/list",
                serde_json::json!({
                    "namespace": namespace.clone().or_else(|| cli.namespace.clone()),
                    "limit": limit
                }),
            )?;
            let docs: Vec<uteke_core::DocumentSummary> =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No documents found.");
            } else {
                println!("Documents");
                println!("─────────────────────────────────────────────────────");
                for d in &docs {
                    let depth_indicator = if d.depth > 0 {
                        format!("  {}", "\u{203a}".repeat(d.depth as usize))
                    } else {
                        String::new()
                    };
                    println!(
                        "  {:<20} {:<30} v{}  {}{}",
                        output::char_prefix(&d.slug, 20),
                        output::char_prefix(&d.title, 30),
                        d.version,
                        output::char_prefix(&d.updated_at, 10),
                        depth_indicator
                    );
                }
                println!();
                println!("{} document(s)", docs.len());
            }
        }
        DocCommands::Children { parent, limit } => {
            let data = post(
                "/doc/list",
                serde_json::json!({ "parent": parent, "limit": limit }),
            )?;
            let docs: Vec<uteke_core::DocumentSummary> =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&docs);
            } else if docs.is_empty() {
                println!("No children for '{parent}'.");
            } else {
                println!("Children of '{parent}'");
                println!("─────────────────────────────────────────────────────");
                for d in &docs {
                    println!(
                        "  {:<20} {:<30} v{}  {}",
                        output::char_prefix(&d.slug, 20),
                        output::char_prefix(&d.title, 30),
                        d.version,
                        output::char_prefix(&d.updated_at, 10)
                    );
                }
                println!();
                println!("{} child(ren)", docs.len());
            }
        }
        DocCommands::Search { query, limit, mode } => {
            let data = post(
                "/doc/search",
                serde_json::json!({ "query": query, "limit": limit, "mode": mode }),
            )?;
            let results: Vec<uteke_core::DocumentSearchResult> =
                serde_json::from_value(data).map_err(|e| format!("Parse error: {e}"))?;
            if cli.json {
                output::print_json(&results);
            } else if results.is_empty() {
                println!("No documents found for '{query}'.");
            } else {
                println!("Search results for '{query}' (mode: {mode})");
                println!("─────────────────────────────────────────────────────");
                for r in &results {
                    let depth_indicator = if r.document.depth > 0 {
                        format!("  {}", "\u{203a}".repeat(r.document.depth as usize))
                    } else {
                        String::new()
                    };
                    println!(
                        "  {:<20} {:<30} {:.3} {}",
                        output::char_prefix(&r.document.slug, 20),
                        output::char_prefix(&r.document.title, 30),
                        r.score,
                        depth_indicator
                    );
                    if !r.chunk_heading.is_empty() {
                        println!("    \u{21b3} {}", output::char_prefix(&r.chunk_heading, 60));
                    }
                    if !r.chunk_snippet.is_empty() {
                        let snippet = output::char_prefix(&r.chunk_snippet, 80);
                        println!("    \"{}\"", snippet);
                    }
                }
                println!();
                println!("{} result(s)", results.len());
            }
        }
        DocCommands::Delete { id } => {
            let field_name = if looks_like_uuid(id) { "id" } else { "slug" };
            let resp = client
                .delete(format!("{base_url}/doc/delete"))
                .query(&[(field_name, id)])
                .send()
                .map_err(|e| format!("Cloud error: {e}"))?;
            let data = parse_json_value(resp)?;
            let deleted = data["deleted"].as_bool().unwrap_or(false);
            let subtree_size = data["subtree_size"].as_u64().unwrap_or(1);
            if !deleted {
                return Err(format!("Document not found: {id}"));
            }
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({ "deleted": id, "subtree_size": subtree_size })
                );
            } else {
                println!("\u{2713} Document deleted: {id}");
                if subtree_size > 1 {
                    println!("  {subtree_size} document(s) removed (cascade)");
                }
            }
        }
        _ => {
            return Err(format!(
                "'doc {}' is not supported in cloud mode ({} set): the remote API has no wire for it. Unset {} for the local store.",
                command_name_doc(command),
                BASE_URL_ENV,
                BASE_URL_ENV,
            ));
        }
    }
    Ok(())
}

/// A full UUID: 36 characters, four dashes at 8/13/18/23, hex elsewhere.
/// Good enough to pick the `id` field over `slug` on `/doc/get` and
/// `/doc/delete`; a slug never carries this shape.
fn looks_like_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(i, b)| {
            if matches!(i, 8 | 13 | 18 | 23) {
                *b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}

fn command_name_doc(command: &DocCommands) -> &'static str {
    match command {
        DocCommands::Create { .. } => "create",
        DocCommands::Get { .. } => "get",
        DocCommands::List { .. } => "list",
        DocCommands::Children { .. } => "children",
        DocCommands::Search { .. } => "search",
        DocCommands::Delete { .. } => "delete",
        _ => "operation",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // All cases mutate process-global env vars, so they run sequentially
    // inside ONE #[test] (same lesson as the ort_init / resolve_store tests).
    #[allow(unsafe_code)] // env var mutation (edition 2024) in one serialized test
    #[test]
    fn resolve_precedence_and_fallbacks() {
        static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _g = ENV.lock().unwrap();
        // SAFETY: serialized by ENV; no other thread of this crate reads
        // these variables while the test runs.
        unsafe {
            std::env::remove_var(BASE_URL_ENV);
            std::env::remove_var(TOKEN_ENV);
            std::env::remove_var(STRICT_ENV);
        }

        // Env unset → local, unchanged.
        assert!(matches!(resolve(None), CloudDecision::Local));

        // `--store` passed → local even with the cloud env set: the flag
        // names a local store, explicit user intent wins (#1791).
        unsafe { std::env::set_var(BASE_URL_ENV, "https://uteke.example") };
        unsafe { std::env::set_var(TOKEN_ENV, "tok") };
        assert!(matches!(
            resolve(Some("/tmp/other-store")),
            CloudDecision::Local
        ));

        // Full setup → cloud, with the trailing slash trimmed once.
        match resolve(None) {
            CloudDecision::Cloud(t) => assert_eq!(t.base_url, "https://uteke.example"),
            _ => panic!("full setup must resolve to cloud"),
        }
        unsafe { std::env::set_var(BASE_URL_ENV, "https://uteke.example/") };
        match resolve(None) {
            CloudDecision::Cloud(t) => assert_eq!(t.base_url, "https://uteke.example"),
            _ => panic!("a trailing slash must be trimmed"),
        }

        // Token missing → warn-and-local by default.
        unsafe { std::env::remove_var(TOKEN_ENV) };
        match resolve(None) {
            CloudDecision::LocalWithWarning(w) => {
                assert!(w.contains(TOKEN_ENV) && w.contains("local store"), "{w}")
            }
            _ => panic!("a missing token must warn and fall back"),
        }

        // ...and a hard error under strict mode.
        unsafe { std::env::set_var(STRICT_ENV, "1") };
        match resolve(None) {
            CloudDecision::StrictError(m) => assert!(m.contains(TOKEN_ENV), "{m}"),
            _ => panic!("strict mode must refuse to fall back"),
        }

        // STRICT values other than "1" are not strict.
        unsafe { std::env::set_var(STRICT_ENV, "0") };
        assert!(matches!(resolve(None), CloudDecision::LocalWithWarning(_)));

        unsafe {
            std::env::remove_var(BASE_URL_ENV);
            std::env::remove_var(TOKEN_ENV);
            std::env::remove_var(STRICT_ENV);
        }
    }

    #[test]
    fn uuid_detection_picks_the_doc_wire_field() {
        assert!(looks_like_uuid("a1b2c3d4-e5f6-4789-a012-3456789abcde"));
        assert!(!looks_like_uuid("release-procedure")); // slug
        assert!(!looks_like_uuid("a1b2c3d4e5f64789a0123456789abcde")); // no dashes
        assert!(!looks_like_uuid("a1b2c3d4-e5f6-4789-a012-3456789abcd")); // 35 chars
        assert!(!looks_like_uuid("a1b2c3d4+e5f6-4789-a012-3456789abcde")); // bad dash spot
        assert!(!looks_like_uuid("g1b2c3d4-e5f6-4789-a012-3456789abcde")); // g is not hex
    }
}
