//! `uteke view`: start the read-only browser viewer (#1470 b).
//!
//! The viewer itself is `uteke-serve --viewer`; this command finds that binary
//! (next to this one, as the release bundle installs it, else on `PATH`), runs
//! it with the store environment this invocation resolved, and relays its
//! launch URL.

use std::path::PathBuf;
use std::process::Command;

const SERVER_BINARY: &str = "uteke-serve";

/// Where `uteke-serve` lives: beside the running `uteke`, else on `PATH`.
fn locate_server() -> Option<PathBuf> {
    let name = format!("{SERVER_BINARY}{}", std::env::consts::EXE_SUFFIX);
    if let Some(sibling) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|d| d.join(&name)))
        && sibling.is_file()
    {
        return Some(sibling);
    }
    which::which(SERVER_BINARY).ok()
}

/// The `uteke-serve` arguments for a viewer on `port` / `host`.
fn viewer_args(port: Option<u16>, host: Option<&str>) -> Vec<String> {
    let mut args = vec!["--viewer".to_string()];
    if let Some(p) = port {
        args.push("--port".into());
        args.push(p.to_string());
    }
    if let Some(h) = host {
        args.push("--host".into());
        args.push(h.to_string());
    }
    args
}

pub(crate) fn run(port: Option<u16>, host: Option<&str>) -> Result<(), String> {
    let server = locate_server().ok_or_else(|| {
        format!("cannot find {SERVER_BINARY}: install the full bundle (`uteke upgrade`) or put it on PATH")
    })?;
    eprintln!("Starting the read-only viewer (Ctrl+C to stop). Open the URL below:");
    let status = Command::new(&server)
        .args(viewer_args(port, host))
        .status()
        .map_err(|e| format!("cannot run {}: {e}", server.display()))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("the viewer exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_default_to_the_viewer_flag_only() {
        assert_eq!(viewer_args(None, None), ["--viewer"]);
    }

    #[test]
    fn args_pass_port_and_host() {
        assert_eq!(
            viewer_args(Some(9000), Some("127.0.0.1")),
            ["--viewer", "--port", "9000", "--host", "127.0.0.1"]
        );
    }
}
