//! Typed `list` request shared by the HTTP server, the MCP server and the
//! CLI-via-server client (#1343, `list` slice; `recall` is the template).
//!
//! A surface decodes its wire format into a [`ListInput`], picks a
//! [`ListPolicy`] and calls [`ListRequest::decode`]. This is an extraction,
//! not a unification: where the surfaces disagree today the disagreement is a
//! [`ListPolicy`] value (or a documented capability difference).
//!
//! | aspect                 | HTTP `/list`              | MCP `uteke_list`   | CLI via server         |
//! |------------------------|---------------------------|--------------------|------------------------|
//! | limit default          | 5 (`default_limit`)       | 20                 | 20 (clap), always sent |
//! | limit cap              | 1000 (#1321)              | none               | none (server caps)     |
//! | bad-typed fields       | 400 (serde error)         | silently defaulted | n/a (typed by clap)    |
//! | `at` (time travel)     | honoured, RFC3339 checked | not accepted       | `--at` forwarded       |
//! | `include_meta` (#1188) | honoured (not with `at`)  | not accepted       | not sent               |
//! | namespace omitted      | all namespaces            | all namespaces     | `"default"` only       |
//! | `full_ids`             | n/a                       | output option      | n/a                    |
//!
//! `namespace: None` means "every namespace"; the CLI resolves its own
//! namespace to `Some(..)` before encoding.

use chrono::{DateTime, Utc};
use std::fmt;

/// Result count the HTTP surface uses when the caller gives no `limit`.
pub const LIST_HTTP_DEFAULT_LIMIT: usize = 5;
/// Result count the MCP tool and the CLI use when the caller gives no `limit`.
pub const LIST_DEFAULT_LIMIT: usize = 20;
/// Cap the HTTP surface puts on `limit` for list-style endpoints (#1321).
pub const LIST_HTTP_MAX_LIMIT: usize = 1000;

/// Where today's surfaces disagree, per surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListPolicy {
    /// `limit` when the caller gives none.
    pub default_limit: usize,
    /// Upper bound applied to `limit`; `None` leaves it unbounded.
    pub max_limit: Option<usize>,
}

impl ListPolicy {
    /// `POST /list`.
    pub const HTTP: Self = Self {
        default_limit: LIST_HTTP_DEFAULT_LIMIT,
        max_limit: Some(LIST_HTTP_MAX_LIMIT),
    };
    /// MCP `uteke_list`.
    pub const MCP: Self = Self {
        default_limit: LIST_DEFAULT_LIMIT,
        max_limit: None,
    };
}

/// Raw, undecoded list arguments as a surface read them.
#[derive(Debug, Clone, Default)]
pub struct ListInput {
    pub tag: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub namespace: Option<String>,
    /// RFC3339 point-in-time.
    pub at: Option<String>,
    /// Ask for the pagination envelope (#1188).
    pub include_meta: bool,
}

impl ListInput {
    /// The `POST /list` body, for clients that call the server (CLI via
    /// server). Nothing is validated here. `tag`, `limit`, `offset` and
    /// `namespace` are always present (`null` when unset); `at` only when set.
    /// `include_meta` is not part of the CLI encoding.
    pub fn to_http_body(&self) -> serde_json::Value {
        let mut body = serde_json::json!({
            "tag": self.tag,
            "limit": self.limit.unwrap_or(LIST_DEFAULT_LIMIT),
            "offset": self.offset.unwrap_or(0),
            "namespace": self.namespace,
        });
        if let Some(a) = &self.at {
            body["at"] = serde_json::json!(a);
        }
        body
    }
}

/// A validation failure. The text is the exact message the surfaces return.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRequestError(String);

impl ListRequestError {
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ListRequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ListRequestError {}

/// Pagination metadata of the `include_meta` envelope (#1188).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListPageMeta {
    pub has_more: bool,
    pub next_offset: Option<usize>,
}

/// A decoded, defaulted and clamped list request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRequest {
    pub tag: Option<String>,
    /// Already defaulted and clamped by the policy.
    pub limit: usize,
    pub offset: usize,
    /// `None` = every namespace.
    pub namespace: Option<String>,
    /// Point-in-time listing when set.
    pub at: Option<DateTime<Utc>>,
    pub include_meta: bool,
}

impl ListRequest {
    /// Decode `input` under `policy`. The only failure is an invalid `at`.
    pub fn decode(input: ListInput, policy: ListPolicy) -> Result<Self, ListRequestError> {
        let limit = input.limit.unwrap_or(policy.default_limit);
        let limit = policy.max_limit.map_or(limit, |max| limit.min(max));
        let at = match input.at {
            None => None,
            Some(ts) => Some(
                DateTime::parse_from_rfc3339(&ts)
                    .map(|dt| dt.with_timezone(&Utc))
                    .map_err(|_| {
                        ListRequestError(format!(
                            "Invalid 'at' timestamp: {ts}. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
                        ))
                    })?,
            ),
        };
        Ok(Self {
            tag: input.tag,
            limit,
            offset: input.offset.unwrap_or(0),
            namespace: input.namespace,
            at,
            include_meta: input.include_meta,
        })
    }

    /// Whether the response is the pagination envelope: `include_meta` is
    /// not supported together with `at` (#1188).
    pub fn wants_envelope(&self) -> bool {
        self.include_meta && self.at.is_none()
    }

    /// `has_more` / `next_offset` for a page of `fetched` rows out of `total`.
    pub fn page_meta(&self, total: usize, fetched: usize) -> ListPageMeta {
        let has_more = self.offset + fetched < total;
        ListPageMeta {
            has_more,
            next_offset: has_more.then_some(self.offset + fetched),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(i: ListInput, p: ListPolicy) -> ListRequest {
        ListRequest::decode(i, p).unwrap()
    }

    #[test]
    fn defaults_differ_per_policy() {
        let h = decode(ListInput::default(), ListPolicy::HTTP);
        let m = decode(ListInput::default(), ListPolicy::MCP);
        assert_eq!((h.limit, h.offset), (5, 0));
        assert_eq!((m.limit, m.offset), (20, 0));
        assert_eq!(
            (h.tag, h.namespace, h.at, h.include_meta),
            (None, None, None, false)
        );
    }

    #[test]
    fn cap_is_http_only_and_boundaries_hold() {
        for (given, http) in [(0, 0), (1, 1), (999, 999), (1000, 1000), (1001, 1000)] {
            let i = ListInput {
                limit: Some(given),
                ..Default::default()
            };
            assert_eq!(decode(i, ListPolicy::HTTP).limit, http);
        }
        let i = ListInput {
            limit: Some(usize::MAX),
            ..Default::default()
        };
        assert_eq!(decode(i.clone(), ListPolicy::HTTP).limit, 1000);
        assert_eq!(decode(i, ListPolicy::MCP).limit, usize::MAX);
    }

    #[test]
    fn at_is_parsed_to_utc_with_the_exact_error() {
        let i = ListInput {
            at: Some("2026-06-01T14:00:00+02:00".into()),
            ..Default::default()
        };
        let r = decode(i, ListPolicy::HTTP);
        assert_eq!(r.at.unwrap().to_rfc3339(), "2026-06-01T12:00:00+00:00");

        let i = ListInput {
            at: Some("nope".into()),
            ..Default::default()
        };
        assert_eq!(
            ListRequest::decode(i, ListPolicy::HTTP)
                .unwrap_err()
                .message(),
            "Invalid 'at' timestamp: nope. Use RFC3339 format (e.g. 2026-06-01T12:00:00Z)"
        );
    }

    #[test]
    fn envelope_is_not_available_with_at() {
        let mut i = ListInput {
            include_meta: true,
            ..Default::default()
        };
        assert!(decode(i.clone(), ListPolicy::HTTP).wants_envelope());
        i.at = Some("2026-06-01T12:00:00Z".into());
        assert!(!decode(i, ListPolicy::HTTP).wants_envelope());
        assert!(!decode(ListInput::default(), ListPolicy::HTTP).wants_envelope());
    }

    #[test]
    fn page_meta_matches_the_old_handler_arithmetic() {
        let r = decode(
            ListInput {
                offset: Some(10),
                ..Default::default()
            },
            ListPolicy::HTTP,
        );
        assert_eq!(
            r.page_meta(25, 5),
            ListPageMeta {
                has_more: true,
                next_offset: Some(15)
            }
        );
        // offset + fetched == total: no more.
        assert_eq!(
            r.page_meta(15, 5),
            ListPageMeta {
                has_more: false,
                next_offset: None
            }
        );
        assert_eq!(
            r.page_meta(0, 0),
            ListPageMeta {
                has_more: false,
                next_offset: None
            }
        );
    }

    #[test]
    fn http_body_matches_the_old_cli_body() {
        let i = ListInput {
            tag: Some("t".into()),
            limit: Some(20),
            offset: Some(3),
            namespace: Some("default".into()),
            ..Default::default()
        };
        assert_eq!(
            i.to_http_body(),
            serde_json::json!({"tag": "t", "limit": 20, "offset": 3, "namespace": "default"})
        );
        let i = ListInput {
            at: Some("2026-06-01T12:00:00Z".into()),
            include_meta: true,
            ..Default::default()
        };
        let b = i.to_http_body();
        assert_eq!(
            b,
            serde_json::json!({"tag": null, "limit": 20, "offset": 0, "namespace": null, "at": "2026-06-01T12:00:00Z"})
        );
    }
}
