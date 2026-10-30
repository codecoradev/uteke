//! Which `redacted` / `redactions` flags a memory carries after a write
//! (#1464). The flags describe the CURRENT content, so the rules depend on what
//! the edit touched: [`resolve`] is that policy as one pure function, testable
//! without a store.

use crate::privacy::RedactionCount;

/// What a masked piece of text looks like. An edit that keeps one of these
/// markers (a read-edit-write round trip of masked text) still holds masked
/// content.
const MASK_MARKER: &str = "[REDACTED:";

/// Merge `redacted: true` and `redactions` into `metadata`. `None` becomes a new
/// object; a metadata value that is not an object is left alone (there is
/// nowhere to merge), and the caller still learns of the redaction from the
/// returned counts.
pub fn with_redaction_flags(
    metadata: Option<serde_json::Value>,
    redactions: &[RedactionCount],
) -> Option<serde_json::Value> {
    if redactions.is_empty() {
        return metadata;
    }
    // A JSON `null` (an import row with `"metadata": null`) is no metadata.
    let mut value = match metadata {
        None | Some(serde_json::Value::Null) => serde_json::json!({}),
        Some(v) => v,
    };
    if let serde_json::Value::Object(map) = &mut value {
        map.insert("redacted".into(), serde_json::json!(true));
        map.insert(
            "redactions".into(),
            serde_json::to_value(redactions).unwrap_or_default(),
        );
    }
    Some(value)
}

/// `metadata` without the `redacted` and `redactions` keys, or `None` when it
/// has neither (nothing to change). The flags describe the CURRENT content, so
/// an update to clean content drops the ones an earlier version left; the
/// `redacted` timeline event keeps the history.
pub fn without_redaction_flags(metadata: &serde_json::Value) -> Option<serde_json::Value> {
    let serde_json::Value::Object(map) = metadata else {
        return None;
    };
    if !map.contains_key("redacted") && !map.contains_key("redactions") {
        return None;
    }
    let mut map = map.clone();
    map.remove("redacted");
    map.remove("redactions");
    Some(serde_json::Value::Object(map))
}

/// `new` with the redaction flags of `old` carried over, for an update that
/// replaces the metadata but leaves the (still masked) content alone. `None`
/// when `old` has no flags, `new` is not an object, or `new` sets its own.
pub fn carry_redaction_flags(
    new: &serde_json::Value,
    old: &serde_json::Value,
) -> Option<serde_json::Value> {
    let (serde_json::Value::Object(new_map), serde_json::Value::Object(old_map)) = (new, old)
    else {
        return None;
    };
    if new_map.contains_key("redacted") || new_map.contains_key("redactions") {
        return None;
    }
    let mut merged = new_map.clone();
    let mut carried = false;
    for key in ["redacted", "redactions"] {
        if let Some(v) = old_map.get(key) {
            merged.insert(key.to_string(), v.clone());
            carried = true;
        }
    }
    carried.then_some(serde_json::Value::Object(merged))
}

/// The metadata to write for an update, when the flag policy changes what the
/// caller sent (`None`: write the caller's metadata, if any, as it is).
///
/// - `content`: the new content after masking, when the update has one.
/// - `caller_metadata`: the metadata the caller supplied, after masking.
/// - `existing`: the metadata the memory has now.
/// - `redactions`: what the filter masked in this update (content, tags and
///   metadata together).
///
/// With something masked, the flags are set on the caller's metadata (or the
/// current one). Otherwise: clean new content drops the old flags unless it
/// still holds a mask marker; a metadata-only update keeps the flags that
/// describe the unchanged content; new content with new metadata follows the
/// content, never what the caller's metadata says about itself.
pub(crate) fn resolve(
    content: Option<&str>,
    caller_metadata: Option<&serde_json::Value>,
    existing: &serde_json::Value,
    redactions: &[RedactionCount],
) -> Option<serde_json::Value> {
    if !redactions.is_empty() {
        return with_redaction_flags(
            Some(caller_metadata.cloned().unwrap_or_else(|| existing.clone())),
            redactions,
        );
    }
    match (content, caller_metadata) {
        // Clean new content and no metadata from the caller: the flags a
        // previous version left no longer describe the content, unless the
        // edit kept a mask marker.
        (Some(c), None) if !c.contains(MASK_MARKER) => without_redaction_flags(existing),
        // A metadata-only update keeps the masked content, so it keeps the
        // flags that describe it.
        (None, Some(m)) => carry_redaction_flags(m, existing),
        // New content that still holds a marker, with new metadata: carry the
        // existing flags, ignoring any the caller wrote.
        (Some(c), Some(m)) if c.contains(MASK_MARKER) => {
            let own = without_redaction_flags(m);
            carry_redaction_flags(own.as_ref().unwrap_or(m), existing).or(own)
        }
        // New content and new metadata: the flags follow the content.
        (Some(_), Some(m)) => without_redaction_flags(m),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn flagged() -> serde_json::Value {
        json!({"k": "v", "redacted": true, "redactions": [{"kind": "jwt", "count": 1}]})
    }

    fn counts() -> Vec<RedactionCount> {
        vec![RedactionCount {
            kind: "github-token",
            count: 2,
        }]
    }

    #[test]
    fn something_masked_sets_the_flags_on_the_callers_metadata_or_the_current_one() {
        let existing = json!({"old": 1});
        let out = resolve(Some("x"), Some(&json!({"new": 2})), &existing, &counts()).unwrap();
        assert_eq!(out["new"], 2);
        assert!(
            out.get("old").is_none(),
            "the caller's metadata replaces the current one"
        );
        assert_eq!(out["redacted"], true);
        assert_eq!(out["redactions"][0]["count"], 2);

        let out = resolve(Some("x"), None, &existing, &counts()).unwrap();
        assert_eq!(
            out["old"], 1,
            "no caller metadata: the current one is flagged"
        );
        assert_eq!(out["redacted"], true);
    }

    #[test]
    fn clean_new_content_drops_the_old_flags_but_keeps_the_rest() {
        let out = resolve(Some("plain"), None, &flagged(), &[]).unwrap();
        assert_eq!(out, json!({"k": "v"}));
        // Nothing to drop: leave the metadata alone.
        assert_eq!(resolve(Some("plain"), None, &json!({"k": "v"}), &[]), None);
    }

    #[test]
    fn content_that_still_holds_a_mask_marker_keeps_the_flags() {
        let c = "edited, still ghp_[REDACTED:github-token]";
        assert_eq!(resolve(Some(c), None, &flagged(), &[]), None);
    }

    #[test]
    fn a_metadata_only_update_carries_the_flags_over() {
        let out = resolve(None, Some(&json!({"n": 1})), &flagged(), &[]).unwrap();
        assert_eq!(out["n"], 1);
        assert_eq!(out["redacted"], true);
        // The caller's own flags win: nothing is carried.
        assert_eq!(
            resolve(None, Some(&json!({"redacted": false})), &flagged(), &[]),
            None
        );
        // The memory has no flags: nothing to carry.
        assert_eq!(resolve(None, Some(&json!({"n": 1})), &json!({}), &[]), None);
    }

    #[test]
    fn new_content_with_new_metadata_follows_the_content_not_the_callers_flags() {
        let own = json!({"k": "v", "redacted": true});
        // Clean content: the caller's flag is dropped.
        assert_eq!(
            resolve(Some("plain"), Some(&own), &flagged(), &[]).unwrap(),
            json!({"k": "v"})
        );
        // Masked content marker: the caller's flag is replaced by the existing ones.
        let c = "still [REDACTED:jwt]";
        let out = resolve(Some(c), Some(&own), &flagged(), &[]).unwrap();
        assert_eq!(out["k"], "v");
        assert_eq!(out["redactions"][0]["kind"], "jwt");
    }

    #[test]
    fn no_content_and_no_metadata_changes_nothing() {
        assert_eq!(resolve(None, None, &flagged(), &[]), None);
    }
}
