//! Read-only remote surface of the skill library (#1731, stage 2).
//!
//! `POST /skill/search|list|show|similar` and the MCP tools `uteke_skill_search`,
//! `uteke_skill_list`, `uteke_skill_show` and `uteke_skill_similar` all call
//! [`crate::Uteke::skill_remote`] with a [`SkillRemoteArgs`], so the two
//! transports validate and bound a request identically. Nothing here writes:
//! `index`, `approve` and `record` stay CLI-only.
//!
//! # Bounds (a read-only token may call these)
//!
//! - search: query at most [`crate::skill_library::MAX_QUERY_CHARS`] characters, `limit` clamped to
//!   `1..=50` (default 5);
//! - list: `limit` clamped to `1..=200` (default 50), `total` says how many
//!   skills match;
//! - show: a slug of at most [`MAX_SLUG_LEN`] characters; the answer holds at
//!   most 50 versions and 20 ledger rows and the manifest of one package;
//! - similar: a stored skill or a draft text of at most
//!   [`MAX_REMOTE_TEXT_CHARS`] characters (32768; the CLI accepts more),
//!   `limit` clamped to `1..=100` (default 10).
//!
//! A request that embeds a string (`similar` with `text`, `search` in
//! `hybrid` or `semantic` mode) says so through
//! [`SkillRemoteArgs::embed_need`]; the transport takes one of
//! [`crate::remote_guard`]'s slots for it (HTTP 429 / a tool error when they
//! are all taken). `skill_remote` itself takes none.
//!
//! The answers never carry `source_path`, the server directory a skill was
//! indexed from: [`SkillRemoteSummary`] and [`SkillRemoteDetail`] are the
//! remote shape of the CLI's `SkillSummary` and `SkillDetail`.
//!
//! A field that does not belong to the operation is refused, not ignored.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::doc_similarity::{DEFAULT_SIMILAR_LIMIT, MAX_SIMILAR_LIMIT};
use crate::error::Error;
use crate::remote_guard::MAX_REMOTE_DRAFT_CHARS;
use crate::skill_library::{
    DEFAULT_LIST_LIMIT, DEFAULT_SEARCH_LIMIT, LedgerEntry, MAX_LIST_LIMIT, MAX_SEARCH_LIMIT,
    SkillDetail, SkillHit, SkillSimilarReport, SkillSummary, SkillUsage, SkillVersion, VersionLine,
};
use crate::skill_package::MAX_SLUG_LEN;

/// Longest draft text `similar` accepts from a remote caller, in characters
/// (the same cap as `POST /doc/similar`; the CLI keeps its larger one).
pub const MAX_REMOTE_TEXT_CHARS: usize = MAX_REMOTE_DRAFT_CHARS;

/// The four read operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillRemoteOp {
    Search,
    List,
    Show,
    Similar,
}

impl SkillRemoteOp {
    fn name(self) -> &'static str {
        match self {
            Self::Search => "skill search",
            Self::List => "skill list",
            Self::Show => "skill show",
            Self::Similar => "skill similar",
        }
    }
}

/// Every parameter of the four operations; each operation takes its own subset
/// and refuses the rest. Unknown JSON fields are an error.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillRemoteArgs {
    /// search: the query.
    #[serde(default)]
    pub query: Option<String>,
    /// search: `hybrid` (default), `semantic` or `fts`.
    #[serde(default)]
    pub mode: Option<String>,
    /// search: only skills whose current version is approved.
    #[serde(default)]
    pub approved_only: Option<bool>,
    /// list: `draft` or `approved`.
    #[serde(default)]
    pub status: Option<String>,
    /// show, similar: the skill.
    #[serde(default)]
    pub slug: Option<String>,
    /// similar: a draft text that is not stored.
    #[serde(default)]
    pub text: Option<String>,
    /// similar: minimum cosine, 0 to 1.
    #[serde(default)]
    pub threshold: Option<f32>,
    /// search, list, similar: how many rows.
    #[serde(default)]
    pub limit: Option<usize>,
}

impl SkillRemoteArgs {
    /// Parse the arguments of an MCP tool call; the error is a client error.
    pub fn from_json(value: &serde_json::Value) -> Result<Self, Error> {
        serde_json::from_value(value.clone())
            .map_err(|e| Error::validation(format!("invalid arguments: {e}")))
    }

    /// What `op` embeds with these arguments, from the same rules
    /// [`crate::Uteke::skill_remote`] applies: `search` embeds its query unless
    /// the mode is `fts`; `similar` embeds a `text` that comes without a
    /// `slug`. A request that fails validation may still report a need (the
    /// slot is released when it is refused).
    pub fn embed_need(&self, op: SkillRemoteOp) -> Option<crate::remote_guard::EmbedNeed<'_>> {
        match op {
            SkillRemoteOp::Search => self.query.as_ref().and_then(|_| {
                crate::remote_guard::EmbedNeed::for_search_mode(
                    self.mode.as_deref().unwrap_or("hybrid"),
                )
            }),
            SkillRemoteOp::Similar => match (self.slug.as_deref(), self.text.as_deref()) {
                (None, Some(text)) => Some(crate::remote_guard::EmbedNeed::Draft(text)),
                _ => None,
            },
            SkillRemoteOp::List | SkillRemoteOp::Show => None,
        }
    }

    fn present(&self) -> [(&'static str, bool); 8] {
        [
            ("query", self.query.is_some()),
            ("mode", self.mode.is_some()),
            ("approved_only", self.approved_only.is_some()),
            ("status", self.status.is_some()),
            ("slug", self.slug.is_some()),
            ("text", self.text.is_some()),
            ("threshold", self.threshold.is_some()),
            ("limit", self.limit.is_some()),
        ]
    }

    fn only(&self, op: SkillRemoteOp, allowed: &[&str]) -> Result<(), Error> {
        for (field, set) in self.present() {
            if set && !allowed.contains(&field) {
                return Err(Error::validation(format!(
                    "'{field}' is not a parameter of {}",
                    op.name()
                )));
            }
        }
        Ok(())
    }
}

/// The answer of one operation; serializes as the object it holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum SkillRemoteAnswer {
    Search(SkillSearchAnswer),
    List(SkillListAnswer),
    Show(Box<SkillRemoteDetail>),
    Similar(SkillSimilarReport),
}

/// `skill search`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillSearchAnswer {
    pub mode: String,
    pub approved_only: bool,
    /// The limit after clamping.
    pub limit: usize,
    pub count: usize,
    pub hits: Vec<SkillHit>,
}

/// `skill list`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillListAnswer {
    /// The limit after clamping.
    pub limit: usize,
    /// Skills that match the status filter, before the limit.
    pub total: usize,
    pub count: usize,
    pub skills: Vec<SkillRemoteSummary>,
}

/// A skill in a remote listing: [`SkillSummary`] without `source_path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillRemoteSummary {
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub hash: String,
    pub valid: bool,
    pub owner: Option<String>,
    pub updated_at: String,
}

impl From<SkillSummary> for SkillRemoteSummary {
    fn from(s: SkillSummary) -> Self {
        Self {
            slug: s.slug,
            name: s.name,
            description: s.description,
            status: s.status,
            hash: s.hash,
            valid: s.valid,
            owner: s.owner,
            updated_at: s.updated_at,
        }
    }
}

/// Everything remote `skill show` returns: [`SkillDetail`] without
/// `source_path`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillRemoteDetail {
    pub slug: String,
    pub owner: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub current: SkillVersion,
    pub approved_hash: Option<String>,
    pub usage: SkillUsage,
    pub versions: Vec<VersionLine>,
    pub ledger: Vec<LedgerEntry>,
}

impl From<SkillDetail> for SkillRemoteDetail {
    fn from(d: SkillDetail) -> Self {
        Self {
            slug: d.slug,
            owner: d.owner,
            created_at: d.created_at,
            updated_at: d.updated_at,
            current: d.current,
            approved_hash: d.approved_hash,
            usage: d.usage,
            versions: d.versions,
            ledger: d.ledger,
        }
    }
}

fn slug_arg(slug: &str) -> Result<&str, Error> {
    if slug.trim().is_empty() {
        return Err(Error::validation("slug must not be empty"));
    }
    if slug.chars().count() > MAX_SLUG_LEN {
        return Err(Error::validation(format!(
            "slug is longer than {MAX_SLUG_LEN} characters"
        )));
    }
    Ok(slug)
}

impl crate::Uteke {
    /// Run one read-only skill operation for a remote caller. The only entry
    /// point of the HTTP routes and the MCP tools.
    ///
    /// # Errors
    ///
    /// `Validation` for a missing, foreign, empty, over-long or malformed
    /// parameter, `NotFound` for an unknown skill, an embedding error when a
    /// draft text or a semantic search needs a model that is not available.
    pub fn skill_remote(
        &self,
        op: SkillRemoteOp,
        args: &SkillRemoteArgs,
    ) -> Result<SkillRemoteAnswer, Error> {
        match op {
            SkillRemoteOp::Search => {
                args.only(op, &["query", "mode", "approved_only", "limit"])?;
                let query = args
                    .query
                    .as_deref()
                    .ok_or_else(|| Error::validation("'query' is required"))?;
                let mode = args.mode.as_deref().unwrap_or("hybrid");
                let limit = args
                    .limit
                    .unwrap_or(DEFAULT_SEARCH_LIMIT)
                    .clamp(1, MAX_SEARCH_LIMIT);
                let approved_only = args.approved_only.unwrap_or(false);
                let hits = self.skill_search(query, limit, mode, approved_only)?;
                Ok(SkillRemoteAnswer::Search(SkillSearchAnswer {
                    mode: mode.to_string(),
                    approved_only,
                    limit,
                    count: hits.len(),
                    hits,
                }))
            }
            SkillRemoteOp::List => {
                args.only(op, &["status", "limit"])?;
                let limit = args
                    .limit
                    .unwrap_or(DEFAULT_LIST_LIMIT)
                    .clamp(1, MAX_LIST_LIMIT);
                let status = args.status.as_deref();
                let skills = self.skill_list(status, limit)?;
                let total: i64 = self
                    .store
                    .conn
                    .query_row(
                        "SELECT COUNT(*) FROM skills s JOIN skill_versions v \
                         ON v.skill_id = s.id AND v.is_current = 1 \
                         WHERE (?1 IS NULL OR v.status = ?1)",
                        params![status],
                        |r| r.get(0),
                    )
                    .map_err(|e| Error::db("count skills", e))?;
                Ok(SkillRemoteAnswer::List(SkillListAnswer {
                    limit,
                    total: total as usize,
                    count: skills.len(),
                    skills: skills.into_iter().map(Into::into).collect(),
                }))
            }
            SkillRemoteOp::Show => {
                args.only(op, &["slug"])?;
                let slug = args
                    .slug
                    .as_deref()
                    .ok_or_else(|| Error::validation("'slug' is required"))?;
                Ok(SkillRemoteAnswer::Show(Box::new(
                    self.skill_show(slug_arg(slug)?)?.into(),
                )))
            }
            SkillRemoteOp::Similar => {
                args.only(op, &["slug", "text", "threshold", "limit"])?;
                let limit = args
                    .limit
                    .unwrap_or(DEFAULT_SIMILAR_LIMIT)
                    .clamp(1, MAX_SIMILAR_LIMIT);
                let report = match (args.slug.as_deref(), args.text.as_deref()) {
                    (Some(_), Some(_)) => {
                        return Err(Error::validation(
                            "provide either 'slug' or 'text', not both",
                        ));
                    }
                    (None, None) => {
                        return Err(Error::validation("provide 'slug' or 'text'"));
                    }
                    (Some(slug), None) => {
                        self.skill_similar(slug_arg(slug)?, args.threshold, limit)?
                    }
                    (None, Some(text)) => {
                        crate::remote_guard::check_draft_len(text)?;
                        self.skill_similar_to_text(text, args.threshold, limit)?
                    }
                };
                Ok(SkillRemoteAnswer::Similar(report))
            }
        }
    }
}
