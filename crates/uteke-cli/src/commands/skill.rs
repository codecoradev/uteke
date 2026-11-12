//! `uteke skill` — the personal skill library (#1731).

use std::io::Read;
use std::path::Path;

use crate::cli::{Cli, SkillCommands};
use crate::output;
use uteke_core::Uteke;
use uteke_core::skill_library::{
    SkillDetail, SkillHit, SkillIndexReport, SkillLintReport, SkillSimilarReport, SkillSummary,
    UsageEvent, parse_skill_ref,
};
use uteke_core::skill_package::Severity;

/// Bytes read from a `--file` or stdin draft: well above the 200,000
/// characters the core accepts, so the core reports the real limit.
const MAX_DRAFT_BYTES: u64 = 1 << 20;

fn read_draft(file: &str) -> Result<String, String> {
    let mut buf = Vec::new();
    let read = if file == "-" {
        std::io::stdin()
            .take(MAX_DRAFT_BYTES + 1)
            .read_to_end(&mut buf)
    } else {
        std::fs::File::open(file).and_then(|f| f.take(MAX_DRAFT_BYTES + 1).read_to_end(&mut buf))
    };
    read.map_err(|e| format!("Failed to read {file}: {e}"))?;
    if buf.len() as u64 > MAX_DRAFT_BYTES {
        return Err(format!("{file} is larger than {MAX_DRAFT_BYTES} bytes"));
    }
    String::from_utf8(buf).map_err(|_| format!("{file} is not valid UTF-8"))
}

fn short(hash: &str) -> &str {
    output::char_prefix(hash, 12)
}

fn render_index(r: &SkillIndexReport) -> Vec<String> {
    let mut lines = vec![format!(
        "Indexed {} skill(s): {} new, {} updated, {} reverted, {} unchanged ({} directories scanned)",
        r.skills.len(),
        r.created,
        r.updated,
        r.reverted,
        r.unchanged,
        r.scanned_dirs
    )];
    for s in &r.skills {
        let action = format!("{:?}", s.action).to_lowercase();
        let mut line = format!(
            "  {action:<9} {:<28} {:<8} {}  {} file(s), {} indexed",
            output::char_prefix(&s.slug, 28),
            s.status,
            short(&s.hash),
            s.files,
            s.indexed_files
        );
        if !s.valid {
            line.push_str("  INVALID");
        }
        lines.push(line);
        for i in &s.issues {
            lines.push(format!(
                "      {}: {} ({})",
                severity(i.severity),
                i.message,
                i.code
            ));
        }
        for w in &s.warnings {
            lines.push(format!("      warning: {w}"));
        }
    }
    for n in &r.near_duplicates {
        lines.push(format!(
            "  {} is {} to {} ({:.3})",
            n.slug, n.relation, n.other, n.score
        ));
    }
    for rej in &r.rejected {
        lines.push(format!("  rejected {}: {}", rej.path, rej.reason));
    }
    for w in &r.warnings {
        lines.push(format!("  warning: {w}"));
    }
    if r.truncated {
        lines
            .push("  The scan stopped at a limit: index a narrower directory for the rest.".into());
    }
    lines
}

fn severity(s: Severity) -> &'static str {
    match s {
        Severity::Error => "error",
        Severity::Warning => "warning",
    }
}

fn render_hits(hits: &[SkillHit]) -> Vec<String> {
    if hits.is_empty() {
        return vec!["No skills found.".into()];
    }
    let mut lines = Vec::new();
    for (i, h) in hits.iter().enumerate() {
        lines.push(format!(
            "{}. {} [{}] {:.4}{}",
            i + 1,
            h.slug,
            h.status,
            h.score,
            if h.valid { "" } else { "  INVALID" }
        ));
        if let Some(d) = &h.description {
            lines.push(format!("   {d}"));
        }
        lines.push(format!("   {}  {}", h.file, short(&h.hash)));
    }
    lines
}

fn render_list(skills: &[SkillSummary]) -> Vec<String> {
    if skills.is_empty() {
        return vec!["No skills indexed. Run `uteke skill index <dir>`.".into()];
    }
    skills
        .iter()
        .map(|s| {
            format!(
                "{:<28} {:<8} {}  {}",
                output::char_prefix(&s.slug, 28),
                s.status,
                short(&s.hash),
                output::char_prefix(s.description.as_deref().unwrap_or(""), 60)
            )
        })
        .collect()
}

fn render_show(d: &SkillDetail) -> Vec<String> {
    let c = &d.current;
    let mut lines = vec![
        format!("{} ({})", d.slug, c.status),
        format!("  hash:     {}", c.hash),
        format!("  source:   {}", d.source_path),
    ];
    if let Some(o) = &d.owner {
        lines.push(format!("  owner:    {o}"));
    }
    if let Some(n) = &c.name {
        lines.push(format!("  name:     {n}"));
    }
    if let Some(desc) = &c.description {
        lines.push(format!("  about:    {desc}"));
    }
    lines.push(format!(
        "  valid:    {}   pinned: {}   locked: {}",
        c.valid, c.pinned, c.locked
    ));
    match (&c.approved_at, &d.approved_hash) {
        (Some(at), Some(_)) => lines.push(format!(
            "  approved: {at} by {}",
            c.approved_by.as_deref().unwrap_or("-")
        )),
        _ => lines.push("  approved: no".into()),
    }
    for i in &c.issues {
        lines.push(format!(
            "  {}: {} ({})",
            severity(i.severity),
            i.message,
            i.code
        ));
    }
    lines.push(format!(
        "  files ({}, {} bytes):",
        c.file_count, c.total_bytes
    ));
    for f in &c.files {
        lines.push(format!(
            "    {}{}  {} bytes",
            f.path,
            if f.indexed { "" } else { " (hashed only)" },
            f.bytes
        ));
    }
    let u = &d.usage;
    lines.push(format!(
        "  usage:    recalled {}, loaded {}, succeeded {}, failed {}",
        u.recalled, u.loaded, u.succeeded, u.failed
    ));
    lines.push("  versions:".into());
    for v in &d.versions {
        lines.push(format!(
            "    {} {:<8} {}{}",
            short(&v.hash),
            v.status,
            v.created_at,
            if v.current { "  (current)" } else { "" }
        ));
    }
    lines.push("  ledger:".into());
    for l in &d.ledger {
        lines.push(format!(
            "    #{} {} {:<9} {} -> {} {}",
            l.seq,
            l.at,
            l.action,
            l.before_hash.as_deref().map_or("-", short),
            l.after_hash.as_deref().map_or("-", short),
            l.actor.as_deref().unwrap_or("")
        ));
    }
    lines
}

fn render_similar(subject: &str, r: &SkillSimilarReport) -> Vec<String> {
    if !r.source_embedded {
        return vec![format!(
            "{subject} has no usable vector, so nothing can be compared. \
             Run `uteke repair --reembed`, or index it again."
        )];
    }
    let mut lines = vec![format!(
        "Skills similar to {subject} (threshold {:.2}; similar_to {:.2}, possible_duplicate {:.2})",
        r.threshold, r.similar_threshold, r.duplicate_threshold
    )];
    for h in &r.hits {
        lines.push(format!(
            "  {:<18} {:.3}  {:<28} {}",
            h.relation,
            h.score,
            output::char_prefix(&h.slug, 28),
            output::char_prefix(h.description.as_deref().unwrap_or(""), 50)
        ));
    }
    if r.hits.is_empty() {
        lines.push("  none".into());
    }
    lines.push(if r.matches > r.hits.len() {
        format!("{} match(es), showing {}", r.matches, r.hits.len())
    } else {
        format!("{} match(es)", r.matches)
    });
    if r.skills_without_vector > 0 {
        lines.push(format!(
            "{} skill(s) skipped: no usable vector",
            r.skills_without_vector
        ));
    }
    lines
}

fn render_lint(r: &SkillLintReport) -> Vec<String> {
    let mut lines = Vec::new();
    for s in &r.skills {
        lines.push(format!(
            "{} {}",
            if s.valid { "ok     " } else { "INVALID" },
            s.slug
        ));
        for i in &s.issues {
            lines.push(format!(
                "    {}: {} ({})",
                severity(i.severity),
                i.message,
                i.code
            ));
        }
    }
    for rej in &r.rejected {
        lines.push(format!("rejected {}: {}", rej.path, rej.reason));
    }
    if r.truncated {
        lines.push("The scan stopped at a limit: lint a narrower directory for the rest.".into());
    }
    let invalid = r.skills.iter().filter(|s| !s.valid).count();
    lines.push(format!(
        "{} skill(s), {} invalid, {} rejected",
        r.skills.len(),
        invalid,
        r.rejected.len()
    ));
    lines
}

fn print_lines(lines: Vec<String>) {
    for l in lines {
        println!("{l}");
    }
}

pub(crate) fn run(cli: &Cli, uteke: &mut Uteke, command: &SkillCommands) -> Result<(), String> {
    match command {
        SkillCommands::Index { dir, owner, actor } => {
            let report = uteke
                .skill_index(Path::new(dir), owner.as_deref(), actor.as_deref())
                .map_err(|e| format!("Failed to index skills: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                print_lines(render_index(&report));
            }
        }
        SkillCommands::Search {
            query,
            limit,
            mode,
            approved,
        } => {
            let hits = uteke
                .skill_search(query, *limit, mode, *approved)
                .map_err(|e| format!("Failed to search skills: {e}"))?;
            if cli.json {
                output::print_json(&hits);
            } else {
                print_lines(render_hits(&hits));
            }
        }
        SkillCommands::List { status, limit } => {
            let skills = uteke
                .skill_list(status.as_deref(), *limit)
                .map_err(|e| format!("Failed to list skills: {e}"))?;
            if cli.json {
                output::print_json(&skills);
            } else {
                print_lines(render_list(&skills));
            }
        }
        SkillCommands::Show { slug } => {
            let detail = uteke
                .skill_show(slug)
                .map_err(|e| format!("Failed to show the skill: {e}"))?;
            if cli.json {
                output::print_json(&detail);
            } else {
                print_lines(render_show(&detail));
            }
        }
        SkillCommands::Similar {
            slug,
            text,
            file,
            threshold,
            limit,
        } => {
            let (subject, report) = match (slug, text, file) {
                (Some(s), None, None) => {
                    (format!("'{s}'"), uteke.skill_similar(s, *threshold, *limit))
                }
                (None, Some(t), None) => (
                    "the text".to_string(),
                    uteke.skill_similar_to_text(t, *threshold, *limit),
                ),
                (None, None, Some(f)) => (
                    "the text".to_string(),
                    uteke.skill_similar_to_text(&read_draft(f)?, *threshold, *limit),
                ),
                (None, None, None) => {
                    return Err(
                        "Provide a skill slug, or --text <text>, or --file <path> (- for stdin)"
                            .into(),
                    );
                }
                _ => return Err("Give only one of: a skill slug, --text, --file".into()),
            };
            let report = report.map_err(|e| format!("Failed to find similar skills: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                print_lines(render_similar(&subject, &report));
            }
        }
        SkillCommands::Lint { dir } => {
            let report = match dir {
                Some(d) => uteke_core::skill_library::lint_directory(Path::new(d)),
                None => uteke.skill_lint_library(uteke_core::skill_library::MAX_LIST_LIMIT),
            }
            .map_err(|e| format!("Failed to lint skills: {e}"))?;
            if cli.json {
                output::print_json(&report);
            } else {
                print_lines(render_lint(&report));
            }
            let invalid = report.skills.iter().filter(|s| !s.valid).count() + report.rejected.len();
            if invalid > 0 {
                return Err(format!("{invalid} skill(s) invalid or unreadable"));
            }
        }
        SkillCommands::Approve { reference, actor } => {
            let (slug, hash) =
                parse_skill_ref(reference).map_err(|e| format!("Invalid skill reference: {e}"))?;
            let a = uteke
                .skill_approve(slug, &hash, actor.as_deref())
                .map_err(|e| format!("Failed to approve: {e}"))?;
            if cli.json {
                output::print_json(&a);
            } else if a.changed {
                println!("Approved {}@{}", a.slug, a.hash);
            } else {
                println!("{}@{} was already approved", a.slug, a.hash);
            }
        }
        SkillCommands::Record { slug, event } => {
            let event = UsageEvent::parse(event)
                .ok_or("event must be recalled, loaded, succeeded or failed")?;
            let usage = uteke
                .skill_record_usage(slug, event)
                .map_err(|e| format!("Failed to record usage: {e}"))?;
            if cli.json {
                output::print_json(&usage);
            } else {
                println!(
                    "{slug}: recalled {}, loaded {}, succeeded {}, failed {}",
                    usage.recalled, usage.loaded, usage.succeeded, usage.failed
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uteke_core::skill_library::{
        IndexAction, NearDuplicate, SkillIndexEntry, SkillLintEntry, SkillSimilarHit,
    };
    use uteke_core::skill_package::{LintIssue, PackageRejection};

    const H: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    #[test]
    fn draft_reading_is_bounded() {
        let dir = std::env::temp_dir().join(format!("uteke_skill_draft_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.join("ok.txt");
        std::fs::write(&ok, vec![b'a'; MAX_DRAFT_BYTES as usize]).unwrap();
        assert_eq!(
            read_draft(ok.to_str().unwrap()).unwrap().len(),
            MAX_DRAFT_BYTES as usize
        );
        let big = dir.join("big.txt");
        std::fs::write(&big, vec![b'a'; MAX_DRAFT_BYTES as usize + 1]).unwrap();
        assert!(
            read_draft(big.to_str().unwrap())
                .unwrap_err()
                .contains("larger than")
        );
        let bin = dir.join("bin.txt");
        std::fs::write(&bin, [0xff, 0xfe]).unwrap();
        assert!(
            read_draft(bin.to_str().unwrap())
                .unwrap_err()
                .contains("not valid UTF-8")
        );
        assert!(read_draft(dir.join("none").to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn index_text_lists_each_skill_issue_duplicate_and_rejection() {
        let report = SkillIndexReport {
            scanned_dirs: 4,
            created: 1,
            updated: 0,
            reverted: 0,
            unchanged: 0,
            rejected: vec![PackageRejection {
                path: "/s/Bad".into(),
                reason: "not a slug".into(),
            }],
            truncated: true,
            skills: vec![SkillIndexEntry {
                slug: "demo".into(),
                action: IndexAction::Created,
                hash: H.into(),
                status: "draft".into(),
                valid: false,
                issues: vec![LintIssue {
                    severity: Severity::Error,
                    code: "missing-description".into(),
                    message: "frontmatter has no 'description'".into(),
                }],
                files: 3,
                indexed_files: 2,
                warnings: vec!["x: symbolic link not followed".into()],
            }],
            near_duplicates: vec![NearDuplicate {
                slug: "demo".into(),
                other: "demo-2".into(),
                score: 0.95,
                relation: "possible_duplicate".into(),
            }],
            warnings: vec![],
        };
        assert_eq!(
            render_index(&report),
            vec![
                "Indexed 1 skill(s): 1 new, 0 updated, 0 reverted, 0 unchanged (4 directories scanned)",
                "  created   demo                         draft    0123456789ab  3 file(s), 2 indexed  INVALID",
                "      error: frontmatter has no 'description' (missing-description)",
                "      warning: x: symbolic link not followed",
                "  demo is possible_duplicate to demo-2 (0.950)",
                "  rejected /s/Bad: not a slug",
                "  The scan stopped at a limit: index a narrower directory for the rest.",
            ]
        );
    }

    #[test]
    fn hits_and_lists_say_when_empty() {
        assert_eq!(render_hits(&[]), ["No skills found."]);
        assert_eq!(
            render_list(&[]),
            ["No skills indexed. Run `uteke skill index <dir>`."]
        );
        let hit = SkillHit {
            slug: "demo".into(),
            name: Some("demo".into()),
            description: Some("Does a demo".into()),
            status: "approved".into(),
            hash: H.into(),
            valid: true,
            score: 0.5,
            file: "SKILL.md".into(),
            snippet: String::new(),
        };
        assert_eq!(
            render_hits(&[hit]),
            [
                "1. demo [approved] 0.5000",
                "   Does a demo",
                "   SKILL.md  0123456789ab"
            ]
        );
    }

    #[test]
    fn similar_text_lists_hits_and_counts() {
        let mut r = SkillSimilarReport {
            source: Some("a".into()),
            threshold: 0.8,
            similar_threshold: 0.8,
            duplicate_threshold: 0.92,
            source_embedded: true,
            skills_without_vector: 2,
            matches: 3,
            hits: vec![SkillSimilarHit {
                slug: "b".into(),
                name: None,
                description: Some("Other".into()),
                status: "draft".into(),
                score: 0.93,
                relation: "possible_duplicate".into(),
            }],
        };
        assert_eq!(
            render_similar("'a'", &r),
            [
                "Skills similar to 'a' (threshold 0.80; similar_to 0.80, possible_duplicate 0.92)",
                "  possible_duplicate 0.930  b                            Other",
                "3 match(es), showing 1",
                "2 skill(s) skipped: no usable vector",
            ]
        );
        r.hits.clear();
        r.matches = 0;
        r.skills_without_vector = 0;
        let lines = render_similar("'a'", &r);
        assert_eq!(&lines[1..], ["  none", "0 match(es)"]);
        r.source_embedded = false;
        assert_eq!(render_similar("'a'", &r).len(), 1);
    }

    #[test]
    fn lint_text_counts_invalid_and_rejected() {
        let r = SkillLintReport {
            scanned_dirs: 3,
            truncated: false,
            skills: vec![
                SkillLintEntry {
                    slug: "good".into(),
                    path: None,
                    valid: true,
                    issues: vec![],
                },
                SkillLintEntry {
                    slug: "bad".into(),
                    path: None,
                    valid: false,
                    issues: vec![LintIssue {
                        severity: Severity::Error,
                        code: "frontmatter".into(),
                        message: "no frontmatter".into(),
                    }],
                },
            ],
            rejected: vec![],
        };
        assert_eq!(
            render_lint(&r),
            [
                "ok      good",
                "INVALID bad",
                "    error: no frontmatter (frontmatter)",
                "2 skill(s), 1 invalid, 0 rejected",
            ]
        );
    }
}
