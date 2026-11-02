//! `uteke hook-run <event>` (#1465, hidden): what an agent's lifecycle hook
//! runs. Reads the agent's JSON payload from stdin, then:
//!
//! - `session-start`: recalls memories for the project and prints them as
//!   context in the agent's format;
//! - `prompt`: queues the user's prompt through `uteke capture` (#1466);
//! - `stop`: queues the assistant's last message the same way.
//!
//! It never fails the agent: every error goes to `~/.codecora/uteke/hook.log`
//! and the exit code is always 0. The work runs in a child `uteke` process,
//! so it takes the same server-or-local-store route as the CLI. No LLM and
//! no network beyond a local uteke server.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// The log is rotated to `hook.log.1` past this size.
const LOG_LIMIT: u64 = 1024 * 1024;
/// Memories considered at session start, and the character budget of the
/// context they fill.
const RECALL_LIMIT: &str = "8";
const RECALL_BUDGET: &str = "3000";

/// Run one hook event. Always returns; the caller exits 0.
pub(crate) fn run(event: &str, agent: &str) {
    let payload = read_payload();
    if let Err(e) = handle(event, agent, &payload) {
        log(agent, event, &e);
    }
}

fn read_payload() -> serde_json::Value {
    let mut stdin = std::io::stdin();
    // Run by hand from a terminal: there is no payload to wait for.
    if stdin.is_terminal() {
        return serde_json::Value::Null;
    }
    let mut text = String::new();
    if stdin.read_to_string(&mut text).is_err() || text.trim().is_empty() {
        return serde_json::Value::Null;
    }
    serde_json::from_str(&text).unwrap_or(serde_json::Value::Null)
}

fn handle(event: &str, agent: &str, payload: &serde_json::Value) -> Result<(), String> {
    match event {
        "session-start" => {
            let context = recall(&project_name(payload))?;
            print_context(agent, event, context.as_deref());
            Ok(())
        }
        "prompt" => {
            let text = payload["prompt"].as_str().unwrap_or_default();
            capture(agent, payload, "user", text)
        }
        "stop" => {
            let text = payload["last_assistant_message"]
                .as_str()
                .unwrap_or_default();
            capture(agent, payload, "assistant", text)
        }
        other => Err(format!("unknown hook event '{other}'")),
    }
}

/// The project to recall for: the last component of the session's working
/// directory (`cwd`, else the first workspace root, else this process's).
fn project_name(payload: &serde_json::Value) -> String {
    let dir = payload["cwd"]
        .as_str()
        .or_else(|| payload["workspace_roots"][0].as_str())
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    dir.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// A child `uteke` with the update check off (no network on the hook path).
fn uteke() -> Result<Command, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find the uteke binary: {e}"))?;
    let mut cmd = Command::new(exe);
    cmd.env("UTEKE_NO_UPDATE_CHECK", "1").stderr(Stdio::piped());
    Ok(cmd)
}

/// `uteke recall <project> --pack` (the budgeted context pack, #1281) as
/// context text; `None` when nothing matched.
fn recall(project: &str) -> Result<Option<String>, String> {
    if project.is_empty() {
        return Ok(None);
    }
    let out = uteke()?
        .args([
            "--json",
            "recall",
            project,
            "--pack",
            "--budget",
            RECALL_BUDGET,
            "--limit",
            RECALL_LIMIT,
        ])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("recall did not start: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "recall failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let pack: serde_json::Value = serde_json::from_slice(&out.stdout)
        .map_err(|e| format!("recall printed no JSON pack: {e}"))?;
    Ok(format_pack(project, &pack))
}

/// The selected items of a context pack as plain statements, one per line.
fn format_pack(project: &str, pack: &serde_json::Value) -> Option<String> {
    let items: Vec<String> = pack["selected"]
        .as_array()?
        .iter()
        .filter_map(|item| {
            let content = item["content"].as_str()?.trim();
            if content.is_empty() {
                return None;
            }
            let title = item["doc_title"]
                .as_str()
                .map(|t| format!("{t}: "))
                .unwrap_or_default();
            Some(format!("- {title}{}", content.replace('\n', "\n  ")))
        })
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(format!(
        "Memories stored in uteke for {project}:\n{}",
        items.join("\n")
    ))
}

/// Print recalled context the way the agent reads it. Claude Code adds plain
/// stdout of `SessionStart` to the context.
fn print_context(_agent: &str, _event: &str, context: Option<&str>) {
    if let Some(c) = context {
        println!("{c}");
    }
}

/// Queue one turn with `uteke capture`. Skipped while capture is off.
fn capture(agent: &str, payload: &serde_json::Value, role: &str, text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        return Ok(());
    }
    if !crate::config::Config::load().engine.capture.enabled {
        return Ok(());
    }
    let session = payload["session_id"]
        .as_str()
        .or_else(|| payload["conversation_id"].as_str())
        .ok_or("the payload has no session id")?;
    let mut child = uteke()?
        .args([
            "capture",
            "--session",
            session,
            "--role",
            role,
            "--source",
            agent,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| format!("capture did not start: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("capture: cannot pass the text: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("capture did not finish: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "capture failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

pub(crate) fn log_path(home: &Path) -> PathBuf {
    crate::connect::uteke_dir(home).join("hook.log")
}

/// Append one line to the hook log; a failure to log is ignored too.
fn log(agent: &str, event: &str, message: &str) {
    let Ok(home) = crate::connect::home_dir() else {
        return;
    };
    let path = log_path(&home);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > LOG_LIMIT) {
        let _ = std::fs::rename(&path, path.with_extension("log.1"));
    }
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(
            f,
            "{} {agent} {event}: {}",
            chrono::Utc::now().to_rfc3339(),
            message.replace('\n', " ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_project_comes_from_cwd_then_the_workspace_root() {
        assert_eq!(
            project_name(&serde_json::json!({"cwd": "/work/gaira"})),
            "gaira"
        );
        assert_eq!(
            project_name(&serde_json::json!({"workspace_roots": ["/w/muran"]})),
            "muran"
        );
    }

    #[test]
    fn a_pack_becomes_one_statement_per_item() {
        let pack = serde_json::json!({"selected": [
            {"content": "gaira deploys on Fridays", "result_type": "memory", "score": 0.4},
            {"content": "line one\nline two", "doc_title": "Runbook", "score": 0.2},
            {"content": "  "}
        ]});
        assert_eq!(
            format_pack("gaira", &pack).unwrap(),
            "Memories stored in uteke for gaira:\n- gaira deploys on Fridays\n- Runbook: line one\n  line two"
        );
        assert_eq!(
            format_pack("gaira", &serde_json::json!({"selected": []})),
            None
        );
    }
}
