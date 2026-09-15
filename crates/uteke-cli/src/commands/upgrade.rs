//! `uteke upgrade` — check for updates and self-upgrade.
//!
//! Reuses the same logic as install.sh: detect OS/arch, fetch latest release
//! from GitHub, download, verify checksum, replace the running binary.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

const REPO: &str = "codecoradev/uteke";
const BINARY_NAME: &str = "uteke";
const SERVER_BINARY_NAME: &str = "uteke-serve";
const MCP_BINARY_NAME: &str = "uteke-mcp";

/// Entry point for `uteke upgrade`.
pub fn run(yes: bool) -> Result<(), String> {
    // 1. Detect current version
    let current_version = env!("CARGO_PKG_VERSION");
    println!("[INFO] Current version: {current_version}");

    // 2. Detect current binary path
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[ERROR] Cannot determine current binary path: {e}");
            eprintln!("        If installed via cargo, run: cargo install --path crates/uteke-cli");
            return Err(format!("Cannot determine current binary path: {e}"));
        }
    };

    // 3. Detect OS and architecture
    let os = detect_os();
    let arch = detect_arch();

    // 4. Get latest release version
    let latest_version = get_latest_version()?;

    // 5. Check if already up to date. The release tag carries a leading `v`
    // that CARGO_PKG_VERSION does not — compare normalized, or an up-to-date
    // install is re-offered the same release (#1245).
    let latest_clean = latest_version.trim_start_matches('v');
    if latest_clean == current_version {
        println!("[INFO] Already up to date ({current_version})");
        return Ok(());
    }

    println!("[INFO] Latest version:  {latest_version}");
    println!("[INFO] Release notes:  https://github.com/{REPO}/releases/tag/{latest_version}");

    // 6. Confirm (unless --yes)
    if !yes {
        print!("? Update to {latest_version}? [y/N] ");
        io::stdout()
            .flush()
            .map_err(|e| format!("stdout flush: {e}"))?;
        let mut input = String::new();
        io::stdin()
            .lock()
            .read_line(&mut input)
            .map_err(|e| format!("stdin read: {e}"))?;
        let input = input.trim().to_lowercase();
        if input != "y" && input != "yes" {
            println!("[INFO] Update cancelled.");
            return Ok(());
        }
    }

    // 7. Build target and download
    let target = get_target(&os, &arch)?;

    let archive_name = format!("{BINARY_NAME}-{target}-{latest_version}.tar.gz");
    let download_url =
        format!("https://github.com/{REPO}/releases/download/{latest_version}/{archive_name}");

    println!("[INFO] Downloading {archive_name} ...");

    let temp_dir = std::env::temp_dir().join(format!("uteke-update-{latest_version}"));
    fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let archive_path = temp_dir.join(&archive_name);

    let client = reqwest::blocking::Client::new();
    let mut resp = client
        .get(&download_url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Download failed (HTTP {status}): {body}"));
    }

    let mut file = fs::File::create(&archive_path)
        .map_err(|e| format!("Failed to create archive file: {e}"))?;
    io::copy(&mut resp, &mut file).map_err(|e| format!("Failed to write archive: {e}"))?;
    drop(file);

    // 8. Verify checksum — fail-hard to prevent MITM on unchecked binaries.
    // Use --no-verify (via env UTEKE_UPGRADE_SKIP_CHECKSUM=1) to opt out.
    let checksums_url = format!(
        "https://github.com/{REPO}/releases/download/{latest_version}/checksums-sha256.txt"
    );

    println!("[INFO] Verifying checksum ...");

    let skip_checksum = std::env::var("UTEKE_UPGRADE_SKIP_CHECKSUM")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    if skip_checksum {
        println!("[WARN] Checksum verification skipped (UTEKE_UPGRADE_SKIP_CHECKSUM=1)");
    } else {
        let checksums_resp = client.get(&checksums_url).send().map_err(|e| {
            format!("Failed to download checksums: {e}. Set UTEKE_UPGRADE_SKIP_CHECKSUM=1 to skip.")
        })?;

        if !checksums_resp.status().is_success() {
            let status = checksums_resp.status();
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(format!(
                "Failed to download checksums (HTTP {status}). \
                 Refusing to install unverified binary. \
                 Set UTEKE_UPGRADE_SKIP_CHECKSUM=1 to bypass."
            ));
        }

        let checksums_text = checksums_resp
            .text()
            .map_err(|e| format!("Failed to read checksums body: {e}"))?;

        let expected = parse_checksum(&checksums_text, &archive_name).ok_or_else(|| {
            let _ = fs::remove_dir_all(&temp_dir);
            format!(
                "Checksum for '{archive_name}' not found in checksums file. \
                 Refusing to install unverified binary. \
                 Set UTEKE_UPGRADE_SKIP_CHECKSUM=1 to bypass."
            )
        })?;

        let actual = sha256_file(&archive_path)?;
        if actual != expected {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(format!(
                "Checksum mismatch! Expected: {expected}, got: {actual}"
            ));
        }
        println!("[INFO] Checksum verified: {actual}");
    }

    // 9. Verify archive integrity (path traversal check)
    let file = fs::File::open(&archive_path).map_err(|e| format!("Failed to open archive: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read archive entries: {e}"))?
    {
        let entry = entry.map_err(|e| format!("Failed to read archive entry: {e}"))?;
        // entry.path() returns Result<Cow<Path>, _> in the tar crate's entry iteration
        // but after .flatten() above and here we use the raw entry — path() returns Result
        let path = entry
            .path()
            .map_err(|e| format!("Archive path error: {e}"))?;
        let path_str = path.to_string_lossy();
        if path_str.starts_with('/') || path_str.contains("..") {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(
                "Archive contains unsafe paths (absolute or directory traversal) — refusing to extract"
                    .to_string(),
            );
        }
    }
    drop(archive);

    // 10. Extract
    println!("[INFO] Extracting ...");
    let file = fs::File::open(&archive_path).map_err(|e| format!("Failed to open archive: {e}"))?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive
        .unpack(&temp_dir)
        .map_err(|e| format!("Failed to extract archive: {e}"))?;

    // 11. Replace binaries and bundled libs from the same verified archive.
    // `uteke upgrade` must keep every installed artifact in sync with the
    // release bundle (#1245): replacing only the CLI left `uteke-serve` and
    // `uteke-mcp` on the old version and discarded the freshly downloaded
    // ONNX Runtime libs (install.sh has installed those since #1221).
    let install_dir = current_exe
        .parent()
        .ok_or_else(|| "Cannot determine install directory".to_string())?;

    // CLI binary first — hard-fail on any problem (existing behavior).
    replace_binary(&temp_dir, BINARY_NAME, install_dir, true)?;

    // Companion binaries — replace when present in the archive; warn+skip
    // otherwise so old installs without them still upgrade cleanly.
    for name in [SERVER_BINARY_NAME, MCP_BINARY_NAME] {
        replace_binary(&temp_dir, name, install_dir, false)?;
    }

    // Bundled ONNX Runtime shared libs — refresh from the archive when present.
    refresh_ort_libs(&temp_dir, install_dir)?;

    // 12. Cleanup
    let _ = fs::remove_dir_all(&temp_dir);

    println!("[INFO] Update complete. ({current_version} → {latest_version})");

    Ok(())
}

/// Verify a freshly extracted binary runs, then atomically move it into
/// `install_dir`. With `required = false`, a missing artifact is skipped
/// with a warning (companion binaries absent from older bundles).
fn replace_binary(
    temp_dir: &std::path::Path,
    name: &str,
    install_dir: &std::path::Path,
    required: bool,
) -> Result<(), String> {
    let extracted = temp_dir.join(name);
    if !extracted.exists() {
        if required {
            return Err(format!("Binary '{name}' not found in archive"));
        }
        println!("[WARN] {name} not in bundle — skipping (left at its installed version)");
        return Ok(());
    }

    // Copy to temp file first, then rename (atomic on POSIX)
    let temp_new = install_dir.join(format!("{name}.new"));
    fs::copy(&extracted, &temp_new).map_err(|e| format!("Failed to copy new {name}: {e}"))?;

    // Verify the new binary runs
    match std::process::Command::new(&temp_new)
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let new_version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // Extract version from clap output like "uteke 0.6.7"
            let extracted_version = new_version.split_whitespace().nth(1).unwrap_or("unknown");
            println!("[INFO] Verified new {name}: {extracted_version}");
        }
        Ok(output) => {
            let _ = fs::remove_file(&temp_new);
            return Err(format!(
                "New {name} failed to run: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_new);
            return Err(format!("Failed to verify new {name}: {e}"));
        }
    }

    fs::rename(&temp_new, install_dir.join(name))
        .map_err(|e| format!("Failed to replace {name}: {e}"))?;
    Ok(())
}

/// Copy bundled ONNX Runtime shared libs from the extracted archive into the
/// install dir when present (mirrors install.sh since #1221). Symlinks are
/// preserved; a bundle without libs (very old releases) leaves existing libs
/// untouched.
fn refresh_ort_libs(
    temp_dir: &std::path::Path,
    install_dir: &std::path::Path,
) -> Result<(), String> {
    let mut refreshed = 0usize;
    let entries = fs::read_dir(temp_dir).map_err(|e| format!("Failed to read bundle dir: {e}"))?;
    for entry in entries.flatten() {
        let fname = entry.file_name();
        let fname = fname.to_string_lossy().to_string();
        if !fname.starts_with("libonnxruntime") {
            continue;
        }
        let dest = install_dir.join(&fname);
        let ft = entry
            .file_type()
            .map_err(|e| format!("Failed to stat {fname}: {e}"))?;
        if ft.is_file() {
            // Stage to a temp name, then rename over the target. Writing in
            // place would truncate the live lib under a running `uteke-serve`
            // (mmap -> SIGBUS) and an interrupted copy would leave a corrupt
            // lib behind; rename is atomic on POSIX.
            let staged = install_dir.join(format!("{fname}.new"));
            fs::copy(entry.path(), &staged).map_err(|e| format!("Failed to stage {fname}: {e}"))?;
            fs::rename(&staged, &dest).map_err(|e| format!("Failed to install {fname}: {e}"))?;
            refreshed += 1;
        } else if ft.is_symlink() {
            #[cfg(unix)]
            {
                let target = fs::read_link(entry.path())
                    .map_err(|e| format!("Failed to read link {fname}: {e}"))?;
                let staged = install_dir.join(format!("{fname}.new"));
                let _ = fs::remove_file(&staged);
                std::os::unix::fs::symlink(&target, &staged)
                    .map_err(|e| format!("Failed to link {fname}: {e}"))?;
                fs::rename(&staged, &dest)
                    .map_err(|e| format!("Failed to install {fname}: {e}"))?;
                refreshed += 1;
            }
            #[cfg(not(unix))]
            {
                let _ = dest;
                // Symlinked libs are a Unix packaging detail; other platforms
                // keep their installed libs untouched.
            }
        }
    }
    if refreshed > 0 {
        println!("[INFO] Refreshed {refreshed} ONNX Runtime lib file(s)");
    } else {
        println!("[WARN] No ONNX Runtime libs in bundle — existing libs left untouched");
    }
    Ok(())
}

fn detect_os() -> String {
    match std::env::consts::OS {
        "linux" => "linux".to_string(),
        "macos" => "darwin".to_string(),
        os => os.to_string(),
    }
}

fn detect_arch() -> String {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64".to_string(),
        "aarch64" => "aarch64".to_string(),
        arch => arch.to_string(),
    }
}

fn get_target(os: &str, arch: &str) -> Result<String, String> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu".into()),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu".into()),
        ("darwin", "aarch64") => Ok("aarch64-apple-darwin".into()),
        ("darwin", "x86_64") => {
            Err("No pre-built binary for x86_64 macOS.\n  Install via: cargo install --path crates/uteke-cli".into())
        }
        _ => Err(format!("Unsupported platform: {os} {arch}")),
    }
}

pub(crate) fn get_latest_version() -> Result<String, String> {
    let client = reqwest::blocking::Client::new();

    // Primary: parse 302 redirect (no API call, no rate limit)
    let resp = client
        .head(format!("https://github.com/{REPO}/releases/latest"))
        .send()
        .map_err(|e| format!("Failed to check latest release: {e}"))?;

    if let Some(location) = resp.headers().get("location") {
        let loc = location.to_str().unwrap_or_default();
        if let Some(tag) = loc.strip_prefix("/codecoradev/uteke/releases/tag/") {
            return Ok(tag.trim_end_matches('?').to_string());
        }
        // Some mirrors might use different prefix
        if let Some(tag) = loc.rsplit('/').next() {
            if tag.starts_with('v') {
                return Ok(tag.trim_end_matches('?').to_string());
            }
        }
    }

    // Fallback: GitHub API
    let api_url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let resp = client
        .get(&api_url)
        .header("User-Agent", "uteke-upgrade")
        .send()
        .map_err(|e| format!("GitHub API failed: {e}"))?;

    if resp.status().is_success() {
        let json: serde_json::Value = resp
            .json()
            .map_err(|e| format!("Failed to parse GitHub API response: {e}"))?;
        if let Some(tag) = json["tag_name"].as_str() {
            return Ok(tag.to_string());
        }
    }

    Err(format!(
        "Failed to determine latest version. Check https://github.com/{REPO}/releases"
    ))
}

fn parse_checksum(checksums_text: &str, archive_name: &str) -> Option<String> {
    for line in checksums_text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() >= 2 && parts[1].contains(archive_name) {
            return Some(parts[0].to_string());
        }
    }
    None
}

fn sha256_file(path: &PathBuf) -> Result<String, String> {
    let mut hasher = Sha256::new();
    let mut file =
        fs::File::open(path).map_err(|e| format!("Failed to open file for hashing: {e}"))?;
    io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("Failed to read file for hashing: {e}"))?;
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn replace_binary_verifies_and_installs() {
        let dir = std::env::temp_dir().join(format!("uteke-upgrade-test-{}", std::process::id()));
        let bundle = dir.join("bundle");
        let inst = dir.join("inst");
        fs::create_dir_all(&bundle).unwrap();
        fs::create_dir_all(&inst).unwrap();

        // Fake binary that runs successfully and reports a version.
        let fake = bundle.join("uteke");
        fs::write(&fake, "#!/bin/sh\necho \"uteke 9.9.9\"\n").unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

        replace_binary(&bundle, "uteke", &inst, true).unwrap();
        assert!(inst.join("uteke").exists(), "binary installed");

        // Optional artifact missing -> skipped with a warning, no error.
        replace_binary(&bundle, "uteke-not-shipped", &inst, false).unwrap();
        assert!(!inst.join("uteke-not-shipped").exists());

        // Required artifact missing -> hard error.
        assert!(replace_binary(&bundle, "uteke-required", &inst, true).is_err());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn replace_binary_rejects_broken_artifact() {
        let dir = std::env::temp_dir().join(format!("uteke-upgrade-broken-{}", std::process::id()));
        let bundle = dir.join("bundle");
        let inst = dir.join("inst");
        fs::create_dir_all(&bundle).unwrap();
        fs::create_dir_all(&inst).unwrap();

        // Binary that exits non-zero must fail verification and not install.
        let bad = bundle.join("uteke-broken");
        fs::write(&bad, "#!/bin/sh\nexit 1\n").unwrap();
        fs::set_permissions(&bad, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(replace_binary(&bundle, "uteke-broken", &inst, true).is_err());
        assert!(
            !inst.join("uteke-broken").exists(),
            "broken binary must not be installed"
        );

        let _ = fs::remove_dir_all(&dir);
    }
}
