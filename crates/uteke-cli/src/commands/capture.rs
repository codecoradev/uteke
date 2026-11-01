//! `uteke capture` and `uteke extract-queue` (#1466): queue raw turns now,
//! extract facts from them later, on a budget.

use crate::cli::{Cli, ExtractQueueCommands};
use crate::config::Config;
use crate::output;
use uteke_core::Uteke;
use uteke_core::capture::{FactSource, OfflineFacts, Role};

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
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("Failed to read stdin: {e}"))?;
            buf
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

/// The fact source of a run: the LLM extractor when `[extraction]` names
/// one (mode `llm`, or an API key with no mode), else the offline rules.
fn fact_source(config: &Config) -> Result<Box<dyn FactSource>, String> {
    let cfg = &config.extraction;
    if uses_llm(cfg) {
        let mut llm = cfg.clone();
        llm.mode = "llm".to_string();
        let extractor = uteke_core::extraction::Extractor::new(&llm)
            .map_err(|e| format!("Failed to initialize extractor: {e}"))?;
        Ok(Box::new(extractor))
    } else {
        Ok(Box::new(OfflineFacts {
            max_facts: cfg.max_facts,
        }))
    }
}

/// Mode `llm`, or no mode and an API key: the same rule as `import --extract`.
fn uses_llm(cfg: &crate::config::ExtractionConfig) -> bool {
    match cfg.mode.as_str() {
        "llm" => true,
        "offline" => false,
        _ => !cfg.api_key.is_empty(),
    }
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
            }
        }
        ExtractQueueCommands::Run { budget } => {
            let settings = config.engine.capture;
            let facts = fact_source(config)?;
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
    use super::uses_llm;
    use crate::config::ExtractionConfig;

    #[test]
    fn the_llm_is_used_only_when_extraction_names_one() {
        let cfg = |mode: &str, key: &str| ExtractionConfig {
            mode: mode.into(),
            api_key: key.into(),
            ..Default::default()
        };
        assert!(!uses_llm(&cfg("", "")), "unconfigured: offline rules");
        assert!(uses_llm(&cfg("", "sk-test")));
        assert!(uses_llm(&cfg("llm", "")));
        assert!(!uses_llm(&cfg("offline", "sk-test")));
    }
}
