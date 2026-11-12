//! Document similarity and near-duplicate detection (#1730).
//!
//! Memories get `similar_to` / `possible_duplicate` edges from
//! [`crate::edges`]; document chunks are deliberately not link targets there.
//! This module answers the same question for documents: which documents are
//! close to this one, which are near-duplicates, and (before anything is
//! stored) which documents are close to a candidate text.
//!
//! # The document vector
//!
//! A document has no embedding of its own. Its vector is the **mean of its
//! chunk vectors, L2-normalised** (the direction of the sum, which is the
//! direction of the mean). The chunk vectors already exist (they are what
//! `doc search` uses and what `uteke repair --reembed` restores), so this costs
//! no extra embedding call and works for restored and imported documents as
//! soon as their chunks are embedded. The vectors are read from SQLite, the
//! source of truth, not from the vector index.
//!
//! A document **without a usable vector** is never a hit and never a source:
//! one with no chunks, with a chunk that has no vector yet (a structural import
//! before `uteke repair --reembed`), with vectors of different sizes (re-embedded
//! by another model) or with an all-zero vector. Such documents are counted in
//! `documents_without_vector` instead of being scored at random, and asking for
//! the neighbours of one returns `source_embedded: false` with no hits.
//!
//! # Edges are derived, not stored
//!
//! A hit carries the edge type the memory graph uses: `possible_duplicate` at
//! or above the duplicate threshold, `similar_to` at or above the similar
//! threshold. Nothing is written: `memory_edges` points at memories, the
//! relation is a pure function of the stored chunk vectors, and a stored copy
//! would go stale on every update, move, delete, re-embed and threshold change.
//! A caller who asks for a threshold below the configured similar one gets
//! `related` hits.
//!
//! # Thresholds
//!
//! Default 0.80 (`similar_to`) and 0.92 (`possible_duplicate`), the values of
//! the memory auto-link, configurable in `[doc_similarity]` of `uteke.toml`.
//! Scores are cosine similarities clamped to `[0, 1]`, compared as `f32` with
//! `>=`: a score exactly at a threshold is a hit.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::edges::{EDGE_POSSIBLE_DUPLICATE, EDGE_SIMILAR_TO};
use crate::error::Error;
use crate::memory::documents::DocumentSummary;

/// Default cosine floor of a `similar_to` document pair (the memory value).
pub const DEFAULT_SIMILAR_THRESHOLD: f32 = 0.80;
/// Default cosine floor of a `possible_duplicate` document pair (the memory value).
pub const DEFAULT_DUPLICATE_THRESHOLD: f32 = 0.92;
/// Relation of a hit that passed a caller-chosen threshold below the configured
/// `similar_to` one.
pub const RELATION_RELATED: &str = "related";
/// Hits returned when a request gives no limit.
pub const DEFAULT_SIMILAR_LIMIT: usize = 10;
/// Most hits one request returns; a larger limit is clamped.
pub const MAX_SIMILAR_LIMIT: usize = 100;
/// Pairs returned by a duplicate report when a request gives no limit.
pub const DEFAULT_REPORT_LIMIT: usize = 50;
/// Most pairs one duplicate report lists; a larger limit is clamped. The
/// groups always cover every pair above the threshold.
pub const MAX_REPORT_LIMIT: usize = 500;
/// Most documents with a vector one duplicate report compares (all pairs are
/// scored, so the work grows with the square). A larger scope is refused with
/// a message asking for a subtree or a namespace.
pub const MAX_REPORT_DOCUMENTS: usize = 2000;
/// Longest candidate text, in characters, `doc_similar_to_text` embeds.
pub const MAX_CANDIDATE_CHARS: usize = 200_000;

// ── Settings ──────────────────────────────────────────────────────────────

/// `[doc_similarity]` in `uteke.toml`: the thresholds of document similarity.
///
/// Both keys are optional. A value that is not a number between 0 and 1 falls
/// back to its default, and a similar threshold above the duplicate threshold
/// resets both to the defaults; each is logged as a warning. (Rejecting the
/// section would skip the whole config file layer, which is a worse answer to
/// a typo in a threshold.) [`Self::validate`] is the strict check.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(from = "RawDocSimilaritySettings")]
pub struct DocSimilaritySettings {
    /// Cosine at or above which two documents are `similar_to`. Default 0.80.
    pub similar_threshold: f32,
    /// Cosine at or above which two documents are `possible_duplicate`.
    /// Default 0.92. Never below `similar_threshold`.
    pub duplicate_threshold: f32,
}

impl Default for DocSimilaritySettings {
    fn default() -> Self {
        Self {
            similar_threshold: DEFAULT_SIMILAR_THRESHOLD,
            duplicate_threshold: DEFAULT_DUPLICATE_THRESHOLD,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
struct RawDocSimilaritySettings {
    similar_threshold: f32,
    duplicate_threshold: f32,
}

impl Default for RawDocSimilaritySettings {
    fn default() -> Self {
        let d = DocSimilaritySettings::default();
        Self {
            similar_threshold: d.similar_threshold,
            duplicate_threshold: d.duplicate_threshold,
        }
    }
}

impl From<RawDocSimilaritySettings> for DocSimilaritySettings {
    fn from(raw: RawDocSimilaritySettings) -> Self {
        let defaults = Self::default();
        let mut s = Self {
            similar_threshold: raw.similar_threshold,
            duplicate_threshold: raw.duplicate_threshold,
        };
        if let Err(e) = check_threshold("doc_similarity.similar_threshold", s.similar_threshold) {
            tracing::warn!("{e}; using {}", defaults.similar_threshold);
            s.similar_threshold = defaults.similar_threshold;
        }
        if let Err(e) = check_threshold("doc_similarity.duplicate_threshold", s.duplicate_threshold)
        {
            tracing::warn!("{e}; using {}", defaults.duplicate_threshold);
            s.duplicate_threshold = defaults.duplicate_threshold;
        }
        if let Err(e) = s.validate() {
            tracing::warn!("{e}; using the defaults of both");
            s = defaults;
        }
        s
    }
}

impl DocSimilaritySettings {
    /// Check both thresholds: finite, within `[0, 1]`, similar not above
    /// duplicate. The message names the key.
    pub fn validate(&self) -> Result<(), String> {
        check_threshold("doc_similarity.similar_threshold", self.similar_threshold)?;
        check_threshold(
            "doc_similarity.duplicate_threshold",
            self.duplicate_threshold,
        )?;
        if self.similar_threshold > self.duplicate_threshold {
            return Err(format!(
                "doc_similarity.similar_threshold ({}) must not be above \
                 doc_similarity.duplicate_threshold ({})",
                self.similar_threshold, self.duplicate_threshold
            ));
        }
        Ok(())
    }

    /// The edge type of a score under these thresholds.
    pub fn relation(&self, score: f32) -> &'static str {
        if score >= self.duplicate_threshold {
            EDGE_POSSIBLE_DUPLICATE
        } else if score >= self.similar_threshold {
            EDGE_SIMILAR_TO
        } else {
            RELATION_RELATED
        }
    }
}

fn check_threshold(name: &str, value: f32) -> Result<(), String> {
    if value.is_finite() && (0.0..=1.0).contains(&value) {
        Ok(())
    } else {
        Err(format!(
            "{name} must be a number between 0 and 1, got {value}"
        ))
    }
}

/// A threshold given by a request: finite and within `[0, 1]`.
fn request_threshold(value: f32) -> Result<f32, Error> {
    check_threshold("threshold", value).map_err(Error::validation)?;
    Ok(value)
}

// ── Vectors ───────────────────────────────────────────────────────────────

/// A document and its vector, `None` when it has no usable one.
#[derive(Debug, Clone)]
pub(crate) struct DocVector {
    pub(crate) summary: DocumentSummary,
    /// Unit vector (`f64`), see the module docs.
    pub(crate) vector: Option<Vec<f64>>,
}

/// Builds a document vector chunk by chunk.
#[derive(Debug, Default)]
pub(crate) struct VectorAccumulator {
    sum: Vec<f64>,
    total: usize,
    embedded: usize,
    unusable: bool,
}

impl VectorAccumulator {
    /// Add one chunk; `None` or an empty slice is a chunk without a vector.
    pub(crate) fn add_chunk(&mut self, embedding: Option<&[f32]>) {
        self.total += 1;
        let Some(v) = embedding.filter(|v| !v.is_empty()) else {
            return;
        };
        if self.sum.is_empty() {
            self.sum = vec![0.0; v.len()];
        } else if self.sum.len() != v.len() {
            self.unusable = true;
            return;
        }
        for (s, x) in self.sum.iter_mut().zip(v) {
            *s += f64::from(*x);
        }
        self.embedded += 1;
    }

    /// The unit vector, or `None` when there is no chunk, a chunk lacks its
    /// vector, the sizes differ or the sum is zero.
    pub(crate) fn finish(self) -> Option<Vec<f64>> {
        if self.total == 0 || self.embedded != self.total || self.unusable {
            return None;
        }
        let norm = self.sum.iter().map(|x| x * x).sum::<f64>().sqrt();
        if !norm.is_finite() || norm == 0.0 {
            return None;
        }
        Some(self.sum.into_iter().map(|x| x / norm).collect())
    }
}

/// Cosine of two unit vectors, clamped to `[0, 1]`; `None` for different sizes.
fn cosine(a: &[f64], b: &[f64]) -> Option<f32> {
    if a.len() != b.len() || a.is_empty() {
        return None;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    Some(dot.clamp(0.0, 1.0) as f32)
}

// ── Result types ──────────────────────────────────────────────────────────

/// One document close to the source.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocSimilarHit {
    pub document: DocumentSummary,
    /// Cosine similarity of the two document vectors, `0.0..=1.0`.
    pub score: f32,
    /// `possible_duplicate`, `similar_to`, or `related` (only when the caller
    /// chose a threshold below the configured `similar_to` one).
    pub relation: String,
}

/// Documents close to an existing document or to a candidate text.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocSimilarReport {
    /// The document asked about; `None` for a candidate text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<DocumentSummary>,
    /// The cosine floor applied (the request's, or the configured `similar_to`).
    pub threshold: f32,
    pub similar_threshold: f32,
    pub duplicate_threshold: f32,
    /// `false` when the source has no usable vector: `hits` is empty.
    pub source_embedded: bool,
    /// Candidate documents skipped because they have no usable vector.
    pub documents_without_vector: usize,
    /// Documents at or above the threshold, before `limit`.
    pub matches: usize,
    /// Best first, at most `limit`.
    pub hits: Vec<DocSimilarHit>,
}

/// Two documents at or above the duplicate threshold.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocDuplicatePair {
    /// The document whose slug sorts first.
    pub a: DocumentSummary,
    pub b: DocumentSummary,
    pub score: f32,
}

/// Near-duplicate documents of a namespace or a subtree.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DocDuplicateReport {
    pub threshold: f32,
    /// Documents in scope.
    pub scanned: usize,
    /// Documents in scope skipped because they have no usable vector.
    pub documents_without_vector: usize,
    /// Pairs at or above the threshold, before `limit`.
    pub total_pairs: usize,
    /// Best first (score, then slugs), at most `limit`.
    pub pairs: Vec<DocDuplicatePair>,
    /// Connected groups of 2 or more slugs over every pair above the
    /// threshold, largest first, slugs sorted.
    pub groups: Vec<Vec<String>>,
}

// ── Ranking (pure) ────────────────────────────────────────────────────────

fn order_by_score(a: (f32, &str), b: (f32, &str)) -> std::cmp::Ordering {
    b.0.partial_cmp(&a.0)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a.1.cmp(b.1))
}

/// Rank `candidates` against `source`: those at or above `threshold`, best
/// first (ties by slug), minus `exclude_id`. Returns the hits before the limit
/// and the number of candidates without a comparable vector.
fn rank_candidates(
    source: &[f64],
    candidates: Vec<DocVector>,
    exclude_id: Option<&str>,
    threshold: f32,
    settings: &DocSimilaritySettings,
) -> (Vec<DocSimilarHit>, usize) {
    let mut hits = Vec::new();
    let mut without_vector = 0;
    for c in candidates {
        if Some(c.summary.id.as_str()) == exclude_id {
            continue;
        }
        let Some(score) = c.vector.as_deref().and_then(|v| cosine(source, v)) else {
            without_vector += 1;
            continue;
        };
        if score >= threshold {
            hits.push(DocSimilarHit {
                relation: settings.relation(score).to_string(),
                document: c.summary,
                score,
            });
        }
    }
    hits.sort_by(|a, b| order_by_score((a.score, &a.document.slug), (b.score, &b.document.slug)));
    (hits, without_vector)
}

/// A pair found by the scan, by reference: the scan can find millions of them
/// (up to every pair of the scope), so owned summaries are built only for the
/// pairs a report returns.
struct RawPair<'a> {
    a: &'a DocumentSummary,
    b: &'a DocumentSummary,
    score: f32,
}

/// Every pair of `docs` at or above `threshold`, best first, `a.slug < b.slug`.
fn duplicate_pairs(docs: &[DocVector], threshold: f32) -> Vec<RawPair<'_>> {
    let with_vector: Vec<(&DocumentSummary, &[f64])> = docs
        .iter()
        .filter_map(|d| d.vector.as_deref().map(|v| (&d.summary, v)))
        .collect();
    let mut pairs = Vec::new();
    for (i, (da, va)) in with_vector.iter().enumerate() {
        for (db, vb) in &with_vector[i + 1..] {
            let Some(score) = cosine(va, vb) else {
                continue;
            };
            if score < threshold {
                continue;
            }
            let (a, b) = if da.slug <= db.slug {
                (da, db)
            } else {
                (db, da)
            };
            pairs.push(RawPair { a, b, score });
        }
    }
    pairs.sort_by(|x, y| {
        order_by_score((x.score, &x.a.slug), (y.score, &y.a.slug))
            .then_with(|| x.b.slug.cmp(&y.b.slug))
    });
    pairs
}

fn find_root<'a>(parent: &mut HashMap<&'a str, &'a str>, x: &'a str) -> &'a str {
    let mut root = x;
    while let Some(&p) = parent.get(root) {
        if p == root {
            break;
        }
        root = p;
    }
    // Path compression.
    let mut cur = x;
    while cur != root {
        let next = parent[cur];
        parent.insert(cur, root);
        cur = next;
    }
    root
}

/// Connected components (2 or more members) of the pair graph, as sorted slug
/// lists, largest first, then by first slug.
fn duplicate_groups<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<Vec<String>> {
    let mut parent: HashMap<&str, &str> = HashMap::new();
    for (a, b) in pairs {
        for s in [a, b] {
            parent.entry(s).or_insert(s);
        }
        let ra = find_root(&mut parent, a);
        let rb = find_root(&mut parent, b);
        if ra != rb {
            parent.insert(ra, rb);
        }
    }
    let members: Vec<&str> = parent.keys().copied().collect();
    let mut groups: HashMap<&str, Vec<String>> = HashMap::new();
    for m in members {
        let root = find_root(&mut parent, m);
        groups.entry(root).or_default().push(m.to_string());
    }
    let mut out: Vec<Vec<String>> = groups.into_values().collect();
    for g in &mut out {
        g.sort();
    }
    out.sort_by(|a, b| b.len().cmp(&a.len()).then_with(|| a[0].cmp(&b[0])));
    out
}

// ── One entry point for the transports ────────────────────────────────────

/// What a similarity request asks for, shared by HTTP (`POST /doc/similar`)
/// and MCP (`uteke_doc_similar`) so both validate and dispatch alike.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocSimilarQuery<'a> {
    /// Documents close to a stored document (slug or id).
    Document(&'a str),
    /// Documents close to a draft text that is not stored.
    Text(&'a str),
    /// Near-duplicate pairs and groups of a namespace or a subtree.
    Report { subtree: Option<&'a str> },
}

impl<'a> DocSimilarQuery<'a> {
    /// Exactly one subject: a document, a text, or `report` (which takes
    /// neither, and is the only one that takes a `subtree`). The error is the
    /// message a transport answers with (HTTP 400).
    pub fn resolve(
        document: Option<&'a str>,
        text: Option<&'a str>,
        report: bool,
        subtree: Option<&'a str>,
    ) -> Result<Self, &'static str> {
        if report {
            if document.is_some() || text.is_some() {
                return Err(
                    "a report takes no 'slug', 'id' or 'text'; use 'subtree' and 'namespace'",
                );
            }
            return Ok(Self::Report { subtree });
        }
        if subtree.is_some() {
            return Err("'subtree' needs \"report\": true");
        }
        match (document, text) {
            (Some(d), None) => Ok(Self::Document(d)),
            (None, Some(t)) => Ok(Self::Text(t)),
            (Some(_), Some(_)) => Err("provide either 'slug' (or 'id') or 'text', not both"),
            (None, None) => Err("provide 'slug' (or 'id'), 'text', or \"report\": true"),
        }
    }
}

/// The answer to a [`DocSimilarQuery`]; serializes as the report it holds.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum DocSimilarAnswer {
    Similar(DocSimilarReport),
    Duplicates(DocDuplicateReport),
}

// ── Uteke API ─────────────────────────────────────────────────────────────

impl crate::Uteke {
    /// Set the `[doc_similarity]` settings. Called from `apply_settings`.
    pub fn set_doc_similarity_settings(&mut self, settings: DocSimilaritySettings) {
        self.doc_similarity_settings = settings;
    }

    /// The `[doc_similarity]` settings in force.
    pub fn doc_similarity_settings(&self) -> DocSimilaritySettings {
        self.doc_similarity_settings
    }

    fn similar_floor(&self, threshold: Option<f32>) -> Result<f32, Error> {
        match threshold {
            Some(t) => request_threshold(t),
            None => Ok(self.doc_similarity_settings.similar_threshold),
        }
    }

    /// Run a [`DocSimilarQuery`]: `limit` `None` is the default of the kind
    /// ([`DEFAULT_SIMILAR_LIMIT`] hits, [`DEFAULT_REPORT_LIMIT`] pairs).
    pub fn doc_similarity_query(
        &self,
        query: &DocSimilarQuery<'_>,
        namespace: Option<&str>,
        threshold: Option<f32>,
        limit: Option<usize>,
    ) -> Result<DocSimilarAnswer, Error> {
        Ok(match *query {
            DocSimilarQuery::Document(d) => DocSimilarAnswer::Similar(self.doc_similar(
                d,
                namespace,
                threshold,
                limit.unwrap_or(DEFAULT_SIMILAR_LIMIT),
            )?),
            DocSimilarQuery::Text(t) => DocSimilarAnswer::Similar(self.doc_similar_to_text(
                t,
                namespace,
                threshold,
                limit.unwrap_or(DEFAULT_SIMILAR_LIMIT),
            )?),
            DocSimilarQuery::Report { subtree } => {
                DocSimilarAnswer::Duplicates(self.doc_duplicate_report(
                    namespace,
                    subtree,
                    threshold,
                    limit.unwrap_or(DEFAULT_REPORT_LIMIT),
                )?)
            }
        })
    }

    /// Documents close to the document `id_or_slug` (a slug or an id).
    ///
    /// `namespace` limits the candidates the way `doc_list` does: `None` is the
    /// global view, a value keeps exact namespace matches (a document without
    /// a namespace is only in the global view). `threshold` defaults to the
    /// configured `similar_to` one; `limit` is clamped to `1..=100`. The
    /// document itself is never a hit. Nothing is written.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown document, `Validation` for a threshold
    /// outside `[0, 1]`.
    pub fn doc_similar(
        &self,
        id_or_slug: &str,
        namespace: Option<&str>,
        threshold: Option<f32>,
        limit: usize,
    ) -> Result<DocSimilarReport, Error> {
        let floor = self.similar_floor(threshold)?;
        let limit = limit.clamp(1, MAX_SIMILAR_LIMIT);
        let doc = self
            .doc_get(id_or_slug)?
            .ok_or_else(|| Error::not_found(format!("document not found: {id_or_slug}")))?;
        let source = self
            .store
            .document_vectors(None, Some(&doc.id), None)?
            .into_iter()
            .next()
            .ok_or_else(|| Error::not_found(format!("document not found: {id_or_slug}")))?;
        let settings = self.doc_similarity_settings;
        let Some(source_vector) = source.vector.as_deref() else {
            return Ok(DocSimilarReport {
                source: Some(source.summary),
                threshold: floor,
                similar_threshold: settings.similar_threshold,
                duplicate_threshold: settings.duplicate_threshold,
                source_embedded: false,
                documents_without_vector: 0,
                matches: 0,
                hits: Vec::new(),
            });
        };
        let candidates = self.store.document_vectors(namespace, None, None)?;
        let (mut hits, without_vector) =
            rank_candidates(source_vector, candidates, Some(&doc.id), floor, &settings);
        let matches = hits.len();
        hits.truncate(limit);
        Ok(DocSimilarReport {
            source: Some(source.summary),
            threshold: floor,
            similar_threshold: settings.similar_threshold,
            duplicate_threshold: settings.duplicate_threshold,
            source_embedded: true,
            documents_without_vector: without_vector,
            matches,
            hits,
        })
    }

    /// Documents close to a text that is not stored (a draft): "is there
    /// already something like this?". The text is chunked and embedded exactly
    /// like a document body and compared with the document vectors, so the
    /// score equals the one the document would get once stored. Nothing is
    /// written. Needs the embedding model.
    ///
    /// # Errors
    ///
    /// `Validation` for an empty or oversized text or a threshold outside
    /// `[0, 1]`; an embedding error when the model is unavailable.
    pub fn doc_similar_to_text(
        &self,
        text: &str,
        namespace: Option<&str>,
        threshold: Option<f32>,
        limit: usize,
    ) -> Result<DocSimilarReport, Error> {
        let floor = self.similar_floor(threshold)?;
        let limit = limit.clamp(1, MAX_SIMILAR_LIMIT);
        if text.trim().is_empty() {
            return Err(Error::validation("text must not be empty"));
        }
        if text.chars().count() > MAX_CANDIDATE_CHARS {
            return Err(Error::validation(format!(
                "text is longer than {MAX_CANDIDATE_CHARS} characters"
            )));
        }
        let chunks = self.embed_document_chunks("", text, &[])?;
        let mut acc = VectorAccumulator::default();
        for (_, embedding) in &chunks {
            acc.add_chunk(Some(embedding));
        }
        let settings = self.doc_similarity_settings;
        let source_vector = acc.finish();
        let (mut hits, without_vector, source_embedded) = match source_vector.as_deref() {
            Some(v) => {
                let candidates = self.store.document_vectors(namespace, None, None)?;
                let (hits, without) = rank_candidates(v, candidates, None, floor, &settings);
                (hits, without, true)
            }
            None => (Vec::new(), 0, false),
        };
        let matches = hits.len();
        hits.truncate(limit);
        Ok(DocSimilarReport {
            source: None,
            threshold: floor,
            similar_threshold: settings.similar_threshold,
            duplicate_threshold: settings.duplicate_threshold,
            source_embedded,
            documents_without_vector: without_vector,
            matches,
            hits,
        })
    }

    /// Near-duplicate documents of a namespace, or of the subtree under
    /// `subtree` (that document and all its descendants).
    ///
    /// Every pair inside the scope at or above `threshold` (default: the
    /// configured `possible_duplicate` one) is a duplicate pair; `groups`
    /// merges the pairs that share a document. `limit` (clamped to `1..=500`)
    /// bounds the pairs listed, not the groups. With both `namespace` and
    /// `subtree` the scope is their intersection. Nothing is written.
    ///
    /// # Errors
    ///
    /// `NotFound` for an unknown `subtree`, `Validation` for a threshold
    /// outside `[0, 1]` or a scope of more than 2000 documents with vectors.
    pub fn doc_duplicate_report(
        &self,
        namespace: Option<&str>,
        subtree: Option<&str>,
        threshold: Option<f32>,
        limit: usize,
    ) -> Result<DocDuplicateReport, Error> {
        let settings = self.doc_similarity_settings;
        let floor = match threshold {
            Some(t) => request_threshold(t)?,
            None => settings.duplicate_threshold,
        };
        let limit = limit.clamp(1, MAX_REPORT_LIMIT);
        let root_doc = match subtree {
            Some(root) => Some(
                self.doc_get(root)?
                    .ok_or_else(|| Error::not_found(format!("document not found: {root}")))?,
            ),
            None => None,
        };
        let pattern = root_doc
            .as_ref()
            .map(crate::memory::Store::subtree_like_pattern);
        let scope = root_doc
            .as_ref()
            .zip(pattern.as_deref())
            .map(|(d, p)| (d.id.as_str(), p));
        let docs = self.store.document_vectors(namespace, None, scope)?;
        let scanned = docs.len();
        let with_vector = docs.iter().filter(|d| d.vector.is_some()).count();
        if with_vector > MAX_REPORT_DOCUMENTS {
            return Err(Error::validation(format!(
                "{with_vector} documents with vectors are more than a report compares \
                 ({MAX_REPORT_DOCUMENTS}): narrow it with a subtree or a namespace"
            )));
        }
        let all_pairs = duplicate_pairs(&docs, floor);
        let groups = duplicate_groups(
            all_pairs
                .iter()
                .map(|p| (p.a.slug.as_str(), p.b.slug.as_str())),
        );
        let total_pairs = all_pairs.len();
        let pairs = all_pairs
            .iter()
            .take(limit)
            .map(|p| DocDuplicatePair {
                a: p.a.clone(),
                b: p.b.clone(),
                score: p.score,
            })
            .collect();
        Ok(DocDuplicateReport {
            threshold: floor,
            scanned,
            documents_without_vector: scanned - with_vector,
            total_pairs,
            pairs,
            groups,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::Embedder;
    use crate::test_support::open_with_embedder;

    /// Maps a marker word in the text to an exact vector, so the cosines of the
    /// fixtures are exact: `R43` is `[4, 3, 0]`, whose cosine with `AXIS`
    /// `[1, 0, 0]` is 4/5 = 0.8, `R247` is 24/25 = 0.96, and so on.
    struct MarkerEmbedder;

    const MARKERS: [(&str, [f32; 3]); 7] = [
        ("AXIS", [1.0, 0.0, 0.0]),
        ("R43", [4.0, 3.0, 0.0]),
        ("R34", [3.0, 4.0, 0.0]),
        ("R247", [24.0, 7.0, 0.0]),
        ("R724", [7.0, 24.0, 0.0]),
        ("R01", [0.0, 1.0, 0.0]),
        // Padding of the long-text test.
        ("xxx", [0.0, 0.0, 1.0]),
    ];

    impl Embedder for MarkerEmbedder {
        fn embed(&self, text: &str) -> Result<Vec<f32>, Error> {
            MARKERS
                .iter()
                .find(|(m, _)| text.contains(m))
                .map(|(_, v)| v.to_vec())
                .ok_or_else(|| Error::validation(format!("no marker in {text:?}")))
        }
        fn dims(&self) -> usize {
            3
        }
        fn max_seq_len(&self) -> usize {
            128
        }
        fn name(&self) -> &str {
            "marker-test-embedder"
        }
    }

    fn open() -> crate::Uteke {
        open_with_embedder(Box::new(MarkerEmbedder))
    }

    fn add(u: &crate::Uteke, slug: &str, marker: &str) {
        u.doc_upsert(slug, slug, &format!("# {slug}\n\n{marker}"), &[], None)
            .unwrap();
    }

    fn add_child(u: &crate::Uteke, slug: &str, marker: &str, parent: &str) {
        u.doc_upsert_with_parent(
            slug,
            slug,
            &format!("# {slug}\n\n{marker}"),
            &[],
            None,
            Some(parent),
        )
        .unwrap();
    }

    /// A document stored with no vectors, like a structural import.
    fn add_unembedded(u: &crate::Uteke, slug: &str) {
        u.doc_upsert_inner(
            slug,
            slug,
            &format!("# {slug}\n\nAXIS"),
            &[],
            None,
            None,
            true,
        )
        .unwrap();
    }

    fn set_namespace(u: &crate::Uteke, slug: &str, ns: &str) {
        let n = u
            .store
            .conn
            .execute(
                "UPDATE documents SET namespace = ?1 WHERE slug = ?2",
                rusqlite::params![ns, slug],
            )
            .unwrap();
        assert_eq!(n, 1, "{slug}");
    }

    fn next_up(x: f32) -> f32 {
        f32::from_bits(x.to_bits() + 1)
    }

    fn next_down(x: f32) -> f32 {
        f32::from_bits(x.to_bits() - 1)
    }

    fn seen(r: &DocSimilarReport) -> Vec<(String, f32, String)> {
        r.hits
            .iter()
            .map(|h| (h.document.slug.clone(), h.score, h.relation.clone()))
            .collect()
    }

    fn row(slug: &str, score: f32, relation: &str) -> (String, f32, String) {
        (slug.to_string(), score, relation.to_string())
    }

    /// `src` and five neighbours at the cosines 1.0, 0.96, 0.8, 0.6, 0.28, 0.0.
    fn library() -> crate::Uteke {
        let u = open();
        add(&u, "src", "AXIS");
        add(&u, "dup-same", "AXIS");
        add(&u, "dup-near", "R247");
        add(&u, "sim-edge", "R43");
        add(&u, "low", "R34");
        add(&u, "far", "R724");
        add(&u, "orth", "R01");
        u
    }

    // ── settings ──────────────────────────────────────────────────────────

    #[test]
    fn defaults_are_the_memory_thresholds() {
        let s = DocSimilaritySettings::default();
        assert_eq!(s.similar_threshold, 0.80);
        assert_eq!(s.duplicate_threshold, 0.92);
        assert_eq!(DEFAULT_SIMILAR_THRESHOLD, 0.80);
        assert_eq!(DEFAULT_DUPLICATE_THRESHOLD, 0.92);
        s.validate().unwrap();
    }

    #[test]
    fn relation_at_just_below_and_just_above_each_threshold() {
        let s = DocSimilaritySettings::default();
        assert_eq!(s.relation(1.0), "possible_duplicate");
        assert_eq!(s.relation(next_up(0.92)), "possible_duplicate");
        assert_eq!(s.relation(0.92), "possible_duplicate");
        assert_eq!(s.relation(next_down(0.92)), "similar_to");
        assert_eq!(s.relation(next_up(0.80)), "similar_to");
        assert_eq!(s.relation(0.80), "similar_to");
        assert_eq!(s.relation(next_down(0.80)), "related");
        assert_eq!(s.relation(0.0), "related");
    }

    #[test]
    fn settings_accept_the_edges_of_the_range_and_equal_thresholds() {
        for (a, b) in [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (0.5, 0.5)] {
            let s = DocSimilaritySettings {
                similar_threshold: a,
                duplicate_threshold: b,
            };
            assert_eq!(s.validate(), Ok(()), "{a} {b}");
        }
    }

    #[test]
    fn settings_reject_bad_values_naming_the_key() {
        let bad = |similar: f32, duplicate: f32| {
            DocSimilaritySettings {
                similar_threshold: similar,
                duplicate_threshold: duplicate,
            }
            .validate()
            .unwrap_err()
        };
        assert!(bad(-0.1, 0.9).contains("doc_similarity.similar_threshold"));
        assert!(bad(0.5, 1.1).contains("doc_similarity.duplicate_threshold"));
        assert!(bad(f32::NAN, 0.9).contains("similar_threshold"));
        assert!(bad(0.5, f32::INFINITY).contains("duplicate_threshold"));
        let order = bad(0.95, 0.9);
        assert!(order.contains("must not be above"), "{order}");
    }

    #[test]
    fn toml_sections_fill_defaults_and_fall_back_on_invalid_values() {
        let parse = |t: &str| toml::from_str::<DocSimilaritySettings>(t).unwrap();
        assert_eq!(parse(""), DocSimilaritySettings::default());
        let s = parse("similar_threshold = 0.7");
        assert_eq!((s.similar_threshold, s.duplicate_threshold), (0.7, 0.92));
        let s = parse("duplicate_threshold = 0.85");
        assert_eq!((s.similar_threshold, s.duplicate_threshold), (0.8, 0.85));
        // One bad key falls back to its own default.
        let s = parse("similar_threshold = 0.7\nduplicate_threshold = 1.5");
        assert_eq!((s.similar_threshold, s.duplicate_threshold), (0.7, 0.92));
        let s = parse("similar_threshold = -1.0\nduplicate_threshold = 0.95");
        assert_eq!((s.similar_threshold, s.duplicate_threshold), (0.8, 0.95));
        let s = parse("similar_threshold = nan");
        assert_eq!(s, DocSimilaritySettings::default());
        // A wrong order resets both.
        let s = parse("similar_threshold = 0.95\nduplicate_threshold = 0.9");
        assert_eq!(s, DocSimilaritySettings::default());
        let s = parse("similar_threshold = 0.95");
        assert_eq!(s, DocSimilaritySettings::default());
        // The edges of the range are valid.
        let s = parse("similar_threshold = 0.0\nduplicate_threshold = 1.0");
        assert_eq!((s.similar_threshold, s.duplicate_threshold), (0.0, 1.0));
    }

    // ── vectors ───────────────────────────────────────────────────────────

    fn acc(chunks: &[Option<&[f32]>]) -> Option<Vec<f64>> {
        let mut a = VectorAccumulator::default();
        for c in chunks {
            a.add_chunk(*c);
        }
        a.finish()
    }

    #[test]
    fn one_chunk_is_normalised_to_unit_length() {
        assert_eq!(acc(&[Some(&[3.0, 4.0])]), Some(vec![0.6, 0.8]));
    }

    #[test]
    fn several_chunks_give_the_direction_of_their_mean() {
        let v = acc(&[Some(&[2.0, 0.0]), Some(&[0.0, 5.0])]).unwrap();
        assert_eq!(v, vec![2.0 / 29f64.sqrt(), 5.0 / 29f64.sqrt()]);
        let v = acc(&[Some(&[1.0, 0.0]), Some(&[0.0, 1.0])]).unwrap();
        assert_eq!(v[0], v[1]);
        assert!((v[0] - 1.0 / 2f64.sqrt()).abs() < 1e-15);
    }

    #[test]
    fn a_document_without_a_complete_usable_vector_has_none() {
        assert_eq!(acc(&[]), None, "no chunk");
        assert_eq!(acc(&[None]), None, "a chunk without a vector");
        assert_eq!(acc(&[Some(&[])]), None, "an empty vector");
        assert_eq!(
            acc(&[Some(&[1.0, 0.0]), None]),
            None,
            "one chunk of two is missing"
        );
        assert_eq!(
            acc(&[Some(&[1.0, 0.0]), Some(&[1.0, 0.0, 0.0])]),
            None,
            "sizes differ"
        );
        assert_eq!(acc(&[Some(&[0.0, 0.0])]), None, "all zero");
        assert_eq!(
            acc(&[Some(&[1.0, 0.0]), Some(&[-1.0, 0.0])]),
            None,
            "the chunks cancel out"
        );
    }

    #[test]
    fn cosine_is_clamped_and_needs_equal_sizes() {
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0]), Some(1.0));
        assert_eq!(cosine(&[1.0, 0.0], &[0.6, 0.8]), Some(0.6));
        assert_eq!(cosine(&[1.0, 0.0], &[-1.0, 0.0]), Some(0.0));
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), None);
        assert_eq!(cosine(&[], &[]), None);
        // A rounding excess over 1 is clamped.
        assert_eq!(cosine(&[1.0000000000000002], &[1.0]), Some(1.0));
    }

    // ── similar documents ─────────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn similar_lists_the_documents_above_the_default_threshold_best_first() {
        let u = library();
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(r.source.as_ref().unwrap().slug, "src");
        assert_eq!(r.threshold, 0.80);
        assert_eq!((r.similar_threshold, r.duplicate_threshold), (0.80, 0.92));
        assert!(r.source_embedded);
        assert_eq!(r.documents_without_vector, 0);
        assert_eq!(r.matches, 3);
        assert_eq!(
            seen(&r),
            vec![
                row("dup-same", 1.0, "possible_duplicate"),
                row("dup-near", 0.96, "possible_duplicate"),
                row("sim-edge", 0.8, "similar_to"),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn a_document_is_found_by_its_id_as_well() {
        let u = library();
        let id = u.doc_get("src").unwrap().unwrap().id;
        assert_eq!(
            u.doc_similar(&id, None, None, 10).unwrap(),
            u.doc_similar("src", None, None, 10).unwrap()
        );
    }

    #[test]
    #[serial_test::serial]
    fn the_score_at_the_threshold_is_a_hit_and_one_step_above_is_not() {
        let u = library();
        let at = u.doc_similar("src", None, Some(0.8), 10).unwrap();
        assert_eq!(at.hits.last().unwrap().score, 0.8);
        assert_eq!(at.matches, 3);
        let above = u.doc_similar("src", None, Some(next_up(0.8)), 10).unwrap();
        assert_eq!(above.matches, 2);
        assert_eq!(above.hits.last().unwrap().document.slug, "dup-near");
        let below = u
            .doc_similar("src", None, Some(next_down(0.8)), 10)
            .unwrap();
        assert_eq!(below.matches, 3);
        // Exactly the score of the 0.96 document, and one step above it.
        let at = u.doc_similar("src", None, Some(0.96), 10).unwrap();
        assert_eq!(
            at.hits
                .iter()
                .map(|h| h.document.slug.as_str())
                .collect::<Vec<_>>(),
            ["dup-same", "dup-near"]
        );
        let above = u.doc_similar("src", None, Some(next_up(0.96)), 10).unwrap();
        assert_eq!(above.hits.len(), 1);
    }

    #[test]
    #[serial_test::serial]
    fn a_lower_threshold_adds_related_hits_and_zero_lists_everything() {
        let u = library();
        let r = u.doc_similar("src", None, Some(0.6), 10).unwrap();
        assert_eq!(r.threshold, 0.6);
        assert_eq!(
            seen(&r),
            vec![
                row("dup-same", 1.0, "possible_duplicate"),
                row("dup-near", 0.96, "possible_duplicate"),
                row("sim-edge", 0.8, "similar_to"),
                row("low", 0.6, "related"),
            ]
        );
        let all = u.doc_similar("src", None, Some(0.0), 100).unwrap();
        assert_eq!(all.matches, 6, "every other document, orthogonal included");
        assert_eq!(all.hits.last().unwrap().score, 0.0);
        let one = u.doc_similar("src", None, Some(1.0), 10).unwrap();
        assert_eq!(seen(&one), vec![row("dup-same", 1.0, "possible_duplicate")]);
    }

    #[test]
    #[serial_test::serial]
    fn the_configured_thresholds_decide_the_relation() {
        let mut u = library();
        u.set_doc_similarity_settings(DocSimilaritySettings {
            similar_threshold: 0.6,
            duplicate_threshold: 0.8,
        });
        assert_eq!(u.doc_similarity_settings().similar_threshold, 0.6);
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(r.threshold, 0.6, "the default floor is the configured one");
        assert_eq!((r.similar_threshold, r.duplicate_threshold), (0.6, 0.8));
        assert_eq!(
            seen(&r),
            vec![
                row("dup-same", 1.0, "possible_duplicate"),
                row("dup-near", 0.96, "possible_duplicate"),
                row("sim-edge", 0.8, "possible_duplicate"),
                row("low", 0.6, "similar_to"),
            ]
        );
    }

    #[test]
    #[serial_test::serial]
    fn limit_cuts_the_hits_but_not_the_match_count() {
        let u = library();
        let r = u.doc_similar("src", None, None, 2).unwrap();
        assert_eq!(r.matches, 3);
        assert_eq!(
            r.hits
                .iter()
                .map(|h| h.document.slug.as_str())
                .collect::<Vec<_>>(),
            ["dup-same", "dup-near"]
        );
        let r = u.doc_similar("src", None, None, 1).unwrap();
        assert_eq!(r.hits.len(), 1);
        let r = u.doc_similar("src", None, None, 0).unwrap();
        assert_eq!(r.hits.len(), 1, "a limit of 0 is raised to 1");
        let r = u.doc_similar("src", None, None, usize::MAX).unwrap();
        assert_eq!(
            r.hits.len(),
            3,
            "an oversized limit is clamped, not an error"
        );
    }

    #[test]
    #[serial_test::serial]
    fn equal_scores_are_ordered_by_slug() {
        let u = open();
        add(&u, "src", "AXIS");
        for slug in ["zeta", "alpha", "mid"] {
            add(&u, slug, "AXIS");
        }
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(
            r.hits
                .iter()
                .map(|h| h.document.slug.as_str())
                .collect::<Vec<_>>(),
            ["alpha", "mid", "zeta"]
        );
    }

    #[test]
    #[serial_test::serial]
    fn deleted_documents_are_ignored() {
        let u = library();
        u.doc_delete("dup-near").unwrap();
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(
            seen(&r),
            vec![
                row("dup-same", 1.0, "possible_duplicate"),
                row("sim-edge", 0.8, "similar_to"),
            ]
        );
        u.doc_delete("dup-same").unwrap();
        assert_eq!(u.doc_similar("src", None, None, 10).unwrap().matches, 1);
        let text = u.doc_similar_to_text("AXIS", None, None, 10).unwrap();
        assert_eq!(
            text.hits
                .iter()
                .map(|h| h.document.slug.as_str())
                .collect::<Vec<_>>(),
            ["src", "sim-edge"]
        );
        // The source itself, once deleted, is not found.
        u.doc_delete("src").unwrap();
        let e = u.doc_similar("src", None, None, 10).unwrap_err();
        assert_eq!(e.class(), crate::ErrorClass::NotFound, "{e}");
    }

    #[test]
    #[serial_test::serial]
    fn an_updated_document_is_compared_by_its_new_content() {
        let u = library();
        u.doc_update("far", None, Some("# far\n\nAXIS"), None, None)
            .unwrap()
            .unwrap();
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(r.hits[1].document.slug, "far");
        assert_eq!(r.hits[1].score, 1.0);
    }

    #[test]
    #[serial_test::serial]
    fn the_namespace_limits_the_candidates() {
        let u = library();
        set_namespace(&u, "dup-same", "alpha");
        set_namespace(&u, "sim-edge", "alpha");
        set_namespace(&u, "dup-near", "beta");
        let alpha = u.doc_similar("src", Some("alpha"), None, 10).unwrap();
        assert_eq!(
            seen(&alpha),
            vec![
                row("dup-same", 1.0, "possible_duplicate"),
                row("sim-edge", 0.8, "similar_to"),
            ]
        );
        assert_eq!(
            alpha.source.unwrap().namespace,
            None,
            "the source may live outside the namespace"
        );
        let beta = u.doc_similar("src", Some("beta"), None, 10).unwrap();
        assert_eq!(
            seen(&beta),
            vec![row("dup-near", 0.96, "possible_duplicate")]
        );
        let none = u.doc_similar("src", Some("gamma"), None, 10).unwrap();
        assert!(none.hits.is_empty());
        assert_eq!(none.matches, 0);
        let global = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(global.matches, 3, "no namespace is the global view");
        // A document without a namespace is only in the global view.
        assert_eq!(
            u.doc_similar("dup-same", Some("alpha"), None, 10)
                .unwrap()
                .matches,
            1
        );
    }

    #[test]
    #[serial_test::serial]
    fn an_empty_library_and_a_library_of_one_have_no_hits() {
        let u = open();
        let e = u.doc_similar("ghost", None, None, 10).unwrap_err();
        assert_eq!(e.class(), crate::ErrorClass::NotFound, "{e}");
        let r = u.doc_similar_to_text("AXIS", None, None, 10).unwrap();
        assert!(r.source_embedded);
        assert_eq!((r.matches, r.documents_without_vector), (0, 0));
        assert!(r.hits.is_empty());
        add(&u, "only", "AXIS");
        let r = u.doc_similar("only", None, None, 10).unwrap();
        assert!(r.source_embedded);
        assert!(r.hits.is_empty(), "a document is never its own neighbour");
        assert_eq!(r.documents_without_vector, 0);
    }

    #[test]
    #[serial_test::serial]
    fn a_document_without_vectors_is_never_a_hit_and_is_counted() {
        let u = library();
        add_unembedded(&u, "pending");
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(r.documents_without_vector, 1);
        assert_eq!(r.matches, 3, "the others are unchanged");
        assert!(r.hits.iter().all(|h| h.document.slug != "pending"));
        let t = u.doc_similar_to_text("AXIS", None, None, 10).unwrap();
        assert_eq!(t.documents_without_vector, 1);
    }

    #[test]
    #[serial_test::serial]
    fn a_source_without_vectors_answers_with_no_hits_and_says_so() {
        let u = library();
        add_unembedded(&u, "pending");
        let r = u.doc_similar("pending", None, None, 10).unwrap();
        assert_eq!(r.source.as_ref().unwrap().slug, "pending");
        assert!(!r.source_embedded);
        assert!(r.hits.is_empty());
        assert_eq!((r.matches, r.documents_without_vector), (0, 0));
        // Once its chunks are embedded (repair --reembed) it is a source again.
        let missing = u.store.chunks_missing_embeddings().unwrap();
        assert_eq!(missing.len(), 1);
        u.store
            .set_chunk_embedding(&missing[0].0, &[1.0, 0.0, 0.0])
            .unwrap();
        let r = u.doc_similar("pending", None, None, 10).unwrap();
        assert!(r.source_embedded);
        assert_eq!(r.matches, 3 + 1, "src, dup-same, dup-near... and sim-edge");
    }

    #[test]
    #[serial_test::serial]
    fn a_document_with_one_chunk_missing_its_vector_has_no_vector() {
        let u = open();
        add(&u, "src", "AXIS");
        u.doc_upsert("two", "two", "# a\n\nAXIS\n\n# b\n\nAXIS", &[], None)
            .unwrap();
        assert_eq!(u.doc_similar("src", None, None, 10).unwrap().matches, 1);
        u.store
            .conn
            .execute(
                "UPDATE document_chunks SET embedding = NULL \
                 WHERE id = (SELECT id FROM document_chunks WHERE heading = 'b')",
                [],
            )
            .unwrap();
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!((r.matches, r.documents_without_vector), (0, 1));
    }

    #[test]
    #[serial_test::serial]
    fn vectors_of_another_size_are_not_comparable() {
        let u = library();
        let chunk = u
            .store
            .conn
            .query_row(
                "SELECT c.id FROM document_chunks c JOIN documents d ON d.id = c.document_id \
                 WHERE d.slug = 'dup-same'",
                [],
                |r| r.get::<_, String>(0),
            )
            .unwrap();
        u.store.set_chunk_embedding(&chunk, &[1.0, 0.0]).unwrap();
        let r = u.doc_similar("src", None, None, 10).unwrap();
        assert_eq!(r.documents_without_vector, 1);
        assert_eq!(
            r.hits
                .iter()
                .map(|h| h.document.slug.as_str())
                .collect::<Vec<_>>(),
            ["dup-near", "sim-edge"]
        );
        // As the source, nothing else shares its size.
        let r = u.doc_similar("dup-same", None, None, 10).unwrap();
        assert!(r.source_embedded);
        assert_eq!((r.matches, r.documents_without_vector), (0, 6));
    }

    #[test]
    #[serial_test::serial]
    fn a_document_vector_is_the_mean_of_its_chunk_vectors() {
        let u = open();
        add(&u, "src", "AXIS");
        u.doc_upsert("multi", "multi", "# a\n\nAXIS\n\n# b\n\nR01", &[], None)
            .unwrap();
        let r = u.doc_similar("src", None, Some(0.7), 10).unwrap();
        assert_eq!(
            seen(&r),
            vec![row("multi", std::f32::consts::FRAC_1_SQRT_2, "related")]
        );
        let none = u.doc_similar("src", None, Some(0.71), 10).unwrap();
        assert!(none.hits.is_empty());
        // Symmetric.
        let back = u.doc_similar("multi", None, Some(0.7), 10).unwrap();
        assert_eq!(back.hits[0].score, std::f32::consts::FRAC_1_SQRT_2);
    }

    #[test]
    #[serial_test::serial]
    fn bad_thresholds_are_refused() {
        let u = library();
        for t in [-0.01, 1.01, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            let e = u.doc_similar("src", None, Some(t), 10).unwrap_err();
            assert_eq!(e.class(), crate::ErrorClass::BadRequest, "{t}: {e}");
            assert!(e.to_string().contains("threshold"), "{e}");
            assert!(u.doc_similar_to_text("AXIS", None, Some(t), 10).is_err());
            assert!(u.doc_duplicate_report(None, None, Some(t), 10).is_err());
        }
    }

    // ── candidate text ────────────────────────────────────────────────────

    #[test]
    #[serial_test::serial]
    fn a_candidate_text_is_scored_like_the_stored_document_would_be() {
        let u = library();
        let before = u.doc_list(None, 100).unwrap().len();
        let r = u.doc_similar_to_text("AXIS", None, None, 10).unwrap();
        assert!(r.source.is_none());
        assert!(r.source_embedded);
        assert_eq!(
            seen(&r),
            vec![
                row("dup-same", 1.0, "possible_duplicate"),
                row("src", 1.0, "possible_duplicate"),
                row("dup-near", 0.96, "possible_duplicate"),
                row("sim-edge", 0.8, "similar_to"),
            ]
        );
        assert_eq!(r.matches, 4);
        // Nothing was stored, and the stored copy scores the same.
        assert_eq!(u.doc_list(None, 100).unwrap().len(), before);
        add(&u, "stored-copy", "R43");
        let stored = u.doc_similar("stored-copy", None, Some(0.0), 100).unwrap();
        let drafted = u.doc_similar_to_text("R43", None, Some(0.0), 100).unwrap();
        let strip = |r: &DocSimilarReport| -> Vec<(String, f32)> {
            r.hits
                .iter()
                .filter(|h| h.document.slug != "stored-copy")
                .map(|h| (h.document.slug.clone(), h.score))
                .collect()
        };
        assert_eq!(strip(&stored), strip(&drafted));
    }

    #[test]
    #[serial_test::serial]
    fn a_candidate_text_with_sections_uses_the_mean_of_its_chunks() {
        let u = open();
        add(&u, "src", "AXIS");
        let r = u
            .doc_similar_to_text("# a\n\nAXIS\n\n# b\n\nR01", None, Some(0.7), 10)
            .unwrap();
        assert_eq!(
            seen(&r),
            vec![row("src", std::f32::consts::FRAC_1_SQRT_2, "related")]
        );
    }

    #[test]
    #[serial_test::serial]
    fn a_candidate_text_honours_namespace_threshold_and_limit() {
        let u = library();
        set_namespace(&u, "dup-near", "alpha");
        let r = u
            .doc_similar_to_text("AXIS", Some("alpha"), None, 10)
            .unwrap();
        assert_eq!(seen(&r), vec![row("dup-near", 0.96, "possible_duplicate")]);
        let r = u.doc_similar_to_text("AXIS", None, Some(0.9), 1).unwrap();
        assert_eq!((r.matches, r.hits.len()), (3, 1));
        assert_eq!(r.hits[0].document.slug, "dup-same");
    }

    #[test]
    #[serial_test::serial]
    fn an_empty_or_oversized_candidate_text_is_refused() {
        let u = library();
        for text in ["", "   \n\t "] {
            let e = u.doc_similar_to_text(text, None, None, 10).unwrap_err();
            assert_eq!(e.class(), crate::ErrorClass::BadRequest, "{e}");
        }
        let at_cap = format!("AXIS {}", "x".repeat(MAX_CANDIDATE_CHARS - 5));
        assert_eq!(at_cap.chars().count(), MAX_CANDIDATE_CHARS);
        u.doc_similar_to_text(&at_cap, None, None, 10).unwrap();
        let over = format!("AXIS {}", "x".repeat(MAX_CANDIDATE_CHARS - 4));
        let e = u.doc_similar_to_text(&over, None, None, 10).unwrap_err();
        assert_eq!(e.class(), crate::ErrorClass::BadRequest, "{e}");
    }

    // ── duplicate report ──────────────────────────────────────────────────

    /// a, b = AXIS (1.0); c = R247 (0.96 with a and b); d, e = R01 (1.0);
    /// f = R724 (0.96 with d and e, 0.28 with a and b, 0.5376 with c).
    fn report_library() -> crate::Uteke {
        let u = open();
        add(&u, "a", "AXIS");
        add_child(&u, "b", "AXIS", "a");
        add_child(&u, "c", "R247", "b");
        add(&u, "d", "R01");
        add(&u, "e", "R01");
        add(&u, "f", "R724");
        u
    }

    fn pair_rows(r: &DocDuplicateReport) -> Vec<(String, String, f32)> {
        r.pairs
            .iter()
            .map(|p| (p.a.slug.clone(), p.b.slug.clone(), p.score))
            .collect()
    }

    fn pair(a: &str, b: &str, score: f32) -> (String, String, f32) {
        (a.to_string(), b.to_string(), score)
    }

    fn groups(g: &[&[&str]]) -> Vec<Vec<String>> {
        g.iter()
            .map(|m| m.iter().map(|s| s.to_string()).collect())
            .collect()
    }

    #[test]
    #[serial_test::serial]
    fn the_report_lists_every_pair_above_the_duplicate_threshold() {
        let u = report_library();
        let r = u.doc_duplicate_report(None, None, None, 50).unwrap();
        assert_eq!(r.threshold, 0.92);
        assert_eq!((r.scanned, r.documents_without_vector), (6, 0));
        assert_eq!(r.total_pairs, 6);
        assert_eq!(
            pair_rows(&r),
            vec![
                pair("a", "b", 1.0),
                pair("d", "e", 1.0),
                pair("a", "c", 0.96),
                pair("b", "c", 0.96),
                pair("d", "f", 0.96),
                pair("e", "f", 0.96),
            ]
        );
        assert_eq!(r.groups, groups(&[&["a", "b", "c"], &["d", "e", "f"]]));
    }

    #[test]
    #[serial_test::serial]
    fn the_report_threshold_is_inclusive_and_a_step_above_drops_the_pair() {
        let u = report_library();
        let at = u.doc_duplicate_report(None, None, Some(0.96), 50).unwrap();
        assert_eq!(at.total_pairs, 6);
        let above = u
            .doc_duplicate_report(None, None, Some(next_up(0.96)), 50)
            .unwrap();
        assert_eq!(
            pair_rows(&above),
            vec![pair("a", "b", 1.0), pair("d", "e", 1.0)]
        );
        assert_eq!(above.groups, groups(&[&["a", "b"], &["d", "e"]]));
        let below = u
            .doc_duplicate_report(None, None, Some(next_down(0.96)), 50)
            .unwrap();
        assert_eq!(below.total_pairs, 6);
        let low = u.doc_duplicate_report(None, None, Some(0.5), 50).unwrap();
        assert_eq!(low.total_pairs, 7, "adds c and f at 0.5376");
        assert_eq!(low.pairs[6].score, 0.5376);
        assert_eq!(low.groups, groups(&[&["a", "b", "c", "d", "e", "f"]]));
    }

    #[test]
    #[serial_test::serial]
    fn the_configured_duplicate_threshold_is_the_report_default() {
        let mut u = report_library();
        u.set_doc_similarity_settings(DocSimilaritySettings {
            similar_threshold: 0.5,
            duplicate_threshold: 1.0,
        });
        let r = u.doc_duplicate_report(None, None, None, 50).unwrap();
        assert_eq!(r.threshold, 1.0);
        assert_eq!(
            pair_rows(&r),
            vec![pair("a", "b", 1.0), pair("d", "e", 1.0)]
        );
    }

    #[test]
    #[serial_test::serial]
    fn the_report_limit_cuts_the_pairs_but_not_the_groups() {
        let u = report_library();
        let r = u.doc_duplicate_report(None, None, None, 3).unwrap();
        assert_eq!(r.total_pairs, 6);
        assert_eq!(
            pair_rows(&r),
            vec![
                pair("a", "b", 1.0),
                pair("d", "e", 1.0),
                pair("a", "c", 0.96)
            ]
        );
        assert_eq!(r.groups, groups(&[&["a", "b", "c"], &["d", "e", "f"]]));
        let r = u.doc_duplicate_report(None, None, None, 0).unwrap();
        assert_eq!(r.pairs.len(), 1, "a limit of 0 is raised to 1");
    }

    #[test]
    #[serial_test::serial]
    fn the_report_of_a_subtree_compares_only_that_subtree() {
        let u = report_library();
        let r = u.doc_duplicate_report(None, Some("a"), None, 50).unwrap();
        assert_eq!(r.scanned, 3, "a, its child b and its grandchild c");
        assert_eq!(
            pair_rows(&r),
            vec![
                pair("a", "b", 1.0),
                pair("a", "c", 0.96),
                pair("b", "c", 0.96)
            ]
        );
        assert_eq!(r.groups, groups(&[&["a", "b", "c"]]));
        let r = u.doc_duplicate_report(None, Some("b"), None, 50).unwrap();
        assert_eq!(r.scanned, 2);
        assert_eq!(pair_rows(&r), vec![pair("b", "c", 0.96)]);
        let id = u.doc_get("c").unwrap().unwrap().id;
        let r = u.doc_duplicate_report(None, Some(&id), None, 50).unwrap();
        assert_eq!(
            (r.scanned, r.total_pairs),
            (1, 0),
            "a leaf is a subtree of one"
        );
        assert!(r.groups.is_empty());
        let e = u
            .doc_duplicate_report(None, Some("ghost"), None, 50)
            .unwrap_err();
        assert_eq!(e.class(), crate::ErrorClass::NotFound, "{e}");
    }

    #[test]
    #[serial_test::serial]
    fn the_report_namespace_scopes_it_and_intersects_with_a_subtree() {
        let u = report_library();
        set_namespace(&u, "d", "alpha");
        set_namespace(&u, "e", "alpha");
        set_namespace(&u, "a", "alpha");
        let r = u
            .doc_duplicate_report(Some("alpha"), None, None, 50)
            .unwrap();
        assert_eq!(r.scanned, 3);
        assert_eq!(pair_rows(&r), vec![pair("d", "e", 1.0)]);
        assert_eq!(r.groups, groups(&[&["d", "e"]]));
        let r = u
            .doc_duplicate_report(Some("alpha"), Some("a"), None, 50)
            .unwrap();
        assert_eq!((r.scanned, r.total_pairs), (1, 0), "only a is in both");
        let r = u
            .doc_duplicate_report(Some("beta"), None, None, 50)
            .unwrap();
        assert_eq!((r.scanned, r.total_pairs), (0, 0));
        let r = u.doc_duplicate_report(None, None, None, 50).unwrap();
        assert_eq!(r.scanned, 6, "no namespace is the global view");
    }

    #[test]
    #[serial_test::serial]
    fn the_report_ignores_deleted_documents_and_counts_those_without_vectors() {
        let u = report_library();
        u.doc_delete("c").unwrap();
        add_unembedded(&u, "pending");
        let r = u.doc_duplicate_report(None, None, None, 50).unwrap();
        assert_eq!((r.scanned, r.documents_without_vector), (6, 1));
        assert_eq!(
            pair_rows(&r),
            vec![
                pair("a", "b", 1.0),
                pair("d", "e", 1.0),
                pair("d", "f", 0.96),
                pair("e", "f", 0.96)
            ]
        );
        assert!(
            r.groups
                .iter()
                .flatten()
                .all(|s| s != "c" && s != "pending")
        );
    }

    #[test]
    #[serial_test::serial]
    fn the_report_of_an_empty_library_is_empty() {
        let u = open();
        let r = u.doc_duplicate_report(None, None, None, 50).unwrap();
        assert_eq!(
            (r.scanned, r.documents_without_vector, r.total_pairs),
            (0, 0, 0)
        );
        assert!(r.pairs.is_empty() && r.groups.is_empty());
        assert_eq!(r.threshold, 0.92);
    }

    #[test]
    fn groups_merge_chains_and_order_by_size_then_slug() {
        // x-y and y-z chain into one group; m-n is a pair; q-r is another.
        let got = duplicate_groups([("m", "n"), ("x", "y"), ("q", "r"), ("y", "z")]);
        assert_eq!(got, groups(&[&["x", "y", "z"], &["m", "n"], &["q", "r"]]));
        assert!(duplicate_groups([]).is_empty());
    }

    #[test]
    fn a_query_has_exactly_one_subject() {
        use DocSimilarQuery::*;
        let r = DocSimilarQuery::resolve;
        assert_eq!(r(Some("a"), None, false, None), Ok(Document("a")));
        assert_eq!(r(None, Some("t"), false, None), Ok(Text("t")));
        assert_eq!(r(None, None, true, None), Ok(Report { subtree: None }));
        assert_eq!(
            r(None, None, true, Some("s")),
            Ok(Report { subtree: Some("s") })
        );
        for bad in [
            r(Some("a"), Some("t"), false, None),
            r(None, None, false, None),
            r(Some("a"), None, true, None),
            r(None, Some("t"), true, None),
            r(Some("a"), None, false, Some("s")),
            r(None, Some("t"), false, Some("s")),
            r(None, None, false, Some("s")),
        ] {
            assert!(bad.is_err(), "{bad:?}");
        }
        assert_eq!(
            r(None, None, false, None),
            Err("provide 'slug' (or 'id'), 'text', or \"report\": true")
        );
    }

    #[test]
    #[serial_test::serial]
    fn the_query_entry_point_dispatches_and_applies_the_default_limits() {
        let u = report_library();
        let ask = |q: DocSimilarQuery<'_>, limit| u.doc_similarity_query(&q, None, None, limit);
        match ask(DocSimilarQuery::Document("a"), None).unwrap() {
            DocSimilarAnswer::Similar(r) => {
                assert_eq!(r.source.unwrap().slug, "a");
                assert_eq!(r.hits.len(), 2, "b and c");
            }
            other => panic!("{other:?}"),
        }
        match ask(DocSimilarQuery::Text("AXIS"), Some(1)).unwrap() {
            DocSimilarAnswer::Similar(r) => {
                assert!(r.source.is_none());
                assert_eq!((r.matches, r.hits.len()), (3, 1));
            }
            other => panic!("{other:?}"),
        }
        match ask(DocSimilarQuery::Report { subtree: Some("a") }, None).unwrap() {
            DocSimilarAnswer::Duplicates(r) => assert_eq!(r.total_pairs, 3),
            other => panic!("{other:?}"),
        }
        assert_eq!(DEFAULT_SIMILAR_LIMIT, 10);
        assert_eq!(DEFAULT_REPORT_LIMIT, 50);
        // The answer serializes as the bare report.
        let json =
            serde_json::to_value(ask(DocSimilarQuery::Report { subtree: None }, None).unwrap())
                .unwrap();
        assert_eq!(json["total_pairs"], 6);
        assert!(json.get("Duplicates").is_none());
    }

    #[test]
    fn the_report_size_limit_is_the_documented_constant() {
        assert_eq!(MAX_REPORT_DOCUMENTS, 2000);
        assert_eq!(MAX_SIMILAR_LIMIT, 100);
        assert_eq!(MAX_REPORT_LIMIT, 500);
    }
}
