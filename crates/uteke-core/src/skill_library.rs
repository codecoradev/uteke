//! The personal skill library (#1731).
//!
//! A skill is a directory with a `SKILL.md` (see [`crate::skill_package`]).
//! The library indexes the skills of a machine so an agent can recall the few
//! that matter to a task instead of carrying every name and description in its
//! prompt, keeps a content hash per version, and records which version a human
//! approved: the trust anchor a client checks before it loads a skill.
//!
//! # Tables
//!
//! Same names and fields as the Cloud registry sketch, so moving a library to
//! Cloud is a copy, not a migration:
//!
//! - `skills`: slug, source path, owner.
//! - `skill_versions`: content hash, status, validity, pinned, locked.
//! - `skill_usage`: recalled, loaded and outcome counters per version.
//! - `skill_ledger`: append-only (triggers refuse UPDATE and DELETE) record of
//!   every mutation with the hash before and after.
//!
//! # Search is the document engine
//!
//! Each indexed text file of the current version of a skill is a document with
//! `content_type = 'skill'`, slug `skill/<slug>/<path>`, `SKILL.md` as the root
//! and the other files as its children. One search engine, one embedding
//! pipeline. Skill documents are hidden from `doc list`, `doc search`, recall
//! and the document similarity answers; only the skill commands read them.
//! Only the current version is indexed: a version that is replaced loses its
//! documents (its row stays, as `stale` or `archived`).
//!
//! # Status
//!
//! `draft` (indexed, not approved), `approved` (a human approved this exact
//! hash), `stale` (approved once, replaced by a newer version: the approval
//! does not carry over), `archived` (a draft that was replaced). Exactly one
//! version of a skill is current, and it is `draft` or `approved`. Putting back
//! the bytes of an earlier version makes that version current again, with its
//! approval if it had one.
//!
//! # Bounds
//!
//! The library holds at most [`MAX_SKILLS`] skills; one `index` run reads at
//! most [`crate::skill_package::MAX_SKILLS_PER_INDEX`] and holds one package
//! in memory at a time; every list, search and report has a limit; strings
//! from the caller are length-checked.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};

use crate::doc_similarity::{MAX_SIMILAR_LIMIT, cosine, request_threshold};
use crate::error::Error;
use crate::memory::documents::DocKind;
use crate::skill_package::{self as pkg, LintIssue, ManifestEntry, SKILL_FILE, SkillPackage};

/// Skills in one library.
pub const MAX_SKILLS: usize = 2_000;
/// Default and largest `skill search` limit.
pub const DEFAULT_SEARCH_LIMIT: usize = 5;
pub const MAX_SEARCH_LIMIT: usize = 50;
/// Longest search query, in characters.
pub const MAX_QUERY_CHARS: usize = 2_000;
/// Default and largest `skill list` limit.
pub const DEFAULT_LIST_LIMIT: usize = 50;
pub const MAX_LIST_LIMIT: usize = 200;
/// Longest owner or actor name.
pub const MAX_NAME_CHARS: usize = 128;
/// Shortest hash prefix `approve` accepts.
pub const MIN_HASH_PREFIX: usize = 12;
/// Versions and ledger rows `show` returns.
const SHOW_HISTORY: usize = 50;
const SHOW_LEDGER: usize = 20;
/// Near-duplicates `index` reports per changed skill and in total.
const NEAR_DUPLICATES_PER_SKILL: usize = 3;
const MAX_NEAR_DUPLICATES: usize = 100;
/// Words of a query used for keyword search.
const MAX_QUERY_TERMS: usize = 16;

pub const STATUS_DRAFT: &str = "draft";
pub const STATUS_APPROVED: &str = "approved";
pub const STATUS_STALE: &str = "stale";
pub const STATUS_ARCHIVED: &str = "archived";

/// The skill library tables. Created with the rest of the schema and by the v27
/// migration, so it must stay idempotent.
pub(crate) const SKILL_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS skills (
    id          TEXT PRIMARY KEY,
    slug        TEXT NOT NULL UNIQUE,
    source_path TEXT NOT NULL,
    owner       TEXT,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS skill_versions (
    id              TEXT PRIMARY KEY,
    skill_id        TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    content_hash    TEXT NOT NULL,
    status          TEXT NOT NULL CHECK (status IN ('draft','approved','stale','archived')),
    valid           INTEGER NOT NULL DEFAULT 1,
    validity_issues TEXT NOT NULL DEFAULT '[]',
    pinned          INTEGER NOT NULL DEFAULT 0,
    locked          INTEGER NOT NULL DEFAULT 0,
    is_current      INTEGER NOT NULL DEFAULT 0,
    name            TEXT,
    description     TEXT,
    file_count      INTEGER NOT NULL DEFAULT 0,
    total_bytes     INTEGER NOT NULL DEFAULT 0,
    manifest        TEXT NOT NULL DEFAULT '[]',
    approved_at     TEXT,
    approved_by     TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    UNIQUE (skill_id, content_hash)
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_skill_versions_current
    ON skill_versions(skill_id) WHERE is_current = 1;
CREATE TABLE IF NOT EXISTS skill_usage (
    skill_id     TEXT NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    content_hash TEXT NOT NULL,
    recalled     INTEGER NOT NULL DEFAULT 0,
    loaded       INTEGER NOT NULL DEFAULT 0,
    succeeded    INTEGER NOT NULL DEFAULT 0,
    failed       INTEGER NOT NULL DEFAULT 0,
    last_used_at TEXT,
    PRIMARY KEY (skill_id, content_hash)
);
CREATE TABLE IF NOT EXISTS skill_ledger (
    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
    at          TEXT NOT NULL,
    skill_id    TEXT NOT NULL,
    slug        TEXT NOT NULL,
    action      TEXT NOT NULL,
    before_hash TEXT,
    after_hash  TEXT,
    actor       TEXT,
    detail      TEXT
);
CREATE TRIGGER IF NOT EXISTS skill_ledger_no_update BEFORE UPDATE ON skill_ledger
BEGIN SELECT RAISE(ABORT, 'skill_ledger is append-only'); END;
CREATE TRIGGER IF NOT EXISTS skill_ledger_no_delete BEFORE DELETE ON skill_ledger
BEGIN SELECT RAISE(ABORT, 'skill_ledger is append-only'); END;
";

// ── Result types ──────────────────────────────────────────────────────────

/// One version of a skill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillVersion {
    pub hash: String,
    pub status: String,
    pub valid: bool,
    pub issues: Vec<LintIssue>,
    pub pinned: bool,
    pub locked: bool,
    pub current: bool,
    pub name: Option<String>,
    pub description: Option<String>,
    pub file_count: i64,
    pub total_bytes: i64,
    pub files: Vec<ManifestEntry>,
    pub approved_at: Option<String>,
    pub approved_by: Option<String>,
    pub created_at: String,
}

/// A skill in a listing: the current version in short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillSummary {
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub hash: String,
    pub valid: bool,
    pub source_path: String,
    pub owner: Option<String>,
    pub updated_at: String,
}

/// Counters of one version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SkillUsage {
    pub recalled: i64,
    pub loaded: i64,
    pub succeeded: i64,
    pub failed: i64,
    pub last_used_at: Option<String>,
}

/// A row of the mutation ledger.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LedgerEntry {
    pub seq: i64,
    pub at: String,
    pub action: String,
    pub before_hash: Option<String>,
    pub after_hash: Option<String>,
    pub actor: Option<String>,
}

/// An older version, in short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VersionLine {
    pub hash: String,
    pub status: String,
    pub current: bool,
    pub created_at: String,
}

/// Everything `skill show` knows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillDetail {
    pub slug: String,
    pub source_path: String,
    pub owner: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub current: SkillVersion,
    /// The hash a client may load: the current version's when it is approved.
    pub approved_hash: Option<String>,
    pub usage: SkillUsage,
    /// Newest first, at most 50.
    pub versions: Vec<VersionLine>,
    /// Newest first, at most 20.
    pub ledger: Vec<LedgerEntry>,
}

/// What `index` did with one skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum IndexAction {
    Created,
    Updated,
    /// The bytes of an earlier version are back.
    Reverted,
    Unchanged,
}

/// One skill of an `index` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillIndexEntry {
    pub slug: String,
    pub action: IndexAction,
    pub hash: String,
    pub status: String,
    pub valid: bool,
    /// Lint errors and warnings of `SKILL.md`.
    pub issues: Vec<LintIssue>,
    /// Files in the package / files indexed for search.
    pub files: usize,
    pub indexed_files: usize,
}

/// A skill close to a skill that was just indexed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NearDuplicate {
    pub slug: String,
    pub other: String,
    pub score: f32,
    /// `possible_duplicate` or `similar_to`.
    pub relation: String,
}

/// The result of `skill index`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillIndexReport {
    pub scanned_dirs: usize,
    pub created: usize,
    pub updated: usize,
    pub reverted: usize,
    pub unchanged: usize,
    /// Directories that could not be read as a skill, with the reason.
    pub rejected: Vec<pkg::PackageRejection>,
    /// `true` when a scan bound stopped the run before the end.
    pub truncated: bool,
    pub skills: Vec<SkillIndexEntry>,
    /// Skills that were created or changed and are close to another skill.
    pub near_duplicates: Vec<NearDuplicate>,
    pub warnings: Vec<String>,
}

/// A skill found by `skill search`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillHit {
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub hash: String,
    pub valid: bool,
    pub score: f32,
    /// The file that matched best.
    pub file: String,
    pub snippet: String,
}

/// A skill close to another skill or to a draft text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillSimilarHit {
    pub slug: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub status: String,
    pub score: f32,
    /// `possible_duplicate`, `similar_to`, or `related`.
    pub relation: String,
}

/// Skills close to a skill or a draft text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkillSimilarReport {
    /// The skill asked about; `None` for a draft text.
    pub source: Option<String>,
    pub threshold: f32,
    pub similar_threshold: f32,
    pub duplicate_threshold: f32,
    /// `false` when the source has no usable vector: `hits` is empty.
    pub source_embedded: bool,
    pub skills_without_vector: usize,
    /// At or above the threshold, before the limit.
    pub matches: usize,
    pub hits: Vec<SkillSimilarHit>,
}

/// The result of `skill approve`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillApproval {
    pub slug: String,
    pub hash: String,
    pub status: String,
    /// `false` when the version was already approved.
    pub changed: bool,
}

/// The result of linting a directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillLintEntry {
    pub slug: String,
    pub path: Option<String>,
    pub valid: bool,
    pub issues: Vec<LintIssue>,
}

/// The result of `skill lint`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillLintReport {
    pub scanned_dirs: usize,
    pub truncated: bool,
    pub skills: Vec<SkillLintEntry>,
    pub rejected: Vec<pkg::PackageRejection>,
}

/// What happened to a skill, for the usage counters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UsageEvent {
    Recalled,
    Loaded,
    Succeeded,
    Failed,
}

impl UsageEvent {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "recalled" => Some(Self::Recalled),
            "loaded" => Some(Self::Loaded),
            "succeeded" => Some(Self::Succeeded),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    fn column(self) -> &'static str {
        match self {
            Self::Recalled => "recalled",
            Self::Loaded => "loaded",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// The hex digits of a hash given to `approve`: `sha256:<hex>`, or bare `<hex>`
/// (case-insensitive), at least [`MIN_HASH_PREFIX`] and at most 64 hexadecimal
/// digits. A full hash (64 digits) is the normal case; a shorter one is a
/// prefix of the digits after `sha256:`.
pub fn normalize_hash_input(s: &str) -> Result<String, Error> {
    let lower = s.to_ascii_lowercase();
    let hex = lower.strip_prefix(pkg::HASH_PREFIX).unwrap_or(&lower);
    if hex.len() < MIN_HASH_PREFIX || hex.len() > 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(Error::validation(format!(
            "the hash must be {MIN_HASH_PREFIX} to 64 hexadecimal characters, optionally \
             preceded by '{}'",
            pkg::HASH_PREFIX
        )));
    }
    Ok(hex.to_string())
}

/// Split `slug@hash`, where the hash is `sha256:<hex>`, a bare `<hex>` or a
/// prefix of at least [`MIN_HASH_PREFIX`] hex digits (see
/// [`normalize_hash_input`]). Returns the slug and the lower-case hex digits.
pub fn parse_skill_ref(s: &str) -> Result<(&str, String), Error> {
    let (slug, hash) = s
        .rsplit_once('@')
        .ok_or_else(|| Error::validation("expected <slug>@<hash>"))?;
    if !pkg::is_valid_slug(slug) {
        return Err(Error::validation(format!(
            "'{slug}' is not a valid skill slug"
        )));
    }
    Ok((slug, normalize_hash_input(hash)?))
}

fn check_label(what: &str, v: Option<&str>) -> Result<Option<String>, Error> {
    let Some(v) = v.map(str::trim).filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if v.chars().count() > MAX_NAME_CHARS || v.chars().any(char::is_control) {
        return Err(Error::validation(format!(
            "{what} must be at most {MAX_NAME_CHARS} characters without control characters"
        )));
    }
    Ok(Some(v.to_string()))
}

fn doc_slug(slug: &str, path: &str) -> String {
    format!("skill/{slug}/{path}")
}

/// The skill slug of a skill document slug (`skill/<slug>/<path>`).
fn skill_of_doc_slug(doc_slug: &str) -> Option<&str> {
    doc_slug.strip_prefix("skill/")?.split('/').next()
}

fn db(ctx: &str, e: rusqlite::Error) -> Error {
    Error::db(ctx, e)
}

/// Keyword query of a text: its first words, each quoted, joined with OR.
fn keyword_query(text: &str) -> Option<String> {
    let terms: Vec<String> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .take(MAX_QUERY_TERMS)
        .map(|w| format!("\"{}\"", w.to_lowercase()))
        .collect();
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Lint every skill under `root` without touching any database.
pub fn lint_directory(root: &Path) -> Result<SkillLintReport, Error> {
    let found = pkg::discover_skills(root).map_err(Error::validation)?;
    let mut skills = Vec::new();
    let mut rejected = Vec::new();
    for dir in &found.skills {
        match pkg::read_package(dir) {
            Ok(p) => {
                let text = skill_md(&p);
                let (_, issues) = pkg::lint_skill(&p.slug, text);
                skills.push(SkillLintEntry {
                    slug: p.slug,
                    path: Some(dir.display().to_string()),
                    valid: pkg::is_valid(&issues),
                    issues,
                });
            }
            Err(r) => rejected.push(r),
        }
    }
    Ok(SkillLintReport {
        scanned_dirs: found.scanned_dirs,
        truncated: found.truncated,
        skills,
        rejected,
    })
}

/// The text of `SKILL.md` (`read_package` guarantees it is there and UTF-8).
fn skill_md(p: &SkillPackage) -> &str {
    p.files
        .iter()
        .find(|f| f.path == SKILL_FILE)
        .and_then(|f| std::str::from_utf8(&f.bytes).ok())
        .unwrap_or("")
}

// ── Library ───────────────────────────────────────────────────────────────

/// `skills` row: id, source path, owner, created at, updated at.
type SkillRow = (String, String, Option<String>, String, String);

struct CurrentRow {
    skill_id: String,
    hash: String,
}

impl crate::Uteke {
    fn skill_row(&self, slug: &str) -> Result<Option<SkillRow>, Error> {
        self.store
            .conn
            .query_row(
                "SELECT id, source_path, owner, created_at, updated_at FROM skills WHERE slug = ?1",
                params![slug],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()
            .map_err(|e| db("read skill", e))
    }

    fn current_row(&self, skill_id: &str) -> Result<Option<CurrentRow>, Error> {
        self.store
            .conn
            .query_row(
                "SELECT skill_id, content_hash FROM skill_versions \
                 WHERE skill_id = ?1 AND is_current = 1",
                params![skill_id],
                |r| {
                    Ok(CurrentRow {
                        skill_id: r.get(0)?,
                        hash: r.get(1)?,
                    })
                },
            )
            .optional()
            .map_err(|e| db("read current skill version", e))
    }

    /// The ids of every skill document of `slug` (the files of the indexed
    /// version, and any a failed run left behind).
    fn skill_doc_ids(&self, slug: &str) -> Result<Vec<String>, Error> {
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT id FROM documents WHERE content_type = 'skill' AND slug LIKE ?1 \
                 ORDER BY depth, slug",
            )
            .map_err(|e| db("prepare skill documents", e))?;
        let rows = stmt
            .query_map(params![format!("skill/{slug}/%")], |r| {
                r.get::<_, String>(0)
            })
            .map_err(|e| db("skill documents query", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| db("read skill documents", e))
    }

    fn drop_skill_docs(&self, slug: &str) -> Result<(), Error> {
        for id in self.skill_doc_ids(slug)? {
            // A parent's delete removes its children: a missing id is fine.
            let _ = self.doc_delete(&id);
        }
        Ok(())
    }

    /// Embed and store the indexed files of a package as skill documents.
    fn index_skill_docs(
        &self,
        p: &SkillPackage,
        manifest: &[ManifestEntry],
        title: &str,
        owner: Option<&str>,
    ) -> Result<(), Error> {
        let root_slug = doc_slug(&p.slug, SKILL_FILE);
        // Root first so the children can name it as their parent.
        let mut order: Vec<&ManifestEntry> = manifest.iter().filter(|m| m.indexed).collect();
        order.sort_by_key(|m| m.path != SKILL_FILE);
        for m in order {
            let file = p
                .files
                .iter()
                .find(|f| f.path == m.path)
                .ok_or_else(|| Error::validation("manifest names a file the package lacks"))?;
            let text = std::str::from_utf8(&file.bytes)
                .map_err(|_| Error::validation(format!("{} is not UTF-8", m.path)))?;
            let is_root = m.path == SKILL_FILE;
            self.doc_upsert_full(
                &doc_slug(&p.slug, &m.path),
                if is_root { title } else { &m.path },
                text,
                &[],
                owner,
                if is_root {
                    None
                } else {
                    Some(root_slug.as_str())
                },
                false,
                "skill",
                serde_json::json!({"skill": p.slug, "hash": p.hash, "file": m.path}),
            )?;
        }
        Ok(())
    }

    /// `true` when the documents of the current version are all there.
    fn skill_docs_complete(&self, slug: &str, indexed_files: usize) -> Result<bool, Error> {
        Ok(self.skill_doc_ids(slug)?.len() == indexed_files)
    }

    /// Index one package. Documents first (they are embedded outside any
    /// transaction), then the tables in one transaction. A failure between the
    /// two leaves the old rows and no documents: the next run of the same
    /// package sees the documents missing and rebuilds them.
    pub(crate) fn skill_index_package(
        &self,
        p: &SkillPackage,
        owner: Option<&str>,
        actor: Option<&str>,
    ) -> Result<SkillIndexEntry, Error> {
        let owner = check_label("owner", owner)?;
        let actor = check_label("actor", actor)?;
        let (meta, issues) = pkg::lint_skill(&p.slug, skill_md(p));
        let valid = pkg::is_valid(&issues);
        let manifest = pkg::manifest(&p.files);
        let indexed_files = manifest.iter().filter(|m| m.indexed).count();
        let total_bytes: i64 = p.files.iter().map(|f| f.bytes.len() as i64).sum();
        let source = p.dir.display().to_string();
        let title = meta.name.clone().unwrap_or_else(|| p.slug.clone());
        let now = chrono::Utc::now().to_rfc3339();

        let existing = self.skill_row(&p.slug)?;
        if existing.is_none() {
            let count: i64 = self
                .store
                .conn
                .query_row("SELECT COUNT(*) FROM skills", [], |r| r.get(0))
                .map_err(|e| db("count skills", e))?;
            if count as usize >= MAX_SKILLS {
                return Err(Error::validation(format!(
                    "the library holds {MAX_SKILLS} skills, the most it takes"
                )));
            }
        }
        let current = match &existing {
            Some((id, ..)) => self.current_row(id)?,
            None => None,
        };

        let entry = |action: IndexAction, status: String| SkillIndexEntry {
            slug: p.slug.clone(),
            action,
            hash: p.hash.clone(),
            status,
            valid,
            issues: issues.clone(),
            files: p.files.len(),
            indexed_files,
        };

        // The documents must not collide with an ordinary document.
        for m in manifest.iter().filter(|m| m.indexed) {
            if let Some(d) = self
                .store
                .get_document_by_slug(&doc_slug(&p.slug, &m.path))?
            {
                if d.content_type != "skill" {
                    return Err(Error::validation(format!(
                        "a document already has the slug '{}'",
                        d.slug
                    )));
                }
            }
        }

        // Unchanged bytes: refresh where it lives, repair missing documents.
        if let (Some((skill_id, old_source, old_owner, ..)), Some(cur)) = (&existing, &current) {
            if cur.hash == p.hash {
                if !self.skill_docs_complete(&p.slug, indexed_files)? {
                    self.drop_skill_docs(&p.slug)?;
                    self.index_skill_docs(
                        p,
                        &manifest,
                        &title,
                        owner.as_deref().or(old_owner.as_deref()),
                    )?;
                }
                if *old_source != source || (owner.is_some() && owner != *old_owner) {
                    self.store
                        .conn
                        .execute(
                            "UPDATE skills SET source_path = ?1, owner = COALESCE(?2, owner), \
                             updated_at = ?3 WHERE id = ?4",
                            params![source, owner, now, skill_id],
                        )
                        .map_err(|e| db("update skill", e))?;
                }
                let status: String = self
                    .store
                    .conn
                    .query_row(
                        "SELECT status FROM skill_versions WHERE skill_id = ?1 AND content_hash = ?2",
                        params![skill_id, p.hash],
                        |r| r.get(0),
                    )
                    .map_err(|e| db("read skill version", e))?;
                return Ok(entry(IndexAction::Unchanged, status));
            }
        }

        // New, changed or reverted: the documents of the old version go, those
        // of the new one come.
        self.drop_skill_docs(&p.slug)?;
        let doc_owner = owner
            .as_deref()
            .or(existing.as_ref().and_then(|e| e.2.as_deref()));
        self.index_skill_docs(p, &manifest, &title, doc_owner)?;

        let tx = self
            .store
            .conn
            .unchecked_transaction()
            .map_err(|e| db("begin skill index", e))?;
        let skill_id = match &existing {
            Some((id, ..)) => {
                tx.execute(
                    "UPDATE skills SET source_path = ?1, owner = COALESCE(?2, owner), \
                     updated_at = ?3 WHERE id = ?4",
                    params![source, owner, now, id],
                )
                .map_err(|e| db("update skill", e))?;
                id.clone()
            }
            None => {
                let id = uuid::Uuid::now_v7().to_string();
                tx.execute(
                    "INSERT INTO skills (id, slug, source_path, owner, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    params![id, p.slug, source, owner, now],
                )
                .map_err(|e| db("insert skill", e))?;
                id
            }
        };
        let before_hash = current.as_ref().map(|c| c.hash.clone());
        let mut old_status = None;
        if let Some(cur) = &current {
            // A replaced approval is stale, a replaced draft is archived.
            tx.execute(
                "UPDATE skill_versions SET is_current = 0, updated_at = ?1, \
                 status = CASE status WHEN 'approved' THEN 'stale' ELSE 'archived' END \
                 WHERE skill_id = ?2 AND is_current = 1",
                params![now, cur.skill_id],
            )
            .map_err(|e| db("retire skill version", e))?;
            old_status = Some(
                tx.query_row(
                    "SELECT status FROM skill_versions WHERE skill_id = ?1 AND content_hash = ?2",
                    params![skill_id, cur.hash],
                    |r| r.get::<_, String>(0),
                )
                .map_err(|e| db("read retired version", e))?,
            );
        }
        let issues_json = serde_json::to_string(&issues).unwrap_or_else(|_| "[]".into());
        let manifest_json = serde_json::to_string(&manifest).unwrap_or_else(|_| "[]".into());
        let earlier: Option<String> = tx
            .query_row(
                "SELECT status FROM skill_versions WHERE skill_id = ?1 AND content_hash = ?2",
                params![skill_id, p.hash],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db("read earlier version", e))?;
        let (action, status) = match earlier {
            Some(prev) => {
                // The same bytes are back: an approval of exactly these bytes
                // is still an approval of them.
                let status = if prev == STATUS_STALE {
                    STATUS_APPROVED
                } else {
                    STATUS_DRAFT
                };
                tx.execute(
                    "UPDATE skill_versions SET is_current = 1, status = ?1, valid = ?2, \
                     validity_issues = ?3, name = ?4, description = ?5, file_count = ?6, \
                     total_bytes = ?7, manifest = ?8, updated_at = ?9 \
                     WHERE skill_id = ?10 AND content_hash = ?11",
                    params![
                        status,
                        valid,
                        issues_json,
                        meta.name,
                        meta.description,
                        p.files.len() as i64,
                        total_bytes,
                        manifest_json,
                        now,
                        skill_id,
                        p.hash
                    ],
                )
                .map_err(|e| db("reactivate skill version", e))?;
                (IndexAction::Reverted, status)
            }
            None => {
                tx.execute(
                    "INSERT INTO skill_versions (id, skill_id, content_hash, status, valid, \
                     validity_issues, is_current, name, description, file_count, total_bytes, \
                     manifest, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, 'draft', ?4, ?5, 1, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
                    params![
                        uuid::Uuid::now_v7().to_string(),
                        skill_id,
                        p.hash,
                        valid,
                        issues_json,
                        meta.name,
                        meta.description,
                        p.files.len() as i64,
                        total_bytes,
                        manifest_json,
                        now
                    ],
                )
                .map_err(|e| db("insert skill version", e))?;
                let action = if existing.is_some() {
                    IndexAction::Updated
                } else {
                    IndexAction::Created
                };
                (action, STATUS_DRAFT)
            }
        };
        let action_name = match action {
            IndexAction::Created => "created",
            IndexAction::Updated => "updated",
            IndexAction::Reverted => "reverted",
            IndexAction::Unchanged => unreachable!("handled above"),
        };
        tx.execute(
            "INSERT INTO skill_ledger (at, skill_id, slug, action, before_hash, after_hash, actor, detail) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                now,
                skill_id,
                p.slug,
                action_name,
                before_hash,
                p.hash,
                actor,
                serde_json::json!({"replaced_status": old_status, "status": status}).to_string()
            ],
        )
        .map_err(|e| db("write skill ledger", e))?;
        tx.commit().map_err(|e| db("commit skill index", e))?;
        Ok(entry(action, status.to_string()))
    }

    /// Index the skills under `root`: every skill directory is read once
    /// (nothing on disk is modified), hashed, linted and its text files
    /// embedded. A directory that is not a readable package is listed in
    /// `rejected` and does not stop the run. Needs the embedding model.
    ///
    /// # Errors
    ///
    /// `Validation` when `root` is not a readable directory; the first embedding
    /// or database error aborts the run (skills already indexed stay indexed).
    pub fn skill_index(
        &self,
        root: &Path,
        owner: Option<&str>,
        actor: Option<&str>,
    ) -> Result<SkillIndexReport, Error> {
        // Validate once, before any work.
        check_label("owner", owner)?;
        check_label("actor", actor)?;
        // Record an absolute source path: a relative one is useless from any
        // other working directory (symbolic links are left as named).
        let root = std::path::absolute(root)
            .map_err(|e| Error::validation(format!("cannot resolve {}: {e}", root.display())))?;
        let found = pkg::discover_skills(&root).map_err(Error::validation)?;
        let mut report = SkillIndexReport {
            scanned_dirs: found.scanned_dirs,
            created: 0,
            updated: 0,
            reverted: 0,
            unchanged: 0,
            rejected: Vec::new(),
            truncated: found.truncated,
            skills: Vec::new(),
            near_duplicates: Vec::new(),
            warnings: found.warnings,
        };
        let mut seen: HashMap<String, String> = HashMap::new();
        for dir in &found.skills {
            // One package in memory at a time.
            let package = match pkg::read_package(dir) {
                Ok(p) => p,
                Err(r) => {
                    report.rejected.push(r);
                    continue;
                }
            };
            if let Some(first) = seen.insert(package.slug.clone(), dir.display().to_string()) {
                report.rejected.push(pkg::PackageRejection {
                    path: dir.display().to_string(),
                    reason: format!("slug '{}' is already used by {first}", package.slug),
                });
                continue;
            }
            let entry = self.skill_index_package(&package, owner, actor)?;
            match entry.action {
                IndexAction::Created => report.created += 1,
                IndexAction::Updated => report.updated += 1,
                IndexAction::Reverted => report.reverted += 1,
                IndexAction::Unchanged => report.unchanged += 1,
            }
            report.skills.push(entry);
        }
        report.near_duplicates = self.near_duplicates_of(&report.skills)?;
        Ok(report)
    }

    /// For each skill an index run created or changed, the skills closest to it
    /// at or above the `similar_to` threshold, from one read of the vectors.
    fn near_duplicates_of(&self, entries: &[SkillIndexEntry]) -> Result<Vec<NearDuplicate>, Error> {
        let changed: Vec<&str> = entries
            .iter()
            .filter(|e| e.action != IndexAction::Unchanged)
            .map(|e| e.slug.as_str())
            .collect();
        if changed.is_empty() {
            return Ok(Vec::new());
        }
        let vectors = self
            .store
            .document_vectors_kind(None, None, None, DocKind::Skill)?;
        let settings = self.doc_similarity_settings();
        let mut out = Vec::new();
        for slug in changed {
            let Some(src) = vectors
                .iter()
                .find(|v| skill_of_doc_slug(&v.summary.slug) == Some(slug))
                .and_then(|v| v.vector.as_deref())
            else {
                continue;
            };
            let mut hits: Vec<(f32, &str)> = vectors
                .iter()
                .filter_map(|v| {
                    let other = skill_of_doc_slug(&v.summary.slug)?;
                    if other == slug {
                        return None;
                    }
                    let score = cosine(src, v.vector.as_deref()?)?;
                    (score >= settings.similar_threshold).then_some((score, other))
                })
                .collect();
            hits.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(b.1))
            });
            for (score, other) in hits.into_iter().take(NEAR_DUPLICATES_PER_SKILL) {
                if out.len() >= MAX_NEAR_DUPLICATES {
                    return Ok(out);
                }
                out.push(NearDuplicate {
                    slug: slug.to_string(),
                    other: other.to_string(),
                    score,
                    relation: settings.relation(score).to_string(),
                });
            }
        }
        Ok(out)
    }

    fn version_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SkillVersion> {
        let issues: String = r.get(3)?;
        let manifest: String = r.get(11)?;
        Ok(SkillVersion {
            hash: r.get(0)?,
            status: r.get(1)?,
            valid: r.get::<_, i64>(2)? != 0,
            issues: serde_json::from_str(&issues).unwrap_or_default(),
            pinned: r.get::<_, i64>(4)? != 0,
            locked: r.get::<_, i64>(5)? != 0,
            current: r.get::<_, i64>(6)? != 0,
            name: r.get(7)?,
            description: r.get(8)?,
            file_count: r.get(9)?,
            total_bytes: r.get(10)?,
            files: serde_json::from_str(&manifest).unwrap_or_default(),
            approved_at: r.get(12)?,
            approved_by: r.get(13)?,
            created_at: r.get(14)?,
        })
    }

    const VERSION_COLUMNS: &'static str = "content_hash, status, valid, validity_issues, pinned, \
         locked, is_current, name, description, file_count, total_bytes, manifest, approved_at, \
         approved_by, created_at";

    /// The skills of the library, by slug, optionally of one status of the
    /// current version. `limit` is clamped to `1..=200`.
    pub fn skill_list(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SkillSummary>, Error> {
        if let Some(s) = status {
            if ![STATUS_DRAFT, STATUS_APPROVED].contains(&s) {
                return Err(Error::validation(
                    "status must be 'draft' or 'approved' (the status of a current version)",
                ));
            }
        }
        let limit = limit.clamp(1, MAX_LIST_LIMIT) as i64;
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT s.slug, v.name, v.description, v.status, v.content_hash, v.valid, \
                 s.source_path, s.owner, s.updated_at \
                 FROM skills s JOIN skill_versions v ON v.skill_id = s.id AND v.is_current = 1 \
                 WHERE (?1 IS NULL OR v.status = ?1) ORDER BY s.slug LIMIT ?2",
            )
            .map_err(|e| db("prepare skill list", e))?;
        let rows = stmt
            .query_map(params![status, limit], |r| {
                Ok(SkillSummary {
                    slug: r.get(0)?,
                    name: r.get(1)?,
                    description: r.get(2)?,
                    status: r.get(3)?,
                    hash: r.get(4)?,
                    valid: r.get::<_, i64>(5)? != 0,
                    source_path: r.get(6)?,
                    owner: r.get(7)?,
                    updated_at: r.get(8)?,
                })
            })
            .map_err(|e| db("skill list query", e))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| db("read skill list", e))
    }

    /// Everything the library knows about one skill.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown slug.
    pub fn skill_show(&self, slug: &str) -> Result<SkillDetail, Error> {
        let (id, source_path, owner, created_at, updated_at) = self
            .skill_row(slug)?
            .ok_or_else(|| Error::not_found(format!("skill not found: {slug}")))?;
        let current = self
            .store
            .conn
            .query_row(
                &format!(
                    "SELECT {} FROM skill_versions WHERE skill_id = ?1 AND is_current = 1",
                    Self::VERSION_COLUMNS
                ),
                params![id],
                Self::version_from_row,
            )
            .map_err(|e| db("read skill version", e))?;
        let usage = self
            .store
            .conn
            .query_row(
                "SELECT recalled, loaded, succeeded, failed, last_used_at FROM skill_usage \
                 WHERE skill_id = ?1 AND content_hash = ?2",
                params![id, current.hash],
                |r| {
                    Ok(SkillUsage {
                        recalled: r.get(0)?,
                        loaded: r.get(1)?,
                        succeeded: r.get(2)?,
                        failed: r.get(3)?,
                        last_used_at: r.get(4)?,
                    })
                },
            )
            .optional()
            .map_err(|e| db("read skill usage", e))?
            .unwrap_or_default();
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT content_hash, status, is_current, created_at FROM skill_versions \
                 WHERE skill_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
            )
            .map_err(|e| db("prepare skill versions", e))?;
        let versions = stmt
            .query_map(params![id, SHOW_HISTORY as i64], |r| {
                Ok(VersionLine {
                    hash: r.get(0)?,
                    status: r.get(1)?,
                    current: r.get::<_, i64>(2)? != 0,
                    created_at: r.get(3)?,
                })
            })
            .map_err(|e| db("skill versions query", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| db("read skill versions", e))?;
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT seq, at, action, before_hash, after_hash, actor FROM skill_ledger \
                 WHERE skill_id = ?1 ORDER BY seq DESC LIMIT ?2",
            )
            .map_err(|e| db("prepare skill ledger", e))?;
        let ledger = stmt
            .query_map(params![id, SHOW_LEDGER as i64], |r| {
                Ok(LedgerEntry {
                    seq: r.get(0)?,
                    at: r.get(1)?,
                    action: r.get(2)?,
                    before_hash: r.get(3)?,
                    after_hash: r.get(4)?,
                    actor: r.get(5)?,
                })
            })
            .map_err(|e| db("skill ledger query", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| db("read skill ledger", e))?;
        let approved_hash = (current.status == STATUS_APPROVED).then(|| current.hash.clone());
        Ok(SkillDetail {
            slug: slug.to_string(),
            source_path,
            owner,
            created_at,
            updated_at,
            current,
            approved_hash,
            usage,
            versions,
            ledger,
        })
    }

    fn skill_summary(&self, slug: &str) -> Result<Option<SkillSummary>, Error> {
        self.store
            .conn
            .query_row(
                "SELECT s.slug, v.name, v.description, v.status, v.content_hash, v.valid, \
                 s.source_path, s.owner, s.updated_at \
                 FROM skills s JOIN skill_versions v ON v.skill_id = s.id AND v.is_current = 1 \
                 WHERE s.slug = ?1",
                params![slug],
                |r| {
                    Ok(SkillSummary {
                        slug: r.get(0)?,
                        name: r.get(1)?,
                        description: r.get(2)?,
                        status: r.get(3)?,
                        hash: r.get(4)?,
                        valid: r.get::<_, i64>(5)? != 0,
                        source_path: r.get(6)?,
                        owner: r.get(7)?,
                        updated_at: r.get(8)?,
                    })
                },
            )
            .optional()
            .map_err(|e| db("read skill", e))
    }

    /// Search the library: the skills whose files match `query`, best first.
    /// `mode` is `hybrid` (default), `semantic` or `fts`; `limit` is clamped to
    /// `1..=50`; `approved_only` keeps only skills whose current version is
    /// approved. Read-only. Semantic and hybrid search need the embedding model
    /// (hybrid falls back to keywords without it).
    ///
    /// # Errors
    ///
    /// `Validation` for an empty or over-long query or an unknown mode.
    pub fn skill_search(
        &self,
        query: &str,
        limit: usize,
        mode: &str,
        approved_only: bool,
    ) -> Result<Vec<SkillHit>, Error> {
        if query.trim().is_empty() {
            return Err(Error::validation("query must not be empty"));
        }
        if query.chars().count() > MAX_QUERY_CHARS {
            return Err(Error::validation(format!(
                "query is longer than {MAX_QUERY_CHARS} characters"
            )));
        }
        if !["hybrid", "semantic", "fts"].contains(&mode) {
            return Err(Error::validation("mode must be hybrid, semantic or fts"));
        }
        let limit = limit.clamp(1, MAX_SEARCH_LIMIT);
        // Several files of one skill can match: ask for more files than skills.
        let fetch = (limit * 5).min(50);
        let semantic = match mode {
            "fts" => Vec::new(),
            "semantic" => self.doc_search_semantic(query, fetch, DocKind::Skill)?,
            _ => self
                .doc_search_semantic(query, fetch, DocKind::Skill)
                .unwrap_or_default(),
        };
        let fts = match (mode, keyword_query(query)) {
            ("semantic", _) | (_, None) => Vec::new(),
            (_, Some(q)) => self.doc_search_fts(&q, fetch, DocKind::Skill)?,
        };

        // Rank skills, not files: a skill's score is that of its best files.
        struct Acc {
            score: f32,
            file: String,
            snippet: String,
            best_semantic: f32,
        }
        let mut by_skill: HashMap<String, Acc> = HashMap::new();
        let mut absorb = |list: &[crate::DocumentSearchResult], semantic: bool| {
            let mut seen_rank = 0usize;
            let mut ranked: Vec<String> = Vec::new();
            for r in list {
                let Some(skill) = skill_of_doc_slug(&r.document.slug) else {
                    continue;
                };
                let file = r
                    .document
                    .slug
                    .strip_prefix(&format!("skill/{skill}/"))
                    .unwrap_or("")
                    .to_string();
                let first = !ranked.iter().any(|s| s == skill);
                if first {
                    ranked.push(skill.to_string());
                    seen_rank += 1;
                }
                let contribution = if mode == "hybrid" {
                    if first {
                        1.0 / (60.0 + seen_rank as f32)
                    } else {
                        0.0
                    }
                } else if semantic {
                    if first { r.score } else { 0.0 }
                } else if first {
                    1.0 / seen_rank as f32
                } else {
                    0.0
                };
                let acc = by_skill.entry(skill.to_string()).or_insert(Acc {
                    score: 0.0,
                    file: file.clone(),
                    snippet: r.chunk_snippet.clone(),
                    best_semantic: f32::MIN,
                });
                acc.score += contribution;
                if semantic && r.score > acc.best_semantic {
                    acc.best_semantic = r.score;
                    acc.file = file;
                    acc.snippet = r.chunk_snippet.clone();
                }
            }
        };
        absorb(&semantic, true);
        absorb(&fts, false);

        let mut ranked: Vec<(String, Acc)> = by_skill.into_iter().collect();
        ranked.sort_by(|a, b| {
            b.1.score
                .partial_cmp(&a.1.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        let mut hits = Vec::new();
        for (slug, acc) in ranked {
            let Some(s) = self.skill_summary(&slug)? else {
                continue;
            };
            if approved_only && s.status != STATUS_APPROVED {
                continue;
            }
            hits.push(SkillHit {
                slug,
                name: s.name,
                description: s.description,
                status: s.status,
                hash: s.hash,
                valid: s.valid,
                score: acc.score,
                file: acc.file,
                snippet: acc.snippet,
            });
            if hits.len() == limit {
                break;
            }
        }
        Ok(hits)
    }

    /// Skills close to the skill `slug`: the cosine of the `SKILL.md` document
    /// vectors ([`crate::doc_similarity`]) at or above `threshold` (default the
    /// configured `similar_to`), `possible_duplicate` from the duplicate
    /// threshold. The skill itself is never a hit. Nothing is written.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown skill, `Validation` for a threshold outside
    /// `[0, 1]`.
    pub fn skill_similar(
        &self,
        slug: &str,
        threshold: Option<f32>,
        limit: usize,
    ) -> Result<SkillSimilarReport, Error> {
        let floor = self.skill_floor(threshold)?;
        if self.skill_row(slug)?.is_none() {
            return Err(Error::not_found(format!("skill not found: {slug}")));
        }
        let root = self
            .store
            .get_document_by_slug(&doc_slug(slug, SKILL_FILE))?;
        let source = match &root {
            Some(d) => self
                .store
                .document_vectors_kind(None, Some(&d.id), None, DocKind::Skill)?
                .into_iter()
                .next()
                .and_then(|v| v.vector),
            None => None,
        };
        self.skill_similar_to(Some(slug.to_string()), source, floor, limit)
    }

    /// Skills close to a draft text that is not stored (a skill someone is about
    /// to add): "is there already something like this?". The text is embedded
    /// like a document body. Needs the embedding model.
    pub fn skill_similar_to_text(
        &self,
        text: &str,
        threshold: Option<f32>,
        limit: usize,
    ) -> Result<SkillSimilarReport, Error> {
        let floor = self.skill_floor(threshold)?;
        let source = self.candidate_text_vector(text)?;
        self.skill_similar_to(None, source, floor, limit)
    }

    fn skill_floor(&self, threshold: Option<f32>) -> Result<f32, Error> {
        match threshold {
            Some(t) => request_threshold(t),
            None => Ok(self.doc_similarity_settings().similar_threshold),
        }
    }

    fn skill_similar_to(
        &self,
        source_slug: Option<String>,
        source: Option<Vec<f64>>,
        floor: f32,
        limit: usize,
    ) -> Result<SkillSimilarReport, Error> {
        let limit = limit.clamp(1, MAX_SIMILAR_LIMIT);
        let settings = self.doc_similarity_settings();
        let mut report = SkillSimilarReport {
            source: source_slug.clone(),
            threshold: floor,
            similar_threshold: settings.similar_threshold,
            duplicate_threshold: settings.duplicate_threshold,
            source_embedded: source.is_some(),
            skills_without_vector: 0,
            matches: 0,
            hits: Vec::new(),
        };
        let Some(source) = source else {
            return Ok(report);
        };
        let mut scored: Vec<(f32, String)> = Vec::new();
        for v in self
            .store
            .document_vectors_kind(None, None, None, DocKind::Skill)?
        {
            let Some(other) = skill_of_doc_slug(&v.summary.slug) else {
                continue;
            };
            if Some(other) == source_slug.as_deref() {
                continue;
            }
            let Some(score) = v.vector.as_deref().and_then(|x| cosine(&source, x)) else {
                report.skills_without_vector += 1;
                continue;
            };
            if score >= floor {
                scored.push((score, other.to_string()));
            }
        }
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.1.cmp(&b.1))
        });
        report.matches = scored.len();
        for (score, other) in scored.into_iter().take(limit) {
            let Some(s) = self.skill_summary(&other)? else {
                continue;
            };
            report.hits.push(SkillSimilarHit {
                slug: other,
                name: s.name,
                description: s.description,
                status: s.status,
                score,
                relation: settings.relation(score).to_string(),
            });
        }
        Ok(report)
    }

    /// Record that a human approved version `hash` of skill `slug`: the trust
    /// anchor a client checks before loading it. `hash` is the package hash
    /// ([`crate::skill_package`], `sha256:<hex>`, with or without the prefix)
    /// or a prefix of at least 12 hexadecimal digits of the part after
    /// `sha256:`, which must name exactly one version.
    ///
    /// Only the current version of a skill can be approved, and only a valid
    /// one. Approving an approved version changes nothing and writes no ledger
    /// row.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown skill or a hash no version has; `Validation`
    /// for an ambiguous prefix, a version that was replaced (index the new
    /// bytes and approve those), or an invalid skill.
    pub fn skill_approve(
        &self,
        slug: &str,
        hash: &str,
        actor: Option<&str>,
    ) -> Result<SkillApproval, Error> {
        let actor = check_label("actor", actor)?;
        let hash = normalize_hash_input(hash)?;
        let (skill_id, ..) = self
            .skill_row(slug)?
            .ok_or_else(|| Error::not_found(format!("skill not found: {slug}")))?;
        let tx = self
            .store
            .conn
            .unchecked_transaction()
            .map_err(|e| db("begin skill approve", e))?;
        let mut stmt = tx
            .prepare(
                "SELECT content_hash, status, valid, is_current FROM skill_versions \
                 WHERE skill_id = ?1 AND content_hash LIKE 'sha256:%' \
                 AND substr(content_hash, 8, ?2) = ?3",
            )
            .map_err(|e| db("prepare skill approve", e))?;
        let found: Vec<(String, String, bool, bool)> = stmt
            .query_map(params![skill_id, hash.len() as i64, hash], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get::<_, i64>(2)? != 0,
                    r.get::<_, i64>(3)? != 0,
                ))
            })
            .map_err(|e| db("skill approve query", e))?
            .collect::<Result<_, _>>()
            .map_err(|e| db("read skill approve", e))?;
        drop(stmt);
        let (full, status, valid, current) = match found.as_slice() {
            [] => {
                return Err(Error::not_found(format!(
                    "skill '{slug}' has no version with hash {}{hash}",
                    pkg::HASH_PREFIX
                )));
            }
            [one] => one.clone(),
            _ => {
                return Err(Error::validation(format!(
                    "the hash prefix {hash} names more than one version of '{slug}'"
                )));
            }
        };
        if !current {
            return Err(Error::validation(format!(
                "version {} of '{slug}' was replaced by newer bytes; index the skill again and \
                 approve the current hash",
                &pkg::hash_hex(&full)[..12]
            )));
        }
        if status == STATUS_APPROVED {
            return Ok(SkillApproval {
                slug: slug.to_string(),
                hash: full,
                status,
                changed: false,
            });
        }
        if !valid {
            return Err(Error::validation(format!(
                "'{slug}' is not valid (run `uteke skill lint`); fix it before approving"
            )));
        }
        let now = chrono::Utc::now().to_rfc3339();
        // The last approval this skill had, for the ledger.
        let before: Option<String> = tx
            .query_row(
                "SELECT content_hash FROM skill_versions \
                 WHERE skill_id = ?1 AND approved_at IS NOT NULL ORDER BY approved_at DESC LIMIT 1",
                params![skill_id],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| db("read last approval", e))?;
        tx.execute(
            "UPDATE skill_versions SET status = 'approved', approved_at = ?1, approved_by = ?2, \
             updated_at = ?1 WHERE skill_id = ?3 AND content_hash = ?4",
            params![now, actor, skill_id, full],
        )
        .map_err(|e| db("approve skill version", e))?;
        tx.execute(
            "INSERT INTO skill_ledger (at, skill_id, slug, action, before_hash, after_hash, actor) \
             VALUES (?1, ?2, ?3, 'approved', ?4, ?5, ?6)",
            params![now, skill_id, slug, before, full, actor],
        )
        .map_err(|e| db("write skill ledger", e))?;
        tx.commit().map_err(|e| db("commit skill approve", e))?;
        Ok(SkillApproval {
            slug: slug.to_string(),
            hash: full,
            status: STATUS_APPROVED.to_string(),
            changed: true,
        })
    }

    /// Count an event on the current version of a skill. Returns the counters.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown skill.
    pub fn skill_record_usage(&self, slug: &str, event: UsageEvent) -> Result<SkillUsage, Error> {
        let (id, ..) = self
            .skill_row(slug)?
            .ok_or_else(|| Error::not_found(format!("skill not found: {slug}")))?;
        let cur = self
            .current_row(&id)?
            .ok_or_else(|| Error::not_found(format!("skill not found: {slug}")))?;
        let now = chrono::Utc::now().to_rfc3339();
        let col = event.column();
        self.store
            .conn
            .execute(
                &format!(
                    "INSERT INTO skill_usage (skill_id, content_hash, {col}, last_used_at) \
                     VALUES (?1, ?2, 1, ?3) \
                     ON CONFLICT(skill_id, content_hash) DO UPDATE SET \
                     {col} = MIN({col}, 9223372036854775806) + 1, last_used_at = ?3"
                ),
                params![id, cur.hash, now],
            )
            .map_err(|e| db("record skill usage", e))?;
        Ok(self.skill_show(slug)?.usage)
    }

    /// The stored lint results of every skill in the library: the current
    /// version's issues, skills with errors first. `limit` is clamped to
    /// `1..=200`.
    pub fn skill_lint_library(&self, limit: usize) -> Result<SkillLintReport, Error> {
        let limit = limit.clamp(1, MAX_LIST_LIMIT) as i64;
        let mut stmt = self
            .store
            .conn
            .prepare(
                "SELECT s.slug, s.source_path, v.valid, v.validity_issues \
                 FROM skills s JOIN skill_versions v ON v.skill_id = s.id AND v.is_current = 1 \
                 ORDER BY v.valid, s.slug LIMIT ?1",
            )
            .map_err(|e| db("prepare skill lint", e))?;
        let skills = stmt
            .query_map(params![limit], |r| {
                let issues: String = r.get(3)?;
                Ok(SkillLintEntry {
                    slug: r.get(0)?,
                    path: Some(r.get(1)?),
                    valid: r.get::<_, i64>(2)? != 0,
                    issues: serde_json::from_str(&issues).unwrap_or_default(),
                })
            })
            .map_err(|e| db("skill lint query", e))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| db("read skill lint", e))?;
        Ok(SkillLintReport {
            scanned_dirs: 0,
            truncated: false,
            skills,
            rejected: Vec::new(),
        })
    }
}
