//! `uteke connect <agent>` / `uteke disconnect <agent>` (#1465): install and
//! remove the lifecycle hooks of an agent app.
//!
//! Each hook entry runs the guarded `uteke hook-run <event>` command, resolved
//! through `PATH` at run time, so nothing in the agent's config changes on an
//! upgrade. `--absolute-path` (#1672) bakes the running binary's path in
//! instead, for users who would rather not trust `PATH`. `connect` records what it added in a manifest
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
    /// The binary path baked into the entries by `--absolute-path` (#1672);
    /// absent for the default `PATH` form, so those manifests are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
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
    /// The entries this version of uteke installs. `bin` is the absolute path of
    /// the binary to run (`--absolute-path`); `None` resolves `uteke` through
    /// `PATH` at run time.
    fn render_entries_for(&self, bin: Option<&str>) -> Vec<Entry>;
    /// The default (`PATH`) entries.
    #[cfg(test)]
    fn render_entries(&self) -> Vec<Entry> {
        self.render_entries_for(None)
    }
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
        "codex" => Some(Box::new(codex::Codex::from_env())),
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
#[cfg(test)]
pub(crate) fn guarded_command(event: &str, agent: &str) -> String {
    guarded_command_with(event, agent, None, None)
}

/// [`guarded_command`] that prints `fallback` when `uteke` is missing, for an
/// event whose agent expects an answer on stdout (Cursor's
/// `beforeSubmitPrompt`). `fallback` is fixed JSON without single quotes or
/// backslashes; it is double-quoted inside the single-quoted `sh -c` script.
#[cfg(test)]
pub(crate) fn guarded_command_or(event: &str, agent: &str, fallback: Option<&str>) -> String {
    guarded_command_with(event, agent, fallback, None)
}

/// Single-quote `s` for a POSIX shell: `'` becomes `'\''`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The guarded command. With `bin` (`--absolute-path`, #1672) the script runs
/// that exact file, if it is still executable, instead of `uteke` from `PATH`;
/// the path is quoted for the inner script and the whole script again for
/// `sh -c`. Without `bin` the text is the one every earlier release wrote.
pub(crate) fn guarded_command_with(
    event: &str,
    agent: &str,
    fallback: Option<&str>,
    bin: Option<&str>,
) -> String {
    let otherwise = match fallback {
        Some(f) => format!("echo \"{}\"", f.replace('"', "\\\"")),
        None => "true".to_string(),
    };
    let script = match bin {
        None => format!(
            "command -v uteke >/dev/null 2>&1 && uteke hook-run {event} --agent {agent} || {otherwise}"
        ),
        Some(bin) => {
            let bin = sh_quote(bin);
            format!("[ -x {bin} ] && {bin} hook-run {event} --agent {agent} || {otherwise}")
        }
    };
    format!("sh -c {}", sh_quote(&script))
}

/// The same guard for PowerShell, for an agent with a Windows command field
/// (Codex's `commandWindows`). With `bin` the path is a PowerShell
/// single-quoted literal (`'` doubled); [`check_binary`] has refused a `"`,
/// which cannot sit in the double-quoted `-Command` argument.
pub(crate) fn guarded_command_windows_with(event: &str, agent: &str, bin: Option<&str>) -> String {
    match bin {
        None => format!(
            "powershell -NoProfile -Command \"if (Get-Command uteke -ErrorAction SilentlyContinue) {{ uteke hook-run {event} --agent {agent} }}; exit 0\""
        ),
        Some(bin) => {
            let bin = format!("'{}'", bin.replace('\'', "''"));
            format!(
                "powershell -NoProfile -Command \"if (Test-Path -LiteralPath {bin} -PathType Leaf) {{ & {bin} hook-run {event} --agent {agent} }}; exit 0\""
            )
        }
    }
}

/// The path `--absolute-path` bakes in: the running binary, which must be UTF-8
/// (it goes into JSON) and free of `"` (it cannot be quoted inside the Windows
/// command).
pub(crate) fn current_binary() -> Result<String, String> {
    let exe = std::env::current_exe()
        .map_err(|e| format!("Cannot find the running uteke binary: {e}"))?;
    check_binary(&exe)
}

fn check_binary(exe: &Path) -> Result<String, String> {
    let s = exe.to_str().ok_or_else(|| {
        format!(
            "{} is not valid UTF-8; cannot use --absolute-path",
            exe.display()
        )
    })?;
    if s.contains('"') {
        return Err(format!(
            "{s} contains a double quote; cannot use --absolute-path"
        ));
    }
    Ok(s.to_string())
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
    /// The bytes as read (`None` when the file did not exist), so the write can
    /// tell that the file changed in between (#1668).
    text: Option<String>,
}

fn read_config(path: &Path) -> Result<ConfigFile, String> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(ConfigFile {
            value: OJson::empty_object(),
            existed: true,
            trailing_newline: true,
            text: Some(text),
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
                text: Some(text),
            })
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFile {
            value: OJson::empty_object(),
            existed: false,
            trailing_newline: true,
            text: None,
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

/// Create the temporary file with the final file's mode (0600 when there is
/// none) from the start, so a private settings file is never briefly readable
/// by others (#1668). A leftover from a crashed run is removed first, which also
/// keeps a planted symlink from being followed.
fn create_temp(tmp: &Path, mode: Option<u32>) -> std::io::Result<std::fs::File> {
    match std::fs::remove_file(tmp) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(mode.unwrap_or(0o600));
    }
    #[cfg(not(unix))]
    let _ = mode;
    opts.open(tmp)
}

/// Write through a temporary file and a rename, so a crash never leaves a
/// half-written config. A symlinked config (a dotfiles checkout) is written
/// at its target: renaming over the link would replace the link itself.
///
/// `read_as` is what the caller read before computing `text` (`None`: the file
/// did not exist). If the file no longer holds exactly that when the write is
/// about to replace it, something else (the agent app itself, say) changed it
/// in between: nothing is written and the error says so, instead of silently
/// dropping the other writer's change. A change in the instant between this
/// check and the rename cannot be excluded without a lock the agent would also
/// have to take.
fn write_atomic(path: &Path, text: &str, read_as: Option<&str>) -> Result<(), String> {
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
    // The rename replaces the file: keep its permissions (a settings file the
    // user made private stays private), from the moment the temp file exists.
    let meta = std::fs::metadata(path).ok();
    #[cfg(unix)]
    let mode = meta.as_ref().map(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o7777
    });
    #[cfg(not(unix))]
    let mode = None;
    let mut file =
        create_temp(&tmp, mode).map_err(|e| format!("Cannot write {}: {e}", tmp.display()))?;
    let written =
        std::io::Write::write_all(&mut file, text.as_bytes()).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("Cannot write {}: {e}", tmp.display()));
    }
    if let Some(meta) = &meta {
        // The umask may have cleared bits the original had: restore it exactly.
        if let Err(e) = std::fs::set_permissions(&tmp, meta.permissions()) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Cannot set permissions on {}: {e}", tmp.display()));
        }
    }
    let now = match std::fs::read_to_string(path) {
        Ok(t) => Some(t),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("Cannot read {}: {e}", path.display()));
        }
    };
    if now.as_deref() != read_as {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!(
            "{} changed while uteke was updating it (another program wrote it); nothing was \
             written. Run the command again.",
            path.display()
        ));
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("Cannot write {}: {e}", path.display())
    })
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

/// Install (or refresh) the hooks of `adapter` under `home`, resolving `uteke`
/// through `PATH`.
#[cfg(test)]
pub(crate) fn connect(adapter: &dyn Adapter, home: &Path, dry_run: bool) -> Result<Report, String> {
    connect_with(adapter, home, dry_run, None)
}

/// [`connect`] with an optional absolute binary path (`--absolute-path`).
pub(crate) fn connect_with(
    adapter: &dyn Adapter,
    home: &Path,
    dry_run: bool,
    bin: Option<&str>,
) -> Result<Report, String> {
    let path = adapter.config_path(home);
    let file = read_config(&path)?;
    let previous = read_manifest(home, adapter.agent())?;
    let mut value = file.value.clone();
    let mut manifest = Manifest {
        agent: adapter.agent().to_string(),
        config_path: path.clone(),
        uteke_version: env!("CARGO_PKG_VERSION").to_string(),
        created_file: !file.existed,
        binary: bin.map(str::to_string),
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
        let entries = adapter.render_entries_for(bin);
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
    let entries = adapter.render_entries_for(bin);
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
    let old_manifest = std::fs::read_to_string(&mpath).ok();
    if old_manifest.as_deref() != Some(manifest_text.as_str()) {
        write_atomic(&mpath, &manifest_text, old_manifest.as_deref())?;
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
        write_atomic(&path, &new_text, file.text.as_deref())?;
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
        write_atomic(&path, &new_text, file.text.as_deref())?;
    }
    let mpath = manifest_path(home, adapter.agent());
    std::fs::remove_file(&mpath).map_err(|e| format!("Cannot remove {}: {e}", mpath.display()))?;
    Ok(report)
}

/// The installed entries differ from what this version renders, or the binary
/// an `--absolute-path` install points at is gone (moved, or replaced by an
/// upgrade): `doctor` reports it and `uteke connect <agent>` (with
/// `--absolute-path` for such an install) refreshes them.
pub(crate) fn is_stale(adapter: &dyn Adapter, manifest: &Manifest) -> bool {
    manifest.entries != adapter.render_entries_for(manifest.binary.as_deref())
        || manifest
            .binary
            .as_deref()
            .is_some_and(|b| !Path::new(b).is_file())
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
    /// The binary baked in by `--absolute-path`, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binary: Option<String>,
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
            binary: manifest.binary.clone(),
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
            "stale" => match &s.binary {
                Some(bin) => format!(
                    "\u{26a0} installed by uteke {}, pinned to {bin}, which is gone or no longer matches: run `uteke connect {} --absolute-path`",
                    s.installed_by, s.agent
                ),
                None => format!(
                    "\u{26a0} installed by uteke {}, this version installs different hooks: run `uteke connect {}`",
                    s.installed_by, s.agent
                ),
            },
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

/// `uteke connect <agent> [--dry-run] [--absolute-path]`.
pub(crate) fn run_connect(
    agent: &str,
    dry_run: bool,
    absolute_path: bool,
    json: bool,
) -> Result<(), String> {
    if provider::PROVIDER_AGENTS.contains(&agent) {
        if absolute_path {
            return Err(format!(
                "--absolute-path applies to the hook agents ({}); {agent} gets a plugin, not a hook command",
                SUPPORTED.join(", ")
            ));
        }
        return provider::connect(agent, dry_run, json);
    }
    let adapter = adapter_for(agent).ok_or_else(|| unsupported(agent))?;
    let home = home_dir()?;
    let bin = if absolute_path {
        Some(current_binary()?)
    } else {
        None
    };
    let mut report = connect_with(adapter.as_ref(), &home, dry_run, bin.as_deref())?;
    if !dry_run {
        let config = crate::config::Config::load();
        if !config.engine.capture.enabled {
            report.notes.push(
                "Note: capture is off, so the prompt and stop hooks do nothing yet. Set \
                 `enabled = true` under [capture] in the global uteke.toml to store turns."
                    .to_string(),
            );
        } else {
            report
                .notes
                .push(uteke_core::capture::extractor_line(&config.extraction));
        }
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
