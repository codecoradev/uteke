//! `connect pi|hermes`: these agents are not shell-hook agents. Their lifecycle
//! hooks ship as a memory-provider plugin, so `connect` installs that plugin
//! through `uteke init <agent> --memory-provider` instead of adding a second
//! mechanism (owner decision on #1465). `disconnect` removes the plugin files
//! only while they are the ones uteke wrote.

use std::path::{Path, PathBuf};

/// The agents whose hooks are a memory-provider plugin.
pub(crate) const PROVIDER_AGENTS: &[&str] = &["pi", "hermes"];

/// The home `init <agent> --memory-provider` installs under: `HOME`, else
/// `USERPROFILE`. The same lookup here, so `disconnect` finds what `init` wrote.
fn plugin_home() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| "Cannot find the home directory".to_string())
}

/// The plugin directory and files `init <agent> --memory-provider` writes.
fn plugin(agent: &str, home: &Path) -> Option<(PathBuf, [(&'static str, &'static str); 2])> {
    match agent {
        "pi" => Some((
            home.join(".pi/agent/extensions/uteke-memory-provider"),
            crate::init::PI_PROVIDER_FILES,
        )),
        "hermes" => Some((
            home.join(".hermes/plugins/uteke"),
            crate::init::HERMES_PROVIDER_FILES,
        )),
        _ => None,
    }
}

/// `connect pi|hermes`: install the memory-provider plugin.
pub(crate) fn connect(agent: &str, dry_run: bool, json: bool) -> Result<(), String> {
    let home = plugin_home()?;
    let (dir, files) = plugin(agent, &home).expect("a provider agent");
    if dry_run {
        println!(
            "Dry run: would install the {agent} memory-provider plugin (`uteke init {agent} \
             --memory-provider`) into {}:",
            dir.display()
        );
        for (name, _) in files {
            println!("  + {name}");
        }
        return Ok(());
    }
    // A plugin file the user changed is kept as `<file>.uteke-bak` before the
    // install overwrites it (once: an older backup is not replaced).
    for (name, text) in files {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).is_ok_and(|current| current != text) {
            let bak = super::backup_path(&path);
            if !bak.exists() {
                std::fs::copy(&path, &bak)
                    .map_err(|e| format!("Cannot back up {}: {e}", path.display()))?;
                if !json {
                    println!("  Your changed {name} is kept as {}", bak.display());
                }
            }
        }
    }
    crate::init::run_init_agent(agent, true, json)
}

/// `disconnect pi|hermes`: remove the plugin files that still match what
/// uteke installed; a file that was changed is left and reported.
pub(crate) fn disconnect(agent: &str, dry_run: bool, json: bool) -> Result<(), String> {
    let home = plugin_home()?;
    let (dir, files) = plugin(agent, &home).expect("a provider agent");
    let mut removed = Vec::new();
    let mut left = Vec::new();
    for (name, text) in files {
        let path = dir.join(name);
        match std::fs::read_to_string(&path) {
            Ok(current) if current == text => {
                if !dry_run {
                    std::fs::remove_file(&path)
                        .map_err(|e| format!("Cannot remove {}: {e}", path.display()))?;
                }
                removed.push(name);
            }
            Ok(_) => left.push(name),
            Err(_) => {}
        }
    }
    if !dry_run && dir.read_dir().is_ok_and(|mut d| d.next().is_none()) {
        let _ = std::fs::remove_dir(&dir);
    }
    if json {
        crate::output::print_json(&serde_json::json!({
            "agent": agent,
            "directory": dir,
            "dry_run": dry_run,
            "removed": removed,
            "left_alone": left,
        }));
        return Ok(());
    }
    if removed.is_empty() && left.is_empty() {
        println!(
            "Nothing to remove: no uteke {agent} plugin in {}",
            dir.display()
        );
        return Ok(());
    }
    let verb = if dry_run { "Would remove" } else { "Removed" };
    for name in &removed {
        println!("  - {name}");
    }
    for name in &left {
        println!("  ! left alone (changed since uteke wrote it): {name}");
    }
    println!(
        "\u{2713} {verb} the {agent} memory-provider plugin: {}",
        dir.display()
    );
    if agent == "hermes" {
        println!("  Also remove `provider: uteke` under `memory:` in ~/.hermes/config.yaml.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_paths_match_what_init_writes() {
        let home = Path::new("/h");
        let (pi, _) = plugin("pi", home).unwrap();
        assert_eq!(
            pi,
            Path::new("/h/.pi/agent/extensions/uteke-memory-provider")
        );
        let (hermes, files) = plugin("hermes", home).unwrap();
        assert_eq!(hermes, Path::new("/h/.hermes/plugins/uteke"));
        assert_eq!(files[0].0, "__init__.py");
        assert!(plugin("claude", home).is_none());
    }
}
