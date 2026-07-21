//! Code indexing operations — turn source files into recallable memories.
//!
//! A source file is chunked by [`crate::chunker::chunk_code`] into semantic
//! units (functions, structs, classes, ...). Each chunk is stored as a memory
//! carrying location metadata so recall can point back to `file:line`.
//!
//! Incremental re-index is content-hash driven: [`IndexOutcome::Skipped`] is
//! returned when a file's hash matches the tracked `indexed_files` record, so
//! unchanged files are never re-embedded.

use crate::chunker::{chunk_code, detect_language};
use crate::memory::IndexedFile;
use crate::Error;

/// Result of indexing a single file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexOutcome {
    /// File content unchanged since last index — no work done.
    Skipped,
    /// File indexed: number of chunks stored.
    Indexed { chunks: usize },
}

/// Aggregate result of indexing a directory tree or file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
pub struct IndexSummary {
    /// Files (re)indexed this run.
    pub indexed: usize,
    /// Files skipped because content was unchanged.
    pub skipped: usize,
    /// Total chunks stored this run.
    pub chunks: usize,
    /// Files pruned (deleted on disk, chunks removed).
    pub pruned: usize,
}

/// Live progress events emitted during [`crate::Uteke::index_tree_cb`].
///
/// Chunking (tree-sitter) is fast; embedding each chunk dominates runtime.
/// These events let a CLI stream per-file progress instead of blocking on a
/// single summary line.
#[derive(Debug, Clone)]
pub enum IndexProgress<'a> {
    /// Emitted once after the walk, before any embedding work. `files` is the
    /// count of candidate source files discovered (post language/exclude
    /// filter).
    Discovered { files: usize },
    /// A file is about to be chunked + embedded (the slow step).
    FileStarted { path: &'a str, index: usize, total: usize },
    /// A file finished embedding: `chunks` stored.
    FileIndexed { path: &'a str, chunks: usize },
    /// A file was skipped (content hash unchanged).
    FileSkipped { path: &'a str },
    /// Pruning deleted-on-disk files completed.
    Pruned { files: usize },
}

/// Directory names always skipped during a walk.
pub const DEFAULT_EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    ".uteke",
    "target",
    "node_modules",
    "dist",
    "build",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".mypy_cache",
    ".pytest_cache",
    "vendor",
    ".idea",
    ".vscode",
];

/// Max file size to index (bytes). Larger files are skipped.
pub const MAX_INDEX_FILE_SIZE: u64 = 1_000_000;

/// Compute a hex content hash for indexing purposes.
pub fn content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let digest = hasher.finalize();
    let mut s = String::with_capacity(digest.len() * 2);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

impl crate::Uteke {
    /// Index a single source file into the given namespace.
    ///
    /// - `rel_path`: repo-relative path stored in metadata (forward-slash form).
    /// - `content`: full file contents.
    /// - `language`: optional language override; when `None` it is detected
    ///   from `rel_path`'s extension.
    /// - `mtime`: file modification time (unix seconds) for the tracking record.
    /// - `force`: when true, re-index even if the content hash is unchanged.
    ///
    /// Prunes any previously-indexed chunks for this file before storing the
    /// new ones, so the index always reflects current file content.
    pub fn index_source_file(
        &self,
        namespace: &str,
        rel_path: &str,
        content: &str,
        language: Option<&str>,
        mtime: i64,
        force: bool,
    ) -> Result<IndexOutcome, Error> {
        let hash = content_hash(content);

        // Skip unchanged files unless forced.
        if !force {
            if let Some(prev) = self.store.indexed_file_hash(namespace, rel_path)? {
                if prev == hash {
                    return Ok(IndexOutcome::Skipped);
                }
            }
        }

        // Remove stale chunks for this file (full replace).
        let stale = self
            .store
            .memory_ids_by_indexed_file(namespace, rel_path)?;
        for id in &stale {
            self.forget(id)?;
        }

        let lang = language
            .map(|s| s.to_string())
            .unwrap_or_else(|| detect_language(rel_path).to_string());
        let chunks = chunk_code(content, &lang);

        let mut stored = 0usize;
        for chunk in &chunks {
            let metadata = serde_json::json!({
                "source_type": "code",
                "file": rel_path,
                "language": chunk.language,
                "symbol_name": chunk.symbol_name,
                "symbol_type": chunk.symbol_type,
                "line_start": chunk.line_start,
                "line_end": chunk.line_end,
            });
            // Prefix the stored content with a compact location header so the
            // embedding text and recall snippet both carry the symbol context.
            let body = format!(
                "{} {} [{}:{}-{}]\n{}",
                chunk.symbol_type,
                chunk.symbol_name,
                rel_path,
                chunk.line_start,
                chunk.line_end,
                chunk.content
            );
            // `code` tag makes filtered recall/prune easy; store as a reference
            // type since these are code artifacts, not user assertions.
            self.remember_typed(&body, &["code"], Some(metadata), Some(namespace), "reference")?;
            stored += 1;
        }

        self.store.upsert_indexed_file(
            namespace,
            &IndexedFile {
                path: rel_path.to_string(),
                content_hash: hash,
                mtime,
                chunk_count: stored as i64,
            },
        )?;

        Ok(IndexOutcome::Indexed { chunks: stored })
    }

    /// Prune index records + chunk memories for files that no longer exist.
    ///
    /// `present` is the set of repo-relative paths currently on disk. Any
    /// tracked file not in `present` has its chunks forgotten and its tracking
    /// record removed. Returns the number of files pruned.
    pub fn prune_deleted_files(
        &self,
        namespace: &str,
        present: &std::collections::HashSet<String>,
    ) -> Result<usize, Error> {
        let tracked = self.store.list_indexed_files(namespace)?;
        let mut pruned = 0usize;
        for f in &tracked {
            if present.contains(&f.path) {
                continue;
            }
            let ids = self.store.memory_ids_by_indexed_file(namespace, &f.path)?;
            for id in &ids {
                self.forget(id)?;
            }
            self.store.delete_indexed_file(namespace, &f.path)?;
            pruned += 1;
        }
        Ok(pruned)
    }

    /// Index a directory tree (or single file) into `namespace`.
    ///
    /// Walks `root` skipping [`DEFAULT_EXCLUDED_DIRS`] and hidden dirs, indexes
    /// each file whose extension maps to a known language, prunes chunks for
    /// files that disappeared, and returns an [`IndexSummary`]. `rel_base` is
    /// the directory that repo-relative metadata paths are computed against
    /// (usually `root` itself, or its parent for a single file).
    ///
    /// When `dry_run` is true, no memories are written and no pruning occurs;
    /// the summary's `indexed` counts files that *would* be indexed.
    pub fn index_tree(
        &self,
        namespace: &str,
        root: &std::path::Path,
        force: bool,
        dry_run: bool,
    ) -> Result<IndexSummary, Error> {
        self.index_tree_cb(namespace, root, force, dry_run, |_| {})
    }

    /// Like [`Self::index_tree`] but streams [`IndexProgress`] events to `cb`.
    ///
    /// The callback fires: once with `Discovered` after the walk, then
    /// `FileStarted`/`FileIndexed`/`FileSkipped` per file, and finally
    /// `Pruned`. This lets callers render live progress — cheap to ignore
    /// (`index_tree` passes a no-op closure).
    pub fn index_tree_cb(
        &self,
        namespace: &str,
        root: &std::path::Path,
        force: bool,
        dry_run: bool,
        mut cb: impl FnMut(IndexProgress<'_>),
    ) -> Result<IndexSummary, Error> {
        use std::collections::HashSet;

        let rel_base = if root.is_file() {
            root.parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."))
        } else {
            root.to_path_buf()
        };

        let mut files = Vec::new();
        if root.is_file() {
            files.push(root.to_path_buf());
        } else {
            collect_files(root, &mut files);
        }

        // Pre-filter to indexable (known-language) files so the discovered
        // count and per-file index/total reflect real work.
        let candidates: Vec<(std::path::PathBuf, String)> = files
            .into_iter()
            .filter_map(|f| {
                let rel = rel_path(&rel_base, &f);
                let lang = detect_language(&rel);
                if lang == "text" {
                    None
                } else {
                    Some((f, rel))
                }
            })
            .collect();

        cb(IndexProgress::Discovered {
            files: candidates.len(),
        });

        let mut summary = IndexSummary::default();
        let mut present: HashSet<String> = HashSet::new();
        let total = candidates.len();

        for (idx, (file, rel)) in candidates.iter().enumerate() {
            let lang = detect_language(rel);
            present.insert(rel.clone());

            let meta = match std::fs::metadata(file) {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!("skip {}: {e}", file.display());
                    continue;
                }
            };
            if meta.len() > MAX_INDEX_FILE_SIZE {
                tracing::warn!("skip {} ({} bytes > limit)", file.display(), meta.len());
                continue;
            }
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(_) => continue, // binary / non-utf8
            };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);

            if dry_run {
                summary.indexed += 1;
                continue;
            }

            cb(IndexProgress::FileStarted {
                path: rel,
                index: idx + 1,
                total,
            });

            match self.index_source_file(namespace, rel, &content, Some(lang), mtime, force)? {
                IndexOutcome::Indexed { chunks } => {
                    summary.indexed += 1;
                    summary.chunks += chunks;
                    cb(IndexProgress::FileIndexed { path: rel, chunks });
                }
                IndexOutcome::Skipped => {
                    summary.skipped += 1;
                    cb(IndexProgress::FileSkipped { path: rel });
                }
            }
        }

        if !dry_run {
            summary.pruned = self.prune_deleted_files(namespace, &present)?;
            cb(IndexProgress::Pruned {
                files: summary.pruned,
            });
        }

        Ok(summary)
    }

    /// Report code-index status for a namespace: number of tracked files and
    /// the sum of their chunk counts.
    pub fn code_index_status(&self, namespace: &str) -> Result<(usize, usize), Error> {
        let files = self.store.list_indexed_files(namespace)?;
        let chunks: i64 = files.iter().map(|f| f.chunk_count).sum();
        Ok((files.len(), chunks as usize))
    }
}

/// Compute a forward-slash repo-relative path.
fn rel_path(base: &std::path::Path, file: &std::path::Path) -> String {
    let rel = file.strip_prefix(base).unwrap_or(file);
    rel.to_string_lossy().replace('\\', "/")
}

/// Recursively collect files, skipping excluded/hidden directories.
fn collect_files(dir: &std::path::Path, acc: &mut Vec<std::path::PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("cannot read {}: {e}", dir.display());
            return;
        }
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if path.is_dir() {
            if DEFAULT_EXCLUDED_DIRS.contains(&name.as_ref()) || name.starts_with('.') {
                continue;
            }
            collect_files(&path, acc);
        } else if path.is_file() {
            acc.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_uteke() -> crate::Uteke {
        // In-memory store, no-op embedder path is fine for indexing logic.
        crate::Uteke::open(":memory:").unwrap()
    }

    #[test]
    fn hash_is_stable_and_content_sensitive() {
        assert_eq!(content_hash("abc"), content_hash("abc"));
        assert_ne!(content_hash("abc"), content_hash("abd"));
    }

    #[test]
    fn index_then_skip_unchanged() {
        let u = tmp_uteke();
        let code = "fn a() {\n    1\n}\n";
        let r1 = u
            .index_source_file("repo", "src/a.rs", code, None, 100, false)
            .unwrap();
        assert!(matches!(r1, IndexOutcome::Indexed { chunks } if chunks >= 1));

        // Same content → skipped.
        let r2 = u
            .index_source_file("repo", "src/a.rs", code, None, 100, false)
            .unwrap();
        assert_eq!(r2, IndexOutcome::Skipped);

        // force → re-indexed.
        let r3 = u
            .index_source_file("repo", "src/a.rs", code, None, 100, true)
            .unwrap();
        assert!(matches!(r3, IndexOutcome::Indexed { .. }));
    }

    #[test]
    fn reindex_replaces_stale_chunks() {
        let u = tmp_uteke();
        u.index_source_file("repo", "src/a.rs", "fn a() {\n    1\n}\n", None, 1, false)
            .unwrap();
        let before = u.store.memory_ids_by_indexed_file("repo", "src/a.rs").unwrap();
        assert_eq!(before.len(), 1);

        // Change content → two functions now.
        u.index_source_file(
            "repo",
            "src/a.rs",
            "fn a() {\n    1\n}\nfn b() {\n    2\n}\n",
            None,
            2,
            false,
        )
        .unwrap();
        let after = u.store.memory_ids_by_indexed_file("repo", "src/a.rs").unwrap();
        assert_eq!(after.len(), 2);
        // Old ids fully replaced.
        assert!(before.iter().all(|id| !after.contains(id)));
    }

    #[test]
    fn index_tree_walks_and_excludes() {
        let u = tmp_uteke();
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\n").unwrap();
        std::fs::write(root.join("src/b.py"), "def b():\n    pass\n").unwrap();
        std::fs::write(root.join("README.md"), "# doc\n").unwrap(); // text -> skipped
        std::fs::write(root.join("target/junk.rs"), "fn junk() {}\n").unwrap(); // excluded dir

        let s = u.index_tree("repo", root, false, false).unwrap();
        assert_eq!(s.indexed, 2, "a.rs + b.py, README skipped, target excluded");
        assert!(s.chunks >= 2);

        // Re-run: unchanged -> all skipped.
        let s2 = u.index_tree("repo", root, false, false).unwrap();
        assert_eq!(s2.skipped, 2);
        assert_eq!(s2.indexed, 0);

        // target/junk.rs must not have been indexed.
        assert_eq!(
            u.store
                .memory_ids_by_indexed_file("repo", "target/junk.rs")
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn index_tree_dry_run_writes_nothing() {
        let u = tmp_uteke();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/a.rs"), "fn a() {}\n").unwrap();

        let s = u.index_tree("repo", tmp.path(), false, true).unwrap();
        assert_eq!(s.indexed, 1);
        assert_eq!(s.chunks, 0);
        assert_eq!(
            u.store.memory_ids_by_indexed_file("repo", "src/a.rs").unwrap().len(),
            0
        );
    }

    #[test]
    fn prune_removes_deleted_files() {
        let u = tmp_uteke();
        u.index_source_file("repo", "src/a.rs", "fn a() {}\n", None, 1, false)
            .unwrap();
        u.index_source_file("repo", "src/b.rs", "fn b() {}\n", None, 1, false)
            .unwrap();

        let mut present = std::collections::HashSet::new();
        present.insert("src/a.rs".to_string());

        let pruned = u.prune_deleted_files("repo", &present).unwrap();
        assert_eq!(pruned, 1);
        assert_eq!(
            u.store.memory_ids_by_indexed_file("repo", "src/b.rs").unwrap().len(),
            0
        );
        assert_eq!(
            u.store.memory_ids_by_indexed_file("repo", "src/a.rs").unwrap().len(),
            1
        );
    }
}
