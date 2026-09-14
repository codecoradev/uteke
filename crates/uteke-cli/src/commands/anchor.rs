//! Timestamp anchor support (#1232).
//!
//! Prepend an absolute-date anchor (`[Session date/time: ...]`) to ingested
//! content so lexical+vector recall can answer temporal questions without
//! metadata joins. Measured impact on the internal conversation eval:
//! temporal QA 15% -> 100% with anchors vs without.

use chrono::{DateTime, Utc};

/// Format an anchor header for the given timestamp string.
/// Accepts any RFC 3339 / common datetime format chrono can parse.
pub(crate) fn apply_timestamp_anchor(content: &str, timestamp: &str) -> Result<String, String> {
    if content.trim().is_empty() {
        return Err("Content is empty; cannot anchor".to_string());
    }
    let parsed: DateTime<Utc> = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S")
                .map(|nd| nd.and_utc())
        })
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(timestamp, "%Y-%m-%d")
                .map(|d| d.and_hms_opt(0, 0, 0).unwrap().and_utc())
        })
        .map_err(|e| format!("Invalid --timestamp value '{timestamp}': {e}"))?;
    let header = format!("[Session date/time: {}]\n", parsed.to_rfc3339());
    if content.starts_with("[Session date/time:") {
        // Already anchored — do not double-prefix.
        return Ok(content.to_string());
    }
    Ok(format!("{header}{content}"))
}

#[cfg(test)]
mod tests {
    use super::apply_timestamp_anchor;

    #[test]
    fn prepends_rfc3339_anchor() {
        let out = apply_timestamp_anchor("hello world", "2023-01-20T16:04:00Z").unwrap();
        assert!(out.starts_with("[Session date/time: 2023-01-20T16:04:00+00:00]"));
        assert!(out.contains("hello world"));
    }

    #[test]
    fn accepts_common_formats() {
        for ts in ["2023-01-20 16:04:00", "2023-01-20"] {
            let out = apply_timestamp_anchor("body", ts).unwrap();
            assert!(out.starts_with("[Session date/time: 2023-01-20"));
        }
    }

    #[test]
    fn rejects_garbage_timestamp() {
        assert!(apply_timestamp_anchor("body", "not-a-date").is_err());
    }

    #[test]
    fn rejects_empty_content() {
        assert!(apply_timestamp_anchor("   ", "2023-01-20T00:00:00Z").is_err());
    }

    #[test]
    fn does_not_double_prefix() {
        let anchored = "[Session date/time: 2023-01-20T00:00:00+00:00]\nbody";
        let out = apply_timestamp_anchor(anchored, "2024-01-01T00:00:00Z").unwrap();
        assert_eq!(out, anchored);
    }
}
