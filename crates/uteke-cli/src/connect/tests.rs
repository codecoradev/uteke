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
    fn render_entries(&self) -> Vec<Entry> {
        let mut entries = claude::ClaudeCode.render_entries();
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
