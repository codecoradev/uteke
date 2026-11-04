//! Codex: hooks live in `~/.codex/hooks.json` (or `$CODEX_HOME/hooks.json`) in
//! the same shape as Claude Code, with a Windows override `commandWindows`.
//! Plain stdout of `SessionStart` is added as developer context; `Stop` carries
//! `last_assistant_message` and must not print plain text. Hooks that Codex
//! does not manage itself run only after they are trusted once in `/hooks`
//! (learn.chatgpt.com/docs/hooks, formerly developers.openai.com/codex/hooks).

use super::{Adapter, Entry, guarded_command_windows_with, guarded_command_with};
use std::path::{Path, PathBuf};

/// `CODEX_HOME`, read by [`super::adapter_for`] from the environment; tests
/// pass `None` so they never touch a real Codex config.
pub(crate) struct Codex {
    pub codex_home: Option<PathBuf>,
}

impl Codex {
    pub(crate) fn from_env() -> Self {
        Self {
            codex_home: std::env::var_os("CODEX_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }
}

/// (Codex event, `hook-run` event, timeout in seconds)
const EVENTS: &[(&str, &str, u64)] = &[
    ("SessionStart", "session-start", 15),
    ("UserPromptSubmit", "prompt", 10),
    ("Stop", "stop", 15),
];

impl Adapter for Codex {
    fn agent(&self) -> &'static str {
        "codex"
    }

    fn display_name(&self) -> &'static str {
        "Codex"
    }

    fn config_path(&self, home: &Path) -> PathBuf {
        let dir = self
            .codex_home
            .clone()
            .unwrap_or_else(|| home.join(".codex"));
        dir.join("hooks.json")
    }

    fn render_entries_for(&self, bin: Option<&str>) -> Vec<Entry> {
        EVENTS
            .iter()
            .map(|(event, run, timeout)| Entry {
                event: event.to_string(),
                entry: serde_json::json!({
                    "hooks": [{
                        "type": "command",
                        "command": guarded_command_with(run, self.agent(), None, bin),
                        "commandWindows": guarded_command_windows_with(run, self.agent(), bin),
                        "timeout": timeout,
                    }]
                }),
            })
            .collect()
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "One more step: Codex runs new hooks only after you trust them. Start Codex, \
             run /hooks, review the three uteke hooks and trust them."
                .to_string(),
        ]
    }
}
