//! `uteke connect <agent>` / `uteke disconnect <agent>` (#1465): install and
//! remove the lifecycle hooks of an agent app.
//!
//! Each hook entry runs the guarded `uteke hook-run <event>` command, resolved
//! through `PATH` at run time, so nothing in the agent's config changes on an
//! upgrade. `connect` records what it added in a manifest
//! (`~/.codecora/uteke/connect/<agent>.json`); `disconnect` removes exactly
//! those entries and leaves anything the user wrote or edited alone.
//! Design: `docs/plans/2026-10-29-hook-adapters-1465.md`.

mod claude;
mod codex;
mod cursor;
pub(crate) use cursor::CONTINUE as CURSOR_CONTINUE;
pub(crate) mod ojson;
mod provider;

use ojson::OJson;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One hook entry an adapter adds: the event it hangs under in the agent's
/// `hooks` map, and the JSON element appended to that event's array.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Entry {
    pub event: String,
    pub entry: serde_json::Value,
}

/// What `connect` added to one agent's config file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Manifest {
    pub agent: String,
    pub config_path: PathBuf,
    pub uteke_version: String,
    pub entries: Vec<Entry>,
    /// `connect` created the file, so `disconnect` may delete it once empty.
    #[serde(default)]
    pub created_file: bool,
    /// Top-level keys `connect` created (`hooks`, Cursor's `version`).
    #[serde(default)]
    pub created_keys: Vec<String>,
    /// Event arrays `connect` created under `hooks`.
    #[serde(default)]
    pub created_events: Vec<String>,
}

/// One agent app. The default `merge`/`unmerge` handle the shape the wave-1
/// agents share: a top-level `hooks` object mapping an event name to an array.
pub(crate) trait Adapter {
    /// The name on the command line (`uteke connect claude`).
    fn agent(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    /// The config file the hooks go into.
    fn config_path(&self, home: &Path) -> PathBuf;
    /// The entries this version of uteke installs.
    fn render_entries(&self) -> Vec<Entry>;
    /// Top-level keys the file needs next to `hooks` (Cursor's `version`).
    fn required_keys(&self) -> Vec<(&'static str, OJson)> {
        Vec::new()
    }
    /// Lines printed after a real `connect` (a trust step, a known bug).
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }

    /// Add `entries` to `existing`; an entry already present is not added
    /// twice. Records in `manifest` which keys and events it created. A
    /// `hooks` that is not an object, or an event that is not an array, is
    /// refused before anything changes.
    fn merge(
        &self,
        existing: &mut OJson,
        entries: &[Entry],
        manifest: &mut Manifest,
    ) -> Result<(), String> {
        match existing.get("hooks") {
            None | Some(OJson::Object(_)) => {}
            Some(_) => return Err("`hooks` is not a JSON object".to_string()),
        }
        for e in entries {
            match existing.get("hooks").and_then(|h| h.get(&e.event)) {
                None | Some(OJson::Array(_)) => {}
                Some(_) => return Err(format!("`hooks.{}` is not a JSON array", e.event)),
            }
        }
        for (key, value) in self.required_keys() {
            if existing.get(key).is_none() {
                existing.insert(key, value);
                push_unique(&mut manifest.created_keys, key);
            }
        }
        if existing.get("hooks").is_none() {
            existing.insert("hooks", OJson::empty_object());
            push_unique(&mut manifest.created_keys, "hooks");
        }
        let hooks = existing.get_mut("hooks").expect("inserted above");
        for e in entries {
            if hooks.get(&e.event).is_none() {
                hooks.insert(&e.event, OJson::Array(Vec::new()));
                push_unique(&mut manifest.created_events, &e.event);
            }
            if let Some(OJson::Array(items)) = hooks.get_mut(&e.event) {
                if !items.iter().any(|i| i.to_value() == e.entry) {
                    items.push(OJson::from_value(&e.entry));
                }
            }
        }
        Ok(())
    }

    /// Remove the manifest's entries from `existing` where they still match
    /// exactly; return the ones that no longer match (edited or removed by
    /// the user), which stay as they are. Arrays and keys `connect` created
    /// are dropped once empty.
    fn unmerge(&self, existing: &mut OJson, manifest: &Manifest) -> Vec<Entry> {
        let mut missing = Vec::new();
        if let Some(hooks) = existing.get_mut("hooks") {
            for e in &manifest.entries {
                let Some(OJson::Array(items)) = hooks.get_mut(&e.event) else {
                    missing.push(e.clone());
                    continue;
                };
                match items.iter().position(|i| i.to_value() == e.entry) {
                    Some(pos) => {
                        items.remove(pos);
                    }
                    None => missing.push(e.clone()),
                }
            }
            for event in &manifest.created_events {
                if hooks.get(event).is_some_and(OJson::is_empty_container) {
                    hooks.remove(event);
                }
            }
        } else {
            missing.extend(manifest.entries.iter().cloned());
        }
        if manifest.created_keys.iter().any(|k| k == "hooks")
            && existing.get("hooks").is_some_and(OJson::is_empty_container)
        {
            existing.remove("hooks");
        }
        // Keys like Cursor's `version` go only when nothing else is left: no
        // `hooks` (a non-empty one holds the user's own hooks) and no key the
        // user wrote.
        let only_created = existing.get("hooks").is_none()
            && match existing {
                OJson::Object(e) => e.iter().all(|(k, _)| manifest.created_keys.contains(k)),
                _ => false,
            };
        if only_created {
            for k in &manifest.created_keys {
                existing.remove(k);
            }
        }
        missing
    }
}

fn push_unique(list: &mut Vec<String>, item: &str) {
    if !list.iter().any(|i| i == item) {
        list.push(item.to_string());
    }
}

/// The adapter for an agent name, if `connect` installs hooks for it.
pub(crate) fn adapter_for(agent: &str) -> Option<Box<dyn Adapter>> {
    match agent {
        "claude" | "claude-code" => Some(Box::new(claude::ClaudeCode)),
        "codex" => Some(Box::new(codex::Codex)),
        "cursor" => Some(Box::new(cursor::Cursor)),
        _ => None,
    }
}

/// The agents `connect` installs shell hooks for, for messages, `doctor` and
/// the `init` hint.
pub(crate) const SUPPORTED: &[&str] = &["claude", "codex", "cursor"];

/// The command a hook entry runs. Guarded: a missing `uteke` binary is a
/// no-op with exit 0 instead of the shell's 127, so the agent never sees a
/// failing hook. The string is the same on every machine.
pub(crate) fn guarded_command(event: &str, agent: &str) -> String {
    guarded_command_or(event, agent, None)
}

/// [`guarded_command`] that prints `fallback` when `uteke` is missing, for an
/// event whose agent expects an answer on stdout (Cursor's
/// `beforeSubmitPrompt`). `fallback` is fixed JSON without single quotes or
/// backslashes; it is double-quoted inside the single-quoted `sh -c` script.
pub(crate) fn guarded_command_or(event: &str, agent: &str, fallback: Option<&str>) -> String {
    let otherwise = match fallback {
        Some(f) => format!("echo \"{}\"", f.replace('"', "\\\"")),
        None => "true".to_string(),
    };
    format!(
        "sh -c 'command -v uteke >/dev/null 2>&1 && uteke hook-run {event} --agent {agent} || {otherwise}'"
    )
}

/// The same guard for PowerShell, for an agent with a Windows command field
/// (Codex's `commandWindows`).
pub(crate) fn guarded_command_windows(event: &str, agent: &str) -> String {
    format!(
        "powershell -NoProfile -Command \"if (Get-Command uteke -ErrorAction SilentlyContinue) {{ uteke hook-run {event} --agent {agent} }}; exit 0\""
    )
}

/// `~/.codecora/uteke`: the hook log and the connect manifests live here,
/// next to the default store.
pub(crate) fn uteke_dir(home: &Path) -> PathBuf {
    home.join(".codecora").join("uteke")
}

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    dirs::home_dir().ok_or_else(|| "Cannot find the home directory".to_string())
}

pub(crate) fn manifest_path(home: &Path, agent: &str) -> PathBuf {
    uteke_dir(home)
        .join("connect")
        .join(format!("{agent}.json"))
}

pub(crate) fn read_manifest(home: &Path, agent: &str) -> Result<Option<Manifest>, String> {
    let path = manifest_path(home, agent);
    match std::fs::read_to_string(&path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("Cannot read {}: {e}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

/// The config file as read: its value, and whether it existed.
struct ConfigFile {
    value: OJson,
    existed: bool,
    trailing_newline: bool,
}

fn read_config(path: &Path) -> Result<ConfigFile, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(ConfigFile {
            value: OJson::empty_object(),
            existed: true,
            trailing_newline: true,
        }),
        Ok(text) => {
            let value: OJson = serde_json::from_str(&text).map_err(|e| {
                format!(
                    "{} is not valid JSON ({e}); fix it or move it away, nothing was changed",
                    path.display()
                )
            })?;
            if !matches!(value, OJson::Object(_)) {
                return Err(format!(
                    "{} is not a JSON object; nothing was changed",
                    path.display()
                ));
            }
            Ok(ConfigFile {
                value,
                existed: true,
                trailing_newline: text.ends_with('\n'),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile {
            value: OJson::empty_object(),
            existed: false,
            trailing_newline: true,
        }),
        Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
    }
}

fn render(value: &OJson, trailing_newline: bool) -> Result<String, String> {
    let mut text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    if trailing_newline {
        text.push('\n');
    }
    Ok(text)
}

/// Write through a temporary file and a rename, so a crash never leaves a
/// half-written config. A symlinked config (a dotfiles checkout) is written
/// at its target: renaming over the link would replace the link itself.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let resolved;
    let path = if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        resolved = std::fs::canonicalize(path)
            .map_err(|e| format!("Cannot resolve the link {}: {e}", path.display()))?;
        resolved.as_path()
    } else {
        path
    };
    let dir = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create {}: {e}", dir.display()))?;
    let tmp = dir.join(format!(
        ".{}.uteke-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    std::fs::write(&tmp, text).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    // The rename replaces the file: keep its permissions (a settings file the
    // user made private stays private).
    if let Ok(meta) = std::fs::metadata(path) {
        std::fs::set_permissions(&tmp, meta.permissions())
            .map_err(|e| format!("Cannot set permissions on {}: {e}", tmp.display()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| format!("Cannot write {}: {e}", path.display()))
}

/// `<file>.uteke-bak`, written once: the copy of the file before uteke first
/// touched it.
pub(crate) fn backup_path(config: &Path) -> PathBuf {
    let mut name = config.file_name().unwrap_or_default().to_os_string();
    name.push(".uteke-bak");
    config.with_file_name(name)
}

fn short(e: &Entry) -> String {
    let command = find_command(&e.entry).unwrap_or_default();
    format!("{}: {command}", e.event)
}

/// The first `command` string inside an entry, for display.
fn find_command(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(m) => m
            .get("command")
            .and_then(|c| c.as_str().map(str::to_string))
            .or_else(|| m.values().find_map(find_command)),
        serde_json::Value::Array(a) => a.iter().find_map(find_command),
        _ => None,
    }
}

/// What a `connect` or `disconnect` run did, for the output.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Report {
    pub agent: String,
    pub config_path: PathBuf,
    pub dry_run: bool,
    /// The file changed (or would change, in a dry run).
    pub changed: bool,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    /// Manifest entries that no longer match the file, left alone.
    pub left_alone: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<PathBuf>,
    pub notes: Vec<String>,
}

/// Install (or refresh) the hooks of `adapter` under `home`.
pub(crate) fn connect(adapter: &dyn Adapter, home: &Path, dry_run: bool) -> Result<Report, String> {
    let path = adapter.config_path(home);
    let file = read_config(&path)?;
    let previous = read_manifest(home, adapter.agent())?;
    let mut value = file.value.clone();
    let mut manifest = Manifest {
        agent: adapter.agent().to_string(),
        config_path: path.clone(),
        uteke_version: env!("CARGO_PKG_VERSION").to_string(),
        created_file: !file.existed,
        ..Default::default()
    };
    let mut report = Report {
        agent: adapter.agent().to_string(),
        config_path: path.clone(),
        dry_run,
        ..Default::default()
    };
    // A re-run replaces only what the previous run added.
    // The config moved (`CODEX_HOME` changed): the old entries would be left
    // behind with nothing tracking them.
    if let Some(prev) = previous.as_ref().filter(|p| p.config_path != path) {
        return Err(format!(
            "uteke's {} hooks are recorded in {}, not {}: run `uteke disconnect {}` \
             with the old location first",
            adapter.display_name(),
            prev.config_path.display(),
            path.display(),
            adapter.agent()
        ));
    }
    if let Some(prev) = &previous {
        let entries = adapter.render_entries();
        let stale: Vec<Entry> = prev
            .entries
            .iter()
            .filter(|e| !entries.contains(e))
            .cloned()
            .collect();
        let only_stale = Manifest {
            entries: stale.clone(),
            created_keys: Vec::new(),
            created_events: Vec::new(),
            ..prev.clone()
        };
        let missing = adapter.unmerge(&mut value, &only_stale);
        report.removed = stale
            .iter()
            .filter(|e| !missing.contains(e))
            .map(short)
            .collect();
        report.left_alone = missing.iter().map(short).collect();
        manifest.created_file = prev.created_file;
        manifest.created_keys = prev.created_keys.clone();
        manifest.created_events = prev.created_events.clone();
    }
    let entries = adapter.render_entries();
    let before = value.to_value();
    adapter
        .merge(&mut value, &entries, &mut manifest)
        .map_err(|e| format!("{}: {e}; nothing was changed", path.display()))?;
    report.added = entries
        .iter()
        .filter(|e| !entry_present(&before, e))
        .map(short)
        .collect();
    manifest.entries = entries;

    let new_text = render(&value, file.trailing_newline)?;
    let old_text = std::fs::read_to_string(&path).ok();
    report.changed = old_text.as_deref() != Some(new_text.as_str());
    if dry_run {
        return Ok(report);
    }
    // The manifest goes first: if the config write then fails, `disconnect`
    // reports the recorded entries as not found and changes nothing, while
    // entries written without a manifest would be untracked.
    let manifest_text = serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?;
    let mpath = manifest_path(home, adapter.agent());
    if std::fs::read_to_string(&mpath).ok().as_deref() != Some(manifest_text.as_str()) {
        write_atomic(&mpath, &manifest_text)?;
    }
    if report.changed {
        if file.existed {
            let bak = backup_path(&path);
            if !bak.exists() {
                std::fs::copy(&path, &bak)
                    .map_err(|e| format!("Cannot back up {}: {e}", path.display()))?;
                report.backup = Some(bak);
            }
        }
        write_atomic(&path, &new_text)?;
    }
    report.notes = adapter.notes();
    Ok(report)
}

fn entry_present(value: &serde_json::Value, e: &Entry) -> bool {
    value["hooks"][&e.event]
        .as_array()
        .is_some_and(|a| a.contains(&e.entry))
}

/// Remove what `connect` installed for `adapter`.
pub(crate) fn disconnect(
    adapter: &dyn Adapter,
    home: &Path,
    dry_run: bool,
) -> Result<Report, String> {
    let Some(manifest) = read_manifest(home, adapter.agent())? else {
        return Ok(Report {
            agent: adapter.agent().to_string(),
            config_path: adapter.config_path(home),
            dry_run,
            notes: vec![format!(
                "Nothing to remove: `uteke connect {}` has not been run on this machine.",
                adapter.agent()
            )],
            ..Default::default()
        });
    };
    let path = manifest.config_path.clone();
    let file = read_config(&path)?;
    let mut value = file.value.clone();
    let missing = adapter.unmerge(&mut value, &manifest);
    let mut report = Report {
        agent: adapter.agent().to_string(),
        config_path: path.clone(),
        dry_run,
        removed: manifest
            .entries
            .iter()
            .filter(|e| !missing.contains(e))
            .map(short)
            .collect(),
        left_alone: missing.iter().map(short).collect(),
        ..Default::default()
    };
    let delete_file =
        manifest.created_file && matches!(&value, OJson::Object(e) if e.is_empty()) && file.existed;
    let new_text = render(&value, file.trailing_newline)?;
    let old_text = std::fs::read_to_string(&path).ok();
    report.changed =
        delete_file || (file.existed && old_text.as_deref() != Some(new_text.as_str()));
    if dry_run {
        return Ok(report);
    }
    if delete_file {
        std::fs::remove_file(&path)
            .map_err(|e| format!("Cannot remove {}: {e}", path.display()))?;
    } else if report.changed {
        write_atomic(&path, &new_text)?;
    }
    let mpath = manifest_path(home, adapter.agent());
    std::fs::remove_file(&mpath).map_err(|e| format!("Cannot remove {}: {e}", mpath.display()))?;
    Ok(report)
}

/// The installed entries differ from what this version renders: `doctor`
/// reports it and `uteke connect <agent>` refreshes them.
pub(crate) fn is_stale(adapter: &dyn Adapter, manifest: &Manifest) -> bool {
    manifest.entries != adapter.render_entries()
}

/// The state of one connected agent, for `uteke doctor`.
#[derive(Debug, Serialize)]
pub(crate) struct HookStatus {
    pub agent: String,
    pub config_path: PathBuf,
    /// `ok`, `stale` (this uteke renders other entries: run `uteke connect
    /// <agent>`) or `missing` (some installed entries are gone from the file).
    pub state: &'static str,
    pub installed_by: String,
}

/// Every agent with a connect manifest under `home`.
pub(crate) fn hook_status(home: &Path) -> Vec<HookStatus> {
    let mut out = Vec::new();
    for agent in SUPPORTED {
        let (Some(adapter), Ok(Some(manifest))) = (adapter_for(agent), read_manifest(home, agent))
        else {
            continue;
        };
        let present = read_config(&manifest.config_path)
            .map(|f| f.value.to_value())
            .unwrap_or_default();
        let state = if !manifest.entries.iter().all(|e| entry_present(&present, e)) {
            "missing"
        } else if is_stale(adapter.as_ref(), &manifest) {
            "stale"
        } else {
            "ok"
        };
        out.push(HookStatus {
            agent: agent.to_string(),
            config_path: manifest.config_path.clone(),
            state,
            installed_by: manifest.uteke_version.clone(),
        });
    }
    out
}

/// The `doctor` lines for connected agents; nothing when none is connected.
pub(crate) fn print_hook_status(statuses: &[HookStatus]) {
    if statuses.is_empty() {
        return;
    }
    println!("\nAgent hooks (uteke connect):");
    for s in statuses {
        let what = match s.state {
            "ok" => "\u{2713} up to date".to_string(),
            "stale" => format!(
                "\u{26a0} installed by uteke {}, this version installs different hooks: run `uteke connect {}`",
                s.installed_by, s.agent
            ),
            _ => format!(
                "\u{26a0} some hooks were removed from {}: run `uteke connect {}` to restore them",
                s.config_path.display(),
                s.agent
            ),
        };
        println!("  {}: {what}", s.agent);
    }
}

fn unsupported(agent: &str) -> String {
    format!(
        "`uteke connect` does not support '{agent}' yet. Supported: {}, {}",
        SUPPORTED.join(", "),
        provider::PROVIDER_AGENTS.join(", ")
    )
}

fn print_report(verb: &str, name: &str, report: &Report, json: bool) {
    if json {
        crate::output::print_json(report);
        return;
    }
    let path = report.config_path.display();
    if report.dry_run {
        println!("Dry run: {verb} {name} ({path}), nothing written.");
    }
    for a in &report.added {
        println!("  + {a}");
    }
    for r in &report.removed {
        println!("  - {r}");
    }
    for l in &report.left_alone {
        println!("  ! left alone (changed since uteke added it): {l}");
    }
    if !report.dry_run {
        if report.changed {
            println!("\u{2713} {verb} {name}: {path}");
        } else {
            println!("\u{2713} {verb} {name}: {path} already up to date, nothing changed");
        }
        if let Some(b) = &report.backup {
            println!("  Backup of the original: {}", b.display());
        }
    }
    for n in &report.notes {
        println!("{n}");
    }
}

/// `uteke connect <agent> [--dry-run]`.
pub(crate) fn run_connect(agent: &str, dry_run: bool, json: bool) -> Result<(), String> {
    if provider::PROVIDER_AGENTS.contains(&agent) {
        return provider::connect(agent, dry_run, json);
    }
    let adapter = adapter_for(agent).ok_or_else(|| unsupported(agent))?;
    let home = home_dir()?;
    let mut report = connect(adapter.as_ref(), &home, dry_run)?;
    if !dry_run && !crate::config::Config::load().engine.capture.enabled {
        report.notes.push(
            "Note: capture is off, so the prompt and stop hooks do nothing yet. Set \
             `enabled = true` under [capture] in the global uteke.toml to store turns."
                .to_string(),
        );
    }
    print_report("Connected", adapter.display_name(), &report, json);
    Ok(())
}

/// `uteke disconnect <agent> [--dry-run]`.
pub(crate) fn run_disconnect(agent: &str, dry_run: bool, json: bool) -> Result<(), String> {
    if provider::PROVIDER_AGENTS.contains(&agent) {
        return provider::disconnect(agent, dry_run, json);
    }
    let adapter = adapter_for(agent).ok_or_else(|| unsupported(agent))?;
    let home = home_dir()?;
    let report = disconnect(adapter.as_ref(), &home, dry_run)?;
    print_report("Disconnected", adapter.display_name(), &report, json);
    Ok(())
}

#[cfg(test)]
mod tests;
