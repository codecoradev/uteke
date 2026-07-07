//! `uteke update` — check for updates and self-upgrade.
//!
//! Reuses the same logic as install.sh: detect OS/arch, fetch latest release
//! from GitHub, download, verify checksum, replace the running binary.

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use sha2::{Digest, Sha256};

const REPO: &str = "codecoradev/uteke";
const BINARY_NAME: &str = "uteke";

/// Entry point for `uteke update`.
pub fn run(yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Detect current version
    let current_version = env!("CARGO_PKG_VERSION");
    println!("[INFO] Current version: {current_version}");

    // 2. Detect current binary path
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("[ERROR] Cannot determine current binary path: {e}");
            eprintln!("        If installed via cargo, run: cargo install --path crates/uteke-cli");
            return Err(e.into());
        }
    };

    // 3. Detect OS and architecture
    let os = detect_os();
    let arch = detect_arch();

    // 4. Get latest release version
    let latest_version = get_latest_version()?;

    // 5. Check if already up to date
    if latest_version == current_version {
        println!("[INFO] Already up to date ({current_version})");
        return Ok(());
    }

    println!("[INFO] Latest version:  {latest_version}");
    println!("[INFO] Release notes:  https://github.com/{REPO}/releases/tag/{latest_version}");

    // 6. Confirm (unless --yes)
    if !yes {
        print!("? Update to {latest_version}? [y/N] ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().lock().read_line(&mut input)?;
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
    fs::create_dir_all(&temp_dir)?;
    let archive_path = temp_dir.join(&archive_name);

    let client = reqwest::blocking::Client::new();
    let mut resp = client
        .get(&download_url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Download failed (HTTP {status}): {body}").into());
    }

    let mut file = fs::File::create(&archive_path)?;
    io::copy(&mut resp, &mut file)?;
    drop(file);

    // 8. Verify checksum
    let checksums_url = format!(
        "https://github.com/{REPO}/releases/download/{latest_version}/checksums-sha256.txt"
    );

    println!("[INFO] Verifying checksum ...");

    if let Ok(checksums_resp) = client.get(&checksums_url).send() {
        if checksums_resp.status().is_success() {
            let checksums_text = checksums_resp.text().unwrap_or_default();
            if let Some(expected) = parse_checksum(&checksums_text, &archive_name) {
                let actual = sha256_file(&archive_path)?;
                if actual != expected {
                    // Clean up on mismatch
                    let _ = fs::remove_dir_all(&temp_dir);
                    return Err(
                        format!("Checksum mismatch! Expected: {expected}, got: {actual}").into(),
                    );
                }
                println!("[INFO] Checksum verified: {actual}");
            } else {
                println!("[WARN] Checksum for {archive_name} not found — skipping verification");
            }
        } else {
            println!("[WARN] Failed to download checksums — skipping verification");
        }
    }

    // 9. Verify archive integrity (path traversal check)
    let file = fs::File::open(&archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    for entry in archive.entries()?.flatten() {
        let path = entry
            .path()?
            .map_err(|e| format!("Archive path error: {e}"))?;
        let path_str = path.to_string_lossy();
        if path_str.starts_with('/') || path_str.contains("..") {
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(
                "Archive contains unsafe paths (absolute or directory traversal) — refusing to extract"
                    .into(),
            );
        }
    }
    drop(archive);

    // 10. Extract
    println!("[INFO] Extracting ...");
    let file = fs::File::open(&archive_path)?;
    let gz = flate2::read::GzDecoder::new(file);
    let mut archive = tar::Archive::new(gz);
    archive.unpack(&temp_dir)?;

    // 11. Find and replace binary
    let extracted_binary = temp_dir.join(BINARY_NAME);
    if !extracted_binary.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
        return Err(format!("Binary '{BINARY_NAME}' not found in archive").into());
    }

    let install_dir = current_exe
        .parent()
        .ok_or("Cannot determine install directory")?;

    // Copy to temp file first, then rename (atomic on POSIX)
    let temp_new = install_dir.join(format!("{BINARY_NAME}.new"));
    fs::copy(&extracted_binary, &temp_new)?;

    // Verify the new binary runs
    match std::process::Command::new(&temp_new)
        .arg("--version")
        .output()
    {
        Ok(output) if output.status.success() => {
            let new_version = String::from_utf8_lossy(&output.stdout).trim().to_string();
            // Extract version from clap output like "uteke 0.6.7"
            let extracted_version = new_version.split_whitespace().nth(1).unwrap_or("unknown");
            println!("[INFO] Verified new binary: {extracted_version}");
        }
        Ok(output) => {
            let _ = fs::remove_file(&temp_new);
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(format!(
                "New binary failed to run: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Err(e) => {
            let _ = fs::remove_file(&temp_new);
            let _ = fs::remove_dir_all(&temp_dir);
            return Err(format!("Failed to verify new binary: {e}").into());
        }
    }

    // Atomic rename
    fs::rename(&temp_new, &current_exe)?;

    // 12. Cleanup
    let _ = fs::remove_dir_all(&temp_dir);

    println!("[INFO] Update complete. ({current_version} → {latest_version})");

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

fn get_target(os: &str, arch: &str) -> Result<String, Box<dyn std::error::Error>> {
    match (os, arch) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu".into()),
        ("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu".into()),
        ("darwin", "aarch64") => Ok("aarch64-apple-darwin".into()),
        ("darwin", "x86_64") => {
            Err("No pre-built binary for x86_64 macOS.\n  Install via: cargo install --path crates/uteke-cli".into())
        }
        _ => Err(format!("Unsupported platform: {os} {arch}")).into(),
    }
}

fn get_latest_version() -> Result<String, Box<dyn std::error::Error>> {
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
        .header("User-Agent", "uteke-update")
        .send()
        .map_err(|e| format!("GitHub API failed: {e}"))?;

    if resp.status().is_success() {
        let json: serde_json::Value = resp.json()?;
        if let Some(tag) = json["tag_name"].as_str() {
            return Ok(tag.to_string());
        }
    }

    Err("Failed to determine latest version. Check https://github.com/{REPO}/releases".into())
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

fn sha256_file(path: &PathBuf) -> Result<String, Box<dyn std::error::Error>> {
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(path)?;
    io::copy(&mut file, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}
