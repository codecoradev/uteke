//! Claude Code: hooks live in `~/.claude/settings.json` under `hooks`, as
//! event → `[{matcher?, hooks: [{type: "command", command, timeout}]}]`.
//! Plain stdout of `SessionStart` and `UserPromptSubmit` is added to the
//! context; `Stop` carries `last_assistant_message` (code.claude.com/docs/en/hooks).

use super::{Adapter, Entry, guarded_command};
use std::path::{Path, PathBuf};

pub(crate) struct ClaudeCode;

/// (Claude Code event, `hook-run` event, timeout in seconds)
const EVENTS: &[(&str, &str, u64)] = &[
    ("SessionStart", "session-start", 15),
    ("UserPromptSubmit", "prompt", 10),
    ("Stop", "stop", 15),
];

impl Adapter for ClaudeCode {
    fn agent(&self) -> &'static str {
        "claude"
    }

    fn display_name(&self) -> &'static str {
        "Claude Code"
    }

    fn config_path(&self, home: &Path) -> PathBuf {
        home.join(".claude").join("settings.json")
    }

    fn render_entries(&self) -> Vec<Entry> {
        EVENTS
            .iter()
            .map(|(event, run, timeout)| Entry {
                event: event.to_string(),
                entry: serde_json::json!({
                    "hooks": [{
                        "type": "command",
                        "command": guarded_command(run, self.agent()),
                        "timeout": timeout,
                    }]
                }),
            })
            .collect()
    }
}
