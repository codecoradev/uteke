//! connect/disconnect on a temp HOME with the fixture configs in
//! `tests/fixtures/connect/` (#1465 acceptance table).

use super::*;

const CLAUDE_FIXTURE: &str = include_str!("../../tests/fixtures/connect/claude-settings.json");

fn home() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

fn json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&read(path)).unwrap()
}

/// An adapter whose entries change, standing in for a newer uteke release.
struct Renamed;

impl Adapter for Renamed {
    fn agent(&self) -> &'static str {
        "claude"
    }
    fn display_name(&self) -> &'static str {
        "Claude Code"
    }
    fn config_path(&self, home: &Path) -> PathBuf {
        claude::ClaudeCode.config_path(home)
    }
    fn render_entries_for(&self, bin: Option<&str>) -> Vec<Entry> {
        let mut entries = claude::ClaudeCode.render_entries_for(bin);
        entries[0].entry["hooks"][0]["timeout"] = serde_json::json!(20);
        entries
    }
}

#[test]
fn connect_adds_the_entries_and_keeps_every_user_key_in_order() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);

    let report = connect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(report.changed);
    assert_eq!(report.added.len(), 3, "{report:?}");

    let after = read(&path);
    let v = json(&path);
    // User keys and the user's own hook are untouched.
    assert_eq!(v["model"], "opus");
    assert_eq!(v["permissions"]["allow"][0], "Bash(git status)");
    assert_eq!(
        v["hooks"]["SessionStart"][0]["hooks"][0]["command"],
        "echo mine"
    );
    assert_eq!(v["hooks"]["SessionStart"].as_array().unwrap().len(), 2);
    for event in ["SessionStart", "UserPromptSubmit", "Stop"] {
        assert!(after.contains(event), "{after}");
    }
    // Key order of the original is kept: `model` still comes before `hooks`.
    assert!(after.find("\"model\"").unwrap() < after.find("\"permissions\"").unwrap());
    assert!(after.find("\"permissions\"").unwrap() < after.find("\"hooks\"").unwrap());
    // The guarded command, resolved through PATH at run time.
    assert!(after.contains(
        "command -v uteke >/dev/null 2>&1 && uteke hook-run session-start --agent claude || true"
    ));
    // The original is backed up once.
    assert_eq!(read(&backup_path(&path)), CLAUDE_FIXTURE);
    let manifest = read_manifest(h.path(), "claude").unwrap().unwrap();
    assert_eq!(manifest.entries, claude::ClaudeCode.render_entries());
    assert_eq!(manifest.uteke_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(manifest.created_events, ["UserPromptSubmit", "Stop"]);
}

#[test]
fn a_second_connect_is_a_byte_identical_no_op() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let first = read(&path);
    let manifest = read(&manifest_path(h.path(), "claude"));

    let report = connect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(!report.changed);
    assert!(report.added.is_empty());
    assert_eq!(read(&path), first);
    assert_eq!(read(&manifest_path(h.path(), "claude")), manifest);
}

#[test]
fn disconnect_removes_only_what_connect_added() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    connect(&claude::ClaudeCode, h.path(), false).unwrap();

    // The user adds an entry of their own after connect.
    let mut v = json(&path);
    v["hooks"]["Stop"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"hooks": [{"type": "command", "command": "say done"}]}));
    write(
        &path,
        &format!("{}\n", serde_json::to_string_pretty(&v).unwrap()),
    );

    let report = disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert_eq!(report.removed.len(), 3, "{report:?}");
    assert!(report.left_alone.is_empty());

    let v = json(&path);
    let mut expected: serde_json::Value = serde_json::from_str(CLAUDE_FIXTURE).unwrap();
    expected["hooks"]["Stop"] =
        serde_json::json!([{"hooks": [{"type": "command", "command": "say done"}]}]);
    assert_eq!(v, expected);
    assert!(!manifest_path(h.path(), "claude").exists());
}

#[test]
fn connect_then_disconnect_restores_the_original_bytes() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert_eq!(read(&path), CLAUDE_FIXTURE);
}

#[test]
fn an_entry_the_user_edited_is_left_alone_and_reported() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let text = read(&path).replace("\"timeout\": 15", "\"timeout\": 99");
    write(&path, &text);

    let report = disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert_eq!(report.left_alone.len(), 2, "{report:?}");
    assert_eq!(report.removed.len(), 1);
    assert!(read(&path).contains("\"timeout\": 99"));
}

#[test]
fn without_a_settings_file_connect_creates_it_and_disconnect_deletes_it() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(path.exists());
    assert!(!backup_path(&path).exists(), "nothing to back up");
    disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(!path.exists());
}

#[test]
fn a_dry_run_writes_nothing() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    let report = connect(&claude::ClaudeCode, h.path(), true).unwrap();
    assert!(report.changed);
    assert_eq!(report.added.len(), 3);
    assert_eq!(read(&path), CLAUDE_FIXTURE);
    assert!(!backup_path(&path).exists());
    assert!(!manifest_path(h.path(), "claude").exists());

    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let connected = read(&path);
    let report = disconnect(&claude::ClaudeCode, h.path(), true).unwrap();
    assert_eq!(report.removed.len(), 3);
    assert_eq!(read(&path), connected);
    assert!(manifest_path(h.path(), "claude").exists());
}

#[test]
fn an_invalid_settings_file_is_refused_untouched() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, "{ \"model\": ");
    assert!(connect(&claude::ClaudeCode, h.path(), false).is_err());
    assert_eq!(read(&path), "{ \"model\": ");
}

#[test]
fn a_newer_version_is_reported_stale_and_connect_rewrites_only_its_entries() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert_eq!(hook_status(h.path())[0].state, "ok");

    // The installed entries no longer match what this version renders.
    let manifest = read_manifest(h.path(), "claude").unwrap().unwrap();
    assert!(is_stale(&Renamed, &manifest));

    let report = connect(&Renamed, h.path(), false).unwrap();
    assert_eq!(report.removed.len(), 1, "{report:?}");
    assert_eq!(report.added.len(), 1);
    let v = json(&path);
    let starts = v["hooks"]["SessionStart"].as_array().unwrap();
    assert_eq!(starts.len(), 2, "the user's hook and the refreshed one");
    assert_eq!(starts[0]["hooks"][0]["command"], "echo mine");
    assert_eq!(starts[1]["hooks"][0]["timeout"], 20);
    // created_events survive the refresh, so disconnect still cleans up.
    disconnect(&Renamed, h.path(), false).unwrap();
    assert_eq!(read(&path), CLAUDE_FIXTURE);
}

#[test]
fn doctor_reports_removed_entries() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let mut v = json(&path);
    v["hooks"].as_object_mut().unwrap().remove("Stop");
    write(&path, &serde_json::to_string_pretty(&v).unwrap());
    assert_eq!(hook_status(h.path())[0].state, "missing");
    // No manifest, no line.
    assert!(hook_status(home().path()).is_empty());
}

#[test]
fn disconnect_without_connect_changes_nothing() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    let report = disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(!report.changed);
    assert_eq!(read(&path), CLAUDE_FIXTURE);
}

#[cfg(unix)]
#[test]
fn a_rewrite_keeps_the_file_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[cfg(unix)]
#[test]
fn a_symlinked_settings_file_is_written_through_the_link() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    let target = h.path().join("dotfiles/claude-settings.json");
    write(&target, CLAUDE_FIXTURE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();

    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(read(&target).contains("uteke hook-run"));
    disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(
        std::fs::symlink_metadata(&path)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(read(&target), CLAUDE_FIXTURE);
}

#[test]
fn hooks_of_the_wrong_shape_are_refused_untouched() {
    for text in [
        "{\"hooks\": []}\n",
        "{\"hooks\": {\"Stop\": {\"command\": \"x\"}}}\n",
    ] {
        let h = home();
        let path = claude::ClaudeCode.config_path(h.path());
        write(&path, text);
        let err = connect(&claude::ClaudeCode, h.path(), false).unwrap_err();
        assert!(err.contains("nothing was changed"), "{err}");
        assert_eq!(read(&path), text);
        assert!(!manifest_path(h.path(), "claude").exists());
    }
}

#[test]
fn a_moved_config_is_refused_while_the_old_one_is_recorded() {
    let h = home();
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let mut manifest = read_manifest(h.path(), "claude").unwrap().unwrap();
    manifest.config_path = h.path().join("elsewhere/settings.json");
    write(
        &manifest_path(h.path(), "claude"),
        &serde_json::to_string_pretty(&manifest).unwrap(),
    );
    let err = connect(&claude::ClaudeCode, h.path(), false).unwrap_err();
    assert!(err.contains("disconnect"), "{err}");
}

#[test]
fn a_user_hook_added_to_a_file_connect_created_survives_disconnect() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let mut v = json(&path);
    v["hooks"]["PreToolUse"] =
        serde_json::json!([{"hooks": [{"type": "command", "command": "mine"}]}]);
    write(&path, &serde_json::to_string_pretty(&v).unwrap());

    disconnect(&claude::ClaudeCode, h.path(), false).unwrap();
    assert!(path.exists());
    assert_eq!(
        json(&path),
        serde_json::json!({"hooks": {"PreToolUse": [{"hooks": [{"type": "command", "command": "mine"}]}]}})
    );
}

const CODEX_FIXTURE: &str = include_str!("../../tests/fixtures/connect/codex-hooks.json");
const CURSOR_FIXTURE: &str = include_str!("../../tests/fixtures/connect/cursor-hooks.json");

/// Every adapter with the fixture of its config file.
fn adapters() -> Vec<(Box<dyn Adapter>, &'static str)> {
    vec![
        (Box::new(claude::ClaudeCode), CLAUDE_FIXTURE),
        (Box::new(codex::Codex { codex_home: None }), CODEX_FIXTURE),
        (Box::new(cursor::Cursor), CURSOR_FIXTURE),
    ]
}

#[test]
fn every_adapter_is_idempotent_and_disconnect_restores_the_fixture() {
    for (adapter, fixture) in adapters() {
        let h = home();
        let path = adapter.config_path(h.path());
        write(&path, fixture);
        let report = connect(adapter.as_ref(), h.path(), false).unwrap();
        assert_eq!(report.added.len(), 3, "{}: {report:?}", adapter.agent());
        let first = read(&path);
        assert!(!connect(adapter.as_ref(), h.path(), false).unwrap().changed);
        assert_eq!(read(&path), first, "{}", adapter.agent());
        assert_eq!(hook_status(h.path())[0].state, "ok");
        disconnect(adapter.as_ref(), h.path(), false).unwrap();
        assert_eq!(read(&path), fixture, "{}", adapter.agent());
    }
}

#[test]
fn every_adapter_creates_and_deletes_a_missing_file() {
    for (adapter, _) in adapters() {
        let h = home();
        let path = adapter.config_path(h.path());
        connect(adapter.as_ref(), h.path(), false).unwrap();
        assert!(path.exists(), "{}", adapter.agent());
        disconnect(adapter.as_ref(), h.path(), false).unwrap();
        assert!(!path.exists(), "{}", adapter.agent());
    }
}

#[test]
fn codex_entries_carry_a_windows_command_and_the_trust_step() {
    let h = home();
    let path = codex::Codex { codex_home: None }.config_path(h.path());
    write(&path, CODEX_FIXTURE);
    let report = connect(&codex::Codex { codex_home: None }, h.path(), false).unwrap();
    let v = json(&path);
    assert_eq!(v["description"], "my codex hooks");
    assert_eq!(
        v["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
        "./check.sh"
    );
    let start = &v["hooks"]["SessionStart"][0]["hooks"][0];
    assert_eq!(start["type"], "command");
    assert!(
        start["command"]
            .as_str()
            .unwrap()
            .contains("uteke hook-run session-start --agent codex")
    );
    assert!(
        start["commandWindows"]
            .as_str()
            .unwrap()
            .contains("Get-Command uteke")
    );
    assert!(
        report.notes.iter().any(|n| n.contains("/hooks")),
        "{report:?}"
    );
}

#[test]
fn cursor_uses_its_own_shape_and_keeps_the_version() {
    let h = home();
    let path = cursor::Cursor.config_path(h.path());
    write(&path, CURSOR_FIXTURE);
    connect(&cursor::Cursor, h.path(), false).unwrap();
    let v = json(&path);
    assert_eq!(v["version"], 1);
    let starts = v["hooks"]["sessionStart"].as_array().unwrap();
    assert_eq!(starts[0]["command"], "./hooks/mine.sh");
    assert!(
        starts[1]["command"]
            .as_str()
            .unwrap()
            .contains("uteke hook-run session-start --agent cursor")
    );
    let prompt = v["hooks"]["beforeSubmitPrompt"][0]["command"]
        .as_str()
        .unwrap();
    assert!(
        prompt.ends_with(r#"|| echo "{\"continue\":true}"'"#),
        "{prompt}"
    );
    assert!(v["hooks"]["afterAgentResponse"][0]["command"].is_string());
    // A file connect created gets `version: 1`, and loses it on disconnect.
    let h = home();
    connect(&cursor::Cursor, h.path(), false).unwrap();
    let path = cursor::Cursor.config_path(h.path());
    assert_eq!(json(&path)["version"], 1);
    assert!(read(&path).find("\"version\"").unwrap() < read(&path).find("\"hooks\"").unwrap());
}

#[test]
fn codex_home_moves_the_codex_config() {
    let h = home();
    let elsewhere = h.path().join("codex-home");
    let codex = codex::Codex {
        codex_home: Some(elsewhere.clone()),
    };
    assert_eq!(codex.config_path(h.path()), elsewhere.join("hooks.json"));
    assert_eq!(
        codex::Codex { codex_home: None }.config_path(h.path()),
        h.path().join(".codex/hooks.json")
    );
}

// --- #1668: temp file mode and concurrent writers ---

#[cfg(unix)]
#[test]
fn the_temp_file_is_created_with_the_original_mode() {
    use std::os::unix::fs::PermissionsExt;
    let dir = home();
    let tmp = dir.path().join(".settings.json.uteke-tmp");
    // Checked while the handle is open, i.e. at creation, before any chmod.
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    let f = create_temp(&tmp, Some(0o600)).unwrap();
    assert_eq!(mode(&tmp), 0o600);
    drop(f);
    // No original file: owner-only, never the umask default.
    let f = create_temp(&tmp, None).unwrap();
    assert_eq!(mode(&tmp), 0o600);
    drop(f);
    // A leftover from a crashed run is replaced, not followed.
    let target = dir.path().join("victim");
    std::fs::remove_file(&tmp).unwrap();
    std::os::unix::fs::symlink(&target, &tmp).unwrap();
    drop(create_temp(&tmp, None).unwrap());
    assert!(!target.exists(), "the leftover link was followed");
}

#[test]
fn a_config_changed_since_it_was_read_is_not_overwritten() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, CLAUDE_FIXTURE);
    // What uteke read, then another program writes before uteke does.
    let read_as = read(&path);
    write(&path, "{\"theirs\": true}\n");

    let err = write_atomic(&path, "{\"ours\": true}\n", Some(&read_as)).unwrap_err();

    assert!(err.contains("changed while uteke was updating it"), "{err}");
    assert_eq!(read(&path), "{\"theirs\": true}\n");
    assert!(
        !path.with_file_name(".settings.json.uteke-tmp").exists(),
        "temp file left behind"
    );
}

#[test]
fn a_file_created_since_it_was_read_is_not_overwritten() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, "{\"theirs\": true}\n");
    let err = write_atomic(&path, "{}\n", None).unwrap_err();
    assert!(err.contains("changed while uteke was updating it"), "{err}");
    assert_eq!(read(&path), "{\"theirs\": true}\n");
}

#[test]
fn an_unchanged_config_is_replaced() {
    let h = home();
    let path = claude::ClaudeCode.config_path(h.path());
    write(&path, "{\"a\": 1}\n");
    write_atomic(&path, "{\"a\": 2}\n", Some("{\"a\": 1}\n")).unwrap();
    assert_eq!(read(&path), "{\"a\": 2}\n");
}

// ---- --absolute-path (#1672) ----

/// A path with spaces and a single quote: the hard case for both shells.
const AWKWARD: &str = "/opt/my tools/it's here/uteke";

#[test]
fn the_default_entries_are_unchanged_without_the_flag() {
    assert_eq!(
        guarded_command("session-start", "claude"),
        "sh -c 'command -v uteke >/dev/null 2>&1 && uteke hook-run session-start --agent claude || true'"
    );
    assert_eq!(
        guarded_command_or("prompt", "cursor", Some(CURSOR_CONTINUE)),
        r#"sh -c 'command -v uteke >/dev/null 2>&1 && uteke hook-run prompt --agent cursor || echo "{\"continue\":true}"'"#
    );
    assert_eq!(
        guarded_command_windows_with("stop", "codex", None),
        r#"powershell -NoProfile -Command "if (Get-Command uteke -ErrorAction SilentlyContinue) { uteke hook-run stop --agent codex }; exit 0""#
    );
    let h = home();
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let text = read(&manifest_path(h.path(), "claude"));
    assert!(!text.contains("binary"), "{text}");
}

#[test]
fn an_absolute_path_is_quoted_for_sh() {
    let cmd = guarded_command_with("prompt", "claude", None, Some(AWKWARD));
    assert_eq!(
        cmd,
        r#"sh -c '[ -x '\''/opt/my tools/it'\''\'\'''\''s here/uteke'\'' ] && '\''/opt/my tools/it'\''\'\'''\''s here/uteke'\'' hook-run prompt --agent claude || true'"#
    );
}

/// Run the command with a real `sh`, with a fake binary at an awkward path.
#[cfg(unix)]
#[test]
fn an_absolute_path_command_runs_the_binary_and_is_silent_when_it_is_gone() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("my tools").join("it's here").join("uteke");
    std::fs::create_dir_all(bin.parent().unwrap()).unwrap();
    std::fs::write(&bin, "#!/bin/sh\necho \"ran $*\"\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let run = |agent: &str, fallback| {
        let cmd = guarded_command_with("prompt", agent, fallback, bin.to_str());
        std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .unwrap()
    };
    let out = run("claude", None);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "ran hook-run prompt --agent claude\n"
    );
    std::fs::remove_file(&bin).unwrap();
    let out = run("claude", None);
    assert!(out.status.success() && out.stdout.is_empty());
    let out = run("cursor", Some(CURSOR_CONTINUE));
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), CURSOR_CONTINUE);
}

#[test]
fn an_absolute_path_is_quoted_for_powershell() {
    assert_eq!(
        guarded_command_windows_with("stop", "codex", Some(r"C:\Program Files\it's\uteke.exe")),
        r#"powershell -NoProfile -Command "if (Test-Path -LiteralPath 'C:\Program Files\it''s\uteke.exe' -PathType Leaf) { & 'C:\Program Files\it''s\uteke.exe' hook-run stop --agent codex }; exit 0""#
    );
    assert!(check_binary(Path::new("/a/b\"c")).is_err());
    assert_eq!(check_binary(Path::new("/a/b")).unwrap(), "/a/b");
}

#[test]
fn absolute_path_is_recorded_idempotent_and_disconnect_removes_it() {
    for (adapter, fixture) in adapters() {
        let h = home();
        let cfg = adapter.config_path(h.path());
        write(&cfg, fixture);
        let bin = tempfile::NamedTempFile::new().unwrap();
        let bin_path = bin.path().to_str().unwrap().to_string();

        connect_with(adapter.as_ref(), h.path(), false, Some(&bin_path)).unwrap();
        let after = read(&cfg);
        assert!(after.contains(&bin_path), "{after}");
        assert!(!after.contains("command -v uteke"), "{after}");
        let manifest = read_manifest(h.path(), adapter.agent()).unwrap().unwrap();
        assert_eq!(manifest.binary.as_deref(), Some(bin_path.as_str()));
        assert_eq!(
            manifest.entries,
            adapter.render_entries_for(Some(&bin_path))
        );
        assert!(!is_stale(adapter.as_ref(), &manifest));
        assert_eq!(hook_status(h.path())[0].state, "ok");

        // A second run changes nothing, in the file or the manifest.
        let mtext = read(&manifest_path(h.path(), adapter.agent()));
        let report = connect_with(adapter.as_ref(), h.path(), false, Some(&bin_path)).unwrap();
        assert!(!report.changed, "{report:?}");
        assert_eq!(read(&cfg), after);
        assert_eq!(read(&manifest_path(h.path(), adapter.agent())), mtext);

        // disconnect removes exactly the installed entries, by the manifest.
        let report = disconnect(adapter.as_ref(), h.path(), false).unwrap();
        assert!(report.left_alone.is_empty(), "{report:?}");
        assert_eq!(read(&cfg), fixture, "{}", adapter.agent());
        assert!(read_manifest(h.path(), adapter.agent()).unwrap().is_none());
    }
}

#[test]
fn a_moved_binary_is_stale_and_a_rerun_refreshes_it() {
    let h = home();
    let cfg = claude::ClaudeCode.config_path(h.path());
    write(&cfg, CLAUDE_FIXTURE);
    let dir = tempfile::tempdir().unwrap();
    let old = dir.path().join("old-uteke");
    std::fs::write(&old, "").unwrap();
    connect_with(&claude::ClaudeCode, h.path(), false, old.to_str()).unwrap();
    assert_eq!(hook_status(h.path())[0].state, "ok");

    std::fs::remove_file(&old).unwrap();
    assert_eq!(hook_status(h.path())[0].state, "stale");
    let manifest = read_manifest(h.path(), "claude").unwrap().unwrap();
    assert!(is_stale(&claude::ClaudeCode, &manifest));

    let new = dir.path().join("new-uteke");
    std::fs::write(&new, "").unwrap();
    let report = connect_with(&claude::ClaudeCode, h.path(), false, new.to_str()).unwrap();
    assert_eq!(report.removed.len(), 3, "{report:?}");
    assert_eq!(report.added.len(), 3);
    assert_eq!(hook_status(h.path())[0].state, "ok");
    let text = read(&cfg);
    assert!(
        text.contains("new-uteke") && !text.contains("old-uteke"),
        "{text}"
    );
    assert!(text.contains("echo mine"), "the user's own hook stays");
}

#[test]
fn connecting_without_the_flag_goes_back_to_path() {
    let h = home();
    let bin = tempfile::NamedTempFile::new().unwrap();
    connect_with(&claude::ClaudeCode, h.path(), false, bin.path().to_str()).unwrap();
    connect(&claude::ClaudeCode, h.path(), false).unwrap();
    let manifest = read_manifest(h.path(), "claude").unwrap().unwrap();
    assert_eq!(manifest.binary, None);
    assert_eq!(manifest.entries, claude::ClaudeCode.render_entries());
}
