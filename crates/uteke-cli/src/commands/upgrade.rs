//! `uteke upgrade` — check for updates and self-upgrade.
//!
//! Reuses the same logic as install.sh: detect OS/arch, fetch latest release
//! from GitHub, download, verify checksum, replace the running binary.

use std::fs;
use std::io::{self, BufRead, Read, Write};
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::release_verify;

const REPO: &str = "codecoradev/uteke";
const BINARY_NAME: &str = "uteke";
const SERVER_BINARY_NAME: &str = "uteke-serve";
const MCP_BINARY_NAME: &str = "uteke-mcp";

/// Hard cap on the downloaded release archive (#1327). Release archives are
/// about 20-35 MiB today (v0.20.1: 22 MiB arm64 Linux, 26 MiB x86_64 Linux,
/// 31 MiB macOS, 35 MiB legacy-ORT Linux; CLI + server + MCP binaries and the
/// bundled ONNX Runtime libs); 128 MiB leaves more than 3x headroom over the
/// largest while still stopping a hostile or broken mirror from filling the disk.
const MAX_ARCHIVE_BYTES: u64 = 128 * 1024 * 1024;

/// Hard cap on `checksums-sha256.txt` (#1327). The real file is a few hundred
/// bytes (one line per artifact); 1 MiB is generous and keeps the in-memory
/// read bounded.
const MAX_CHECKSUMS_BYTES: u64 = 1024 * 1024;

/// The cosign bundle (signature, Fulcio certificate, Rekor entry) is about
/// 8 KiB; 64 KiB is generous and keeps the in-memory read bounded.
const MAX_BUNDLE_BYTES: u64 = 64 * 1024;

/// First release whose `checksums-sha256.txt` is verified with the keyless
/// cosign scheme (#1610). A target below it is refused, never checked against
/// a scheme it was not signed with (and never falls back to a weaker one).
///
/// The roots the bundle is checked against are embedded in the binary
/// (`trust/`); a Sigstore root rotation requires a uteke release that carries
/// the new root, because installed binaries cannot learn it themselves.
const MIN_VERIFIABLE_VERSION: (u64, u64, u64) = (0, 22, 1);

/// Appended to every signature failure so the user can verify by hand.
const SIGNATURE_HELP: &str = "See the \"Verifying a release\" section of SECURITY.md \
    (https://github.com/codecoradev/uteke/blob/main/SECURITY.md) to verify the release manually.";

/// Error for a checksums download that did not work. A transient failure must
/// not point the user at the bypass: `UTEKE_UPGRADE_SKIP_CHECKSUM=1` also drops
/// the signature check, so the hint is to retry or verify by hand.
fn checksums_unavailable(detail: &str) -> String {
    format!(
        "Failed to download checksums ({detail}). Refusing to install an unverified \
         binary; retry later. {SIGNATURE_HELP}"
    )
}

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
    // Only stable releases are ever installed (owner rule): enforced here,
    // before the prompt and any download, with no override. The tag is
    // normalised to `vX.Y.Z` (the API fallback may return it without `v`).
    let latest_version = ensure_stable_tag(&get_latest_version()?)?;

    // 5. Check if already up to date. The release tag carries a leading `v`
    // that CARGO_PKG_VERSION does not — compare normalized, or an up-to-date
    // install is re-offered the same release (#1245).
    match compare_versions(current_version, &latest_version)? {
        std::cmp::Ordering::Equal => {
            println!("[INFO] Already up to date ({current_version})");
            return Ok(());
        }
        std::cmp::Ordering::Greater => {
            return Err(format!(
                "Refusing to downgrade from {current_version} to {}. The release \
                 pointed to by \"latest\" is older than this install; if that is \
                 intended, reinstall the older version by hand.",
                latest_version.trim_start_matches('v')
            ));
        }
        std::cmp::Ordering::Less => {}
    }
    ensure_verifiable_tag(&latest_version)?;

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

    // The tag ends up in the URL and the file name: re-validate it here so
    // no caller path can feed an unchecked value into either (#1327).
    if !is_valid_version_tag(&latest_version) {
        return Err(format!(
            "Refusing unexpected version tag '{latest_version}'"
        ));
    }

    let archive_name = format!("{BINARY_NAME}-{target}-{latest_version}.tar.gz");
    let download_url =
        format!("https://github.com/{REPO}/releases/download/{latest_version}/{archive_name}");

    println!("[INFO] Downloading {archive_name} ...");

    // Private temp dir (random suffix, 0700): a predictable shared path like
    // /tmp/uteke-update-{version} lets a local attacker pre-create it as a
    // symlink and clobber arbitrary files during extraction; TempDir also
    // removes leftovers on every early-return path (cora scan #1307).
    let temp = tempfile::Builder::new()
        .prefix("uteke-update-")
        .tempdir()
        .map_err(|e| format!("Failed to create temp dir: {e}"))?;
    let temp_dir = temp.path().to_path_buf();
    let archive_path = temp_dir.join(&archive_name);

    // Bounded timeouts so a stalled connection cannot hang the upgrade.
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;
    // 8. Authenticate the checksums BEFORE the (up to 128 MiB) archive download:
    // fetch checksums + cosign bundle, verify, and only then trust a hash.
    // Use UTEKE_UPGRADE_SKIP_CHECKSUM=1 to opt out of both checks.
    let checksums_url = format!(
        "https://github.com/{REPO}/releases/download/{latest_version}/checksums-sha256.txt"
    );
    let bundle_url = format!("{checksums_url}.bundle");

    println!("[INFO] Verifying checksum ...");

    let skip_checksum = std::env::var("UTEKE_UPGRADE_SKIP_CHECKSUM")
        .map(|v| v == "1" || v == "true")
        .unwrap_or(false);

    let expected_hash: Option<String> = if skip_checksum {
        eprintln!("[WARN] ============================================================");
        eprintln!("[WARN] UTEKE_UPGRADE_SKIP_CHECKSUM is set: the downloaded archive is");
        eprintln!("[WARN] NOT verified, and neither is the cosign signature of the");
        eprintln!("[WARN] checksums file. A corrupted, tampered or malicious download will");
        eprintln!("[WARN] be installed and executed as-is. Unset it unless you fully");
        eprintln!("[WARN] trust the network path and the release source.");
        eprintln!("[WARN] ============================================================");
        None
    } else {
        let checksums_resp = client
            .get(&checksums_url)
            .send()
            .map_err(|e| checksums_unavailable(&e.to_string()))?;

        if !checksums_resp.status().is_success() {
            let status = checksums_resp.status();
            return Err(checksums_unavailable(&format!("HTTP {status}")));
        }

        check_declared_len(
            checksums_resp.content_length(),
            MAX_CHECKSUMS_BYTES,
            "Checksums file",
        )?;
        let mut checksums_resp = checksums_resp;
        let mut checksums_buf: Vec<u8> = Vec::new();
        copy_capped(
            &mut checksums_resp,
            &mut checksums_buf,
            MAX_CHECKSUMS_BYTES,
            "Checksums file",
        )?;

        println!("[INFO] Verifying checksums signature (cosign, keyless) ...");
        let bundle = fetch_bundle(&client, &bundle_url, &latest_version);
        let hash = verified_expected_hash(
            &checksums_buf,
            bundle,
            &latest_version,
            &release_verify::PUBLIC_GOOD,
            &archive_name,
        )?;
        println!("[INFO] Signature verified (uteke {latest_version})");
        Some(hash)
    };

    let mut resp = client
        .get(&download_url)
        .send()
        .map_err(|e| format!("Download failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().unwrap_or_default();
        return Err(format!("Download failed (HTTP {status}): {body}"));
    }

    // Cheap early reject from the header, then enforce the cap while
    // streaming: Content-Length may be absent (chunked) or simply wrong.
    check_declared_len(resp.content_length(), MAX_ARCHIVE_BYTES, "Release archive")?;
    let mut file = fs::File::create(&archive_path)
        .map_err(|e| format!("Failed to create archive file: {e}"))?;
    copy_capped(&mut resp, &mut file, MAX_ARCHIVE_BYTES, "Release archive")?;
    drop(file);

    if let Some(expected) = expected_hash {
        let actual = sha256_file(&archive_path)?;
        if actual != expected {
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
        if !is_safe_archive_path(&path) {
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

    // Stage and verify EVERY binary first, then install them. The CLI used to
    // be replaced before the companions were even looked at, so a broken
    // uteke-serve left a new CLI next to an old (or half-replaced) server
    // (#1332). Now a bad bundle fails before the installed files are touched.
    install_bundle(&temp_dir, install_dir)?;

    // Bundled ONNX Runtime shared libs — refresh from the archive when present.
    refresh_ort_libs(&temp_dir, install_dir)?;

    // TempDir cleans itself up on drop — no manual removal needed.

    println!("[INFO] Update complete. ({current_version} → {latest_version})");

    Ok(())
}

/// Run `<binary> --version`, retrying while the kernel reports the freshly
/// written file as busy.
///
/// Executing a binary right after writing it can fail with `ETXTBSY`
/// ("Text file busy", os error 26) when another thread/process forks while
/// our write descriptor is still open — the child briefly inherits it. The
/// condition clears as soon as that child execs, so a short retry is enough.
fn run_version_check(binary: &std::path::Path) -> std::io::Result<std::process::Output> {
    const ETXTBSY: i32 = 26;
    const MAX_ATTEMPTS: u32 = 8;
    let mut attempt = 0;
    loop {
        match std::process::Command::new(binary).arg("--version").output() {
            Err(e) if e.raw_os_error() == Some(ETXTBSY) && attempt + 1 < MAX_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(25 * u64::from(attempt)));
            }
            other => return other,
        }
    }
}

/// Verify a freshly extracted binary runs, then atomically move it into
/// `install_dir`. With `required = false`, a missing artifact is skipped
/// with a warning (companion binaries absent from older bundles).
#[cfg(test)]
fn replace_binary(
    temp_dir: &std::path::Path,
    name: &str,
    install_dir: &std::path::Path,
    required: bool,
) -> Result<(), String> {
    match stage_binary(temp_dir, name, install_dir, required)? {
        Some(staged) => commit_staged(staged),
        None => Ok(()),
    }
}

/// A verified binary waiting next to its destination as `<name>.new`.
struct StagedBinary {
    staged: std::path::PathBuf,
    dest: std::path::PathBuf,
    name: String,
}

/// Copy `name` from the bundle to `<install_dir>/<name>.new` and check that it
/// runs. Nothing installed is modified. `Ok(None)` = an optional artifact that
/// is not in the bundle.
fn stage_binary(
    temp_dir: &std::path::Path,
    name: &str,
    install_dir: &std::path::Path,
    required: bool,
) -> Result<Option<StagedBinary>, String> {
    let extracted = temp_dir.join(name);
    if !extracted.exists() {
        if required {
            return Err(format!("Binary '{name}' not found in archive"));
        }
        println!("[WARN] {name} not in bundle — skipping (left at its installed version)");
        return Ok(None);
    }

    // Copy to temp file first, then rename (atomic on POSIX)
    let temp_new = install_dir.join(format!("{name}.new"));
    fs::copy(&extracted, &temp_new).map_err(|e| format!("Failed to copy new {name}: {e}"))?;

    // Verify the new binary runs
    match run_version_check(&temp_new) {
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

    Ok(Some(StagedBinary {
        dest: install_dir.join(name),
        staged: temp_new,
        name: name.to_string(),
    }))
}

/// Atomically move a staged binary over its destination.
fn commit_staged(b: StagedBinary) -> Result<(), String> {
    fs::rename(&b.staged, &b.dest).map_err(|e| format!("Failed to replace {}: {e}", b.name))
}

/// Stage + verify the CLI (required) and the companion binaries (optional),
/// then install them. If anything fails while staging, every staged file is
/// removed and the installed binaries are left exactly as they were.
fn install_bundle(temp_dir: &std::path::Path, install_dir: &std::path::Path) -> Result<(), String> {
    let mut staged: Vec<StagedBinary> = Vec::new();
    let mut failure: Option<String> = None;
    for (name, required) in [
        (BINARY_NAME, true),
        (SERVER_BINARY_NAME, false),
        (MCP_BINARY_NAME, false),
    ] {
        match stage_binary(temp_dir, name, install_dir, required) {
            Ok(Some(b)) => staged.push(b),
            Ok(None) => {}
            Err(e) => {
                failure = Some(e);
                break;
            }
        }
    }
    if let Some(e) = failure {
        for b in &staged {
            let _ = fs::remove_file(&b.staged);
        }
        return Err(e);
    }
    for b in staged {
        commit_staged(b)?;
    }
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
    // Primary: HEAD /releases/latest returns a 302 whose `location` header
    // carries the latest tag — no API call, no rate limit. reqwest follows
    // redirects by default, which lands on the tag page (200, no location
    // header), so redirect following must be disabled for this probe (#1307).
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;

    if let Ok(resp) = client
        .head(format!("https://github.com/{REPO}/releases/latest"))
        .send()
    {
        if let Some(location) = resp.headers().get("location") {
            if let Some(tag) = parse_tag_from_location(location.to_str().unwrap_or_default()) {
                return Ok(tag);
            }
        }
    }

    // Fallback: GitHub API (60 req/h unauthenticated — best effort only).
    let api_client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;
    let api_url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let mut req = api_client
        .get(&api_url)
        .header("User-Agent", "uteke-upgrade")
        .header("Accept", "application/vnd.github+json");
    if let Ok(token) = std::env::var("GITHUB_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            req = req.bearer_auth(&token);
        }
    }
    let resp = req.send().map_err(|e| format!("GitHub API failed: {e}"))?;

    if matches!(resp.status().as_u16(), 403 | 429) {
        if std::env::var("GITHUB_TOKEN").is_ok_and(|t| !t.trim().is_empty()) {
            return Err(
                "GitHub API rate limit exceeded even with GITHUB_TOKEN set. \
                 Retry later — the 302-redirect primary path needs no token."
                    .into(),
            );
        }
        return Err(
            "GitHub API rate limit exceeded (unauthenticated: 60 req/h per IP). \
             Retry later or set GITHUB_TOKEN to raise the limit."
                .into(),
        );
    }

    if resp.status().is_success() {
        let json: serde_json::Value = resp
            .json()
            .map_err(|e| format!("Failed to parse GitHub API response: {e}"))?;
        if let Some(tag) = json["tag_name"].as_str() {
            if is_valid_version_tag(tag) {
                return Ok(tag.to_string());
            }
        }
    }

    Err(format!(
        "Failed to determine latest version. Check https://github.com/{REPO}/releases"
    ))
}

/// Extract a version tag from a `releases/tag/<tag>` URL (absolute or relative).
fn parse_tag_from_location(loc: &str) -> Option<String> {
    let tag_marker = format!("/{REPO}/releases/tag/");
    if let Some((_, tag)) = loc.split_once(&tag_marker) {
        let tag = tag.split('?').next().unwrap_or_default();
        if is_valid_version_tag(tag) {
            return Some(tag.to_string());
        }
    }
    // Mirror-style alternate URL: last path segment when it looks like a tag.
    let last = loc.rsplit('/').next()?;
    let tag = last.split('?').next().unwrap_or_default();
    if tag.starts_with('v') && is_valid_version_tag(tag) {
        return Some(tag.to_string());
    }
    None
}

/// A version tag looks like `v0.19.0` / `0.19.0` (optional suffix after `-`).
/// The whole tag (suffix included) is restricted to `[0-9A-Za-z.-]` because it
/// is interpolated into the download URL and the archive file name (#1327).
fn is_valid_version_tag(tag: &str) -> bool {
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    if !tag
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        return false;
    }
    let mut parts = tag.strip_prefix('v').unwrap_or(tag).split('.');
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().unwrap_or_default();
    let patch_raw = parts.next().unwrap_or_default();
    let (patch, _) = patch_raw.split_once('-').unwrap_or((patch_raw, ""));
    numeric(major) && numeric(minor) && numeric(patch)
}

/// Reject a download whose declared size already exceeds `max`.
/// `None` (no Content-Length) passes: [`copy_capped`] still enforces the cap.
fn check_declared_len(declared: Option<u64>, max: u64, what: &str) -> Result<(), String> {
    match declared {
        Some(n) if n > max => Err(format!(
            "{what} is too large ({n} bytes declared, limit {max}) — refusing to download"
        )),
        _ => Ok(()),
    }
}

/// Copy `reader` into `writer`, failing as soon as more than `max` bytes
/// arrive. Independent of any Content-Length header, so a missing or lying
/// header cannot bypass the cap. Returns the number of bytes copied.
fn copy_capped<R: Read, W: Write>(
    reader: &mut R,
    writer: &mut W,
    max: u64,
    what: &str,
) -> Result<u64, String> {
    // Read at most max+1 bytes: getting the extra byte proves the overflow.
    let copied = io::copy(&mut reader.take(max.saturating_add(1)), writer)
        .map_err(|e| format!("Failed to write {what}: {e}"))?;
    if copied > max {
        return Err(format!(
            "{what} exceeds the size limit of {max} bytes — aborting download"
        ));
    }
    Ok(copied)
}

/// Component-based check for an archive entry path: only normal components
/// (and `.`) are allowed. Absolute paths, drive prefixes and any `..`
/// component are rejected, so the entry cannot resolve outside the target dir.
/// Names that merely contain dots (`a..b`) are fine.
fn is_safe_archive_path(path: &Path) -> bool {
    path.components()
        .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

/// Download the cosign bundle for the release, bounded and fail-closed.
fn fetch_bundle(
    client: &reqwest::blocking::Client,
    url: &str,
    tag: &str,
) -> Result<Vec<u8>, String> {
    let resp = client.get(url).send().map_err(|e| {
        format!(
            "Failed to download the checksums signature: {e}. \
             Refusing to install an unverified binary. {SIGNATURE_HELP}"
        )
    })?;
    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!(
            "No checksums signature for {tag} (HTTP {status}); the release may not be signed. \
             Refusing to install an unverified binary. {SIGNATURE_HELP}"
        ));
    }
    check_declared_len(resp.content_length(), MAX_BUNDLE_BYTES, "Signature bundle")?;
    let mut resp = resp;
    let mut buf: Vec<u8> = Vec::new();
    copy_capped(&mut resp, &mut buf, MAX_BUNDLE_BYTES, "Signature bundle")?;
    Ok(buf)
}

/// Fail-closed glue between the bundle fetch and the verifier: a fetch error
/// (404, network, oversize) is an error, never a skipped check.
fn authenticate_checksums(
    checksums: &[u8],
    bundle: Result<Vec<u8>, String>,
    tag: &str,
    trust: &release_verify::TrustRoots<'_>,
) -> Result<(), String> {
    let bundle = bundle?;
    release_verify::verify_bundle(checksums, &bundle, tag, trust).map_err(|why| {
        format!(
            "Checksums signature verification failed: {why}. Refusing to install. {SIGNATURE_HELP}"
        )
    })
}

/// The only way `run()` obtains an expected archive hash: the checksums bytes
/// must first be authenticated by the signature (fail closed), and only then
/// is the hash for `archive_name` read from them.
fn verified_expected_hash(
    checksums: &[u8],
    bundle: Result<Vec<u8>, String>,
    tag: &str,
    trust: &release_verify::TrustRoots<'_>,
    archive_name: &str,
) -> Result<String, String> {
    authenticate_checksums(checksums, bundle, tag, trust)?;
    let text = std::str::from_utf8(checksums)
        .map_err(|e| format!("Checksums file is not valid UTF-8: {e}"))?;
    parse_checksum(text, archive_name).ok_or_else(|| {
        format!(
            "Checksum for '{archive_name}' not found in checksums file. \
             Refusing to install unverified binary. {SIGNATURE_HELP}"
        )
    })
}

/// Accept only a stable release tag: `vX.Y.Z` (the `v` may be missing, it is
/// normalised) with numeric components only; no pre-release, no build
/// metadata. Returns the normalised `vX.Y.Z`.
fn ensure_stable_tag(tag: &str) -> Result<String, String> {
    let bare = tag.strip_prefix('v').unwrap_or(tag);
    let parts: Vec<&str> = bare.split('.').collect();
    let stable = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if stable {
        Ok(format!("v{bare}"))
    } else {
        Err(format!("Refusing to install a non-stable release: {tag}"))
    }
}

/// Refuse a target that predates the keyless signing scheme (#1610). The scheme
/// is chosen by the version of the tag being installed, never by which files
/// the release happens to carry, so a missing bundle cannot downgrade the check.
fn ensure_verifiable_tag(tag: &str) -> Result<(), String> {
    let (ma, mi, pa, _) =
        parse_version(tag).ok_or_else(|| format!("Unparseable release version '{tag}'"))?;
    if (ma, mi, pa) < MIN_VERIFIABLE_VERSION {
        let (a, b, c) = MIN_VERIFIABLE_VERSION;
        return Err(format!(
            "Refusing to install {tag}: it predates signature verification by uteke upgrade \
             (v{a}.{b}.{c} is the first release this build can verify). Reinstall by hand \
             (package manager or a manual download). {SIGNATURE_HELP}"
        ));
    }
    Ok(())
}

/// Parsed `major.minor.patch[-pre][+build]`.
fn parse_version(v: &str) -> Option<(u64, u64, u64, Option<Vec<String>>)> {
    let v = v.trim().trim_start_matches('v');
    let v = v.split('+').next()?;
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (v, None),
    };
    let mut it = core.split('.');
    let mut num = || -> Option<u64> {
        let part = it.next()?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    let (ma, mi, pa) = (num()?, num()?, num()?);
    if it.next().is_some() {
        return None;
    }
    let pre = match pre {
        None => None,
        Some(p) => {
            let ids: Vec<String> = p.split('.').map(str::to_string).collect();
            if ids.iter().any(|i| i.is_empty()) {
                return None;
            }
            Some(ids)
        }
    };
    Some((ma, mi, pa, pre))
}

/// Semver ordering of `latest` relative to `current` (`Less` means latest is
/// newer, i.e. an upgrade). Unparseable versions are an error.
fn compare_versions(current: &str, latest: &str) -> Result<std::cmp::Ordering, String> {
    use std::cmp::Ordering;
    let c =
        parse_version(current).ok_or_else(|| format!("Unparseable current version '{current}'"))?;
    let l =
        parse_version(latest).ok_or_else(|| format!("Unparseable release version '{latest}'"))?;
    let core = (c.0, c.1, c.2).cmp(&(l.0, l.1, l.2));
    if core != Ordering::Equal {
        return Ok(core);
    }
    Ok(match (&c.3, &l.3) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Greater,
        (Some(_), None) => Ordering::Less,
        (Some(a), Some(b)) => {
            let mut ord = Ordering::Equal;
            for (x, y) in a.iter().zip(b.iter()) {
                let o = match (x.parse::<u64>(), y.parse::<u64>()) {
                    (Ok(x), Ok(y)) => x.cmp(&y),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => x.cmp(y),
                };
                if o != Ordering::Equal {
                    ord = o;
                    break;
                }
            }
            if ord == Ordering::Equal {
                a.len().cmp(&b.len())
            } else {
                ord
            }
        }
    })
}

fn parse_checksum(checksums_text: &str, archive_name: &str) -> Option<String> {
    for line in checksums_text.lines() {
        let parts: Vec<&str> = line.split_whitespace().collect();
        // Exact filename match — substring matching can pick a different
        // artifact's line (e.g. a `.sig` companion of the same archive).
        if parts.len() >= 2 && parts[1] == archive_name {
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
    fn parse_checksum_requires_exact_filename() {
        let text = concat!(
            "aaaa  uteke-x86_64-unknown-linux-gnu-v0.19.0.tar.gz.sig\n",
            "bbbb  uteke-x86_64-unknown-linux-gnu-v0.19.0.tar.gz\n",
            "cccc  uteke-aarch64-unknown-linux-gnu-v0.19.0.tar.gz\n",
        );
        // Substring candidates (the `.sig` companion) must not win.
        assert_eq!(
            parse_checksum(text, "uteke-x86_64-unknown-linux-gnu-v0.19.0.tar.gz"),
            Some("bbbb".to_string())
        );
        assert_eq!(parse_checksum(text, "missing.tar.gz"), None);
    }

    #[test]
    fn test_parse_tag_from_location_absolute() {
        assert_eq!(
            parse_tag_from_location("https://github.com/codecoradev/uteke/releases/tag/v0.19.0"),
            Some("v0.19.0".to_string())
        );
        assert_eq!(
            parse_tag_from_location("/codecoradev/uteke/releases/tag/v0.18.1"),
            Some("v0.18.1".to_string())
        );
        assert_eq!(
            parse_tag_from_location(
                "https://github.com/codecoradev/uteke/releases/tag/v0.19.0?utm_source=test"
            ),
            Some("v0.19.0".to_string())
        );
    }

    #[test]
    fn test_parse_tag_from_location_rejects_non_tag() {
        assert_eq!(
            parse_tag_from_location("https://github.com/codecoradev/uteke/releases"),
            None
        );
        assert_eq!(
            parse_tag_from_location("/codecoradev/uteke/releases/tag/latest"),
            None
        );
    }

    #[test]
    fn test_is_valid_version_tag() {
        assert!(is_valid_version_tag("v0.19.0"));
        assert!(is_valid_version_tag("0.19.0"));
        assert!(is_valid_version_tag("v1.0.0-rc.1"));
        assert!(!is_valid_version_tag("latest"));
        assert!(!is_valid_version_tag(""));
        assert!(!is_valid_version_tag("v1"));
        assert!(!is_valid_version_tag("v.alpha"));
    }

    #[test]
    fn version_tag_suffix_is_restricted_to_url_safe_chars() {
        // Previously accepted: the suffix after `-` was never inspected.
        assert!(!is_valid_version_tag("v1.2.3-../../evil"));
        assert!(!is_valid_version_tag("v1.2.3-rc/1"));
        assert!(!is_valid_version_tag("v1.2.3-rc 1"));
        assert!(!is_valid_version_tag("v1.2.3-rc?x=1"));
        assert!(!is_valid_version_tag("v1.2.3-rc\\1"));
        assert!(!is_valid_version_tag("vv1.2.3"));
        assert!(is_valid_version_tag("v1.2.3-rc.1"));
        assert!(is_valid_version_tag("v1.2.3-Beta-2"));
    }

    #[test]
    fn copy_capped_enforces_limit_without_content_length() {
        let data = [7u8; 100];
        let mut out = Vec::new();
        assert_eq!(
            copy_capped(&mut &data[..], &mut out, 100, "x").unwrap(),
            100
        );
        assert_eq!(out.len(), 100);

        let mut out = Vec::new();
        let err = copy_capped(&mut &data[..], &mut out, 99, "Release archive").unwrap_err();
        assert!(
            err.contains("Release archive") && err.contains("99"),
            "{err}"
        );
        // Streaming stopped at the cap + 1 byte, not the whole body.
        assert_eq!(out.len(), 100);
        let big = [0u8; 10_000];
        let mut out = Vec::new();
        assert!(copy_capped(&mut &big[..], &mut out, 10, "x").is_err());
        assert_eq!(out.len(), 11);
    }

    #[test]
    fn declared_length_over_the_cap_is_rejected() {
        assert!(check_declared_len(Some(10), 10, "x").is_ok());
        assert!(check_declared_len(None, 10, "x").is_ok());
        let err = check_declared_len(Some(11), 10, "Checksums file").unwrap_err();
        assert!(
            err.contains("Checksums file") && err.contains("too large"),
            "{err}"
        );
    }

    #[test]
    fn archive_path_check_is_component_based() {
        let ok = |p: &str| is_safe_archive_path(Path::new(p));
        // Layouts the release bundles use today.
        assert!(ok("uteke"));
        assert!(ok("./uteke"));
        assert!(ok("libonnxruntime.so.1.22.0"));
        assert!(ok("dir/uteke-serve"));
        // Dots inside a name are not traversal (the old `contains("..")` refused these).
        assert!(ok("uteke..bak"));
        assert!(ok("lib..so"));
        // Unsafe.
        assert!(!ok("/etc/passwd"));
        assert!(!ok("../evil"));
        assert!(!ok("a/../../evil"));
        assert!(!ok("a/.."));
        assert!(!ok(".."));
    }

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

    fn write_exec(path: &std::path::Path, script: &str) {
        fs::write(path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    fn scratch(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("uteke-upgrade-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let bundle = dir.join("bundle");
        let inst = dir.join("inst");
        fs::create_dir_all(&bundle).unwrap();
        fs::create_dir_all(&inst).unwrap();
        (dir, bundle, inst)
    }

    const OK_NEW: &str = "#!/bin/sh\necho \"uteke 9.9.9\"\n";
    const OLD: &str = "#!/bin/sh\necho \"uteke 0.0.1\"\n";

    #[test]
    fn install_bundle_installs_cli_and_companions() {
        let (dir, bundle, inst) = scratch("bundle-ok");
        for n in ["uteke", "uteke-serve", "uteke-mcp"] {
            write_exec(&bundle.join(n), OK_NEW);
            write_exec(&inst.join(n), OLD);
        }
        install_bundle(&bundle, &inst).unwrap();
        for n in ["uteke", "uteke-serve", "uteke-mcp"] {
            assert_eq!(fs::read_to_string(inst.join(n)).unwrap(), OK_NEW, "{n}");
            assert!(!inst.join(format!("{n}.new")).exists());
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// #1332: the CLI used to be replaced before the companions were checked,
    /// leaving a new CLI next to an old server when a companion was broken.
    #[test]
    fn a_broken_companion_leaves_every_installed_binary_untouched() {
        let (dir, bundle, inst) = scratch("bundle-broken");
        write_exec(&bundle.join("uteke"), OK_NEW);
        write_exec(&bundle.join("uteke-serve"), "#!/bin/sh\nexit 1\n");
        write_exec(&bundle.join("uteke-mcp"), OK_NEW);
        for n in ["uteke", "uteke-serve", "uteke-mcp"] {
            write_exec(&inst.join(n), OLD);
        }

        let err = install_bundle(&bundle, &inst).unwrap_err();
        assert!(err.contains("uteke-serve"), "{err}");
        for n in ["uteke", "uteke-serve", "uteke-mcp"] {
            assert_eq!(
                fs::read_to_string(inst.join(n)).unwrap(),
                OLD,
                "{n} must still be the old version"
            );
            assert!(
                !inst.join(format!("{n}.new")).exists(),
                "no staged leftovers for {n}"
            );
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn install_bundle_requires_the_cli_but_not_the_companions() {
        let (dir, bundle, inst) = scratch("bundle-cli-only");
        write_exec(&bundle.join("uteke"), OK_NEW);
        install_bundle(&bundle, &inst).unwrap();
        assert!(inst.join("uteke").exists());
        assert!(!inst.join("uteke-serve").exists());

        let (dir2, bundle2, inst2) = scratch("bundle-no-cli");
        write_exec(&bundle2.join("uteke-serve"), OK_NEW);
        assert!(install_bundle(&bundle2, &inst2).is_err());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&dir2);
    }
}

#[cfg(test)]
mod signature_tests {
    use super::*;

    // The real v0.22.0 release: its checksums file and cosign bundle.
    const CHECKSUMS: &[u8] = include_bytes!("../../tests/fixtures/cosign/checksums-sha256.txt");
    const BUNDLE: &[u8] = include_bytes!("../../tests/fixtures/cosign/checksums-sha256.txt.bundle");
    const TAG: &str = "v0.22.0";
    const ARCHIVE: &str = "uteke-aarch64-apple-darwin-v0.22.0.tar.gz";

    fn check(data: &[u8], bundle: &[u8], tag: &str) -> Result<(), String> {
        authenticate_checksums(data, Ok(bundle.to_vec()), tag, &release_verify::PUBLIC_GOOD)
    }

    #[test]
    fn the_real_release_bundle_authenticates_its_checksums() {
        assert_eq!(check(CHECKSUMS, BUNDLE, TAG), Ok(()));
    }

    #[test]
    fn tampered_data_is_a_data_mismatch() {
        let mut bad = CHECKSUMS.to_vec();
        bad[0] = if bad[0] == b'a' { b'c' } else { b'a' };
        let e = check(&bad, BUNDLE, TAG).unwrap_err();
        assert!(e.contains("data mismatch"), "{e}");
        assert!(e.contains("SECURITY.md"), "{e}");
    }

    #[test]
    fn a_bundle_for_another_tag_is_rejected() {
        // The v0.22.0 bundle offered for another release: the identity names v0.22.0.
        let e = check(CHECKSUMS, BUNDLE, "v0.22.1").unwrap_err();
        assert!(e.contains("different workflow or release"), "{e}");
        assert!(e.contains("refs/tags/v0.22.0"), "{e}");
        assert!(e.contains("SECURITY.md"), "{e}");
    }

    #[test]
    fn garbage_empty_and_truncated_bundles_are_rejected() {
        for bundle in [
            &b"this is not a bundle"[..],
            b"",
            b"{}",
            b"   \n",
            &BUNDLE[..BUNDLE.len() / 2],
        ] {
            let e = check(CHECKSUMS, bundle, TAG).unwrap_err();
            assert!(e.contains("not a valid cosign bundle"), "{bundle:?}: {e}");
            assert!(e.contains("SECURITY.md"), "{e}");
        }
    }

    #[test]
    fn authenticate_fails_closed_on_fetch_errors() {
        let trust = &release_verify::PUBLIC_GOOD;
        let e = authenticate_checksums(
            CHECKSUMS,
            Err("No checksums signature for v0.22.0 (HTTP 404 Not Found)".into()),
            TAG,
            trust,
        )
        .unwrap_err();
        assert!(e.contains("404"), "{e}");
        assert!(authenticate_checksums(CHECKSUMS, Ok(Vec::new()), TAG, trust).is_err());
    }

    fn hash(data: &[u8], bundle: Result<Vec<u8>, String>, archive: &str) -> Result<String, String> {
        verified_expected_hash(data, bundle, TAG, &release_verify::PUBLIC_GOOD, archive)
    }

    #[test]
    fn expected_hash_requires_a_good_bundle() {
        let ok = || Ok(BUNDLE.to_vec());
        let got = hash(CHECKSUMS, ok(), ARCHIVE).unwrap();
        assert_eq!(got.len(), 64);
        assert_eq!(
            Some(got),
            parse_checksum(std::str::from_utf8(CHECKSUMS).unwrap(), ARCHIVE)
        );
        assert!(hash(CHECKSUMS, Err("HTTP 404".into()), ARCHIVE).is_err());
        assert!(hash(CHECKSUMS, Ok(Vec::new()), ARCHIVE).is_err());
        // A signature for other bytes: tampered checksums never yield a hash.
        let mut tampered = CHECKSUMS.to_vec();
        tampered[0] = if tampered[0] == b'a' { b'c' } else { b'a' };
        let e = hash(&tampered, ok(), ARCHIVE).unwrap_err();
        assert!(e.contains("data mismatch"), "{e}");
        let e = hash(CHECKSUMS, ok(), "uteke-missing.tar.gz").unwrap_err();
        assert!(e.contains("not found in checksums file"), "{e}");
    }

    #[test]
    fn error_hints_never_recommend_the_bypass() {
        // The variable also disables signature verification, so a transient
        // failure must point at retrying and manual verification instead.
        let missing = hash(CHECKSUMS, Ok(BUNDLE.to_vec()), "uteke-missing.tar.gz").unwrap_err();
        for e in [
            missing,
            checksums_unavailable("HTTP 503 Service Unavailable"),
            checksums_unavailable("connection reset"),
        ] {
            assert!(!e.contains("UTEKE_UPGRADE_SKIP_CHECKSUM"), "{e}");
            assert!(e.contains("SECURITY.md"), "{e}");
        }
        let e = checksums_unavailable("HTTP 503");
        assert!(e.contains("HTTP 503") && e.contains("retry later"), "{e}");
    }

    #[test]
    fn a_target_below_the_first_verifiable_release_is_refused() {
        assert!(ensure_verifiable_tag("v0.22.1").is_ok());
        assert!(ensure_verifiable_tag("v0.23.0").is_ok());
        assert!(ensure_verifiable_tag("v1.0.0").is_ok());
        for old in ["v0.22.0", "v0.21.9", "v0.1.0"] {
            let e = ensure_verifiable_tag(old).unwrap_err();
            assert!(e.contains("predates signature verification"), "{old}: {e}");
            assert!(e.contains("v0.22.1"), "{e}");
        }
        assert!(ensure_verifiable_tag("garbage").is_err());
    }

    #[test]
    fn version_comparison() {
        use std::cmp::Ordering::*;
        let c = |a, b| compare_versions(a, b).unwrap();
        assert_eq!(c("0.21.0", "v0.21.0"), Equal);
        assert_eq!(c("0.21.0", "v0.21.1"), Less);
        assert_eq!(c("0.21.1", "v0.21.0"), Greater);
        assert_eq!(c("0.21.0", "v0.22.0"), Less);
        assert_eq!(c("0.22.0", "v0.21.9"), Greater);
        assert_eq!(c("0.21.0", "v1.0.0"), Less);
        assert_eq!(c("1.0.0", "v0.99.99"), Greater);
        assert_eq!(c("0.9.0", "v0.10.0"), Less);
        assert_eq!(c("0.10.0", "v0.9.0"), Greater);
        assert_eq!(c("0.22.0-rc.1", "v0.22.0"), Less);
        assert_eq!(c("0.22.0", "v0.22.0-rc.1"), Greater);
        assert_eq!(c("0.22.0-rc.1", "v0.22.0-rc.2"), Less);
        assert_eq!(c("0.22.0-rc.2", "v0.22.0-rc.10"), Less);
        assert_eq!(c("0.22.0-alpha", "v0.22.0-alpha.1"), Less);
        assert_eq!(c("0.22.0-1", "v0.22.0-alpha"), Less);
        for bad in ["vfoo", "v1.2", "v1.2.3.4", "v1.2.x", "v1..3", "v1.2.3-", ""] {
            assert!(compare_versions("0.21.0", bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_stable_tags_are_installable() {
        assert_eq!(ensure_stable_tag("v0.22.0").unwrap(), "v0.22.0");
        assert_eq!(ensure_stable_tag("0.22.0").unwrap(), "v0.22.0");
        for bad in [
            "v0.23.0-rc.1",
            "v0.23.0-beta",
            "v0.23.0-alpha.2",
            "v0.23.0+build",
            "v0.23.0-rc.1+build",
            "0.23.0-rc.1",
            "vv0.23.0",
            "v1.2",
            "1.2",
            "v1.2.3.4",
            "v1..3",
            "v1.2.",
            "v.1.2",
            "v1.2.x",
            "v-1.2.3",
            "",
            "v",
        ] {
            let e = ensure_stable_tag(bad).unwrap_err();
            assert_eq!(
                e,
                format!("Refusing to install a non-stable release: {bad}")
            );
        }
    }

    #[test]
    fn stable_current_refuses_prerelease_latest_and_prerelease_current_upgrades() {
        use std::cmp::Ordering::*;
        // 0.22.0 vs latest 0.22.0-rc.1 is a downgrade (and rc.1 is not stable anyway).
        assert_eq!(compare_versions("0.22.0", "v0.22.0-rc.1"), Ok(Greater));
        assert!(ensure_stable_tag("v0.22.0-rc.1").is_err());
        // A dev/pre-release build upgrades to a newer stable.
        assert_eq!(compare_versions("0.22.0-dev", "v0.22.0"), Ok(Less));
        assert_eq!(compare_versions("0.22.0-dev", "v0.23.0"), Ok(Less));
        assert!(ensure_stable_tag("v0.23.0").is_ok());
    }
}
