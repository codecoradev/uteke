//! `uteke capture` and `uteke extract-queue` (#1466): queue raw turns now,
//! extract facts from them later, on a budget.

use crate::cli::{Cli, ExtractQueueCommands};
use crate::config::Config;
use crate::output;
use uteke_core::Uteke;
use uteke_core::capture::Role;

/// Queue one turn. Refused while `[capture] enabled` is off (opt-in).
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_capture(
    cli: &Cli,
    uteke: &Uteke,
    config: &Config,
    ns: Option<&str>,
    session: &str,
    role: &str,
    source: Option<&str>,
    text: Option<&str>,
) -> Result<(), String> {
    if !config.engine.capture.enabled {
        return Err(
            "Capture is off: set `enabled = true` under [capture] in the global uteke.toml"
                .to_string(),
        );
    }
    let role: Role = role.parse().map_err(|e: uteke_core::Error| e.to_string())?;
    let content = match text {
        Some(t) => t.to_string(),
        None => {
            let limit = config.engine.capture.turn_limit();
            read_turn(std::io::stdin().lock(), limit)?
        }
    };
    let out = uteke
        .capture(session, role, &content, source, ns)
        .map_err(|e| format!("Failed to capture: {e}"))?;
    if cli.json {
        output::print_json(&out);
    } else if out.duplicate {
        println!("Already captured: {}", out.id);
    } else {
        println!("\u{2713} Captured: {}", out.id);
    }
    Ok(())
}

/// Read a turn from `input`, at most `limit` bytes: one byte more is read to
/// tell a turn at the limit from one over it, and nothing beyond that.
fn read_turn(input: impl std::io::Read, limit: usize) -> Result<String, String> {
    use std::io::Read;
    let mut buf = Vec::new();
    input
        .take(limit as u64 + 1)
        .read_to_end(&mut buf)
        .map_err(|e| format!("Failed to read stdin: {e}"))?;
    if buf.len() > limit {
        return Err(format!(
            "The turn is too large: more than {limit} bytes on stdin (`max_turn_bytes` under [capture])"
        ));
    }
    String::from_utf8(buf)
        .map_err(|_| "Failed to read stdin: the turn is not valid UTF-8".to_string())
}

pub(crate) fn run_extract_queue(
    cli: &Cli,
    uteke: &Uteke,
    config: &Config,
    command: &ExtractQueueCommands,
) -> Result<(), String> {
    match command {
        ExtractQueueCommands::Status => {
            let s = uteke
                .capture_status()
                .map_err(|e| format!("Failed to read the queue: {e}"))?;
            if cli.json {
                output::print_json(&s);
            } else {
                println!(
                    "Capture queue: {} pending ({} sessions), {} extracting, {} done, {} failed",
                    s.pending, s.sessions_pending, s.extracting, s.done, s.failed
                );
                println!("Waiting text: {} bytes", s.pending_bytes);
                if let Some(w) = &s.warning {
                    println!("  \u{26a0} {w}");
                }
            }
        }
        ExtractQueueCommands::Run { budget } => {
            let settings = config.engine.capture;
            let facts = uteke_core::capture::fact_source(&config.extraction)
                .map_err(|e| format!("Failed to initialize extractor: {e}"))?;
            if !cli.json {
                println!(
                    "{}",
                    uteke_core::capture::extractor_line(&config.extraction)
                );
            }
            let r = uteke
                .run_capture_extraction(
                    facts.as_ref(),
                    budget.unwrap_or(settings.budget_per_run),
                    &settings,
                )
                .map_err(|e| format!("Extraction run failed: {e}"))?;
            if cli.json {
                output::print_json(&r);
            } else {
                println!(
                    "\u{2713} Extracted {} session(s): {} facts stored, {} refused; {} session(s) still pending",
                    r.sessions_done, r.facts_stored, r.facts_refused, r.sessions_pending
                );
                if r.sessions_failed > 0 {
                    println!(
                        "  \u{26a0} {} session(s) marked failed after too many attempts",
                        r.sessions_failed
                    );
                }
                if r.reclaimed > 0 {
                    println!(
                        "  {} session(s) from an interrupted run were requeued",
                        r.reclaimed
                    );
                }
                if r.pruned > 0 {
                    println!("  {} old turn(s) deleted (retention)", r.pruned);
                }
                if let Some(why) = &r.stopped {
                    println!("  \u{26a0} Stopped early, the extractor failed: {why}");
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::read_turn;

    #[test]
    fn stdin_is_read_up_to_the_limit_and_no_further() {
        assert_eq!(read_turn(&b"hello"[..], 5).unwrap(), "hello");
        let e = read_turn(&b"hello!"[..], 5).unwrap_err();
        assert!(e.contains("too large") && e.contains('5'), "{e}");
        // An endless input is cut at limit + 1 instead of read to the end.
        let endless = std::io::repeat(b'x');
        assert!(read_turn(endless, 1000).is_err());
        assert!(
            read_turn(&[0xff, 0xfe][..], 10)
                .unwrap_err()
                .contains("UTF-8")
        );
    }
}
