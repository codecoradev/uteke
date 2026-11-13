//! Skill packages on disk (#1731): discovery, reading, the package hash and
//! lint. Pure file-system code: nothing here touches the database, and nothing
//! ever writes to a skill directory.
//!
//! # What a package is
//!
//! A skill is a directory with a `SKILL.md` at its root (the Agent Skills
//! layout: YAML frontmatter with `name` and `description`, then the
//! instructions) plus any number of reference files below it. The directory
//! name is the skill's slug.
//!
//! # The package hash
//!
//! `uteke skill approve <slug>@<hash>` records the hash of the **whole
//! package**, so a client that checks the hash before loading a skill knows it
//! holds exactly the bytes that were approved. The hash is byte-identical to
//! the one tole computes (`tole-core`, `skill_approval::package_hash`, tole
//! `TOLE_SKILL_APPROVAL=required`): SHA-256 over every file of the package,
//! sorted by relative path (byte order, `/` separators), each file
//! contributing, in this order:
//!
//! 1. the length of its relative path in bytes, as 8 bytes **little-endian**,
//! 2. the path (UTF-8),
//! 3. the length of its content in bytes, as 8 bytes **little-endian**,
//! 4. the content.
//!
//! The length prefixes make the encoding unambiguous: no two different file
//! sets hash the same. The result is written `sha256:<64 lower-case hex>`
//! ([`HASH_PREFIX`]); that string is what is stored, listed and approved.
//!
//! Every regular file under the directory counts, at any depth, hidden files
//! and `.git` included (tole hashes them too). A package that contains a
//! symbolic link or any other non-regular file (FIFO, socket, device) is
//! **rejected whole**, never skipped: a link's target can change without the
//! hash changing, and tole refuses such a package too, so both sides accept
//! and reject the same packages. Empty directories are not hashed (but count
//! against [`MAX_PACKAGE_ENTRIES`]). `.git` is skipped only when *discovering*
//! skill directories under a root, never inside a package.
//!
//! # Bounds
//!
//! Everything read from disk is bounded, so a hostile or accidental tree
//! (a huge file, a million files, a deep or cyclic layout) cannot exhaust
//! memory or time: see the `MAX_*` constants. A package over a bound is
//! rejected whole, never truncated, because a hash of part of a package would
//! not be the hash of the package. The package bounds are at least as strict
//! as tole's (256 files, 4 MiB, 1024 entries), so every package uteke accepts,
//! tole accepts with the same hash.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The file that makes a directory a skill.
pub const SKILL_FILE: &str = "SKILL.md";
/// Longest slug (the Agent Skills limit for `name`).
pub const MAX_SLUG_LEN: usize = 64;
/// Files in one package (all files, indexed or not). Below tole's 256.
pub const MAX_PACKAGE_FILES: usize = 200;
/// Bytes of one file of a package.
pub const MAX_FILE_BYTES: u64 = 1 << 20;
/// Bytes of a whole package (tole's limit, 4 MiB).
pub const MAX_PACKAGE_BYTES: u64 = 4 << 20;
/// Directory entries (files and directories, any depth) one package walk
/// visits (tole's limit), so a tree of countless empty directories is refused
/// before it costs a long walk.
pub const MAX_PACKAGE_ENTRIES: usize = 1024;
/// How a package hash starts: the algorithm, then 64 lower-case hex digits.
pub const HASH_PREFIX: &str = "sha256:";
/// Directory levels inside one package.
pub const MAX_PACKAGE_DEPTH: usize = 8;
/// Text files of one package that are indexed for search.
pub const MAX_INDEXED_FILES: usize = 32;
/// Bytes of one indexed file (`SKILL.md` included).
pub const MAX_INDEXED_FILE_BYTES: u64 = 128 * 1024;
/// Bytes of all indexed files of one package.
pub const MAX_INDEXED_TOTAL_BYTES: u64 = 512 * 1024;
/// Directory levels `skill index` descends below the directory it is given.
pub const MAX_SCAN_DEPTH: usize = 3;
/// Directories `skill index` looks at in one run.
pub const MAX_SCAN_DIRS: usize = 5_000;
/// Skills one `skill index` run reads.
pub const MAX_SKILLS_PER_INDEX: usize = 500;
/// Longest description (the Agent Skills limit).
pub const MAX_DESCRIPTION_CHARS: usize = 1024;
/// Longest frontmatter block read.
const MAX_FRONTMATTER_BYTES: usize = 16 * 1024;

/// Extensions of the files that are indexed for search (the rest are hashed
/// only).
const INDEXED_EXTENSIONS: &[&str] = &["md", "markdown", "txt"];

/// A file of a package, read once: the hash and the index see the same bytes.
#[derive(Debug, Clone)]
pub struct PackageFile {
    /// Path relative to the skill directory, `/` separated.
    pub path: String,
    pub bytes: Vec<u8>,
}

/// A skill package read from disk.
#[derive(Debug, Clone)]
pub struct SkillPackage {
    /// The directory name.
    pub slug: String,
    /// The directory the package was read from, as given (absolute).
    pub dir: PathBuf,
    /// Sorted by path.
    pub files: Vec<PackageFile>,
    /// [`package_hash`] of `files`.
    pub hash: String,
}

/// Why a directory could not be read as a skill package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageRejection {
    pub path: String,
    pub reason: String,
}

fn reject(dir: &Path, reason: impl Into<String>) -> PackageRejection {
    PackageRejection {
        path: dir.display().to_string(),
        reason: reason.into(),
    }
}

/// `true` for a slug: lower-case letters and digits in groups joined by single
/// hyphens, 1 to 64 characters (the Agent Skills `name` rule).
pub fn is_valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= MAX_SLUG_LEN
        && s.split('-').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// The package hash (module docs), `sha256:<hex>`, identical to tole's:
/// `files` is sorted here, so the order the caller holds them in does not
/// matter.
pub fn package_hash(files: &[PackageFile]) -> String {
    let mut order: Vec<&PackageFile> = files.iter().collect();
    order.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let mut h = Sha256::new();
    for f in order {
        h.update((f.path.len() as u64).to_le_bytes());
        h.update(f.path.as_bytes());
        h.update((f.bytes.len() as u64).to_le_bytes());
        h.update(&f.bytes);
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    format!("{HASH_PREFIX}{hex}")
}

/// The hex digits of a package hash, without [`HASH_PREFIX`]. A string
/// without the prefix is returned as is.
pub fn hash_hex(hash: &str) -> &str {
    hash.strip_prefix(HASH_PREFIX).unwrap_or(hash)
}

// ── Discovery ─────────────────────────────────────────────────────────────

/// What a scan of a directory found.
#[derive(Debug, Default)]
pub struct Discovery {
    /// Skill directories (each holds a `SKILL.md`), sorted.
    pub skills: Vec<PathBuf>,
    /// Directories looked at.
    pub scanned_dirs: usize,
    /// `true` when a bound stopped the scan early.
    pub truncated: bool,
    pub warnings: Vec<String>,
}

/// Find the skill directories under `root`: `root` itself when it holds a
/// `SKILL.md`, else every directory below it, to [`MAX_SCAN_DEPTH`] levels,
/// that does. A skill directory is not searched further (its sub-directories
/// are reference material), symbolic links are not followed, and the scan
/// stops at [`MAX_SCAN_DIRS`] directories or [`MAX_SKILLS_PER_INDEX`] skills.
pub fn discover_skills(root: &Path) -> Result<Discovery, String> {
    let meta =
        fs::symlink_metadata(root).map_err(|e| format!("cannot read {}: {e}", root.display()))?;
    // The directory the user names is followed even if it is a link; nothing
    // below it is.
    let root = if meta.file_type().is_symlink() {
        fs::canonicalize(root).map_err(|e| format!("cannot read {}: {e}", root.display()))?
    } else {
        root.to_path_buf()
    };
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    let mut out = Discovery::default();
    let mut stack: Vec<(PathBuf, usize)> = vec![(root, 0)];
    while let Some((dir, depth)) = stack.pop() {
        if out.scanned_dirs >= MAX_SCAN_DIRS {
            out.truncated = true;
            break;
        }
        out.scanned_dirs += 1;
        if is_regular_file(&dir.join(SKILL_FILE)) {
            if out.skills.len() >= MAX_SKILLS_PER_INDEX {
                out.truncated = true;
                break;
            }
            out.skills.push(dir);
            continue;
        }
        if depth >= MAX_SCAN_DEPTH {
            continue;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                out.warnings
                    .push(format!("cannot read {}: {e}", dir.display()));
                continue;
            }
        };
        let mut subdirs: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let Ok(ft) = entry.file_type() else { continue };
            // `file_type` does not follow links: a link to a directory is a
            // symlink here and is skipped.
            if ft.is_dir() && entry.file_name() != ".git" {
                subdirs.push(entry.path());
            }
            // Bound the directory listing itself.
            if subdirs.len() > MAX_SCAN_DIRS {
                out.truncated = true;
                break;
            }
        }
        subdirs.sort();
        // Reversed so the stack pops in sorted order.
        for d in subdirs.into_iter().rev() {
            stack.push((d, depth + 1));
        }
    }
    out.skills.sort();
    Ok(out)
}

fn is_regular_file(p: &Path) -> bool {
    fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file())
}

// ── Reading ───────────────────────────────────────────────────────────────

/// Read the package in `dir`: every regular file (`.git` included), once,
/// within the bounds.
///
/// # Errors
///
/// A [`PackageRejection`] when the directory name is not a slug, a bound is
/// exceeded (files, bytes, entries, depth), a path is not UTF-8, the package
/// holds a symbolic link or a special file, `SKILL.md` is missing, not UTF-8
/// or larger than [`MAX_INDEXED_FILE_BYTES`], or a read fails.
pub fn read_package(dir: &Path) -> Result<SkillPackage, PackageRejection> {
    let slug = dir
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| reject(dir, "directory name is not valid UTF-8"))?
        .to_string();
    if !is_valid_slug(&slug) {
        return Err(reject(
            dir,
            format!(
                "directory name '{slug}' is not a valid skill slug \
                 (lower-case letters, digits and single hyphens, at most {MAX_SLUG_LEN} characters)"
            ),
        ));
    }
    let mut files: Vec<PackageFile> = Vec::new();
    let mut total: u64 = 0;
    let mut seen: usize = 0;
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(dir.to_path_buf(), String::new(), 0)];
    while let Some((cur, rel, depth)) = stack.pop() {
        let entries = fs::read_dir(&cur)
            .map_err(|e| reject(dir, format!("cannot read {}: {e}", cur.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| reject(dir, format!("cannot list a directory: {e}")))?;
            seen += 1;
            if seen > MAX_PACKAGE_ENTRIES {
                return Err(reject(
                    dir,
                    format!("more than {MAX_PACKAGE_ENTRIES} entries (files and directories)"),
                ));
            }
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| reject(dir, "a file name is not valid UTF-8"))?;
            let rel_path = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            let ft = entry
                .file_type()
                .map_err(|e| reject(dir, format!("cannot stat {rel_path}: {e}")))?;
            if ft.is_symlink() {
                return Err(reject(
                    dir,
                    format!("{rel_path}: symbolic link in a skill package"),
                ));
            } else if ft.is_dir() {
                if depth + 1 > MAX_PACKAGE_DEPTH {
                    return Err(reject(
                        dir,
                        format!("{rel_path}: deeper than {MAX_PACKAGE_DEPTH} directory levels"),
                    ));
                }
                stack.push((entry.path(), rel_path, depth + 1));
            } else if ft.is_file() {
                if files.len() >= MAX_PACKAGE_FILES {
                    return Err(reject(
                        dir,
                        format!("more than {MAX_PACKAGE_FILES} files in the package"),
                    ));
                }
                let bytes = read_bounded(&entry.path(), MAX_FILE_BYTES)
                    .map_err(|e| reject(dir, format!("{rel_path}: {e}")))?;
                total += bytes.len() as u64;
                if total > MAX_PACKAGE_BYTES {
                    return Err(reject(
                        dir,
                        format!("the package is larger than {MAX_PACKAGE_BYTES} bytes"),
                    ));
                }
                files.push(PackageFile {
                    path: rel_path,
                    bytes,
                });
            } else {
                return Err(reject(
                    dir,
                    format!("{rel_path}: not a regular file (special file in a skill package)"),
                ));
            }
        }
    }
    files.sort_by(|a, b| a.path.as_bytes().cmp(b.path.as_bytes()));
    let skill = files
        .iter()
        .find(|f| f.path == SKILL_FILE)
        .ok_or_else(|| reject(dir, format!("no {SKILL_FILE} at the package root")))?;
    if skill.bytes.len() as u64 > MAX_INDEXED_FILE_BYTES {
        return Err(reject(
            dir,
            format!("{SKILL_FILE} is larger than {MAX_INDEXED_FILE_BYTES} bytes"),
        ));
    }
    if std::str::from_utf8(&skill.bytes).is_err() {
        return Err(reject(dir, format!("{SKILL_FILE} is not valid UTF-8")));
    }
    let hash = package_hash(&files);
    Ok(SkillPackage {
        slug,
        dir: dir.to_path_buf(),
        files,
        hash,
    })
}

/// Read a whole file, failing when it is longer than `max` bytes (the read
/// itself is capped, so a file that grows while it is read cannot overrun).
fn read_bounded(path: &Path, max: u64) -> Result<Vec<u8>, String> {
    let file = fs::File::open(path).map_err(|e| format!("cannot open: {e}"))?;
    let mut buf = Vec::new();
    file.take(max + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("cannot read: {e}"))?;
    if buf.len() as u64 > max {
        return Err(format!("larger than {max} bytes"));
    }
    Ok(buf)
}

/// One file of a package as the index sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct ManifestEntry {
    pub path: String,
    pub bytes: u64,
    /// `true` when the file is searchable (indexed as a document).
    pub indexed: bool,
}

/// The manifest of a package, and which of its files are indexed: text files
/// (`.md`, `.markdown`, `.txt`) that are UTF-8 and within
/// [`MAX_INDEXED_FILE_BYTES`], `SKILL.md` first, up to [`MAX_INDEXED_FILES`]
/// files and [`MAX_INDEXED_TOTAL_BYTES`] bytes. Files over a bound are still
/// hashed.
pub fn manifest(files: &[PackageFile]) -> Vec<ManifestEntry> {
    let mut count = 0usize;
    let mut total = 0u64;
    let mut order: Vec<usize> = (0..files.len()).collect();
    // SKILL.md first so a bound never drops it.
    order.sort_by_key(|&i| (files[i].path != SKILL_FILE, i));
    let mut indexed = vec![false; files.len()];
    for i in order {
        let f = &files[i];
        let ext_ok = f
            .path
            .rsplit_once('.')
            .is_some_and(|(_, e)| INDEXED_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
        let len = f.bytes.len() as u64;
        if ext_ok
            && len <= MAX_INDEXED_FILE_BYTES
            && count < MAX_INDEXED_FILES
            && total + len <= MAX_INDEXED_TOTAL_BYTES
            && std::str::from_utf8(&f.bytes).is_ok()
        {
            indexed[i] = true;
            count += 1;
            total += len;
        }
    }
    files
        .iter()
        .zip(indexed)
        .map(|(f, indexed)| ManifestEntry {
            path: f.path.clone(),
            bytes: f.bytes.len() as u64,
            indexed,
        })
        .collect()
}

// ── Frontmatter ───────────────────────────────────────────────────────────

/// The top-level keys of a `SKILL.md` frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frontmatter {
    /// Top-level `key: value` pairs, values unquoted. A key with a nested
    /// block (`metadata:`) has an empty value; the block is skipped.
    pub fields: BTreeMap<String, String>,
}

/// Parse the frontmatter of `text`: `---` on the first line, `key: value`
/// lines, `---` on a line of its own. Plain, single- and double-quoted
/// scalars and the `>` / `|` block scalars are understood; anything nested is
/// skipped. This is deliberately not a YAML parser: it reads the keys a skill
/// needs and reports what it cannot.
///
/// # Errors
///
/// A message when there is no frontmatter, it is not closed within 16 KiB, or
/// a line is not `key: value`.
pub fn parse_frontmatter(text: &str) -> Result<Frontmatter, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.split('\n');
    let first = lines.next().unwrap_or("").trim_end_matches('\r');
    if first.trim_end() != "---" {
        return Err("no frontmatter: the first line must be '---'".into());
    }
    let mut consumed = first.len() + 1;
    let mut fields: BTreeMap<String, String> = BTreeMap::new();
    // key, value, literal block (`|`), takes continuation lines
    let mut current: Option<(String, String, bool, bool)> = None;
    let mut closed = false;
    for raw in lines {
        consumed += raw.len() + 1;
        if consumed > MAX_FRONTMATTER_BYTES {
            return Err("frontmatter is not closed within 16 KiB".into());
        }
        let line = raw.trim_end_matches('\r');
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        let indented = line.starts_with(' ') || line.starts_with('\t');
        if indented || line.trim().is_empty() {
            // A continuation of a block or plain multi-line scalar, or a
            // nested block we skip.
            if let Some((_, value, block, takes)) = current.as_mut() {
                let piece = line.trim();
                if *takes && !piece.is_empty() {
                    if !value.is_empty() {
                        value.push(if *block { '\n' } else { ' ' });
                    }
                    value.push_str(piece);
                }
            }
            continue;
        }
        if line.trim_start().starts_with('#') {
            continue;
        }
        if let Some((k, v, _, _)) = current.take() {
            fields.insert(k, v);
        }
        let Some((key, value)) = line.split_once(':') else {
            return Err(format!("frontmatter line is not 'key: value': {line}"));
        };
        let key = key.trim();
        if key.is_empty() {
            return Err(format!("frontmatter line has no key: {line}"));
        }
        let value = value.trim();
        let (value, block, takes) = if matches!(value, ">" | ">-" | ">+" | "|" | "|-" | "|+") {
            (String::new(), value.starts_with('|'), true)
        } else {
            // A plain scalar may continue on indented lines; an empty value
            // opens a nested block, which is skipped.
            (unquote(value), false, !value.is_empty())
        };
        current = Some((key.to_string(), value, block, takes));
    }
    if let Some((k, v, _, _)) = current.take() {
        fields.insert(k, v);
    }
    if !closed {
        return Err("frontmatter is not closed: no closing '---' line".into());
    }
    Ok(Frontmatter { fields })
}

fn unquote(v: &str) -> String {
    let b = v.as_bytes();
    if b.len() >= 2 && (b[0] == b'"' || b[0] == b'\'') && b[b.len() - 1] == b[0] {
        let inner = &v[1..v.len() - 1];
        if b[0] == b'\'' {
            return inner.replace("''", "'");
        }
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    // A plain scalar ends at ' #' (a comment).
    match v.find(" #") {
        Some(i) => v[..i].trim_end().to_string(),
        None => v.to_string(),
    }
}

// ── Lint ──────────────────────────────────────────────────────────────────

/// How serious a lint finding is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The skill is invalid and cannot be approved.
    Error,
    Warning,
}

/// One finding of [`lint_skill`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct LintIssue {
    pub severity: Severity,
    /// A stable code, e.g. `missing-description`.
    pub code: String,
    pub message: String,
}

fn issue(severity: Severity, code: &str, message: impl Into<String>) -> LintIssue {
    LintIssue {
        severity,
        code: code.to_string(),
        message: message.into(),
    }
}

/// What a skill's `SKILL.md` says about itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillMeta {
    pub name: Option<String>,
    pub description: Option<String>,
}

/// Check the `SKILL.md` of the skill in directory `slug`.
///
/// Errors (the skill is invalid): no or unclosed frontmatter, no `name`, a
/// `name` that is not a slug or differs from the directory name, no or
/// over-long `description`. Warnings: an empty body, an over-long
/// `compatibility`.
pub fn lint_skill(slug: &str, skill_md: &str) -> (SkillMeta, Vec<LintIssue>) {
    let mut issues = Vec::new();
    let fm = match parse_frontmatter(skill_md) {
        Ok(fm) => fm,
        Err(msg) => {
            issues.push(issue(Severity::Error, "frontmatter", msg));
            return (SkillMeta::default(), issues);
        }
    };
    let name = fm.fields.get("name").map(|s| s.trim().to_string());
    match name.as_deref() {
        None | Some("") => issues.push(issue(
            Severity::Error,
            "missing-name",
            "frontmatter has no 'name'",
        )),
        Some(n) => {
            if !is_valid_slug(n) {
                issues.push(issue(
                    Severity::Error,
                    "invalid-name",
                    format!(
                        "name '{n}' is not a valid skill name (lower-case letters, digits and \
                         single hyphens, at most {MAX_SLUG_LEN} characters)"
                    ),
                ));
            } else if n != slug {
                issues.push(issue(
                    Severity::Error,
                    "name-mismatch",
                    format!("name '{n}' differs from the directory name '{slug}'"),
                ));
            }
        }
    }
    let description = fm.fields.get("description").map(|s| s.trim().to_string());
    match description.as_deref() {
        None | Some("") => issues.push(issue(
            Severity::Error,
            "missing-description",
            "frontmatter has no 'description'",
        )),
        Some(d) => {
            let n = d.chars().count();
            if n > MAX_DESCRIPTION_CHARS {
                issues.push(issue(
                    Severity::Error,
                    "description-too-long",
                    format!("description is {n} characters, the limit is {MAX_DESCRIPTION_CHARS}"),
                ));
            }
        }
    }
    if let Some(c) = fm.fields.get("compatibility") {
        let n = c.chars().count();
        if n > 500 {
            issues.push(issue(
                Severity::Warning,
                "compatibility-too-long",
                format!("compatibility is {n} characters, the limit is 500"),
            ));
        }
    }
    if skill_md_body_is_blank(skill_md) {
        issues.push(issue(
            Severity::Warning,
            "empty-body",
            "SKILL.md has no instructions after the frontmatter",
        ));
    }
    (
        SkillMeta {
            name: name.filter(|n| !n.is_empty()),
            description: description.filter(|d| !d.is_empty()),
        },
        issues,
    )
}

fn skill_md_body_is_blank(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut seen_open = false;
    let mut lines = text.split('\n');
    for l in lines.by_ref() {
        if l.trim_end() == "---" {
            if seen_open {
                break;
            }
            seen_open = true;
        }
    }
    lines.all(|l| l.trim().is_empty())
}

/// `true` when `issues` holds no error.
pub fn is_valid(issues: &[LintIssue]) -> bool {
    !issues.iter().any(|i| i.severity == Severity::Error)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pf(path: &str, bytes: &[u8]) -> PackageFile {
        PackageFile {
            path: path.to_string(),
            bytes: bytes.to_vec(),
        }
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, content).unwrap();
    }

    const GOOD: &str = "---\nname: demo\ndescription: Does a demo.\n---\n# Demo\n\nSteps.\n";

    #[test]
    fn slug_rules() {
        for ok in ["a", "demo", "a-b-c", "x1", "1x", &"a".repeat(64)] {
            assert!(is_valid_slug(ok), "{ok}");
        }
        for bad in [
            "",
            "-a",
            "a-",
            "a--b",
            "A",
            "a_b",
            "a b",
            "a/b",
            "é",
            &"a".repeat(65),
        ] {
            assert!(!is_valid_slug(bad), "{bad}");
        }
    }

    #[test]
    fn hash_is_pinned_and_order_independent() {
        // One file: SHA-256 over len(path)=8 LE, "a", len(content)=1 LE, "x",
        // written `sha256:<hex>` (tole's encoding).
        let one = package_hash(&[pf("a", b"x")]);
        let mut h = Sha256::new();
        h.update(1u64.to_le_bytes());
        h.update(b"a");
        h.update(1u64.to_le_bytes());
        h.update(b"x");
        let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(one, format!("sha256:{hex}"));
        assert_eq!(one.len(), 71);
        assert_eq!(hash_hex(&one), hex);
        assert_eq!(hash_hex(&hex), hex, "a bare hash is left as it is");
        assert_eq!(
            package_hash(&[pf("b", b"2"), pf("a", b"1")]),
            package_hash(&[pf("a", b"1"), pf("b", b"2")])
        );
        // Empty package is the hash of nothing.
        assert_eq!(
            package_hash(&[]),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// Test vectors computed with tole's own `skill_approval::package_hash`
    /// and `read_package` (codecoradev/tole @ 0c212a6), on the file sets below.
    /// Not derived from this implementation: they pin cross-implementation
    /// compatibility (`TOLE_SKILL_APPROVAL=required`). If one fails, uteke no
    /// longer hashes a package the way tole does; fix uteke, do not edit the
    /// vector unless tole changed its encoding (then re-derive from tole).
    const TOLE_EMPTY: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const TOLE_SINGLE: &str =
        "sha256:f0f5340d3d4d2e2e64563d9fa530ba57d3a865733f3761586a6d0919fe6c4399";
    const TOLE_FULL: &str =
        "sha256:0886321c37ecf45e90fff052843313420e85079649dd23566c03833f4e07f8d2";
    const TOLE_SKILL_MD: &[u8] =
        b"---\nname: demo\ndescription: A demo skill.\n---\n# Demo\n\nBody.\n";

    /// A nested path, an empty file, binary bytes, non-ASCII content and file
    /// name, and a `.git` file, all in tole's walk.
    fn tole_full_set() -> Vec<(&'static str, Vec<u8>)> {
        vec![
            ("SKILL.md", TOLE_SKILL_MD.to_vec()),
            ("references/a.md", b"ref a".to_vec()),
            (
                "references/sub/deep.md",
                "h\u{e9}llo w\u{f6}rld \u{65e5}\u{672c}\u{8a9e}\n"
                    .as_bytes()
                    .to_vec(),
            ),
            ("scripts/run.sh", b"echo hi".to_vec()),
            ("assets/empty.txt", vec![]),
            ("assets/blob.bin", vec![0xff, 0x00, 0x80, 0x7f, 0x00]),
            ("references/\u{fc}n\u{ef}.md", b"unicode name".to_vec()),
            (".git/config", b"[core]\n".to_vec()),
        ]
    }

    #[test]
    fn hash_matches_tole_test_vectors() {
        assert_eq!(package_hash(&[]), TOLE_EMPTY);
        assert_eq!(package_hash(&[pf("SKILL.md", TOLE_SKILL_MD)]), TOLE_SINGLE);
        let full: Vec<PackageFile> = tole_full_set()
            .into_iter()
            .map(|(p, b)| pf(p, &b))
            .collect();
        assert_eq!(package_hash(&full), TOLE_FULL);
    }

    #[test]
    fn read_package_of_the_tole_vector_directory_matches_tole() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("demo");
        for (path, bytes) in tole_full_set() {
            let f = dir.join(path);
            fs::create_dir_all(f.parent().unwrap()).unwrap();
            fs::write(f, bytes).unwrap();
        }
        let p = read_package(&dir).unwrap();
        assert_eq!(p.files.len(), 8, ".git is part of the package");
        assert_eq!(p.hash, TOLE_FULL);
        let single = t.path().join("single");
        write(
            &single,
            "SKILL.md",
            std::str::from_utf8(TOLE_SKILL_MD).unwrap(),
        );
        assert_eq!(read_package(&single).unwrap().hash, TOLE_SINGLE);
    }

    #[test]
    fn hash_changes_with_path_length_and_bytes() {
        let base = package_hash(&[pf("a", b"x")]);
        assert_ne!(base, package_hash(&[pf("b", b"x")]), "path");
        assert_ne!(base, package_hash(&[pf("a", b"y")]), "bytes");
        assert_ne!(base, package_hash(&[pf("a", b"xx")]), "length");
        // The length prefixes keep a moved byte from colliding.
        assert_ne!(
            package_hash(&[pf("ab", b"c")]),
            package_hash(&[pf("a", b"bc")])
        );
        assert_ne!(
            package_hash(&[pf("a", b"x"), pf("b", b"")]),
            package_hash(&[pf("a", b"x")])
        );
    }

    #[test]
    fn frontmatter_scalars() {
        let fm = parse_frontmatter(
            "---\nname: demo\ndescription: \"Say \\\"hi\\\": now\"\nauthor: 'it''s'\n\
             note: plain # comment\nmetadata:\n  a: 1\n  b: 2\nlong: >\n  one\n  two\n---\nbody",
        )
        .unwrap();
        assert_eq!(fm.fields["name"], "demo");
        assert_eq!(fm.fields["description"], "Say \"hi\": now");
        assert_eq!(fm.fields["author"], "it's");
        assert_eq!(fm.fields["note"], "plain");
        assert_eq!(fm.fields["metadata"], "");
        assert_eq!(fm.fields["long"], "one two");
    }

    #[test]
    fn frontmatter_errors() {
        assert!(parse_frontmatter("# no fm").is_err());
        assert!(parse_frontmatter("---\nname: x\n").is_err());
        assert!(parse_frontmatter("---\nnot a pair\n---\n").is_err());
        assert!(parse_frontmatter("---\n: v\n---\n").is_err());
        let big = format!("---\nname: x\n{}", "# pad\n".repeat(4000));
        assert!(parse_frontmatter(&big).is_err());
        // CRLF and BOM are fine.
        let fm = parse_frontmatter("\u{feff}---\r\nname: x\r\n---\r\nbody").unwrap();
        assert_eq!(fm.fields["name"], "x");
    }

    #[test]
    fn lint_accepts_a_good_skill_and_flags_each_problem() {
        let (meta, issues) = lint_skill("demo", GOOD);
        assert!(issues.is_empty(), "{issues:?}");
        assert_eq!(meta.name.as_deref(), Some("demo"));
        assert_eq!(meta.description.as_deref(), Some("Does a demo."));

        let codes = |slug: &str, text: &str| -> Vec<String> {
            lint_skill(slug, text)
                .1
                .into_iter()
                .map(|i| i.code)
                .collect()
        };
        assert_eq!(codes("demo", "# plain"), ["frontmatter"]);
        assert_eq!(
            codes("demo", "---\ndescription: d\n---\nb"),
            ["missing-name"]
        );
        assert_eq!(
            codes("demo", "---\nname: demo\n---\nb"),
            ["missing-description"]
        );
        assert_eq!(
            codes("demo", "---\nname: Demo\ndescription: d\n---\nb"),
            ["invalid-name"]
        );
        assert_eq!(
            codes("demo", "---\nname: other\ndescription: d\n---\nb"),
            ["name-mismatch"]
        );
        let long = format!("---\nname: demo\ndescription: {}\n---\nb", "x".repeat(1025));
        assert_eq!(codes("demo", &long), ["description-too-long"]);
        let exact = format!("---\nname: demo\ndescription: {}\n---\nb", "x".repeat(1024));
        assert!(codes("demo", &exact).is_empty());
        assert_eq!(
            codes("demo", "---\nname: demo\ndescription: d\n---\n\n  \n"),
            ["empty-body"]
        );
        let (_, w) = lint_skill("demo", "---\nname: demo\ndescription: d\n---\n");
        assert_eq!(w[0].severity, Severity::Warning);
        assert!(is_valid(&w));
        assert!(!is_valid(&lint_skill("demo", "x").1));
    }

    #[test]
    fn discovery_finds_skills_and_does_not_descend_into_them() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a/SKILL.md", GOOD);
        write(t.path(), "a/inner/SKILL.md", GOOD);
        write(t.path(), "group/b/SKILL.md", GOOD);
        write(t.path(), "group/deep/er/est/c/SKILL.md", GOOD);
        write(t.path(), ".git/x/SKILL.md", GOOD);
        write(t.path(), "plain/readme.md", "x");
        let d = discover_skills(t.path()).unwrap();
        let names: Vec<_> = d
            .skills
            .iter()
            .map(|p| {
                p.strip_prefix(t.path())
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(names, ["a", "group/b"]);
        assert!(!d.truncated);
        // The directory itself is a skill.
        let d = discover_skills(&t.path().join("a")).unwrap();
        assert_eq!(d.skills.len(), 1);
        assert!(discover_skills(&t.path().join("none")).is_err());
        assert!(discover_skills(&t.path().join("a/SKILL.md")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn discovery_and_reading_never_follow_links() {
        let t = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "evil/SKILL.md", GOOD);
        write(outside.path(), "secret.txt", "top secret");
        write(t.path(), "demo/SKILL.md", GOOD);
        std::os::unix::fs::symlink(outside.path().join("evil"), t.path().join("evil")).unwrap();
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            t.path().join("demo/secret.txt"),
        )
        .unwrap();
        let d = discover_skills(t.path()).unwrap();
        assert_eq!(d.skills.len(), 1, "a linked directory is not followed");
        // tole refuses a package with a symlink, so it cannot be approved
        // here either (a skipped link would give a hash tole never computes).
        let r = read_package(&d.skills[0]).unwrap_err();
        assert!(
            r.reason.contains("secret.txt: symbolic link"),
            "{}",
            r.reason
        );
        // A link to a directory is refused the same way.
        fs::remove_file(t.path().join("demo/secret.txt")).unwrap();
        std::os::unix::fs::symlink(outside.path(), t.path().join("demo/linked")).unwrap();
        assert!(
            read_package(&t.path().join("demo"))
                .unwrap_err()
                .reason
                .contains("linked: symbolic link")
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_special_file_makes_the_package_unreadable() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "demo/SKILL.md", GOOD);
        let fifo = t.path().join("demo/pipe");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if !made.is_ok_and(|s| s.success()) {
            return; // no mkfifo on this machine
        }
        let r = read_package(&t.path().join("demo")).unwrap_err();
        assert!(
            r.reason.contains("pipe: not a regular file"),
            "{}",
            r.reason
        );
    }

    #[test]
    fn read_package_hashes_every_file_at_any_depth() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("demo");
        write(&dir, "SKILL.md", GOOD);
        write(&dir, "references/a.md", "ref a");
        write(&dir, "scripts/run.sh", "echo hi");
        // `.git` is part of the package, as it is for tole.
        write(&dir, ".git/config", "[core]");
        let p = read_package(&dir).unwrap();
        assert_eq!(p.slug, "demo");
        assert_eq!(
            p.files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
            [
                ".git/config",
                "SKILL.md",
                "references/a.md",
                "scripts/run.sh"
            ]
        );
        assert_eq!(p.hash, package_hash(&p.files));
        // Same bytes elsewhere, same hash: the directory is not part of it.
        let other = t.path().join("copy");
        write(&other, "SKILL.md", GOOD);
        write(&other, "references/a.md", "ref a");
        write(&other, "scripts/run.sh", "echo hi");
        write(&other, ".git/config", "[core]");
        assert_eq!(read_package(&other).unwrap().hash, p.hash);
        // A changed byte changes it.
        write(&other, "scripts/run.sh", "echo ho");
        assert_ne!(read_package(&other).unwrap().hash, p.hash);
    }

    #[test]
    fn read_package_rejects_what_it_must() {
        let t = tempfile::tempdir().unwrap();
        let rej = |name: &str| read_package(&t.path().join(name)).unwrap_err().reason;

        write(t.path(), "Bad_Name/SKILL.md", GOOD);
        assert!(rej("Bad_Name").contains("not a valid skill slug"));

        fs::create_dir_all(t.path().join("empty")).unwrap();
        assert!(rej("empty").contains("no SKILL.md"));

        write(t.path(), "bin/SKILL.md", "x");
        fs::write(t.path().join("bin/SKILL.md"), [0xff, 0xfe, 0x00]).unwrap();
        assert!(rej("bin").contains("not valid UTF-8"));

        write(t.path(), "huge/SKILL.md", &"x".repeat(128 * 1024 + 1));
        assert!(rej("huge").contains("SKILL.md is larger than 131072"));
        // Exactly at the bound is accepted.
        write(t.path(), "fits/SKILL.md", &"x".repeat(128 * 1024));
        assert!(read_package(&t.path().join("fits")).is_ok());

        write(t.path(), "bigfile/SKILL.md", GOOD);
        fs::write(
            t.path().join("bigfile/data.bin"),
            vec![0u8; (MAX_FILE_BYTES + 1) as usize],
        )
        .unwrap();
        assert!(rej("bigfile").contains("data.bin: larger than 1048576"));

        write(t.path(), "many/SKILL.md", GOOD);
        for i in 0..MAX_PACKAGE_FILES {
            write(t.path(), &format!("many/f{i}.txt"), "x");
        }
        assert!(rej("many").contains("more than 200 files"));

        write(t.path(), "deep/SKILL.md", GOOD);
        write(t.path(), "deep/a/b/c/d/e/f/g/h/i/x.txt", "x");
        assert!(rej("deep").contains("deeper than 8"));
        write(t.path(), "deep8/SKILL.md", GOOD);
        write(t.path(), "deep8/a/b/c/d/e/f/g/h/x.txt", "x");
        assert!(read_package(&t.path().join("deep8")).is_ok());

        write(t.path(), "total/SKILL.md", GOOD);
        for i in 0..4 {
            fs::write(
                t.path().join(format!("total/f{i}.bin")),
                vec![0u8; MAX_FILE_BYTES as usize],
            )
            .unwrap();
        }
        assert!(rej("total").contains("larger than 4194304"));

        // Directories count against the entry limit like files (tole's rule),
        // so a tree of countless empty directories is refused.
        write(t.path(), "dirs/SKILL.md", GOOD);
        for i in 0..MAX_PACKAGE_ENTRIES {
            fs::create_dir_all(t.path().join(format!("dirs/d{i}"))).unwrap();
        }
        assert!(rej("dirs").contains("more than 1024 entries"));
    }

    #[test]
    fn manifest_indexes_text_files_within_bounds_and_keeps_skill_md() {
        let mut files = vec![
            pf("SKILL.md", GOOD.as_bytes()),
            pf("references/a.md", b"a"),
            pf("references/b.TXT", b"b"),
            pf("scripts/run.sh", b"echo"),
            pf("notes.md", &[0xff, 0xfe]),
        ];
        let m = manifest(&files);
        let idx: Vec<(&str, bool)> = m.iter().map(|e| (e.path.as_str(), e.indexed)).collect();
        assert_eq!(
            idx,
            [
                ("SKILL.md", true),
                ("references/a.md", true),
                ("references/b.TXT", true),
                ("scripts/run.sh", false),
                ("notes.md", false),
            ]
        );
        // The file count bound drops later files, never SKILL.md.
        files.clear();
        files.push(pf("SKILL.md", b"x"));
        for i in 0..40 {
            files.push(pf(&format!("r{i:02}.md"), b"x"));
        }
        let m = manifest(&files);
        assert_eq!(m.iter().filter(|e| e.indexed).count(), MAX_INDEXED_FILES);
        assert!(m[0].indexed);
        assert!(m[MAX_INDEXED_FILES - 1].indexed);
        assert!(!m[MAX_INDEXED_FILES].indexed);
        // The byte bound: 4 files of 128 KiB fill the 512 KiB budget.
        let big = vec![b'x'; 128 * 1024];
        let files: Vec<PackageFile> = std::iter::once(pf("SKILL.md", &big))
            .chain((0..4).map(|i| pf(&format!("r{i}.md"), &big)))
            .collect();
        let m = manifest(&files);
        assert_eq!(m.iter().filter(|e| e.indexed).count(), 4);
        // A file one byte over the per-file bound is not indexed.
        let over = vec![b'x'; 128 * 1024 + 1];
        assert!(!manifest(&[pf("SKILL.md", b"x"), pf("a.md", &over)])[1].indexed);
    }

    #[test]
    fn discovery_counts_every_directory_it_scans() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a/SKILL.md", GOOD);
        write(t.path(), "b/inner/SKILL.md", GOOD);
        fs::create_dir_all(t.path().join("c")).unwrap();
        let d = discover_skills(t.path()).unwrap();
        // root, a, b, b/inner, c
        assert_eq!(d.scanned_dirs, 5);
        assert_eq!(d.skills.len(), 2);
        assert!(!d.truncated);
    }

    #[test]
    fn a_package_may_hold_exactly_the_entry_limit_and_no_more() {
        let t = tempfile::tempdir().unwrap();
        let ok = t.path().join("ok");
        write(&ok, "SKILL.md", GOOD);
        // SKILL.md plus empty directories: entries count files and directories.
        for i in 0..MAX_PACKAGE_ENTRIES - 1 {
            fs::create_dir_all(ok.join(format!("d{i}"))).unwrap();
        }
        assert!(read_package(&ok).is_ok());
        fs::create_dir_all(ok.join("one-more")).unwrap();
        let e = read_package(&ok).unwrap_err();
        assert!(
            e.reason.contains("more than 1024 entries"),
            "unexpected reason: {}",
            e.reason
        );
    }

    #[test]
    fn a_package_may_hold_exactly_the_byte_limit_and_no_more() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("big");
        write(&dir, "SKILL.md", GOOD);
        let mib = usize::try_from(MAX_FILE_BYTES).unwrap();
        let total = usize::try_from(MAX_PACKAGE_BYTES).unwrap();
        // Three files of exactly the per-file limit, and one that makes the
        // package exactly `MAX_PACKAGE_BYTES`.
        for i in 0..3 {
            fs::write(dir.join(format!("f{i}.bin")), vec![b'a'; mib]).unwrap();
        }
        let rest = total - GOOD.len() - 3 * mib;
        fs::write(dir.join("last.bin"), vec![b'b'; rest]).unwrap();
        let p = read_package(&dir).unwrap();
        assert_eq!(p.files.iter().map(|f| f.bytes.len()).sum::<usize>(), total);
        fs::write(dir.join("last.bin"), vec![b'b'; rest + 1]).unwrap();
        let e = read_package(&dir).unwrap_err();
        assert!(
            e.reason.contains("larger than 4194304 bytes"),
            "unexpected reason: {}",
            e.reason
        );
    }

    /// A frontmatter whose closing `---` line ends exactly `consumed` bytes in,
    /// counting the newline after it (the parser counts one for the last line
    /// even when the text ends there).
    fn frontmatter_of_consumed_size(consumed: usize) -> String {
        let head = "---\nname: a\n";
        let tail = "---";
        // "k: " + pad + "\n"
        let pad = consumed - 1 - head.len() - tail.len() - 4;
        format!("{head}k: {}\n{tail}", "y".repeat(pad))
    }

    #[test]
    fn frontmatter_closes_at_exactly_sixteen_kib_and_not_one_byte_later() {
        let ok = frontmatter_of_consumed_size(MAX_FRONTMATTER_BYTES);
        assert!(parse_frontmatter(&ok).is_ok());
        let over = frontmatter_of_consumed_size(MAX_FRONTMATTER_BYTES + 1);
        let e = parse_frontmatter(&over).unwrap_err();
        assert_eq!(e, "frontmatter is not closed within 16 KiB");
    }

    #[test]
    fn a_description_may_be_exactly_the_limit_long() {
        let md = |n: usize| {
            format!(
                "---\nname: demo\ndescription: {}\n---\n# Demo\n\nSteps.\n",
                "d".repeat(n)
            )
        };
        let codes = |n: usize| -> Vec<String> {
            lint_skill("demo", &md(n))
                .1
                .into_iter()
                .map(|i| i.code)
                .collect()
        };
        assert!(!codes(MAX_DESCRIPTION_CHARS).contains(&"description-too-long".to_string()));
        assert!(codes(MAX_DESCRIPTION_CHARS + 1).contains(&"description-too-long".to_string()));
        assert!(!codes(1).contains(&"description-too-long".to_string()));
    }

    #[test]
    fn compatibility_may_be_exactly_five_hundred_characters() {
        let md = |n: usize| {
            format!(
                "---\nname: demo\ndescription: Does a demo.\ncompatibility: {}\n---\n# Demo\n\nSteps.\n",
                "c".repeat(n)
            )
        };
        let long = |n: usize| -> bool {
            lint_skill("demo", &md(n))
                .1
                .iter()
                .any(|i| i.code == "compatibility-too-long")
        };
        assert!(!long(1));
        assert!(!long(500));
        assert!(long(501));
    }
}
