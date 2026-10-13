//! Namespace deprecation signal (#1427, phase 1).
//!
//! Namespaces are deprecated: one brain per database, organised with rooms,
//! tags and author. Phase 1 is a SIGNAL only: writing to a non-default
//! namespace produces a one-sentence warning on every surface, and routing,
//! storage and reads are untouched. The wording lives here, once, so the
//! HTTP server, the MCP server and the CLI cannot drift.
//!
//! Detection rule: a write is deprecated when a namespace is given and it is
//! not exactly [`DEFAULT_NAMESPACE`]. Omitted and `"default"` are the same
//! thing. The match is exact (case and whitespace included) because storage
//! keeps the namespace as given: `"Default"` and `" default"` are different
//! namespaces and are signalled.

use crate::memory::types::DEFAULT_NAMESPACE;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// The deprecation sentence, identical on every surface.
pub const NAMESPACE_DEPRECATION_WARNING: &str = "Namespaces are deprecated and will be removed in a future 0.x release; use rooms, tags and author instead (see https://github.com/codecoradev/uteke/blob/develop/docs/multi-agent.md#namespaces-are-deprecated).";

/// Short pointer appended to MCP/CLI help text.
pub const NAMESPACE_DEPRECATION_HINT: &str = "DEPRECATED: namespaces will be removed in a future 0.x release; prefer rooms, tags and author.";

/// True when a write to `namespace` should carry the deprecation signal.
pub fn is_deprecated_namespace(namespace: Option<&str>) -> bool {
    matches!(namespace, Some(ns) if ns != DEFAULT_NAMESPACE)
}

fn seen() -> &'static Mutex<HashSet<String>> {
    static SEEN: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashSet::new()))
}

/// True the first time `namespace` is seen in this process (log rate limit:
/// one line per distinct namespace). A poisoned lock degrades to "log".
pub fn first_deprecated_write(namespace: &str) -> bool {
    match seen().lock() {
        Ok(mut set) => set.insert(namespace.to_string()),
        Err(_) => true,
    }
}

/// The signal for a write to `namespace`: returns the warning text when the
/// namespace is deprecated and logs it at WARN, once per distinct namespace
/// per process. Call once per write request.
pub fn namespace_write_warning(namespace: Option<&str>) -> Option<&'static str> {
    if !is_deprecated_namespace(namespace) {
        return None;
    }
    if let Some(ns) = namespace
        && first_deprecated_write(ns)
    {
        tracing::warn!(namespace = ns, "{NAMESPACE_DEPRECATION_WARNING}");
    }
    Some(NAMESPACE_DEPRECATION_WARNING)
}

/// Combine an existing warning (e.g. the #1273 embedding warning) with the
/// deprecation signal. With no signal the existing value is returned
/// untouched, so default-namespace responses stay byte-identical.
pub fn merge_warning(existing: Option<String>, deprecation: Option<&str>) -> Option<String> {
    match (existing, deprecation) {
        (e, None) => e,
        (None, Some(d)) => Some(d.to_string()),
        (Some(e), Some(d)) => Some(format!("{e} {d}")),
    }
}

/// Inverse of [`merge_warning`]: remove the deprecation sentence from a
/// merged `warning`, returning what is left (`None` when nothing is). Used by
/// the CLI so the signal stays on stderr and stdout JSON is unchanged.
pub fn strip_deprecation(warning: &str) -> Option<String> {
    let rest = warning
        .strip_suffix(NAMESPACE_DEPRECATION_WARNING)
        .unwrap_or(warning)
        .trim_end();
    if rest.is_empty() {
        None
    } else {
        Some(rest.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_and_omitted_are_not_deprecated() {
        assert!(!is_deprecated_namespace(None));
        assert!(!is_deprecated_namespace(Some("default")));
    }

    #[test]
    fn any_other_namespace_is_deprecated() {
        assert!(is_deprecated_namespace(Some("research-x")));
        assert!(is_deprecated_namespace(Some("shared")));
    }

    #[test]
    fn match_is_exact_on_case_and_whitespace() {
        for ns in ["Default", "DEFAULT", " default", "default ", "", " "] {
            assert!(is_deprecated_namespace(Some(ns)), "{ns:?}");
        }
    }

    #[test]
    fn warning_only_for_deprecated() {
        assert_eq!(namespace_write_warning(None), None);
        assert_eq!(namespace_write_warning(Some("default")), None);
        assert_eq!(
            namespace_write_warning(Some("ns-warn-a")),
            Some(NAMESPACE_DEPRECATION_WARNING)
        );
        // Still returned on repeat; only the log is rate limited.
        assert_eq!(
            namespace_write_warning(Some("ns-warn-a")),
            Some(NAMESPACE_DEPRECATION_WARNING)
        );
    }

    #[test]
    fn log_is_once_per_distinct_namespace() {
        assert!(first_deprecated_write("ns-once-a"));
        assert!(!first_deprecated_write("ns-once-a"));
        assert!(!first_deprecated_write("ns-once-a"));
        assert!(first_deprecated_write("ns-once-b"));
        assert!(!first_deprecated_write("ns-once-b"));
    }

    #[test]
    fn merge_warning_keeps_untouched_without_signal() {
        assert_eq!(merge_warning(None, None), None);
        assert_eq!(
            merge_warning(Some("embed failed".into()), None),
            Some("embed failed".to_string())
        );
        assert_eq!(merge_warning(None, Some("dep")), Some("dep".to_string()));
        assert_eq!(
            merge_warning(Some("embed failed".into()), Some("dep")),
            Some("embed failed dep".to_string())
        );
    }

    #[test]
    fn strip_deprecation_inverts_merge() {
        let merged = merge_warning(
            Some("embed failed".into()),
            Some(NAMESPACE_DEPRECATION_WARNING),
        );
        assert_eq!(
            strip_deprecation(&merged.unwrap()),
            Some("embed failed".to_string())
        );
        assert_eq!(strip_deprecation(NAMESPACE_DEPRECATION_WARNING), None);
        assert_eq!(
            strip_deprecation("embed failed"),
            Some("embed failed".to_string())
        );
    }

    #[test]
    fn sentence_is_single_and_links_the_docs() {
        assert!(NAMESPACE_DEPRECATION_WARNING.contains("deprecated"));
        assert!(NAMESPACE_DEPRECATION_WARNING.contains("rooms, tags and author"));
        assert!(NAMESPACE_DEPRECATION_WARNING.contains("#namespaces-are-deprecated"));
    }
}
