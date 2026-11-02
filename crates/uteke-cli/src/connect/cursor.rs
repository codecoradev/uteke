//! Cursor: user hooks live in `~/.cursor/hooks.json` as
//! `{"version": 1, "hooks": {"<event>": [{"command": ..., "timeout": ...}]}}`
//! (cursor.com/docs/hooks). `sessionStart` returns `additional_context`;
//! `beforeSubmitPrompt` carries the prompt and expects `{"continue": true}`;
//! `afterAgentResponse` carries the reply as `text`. `stop` and `sessionEnd`
//! carry no message text, so capture hangs on the prompt and response events.
//! A non-zero exit other than 2 fails open.

use super::ojson::OJson;
use super::{Adapter, Entry, guarded_command_or};
use std::path::{Path, PathBuf};

pub(crate) struct Cursor;

/// What `beforeSubmitPrompt` prints whatever happens, so a prompt is never
/// held back by uteke (also when `uteke` is not on PATH).
pub(crate) const CONTINUE: &str = r#"{"continue":true}"#;

/// (Cursor event, `hook-run` event, timeout in seconds, fallback output)
const EVENTS: &[(&str, &str, u64, Option<&str>)] = &[
    ("sessionStart", "session-start", 15, None),
    ("beforeSubmitPrompt", "prompt", 10, Some(CONTINUE)),
    ("afterAgentResponse", "stop", 15, None),
];

impl Adapter for Cursor {
    fn agent(&self) -> &'static str {
        "cursor"
    }

    fn display_name(&self) -> &'static str {
        "Cursor"
    }

    fn config_path(&self, home: &Path) -> PathBuf {
        home.join(".cursor").join("hooks.json")
    }

    fn render_entries(&self) -> Vec<Entry> {
        EVENTS
            .iter()
            .map(|(event, run, timeout, fallback)| Entry {
                event: event.to_string(),
                entry: serde_json::json!({
                    "command": guarded_command_or(run, self.agent(), *fallback),
                    "timeout": timeout,
                }),
            })
            .collect()
    }

    fn required_keys(&self) -> Vec<(&'static str, OJson)> {
        vec![("version", OJson::Number(1.into()))]
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "Restart Cursor to load the hooks. Some Cursor versions (reported on 3.2.7) drop \
             the session-start context; capture still works. See docs/integrations/cursor.md."
                .to_string(),
        ]
    }
}
