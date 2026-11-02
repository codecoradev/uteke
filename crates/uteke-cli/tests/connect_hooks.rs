//! `uteke connect` and `uteke hook-run` end to end (#1465), on a temp HOME:
//! the installed command is run the way the agent runs it, with `uteke` on
//! PATH and without it. Unix only: the hooks run through `sh -c`.
#![cfg(unix)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

struct Env {
    dir: PathBuf,
}

impl Env {
    fn new(name: &str, capture: bool) -> Self {
        let dir = std::env::temp_dir().join(format!("uteke_connect_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for d in ["home", "uhome", "bin", "empty-bin"] {
            std::fs::create_dir_all(dir.join(d)).unwrap();
        }
        // Server routing off, so every command runs against the temp store.
        let mut toml = "[server]\nenabled = false\n".to_string();
        if capture {
            toml.push_str("\n[capture]\nenabled = true\n");
        }
        std::fs::write(dir.join("uhome/uteke.toml"), toml).unwrap();
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_uteke"), dir.join("bin/uteke")).unwrap();
        Self { dir }
    }

    fn home(&self) -> PathBuf {
        self.dir.join("home")
    }

    fn command(&self, program: &Path, path_dir: &str) -> Command {
        let mut cmd = Command::new(program);
        cmd.env("HOME", self.home())
            .env("UTEKE_HOME", self.dir.join("uhome"))
            .env("UTEKE_NO_UPDATE_CHECK", "1")
            // No embedder: memories are stored FTS-only, commands still run.
            .env("ORT_LIB_PATH", "/nonexistent/uteke-test/libonnxruntime.so")
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.dir.join(path_dir).display()),
            )
            .env_remove("UTEKE_AUTH_TOKEN")
            // Keep `connect codex` inside the temp HOME.
            .env_remove("CODEX_HOME");
        cmd
    }

    fn uteke(&self, args: &[&str]) -> Output {
        self.command(Path::new(env!("CARGO_BIN_EXE_uteke")), "bin")
            .args(args)
            .output()
            .expect("run uteke")
    }

    /// Run a hook command string as the agent does (`sh -c <command>`),
    /// with `payload` on stdin.
    fn hook(&self, command: &str, payload: &serde_json::Value, path_dir: &str) -> Output {
        let mut child = self
            .command(Path::new("/bin/sh"), path_dir)
            .args(["-c", command])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn settings(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.home().join(".claude/settings.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// The command `connect` installed for a Claude Code event.
    fn installed(&self, event: &str) -> String {
        self.settings()["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.home().join(".codecora/uteke/hook.log")).unwrap_or_default()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn connect_and_disconnect_on_a_temp_home() {
    let env = Env::new("roundtrip", false);
    let dry = ok(&env.uteke(&["connect", "claude", "--dry-run"]));
    assert!(dry.contains("nothing written"), "{dry}");
    assert!(!env.home().join(".claude/settings.json").exists());

    ok(&env.uteke(&["connect", "claude"]));
    for event in ["SessionStart", "UserPromptSubmit", "Stop"] {
        assert!(env.installed(event).contains("uteke hook-run"), "{event}");
    }
    assert!(
        env.home()
            .join(".codecora/uteke/connect/claude.json")
            .exists()
    );
    let again = ok(&env.uteke(&["connect", "claude"]));
    assert!(again.contains("already up to date"), "{again}");

    ok(&env.uteke(&["disconnect", "claude"]));
    assert!(!env.home().join(".claude/settings.json").exists());
    assert!(
        !env.home()
            .join(".codecora/uteke/connect/claude.json")
            .exists()
    );
}

#[test]
fn an_unsupported_agent_is_an_error() {
    let env = Env::new("unsupported", false);
    let out = env.uteke(&["connect", "notepad"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("Supported: claude"));
}

#[test]
fn every_installed_hook_exits_0_without_uteke_on_path() {
    let env = Env::new("no_path", true);
    ok(&env.uteke(&["connect", "claude"]));
    let payload = serde_json::json!({
        "session_id": "s1", "cwd": "/w/proj", "prompt": "hi", "last_assistant_message": "hello"
    });
    for event in ["SessionStart", "UserPromptSubmit", "Stop"] {
        let out = env.hook(&env.installed(event), &payload, "empty-bin");
        assert_eq!(out.status.code(), Some(0), "{event}: {out:?}");
        assert!(out.stdout.is_empty(), "{event}: {out:?}");
    }
}

#[test]
fn hook_run_exits_0_and_logs_on_bad_input() {
    let env = Env::new("bad_input", true);
    // An unknown event.
    let out = env.hook("uteke hook-run nonsense", &serde_json::json!({}), "bin");
    assert_eq!(out.status.code(), Some(0));
    assert!(
        env.log().contains("unknown hook event 'nonsense'"),
        "{}",
        env.log()
    );
    // A prompt without a session id.
    let out = env.hook(
        "uteke hook-run prompt",
        &serde_json::json!({"prompt": "x"}),
        "bin",
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(env.log().contains("no session id"), "{}", env.log());
    // A payload that is not JSON at all.
    let mut child = env
        .command(Path::new(env!("CARGO_BIN_EXE_uteke")), "bin")
        .args(["hook-run", "stop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(b"not json").unwrap();
    assert_eq!(child.wait_with_output().unwrap().status.code(), Some(0));
}

#[test]
fn prompt_and_stop_hooks_queue_the_turns_for_extraction() {
    let env = Env::new("capture", true);
    ok(&env.uteke(&["connect", "claude"]));
    let out = env.hook(
        &env.installed("UserPromptSubmit"),
        &serde_json::json!({"session_id": "sess-1", "prompt": "the staging host is blue-7"}),
        "bin",
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    let out = env.hook(
        &env.installed("Stop"),
        &serde_json::json!({"session_id": "sess-1", "last_assistant_message": "Noted: blue-7."}),
        "bin",
    );
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert!(env.log().is_empty(), "{}", env.log());

    let status: serde_json::Value =
        serde_json::from_str(&ok(&env.uteke(&["--json", "extract-queue", "status"]))).unwrap();
    assert_eq!(status["pending"], 2, "{status}");
    assert_eq!(status["sessions_pending"], 1, "{status}");
}

#[test]
fn capture_off_makes_the_capture_hooks_a_quiet_no_op() {
    let env = Env::new("capture_off", false);
    let out = env.hook(
        "uteke hook-run prompt",
        &serde_json::json!({"session_id": "s", "prompt": "x"}),
        "bin",
    );
    assert_eq!(out.status.code(), Some(0));
    assert!(env.log().is_empty(), "{}", env.log());
}

/// Without an embedder (as in CI) the memory recall finds nothing, so this
/// checks the quiet path end to end; the formatting is unit-tested in
/// `hook_run`.
#[test]
fn session_start_exits_0_and_prints_nothing_when_nothing_matches() {
    let env = Env::new("recall", false);
    ok(&env.uteke(&["remember", "gaira deploys on Fridays after the freeze"]));
    ok(&env.uteke(&["connect", "claude"]));
    let out = env.hook(
        &env.installed("SessionStart"),
        &serde_json::json!({"session_id": "s", "cwd": "/work/zzqx", "source": "startup"}),
        "bin",
    );
    assert!(ok(&out).trim().is_empty(), "{out:?}");
    assert!(env.log().is_empty(), "{}", env.log());
}

/// Every command installed for an agent, read from its config file.
fn installed_commands(env: &Env, agent: &str) -> Vec<(String, String)> {
    let (file, nested) = match agent {
        "codex" => (".codex/hooks.json", true),
        "cursor" => (".cursor/hooks.json", false),
        _ => (".claude/settings.json", true),
    };
    let text = std::fs::read_to_string(env.home().join(file)).unwrap();
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    let mut out = Vec::new();
    for (event, entries) in v["hooks"].as_object().unwrap() {
        for e in entries.as_array().unwrap() {
            let command = if nested {
                &e["hooks"][0]["command"]
            } else {
                &e["command"]
            };
            out.push((event.clone(), command.as_str().unwrap().to_string()));
        }
    }
    out
}

#[test]
fn codex_and_cursor_hooks_exit_0_without_uteke_and_cursor_prompts_continue() {
    let env = Env::new("wave1_no_path", true);
    let payload = serde_json::json!({
        "session_id": "s", "conversation_id": "c", "cwd": "/w/p", "prompt": "hi", "text": "ok"
    });
    for agent in ["codex", "cursor"] {
        let out = ok(&env.uteke(&["connect", agent]));
        if agent == "codex" {
            assert!(out.contains("/hooks"), "{out}");
        }
        let commands = installed_commands(&env, agent);
        assert_eq!(commands.len(), 3, "{agent}: {commands:?}");
        for (event, command) in commands {
            for path in ["empty-bin", "bin"] {
                let out = env.hook(&command, &payload, path);
                assert_eq!(
                    out.status.code(),
                    Some(0),
                    "{agent} {event} {path}: {out:?}"
                );
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if event == "beforeSubmitPrompt" {
                    assert_eq!(stdout, r#"{"continue":true}"#, "{path}");
                } else {
                    assert!(stdout.is_empty(), "{agent} {event} {path}: {stdout}");
                }
            }
        }
        ok(&env.uteke(&["disconnect", agent]));
    }
    assert!(env.log().is_empty(), "{}", env.log());
}

#[test]
fn cursor_reply_and_prompt_are_queued_under_the_conversation() {
    let env = Env::new("cursor_capture", true);
    let prompt = serde_json::json!({"conversation_id": "conv-9", "session_id": "other", "prompt": "use port 8080"});
    let reply = serde_json::json!({"conversation_id": "conv-9", "text": "Port 8080 it is."});
    env.hook("uteke hook-run prompt --agent cursor", &prompt, "bin");
    env.hook("uteke hook-run stop --agent cursor", &reply, "bin");
    let status: serde_json::Value =
        serde_json::from_str(&ok(&env.uteke(&["--json", "extract-queue", "status"]))).unwrap();
    assert_eq!(status["pending"], 2, "{status}");
    assert_eq!(status["sessions_pending"], 1, "{status}");
}

#[test]
fn connect_pi_installs_the_memory_provider_and_disconnect_removes_it() {
    let env = Env::new("pi", false);
    let dry = ok(&env.uteke(&["connect", "pi", "--dry-run"]));
    assert!(dry.contains("index.ts"), "{dry}");
    let dir = env
        .home()
        .join(".pi/agent/extensions/uteke-memory-provider");
    assert!(!dir.exists());
    ok(&env.uteke(&["connect", "pi"]));
    assert!(dir.join("index.ts").exists());

    // A file the user changed stays.
    std::fs::write(dir.join("package.json"), "{}").unwrap();
    let out = ok(&env.uteke(&["disconnect", "pi"]));
    assert!(out.contains("left alone"), "{out}");
    assert!(!dir.join("index.ts").exists());
    assert!(dir.join("package.json").exists());

    std::fs::remove_file(dir.join("package.json")).unwrap();

    // connect over a file the user changed keeps it as a backup.
    ok(&env.uteke(&["connect", "pi"]));
    std::fs::write(dir.join("index.ts"), "// mine").unwrap();
    let out = ok(&env.uteke(&["connect", "pi"]));
    assert!(out.contains("index.ts.uteke-bak"), "{out}");
    assert_eq!(
        std::fs::read_to_string(dir.join("index.ts.uteke-bak")).unwrap(),
        "// mine"
    );
    ok(&env.uteke(&["disconnect", "pi"]));

    ok(&env.uteke(&["connect", "hermes"]));
    ok(&env.uteke(&["disconnect", "hermes"]));
    assert!(!env.home().join(".hermes/plugins/uteke").exists());
}
