//! Update check — compare installed version against latest GitHub release.
//!
//! Shared by CLI, MCP server, and HTTP server. Each surface calls
//! [`check_and_notify`] on startup (and periodically, for long-running servers).
//!
//! Uses a 24h cache file to avoid hammering GitHub. Network failures are
//! silently swallowed — this is a notification, not a critical path.

use std::time::{SystemTime, UNIX_EPOCH};

/// Crate name on GitHub (`codecoradev/uteke`).
const REPO: &str = "codecoradev/uteke";

/// Seconds before re-checking GitHub (24 hours).
const CACHE_TTL: u64 = 86_400;

/// Cache payload persisted between runs.
#[derive(serde::Serialize, serde::Deserialize)]
struct Cache {
    /// Unix timestamp (seconds) of last check.
    checked_at: u64,
    /// Latest version tag found (e.g. `v0.13.1`).
    latest: String,
}

/// Result of an update check.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    /// Latest version tag from GitHub (e.g. `v0.14.0`).
    pub latest: String,
    /// Current installed version (e.g. `0.13.1`).
    pub current: String,
}

impl UpdateInfo {
    /// Returns `true` if `latest` is newer than `current`.
    pub fn is_update_available(&self) -> bool {
        is_newer(&self.latest, &self.current)
    }

    /// One-line banner for stderr/logs.
    ///
    /// ```text
    /// ⚠ Update available: v0.14.0 (currently v0.13.1) — run `uteke upgrade`
    /// ```
    pub fn banner(&self) -> String {
        format!(
            "⚠ Update available: {} (currently v{}) — run `uteke upgrade` or visit https://github.com/{}/releases/tag/{}",
            self.latest, self.current, REPO, self.latest
        )
    }
}

/// Check for updates using cached data (no network I/O).
///
/// Returns `Some(UpdateInfo)` if the cache is fresh (< 24h) and contains
/// a version string. Returns `None` if cache is stale, missing, or corrupt.
pub fn check_cached() -> Option<UpdateInfo> {
    let latest = read_cache()?;
    Some(UpdateInfo {
        latest,
        current: current_version().to_string(),
    })
}

/// Synchronous network check. Fetches latest version from GitHub,
/// updates the cache, and returns `UpdateInfo`.
///
/// # Errors
/// Returns `Err` only on network failure. Callers should silently ignore.
pub fn check_network() -> Result<UpdateInfo, String> {
    let latest = get_latest_version()?;
    write_cache(&latest);
    Ok(UpdateInfo {
        latest,
        current: current_version().to_string(),
    })
}

/// Convenience: check cached first, fall back to network.
///
/// If you need non-blocking behaviour, call [`check_cached`] yourself
/// and spawn [`check_network`] in a background thread.
pub fn check() -> Option<UpdateInfo> {
    if let Some(info) = check_cached() {
        return Some(info);
    }
    check_network().ok()
}

/// Check and print a banner to stderr if an update is available.
///
/// Tries cache first (instant). If cache is stale, spawns a background
/// thread for the network check and returns a [`std::thread::JoinHandle`].
/// The caller **must** `.join()` this handle before process exit to avoid
/// the thread being killed prematurely.
///
/// If cache is fresh, prints immediately and returns `None`.
/// If no update is available, returns `None`.
pub fn check_and_notify() -> Option<std::thread::JoinHandle<()>> {
    // Fast path: cache is fresh.
    if let Some(info) = check_cached() {
        if info.is_update_available() {
            eprintln!("\n{}\n", info.banner());
        }
        return None;
    }

    // Slow path: spawn background thread.
    let handle = std::thread::spawn(move || {
        let _ = std::panic::catch_unwind(|| {
            if let Some(info) = check_network().ok().filter(|i| i.is_update_available()) {
                eprintln!("\n{}\n", info.banner());
            }
        });
    });

    Some(handle)
}

// ── Internals ───────────────────────────────────────────────────────────────

/// Compile-time current version from `CARGO_PKG_VERSION`.
fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Cache file path: `~/.config/uteke/update-cache.json` (or platform equivalent).
fn cache_path() -> Option<std::path::PathBuf> {
    dirs::config_dir().map(|d| d.join("uteke").join("update-cache.json"))
}

/// Read cache. Returns `Some(latest_version)` if cache is fresh (< 24h).
fn read_cache() -> Option<String> {
    let path = cache_path()?;
    let data = std::fs::read_to_string(&path).ok()?;
    let cache: Cache = serde_json::from_str(&data).ok()?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if now.saturating_sub(cache.checked_at) < CACHE_TTL {
        Some(cache.latest)
    } else {
        None
    }
}

/// Persist cache to disk. Best-effort — ignores errors.
fn write_cache(latest: &str) {
    let Some(path) = cache_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let cache = Cache {
        checked_at: now,
        latest: latest.to_string(),
    };

    if let Ok(json) = serde_json::to_string(&cache) {
        let _ = std::fs::write(&path, json);
    }
}

/// Fetch latest release tag from GitHub.
///
/// Primary: follow the `/releases/latest` 302 redirect (no API call, no rate limit).
/// Fallback: GitHub REST API.
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
    let resp = api_client
        .get(&api_url)
        .header("User-Agent", "uteke-update-check")
        .header("Accept", "application/vnd.github+json")
        .send()
        .map_err(|e| format!("GitHub API failed: {e}"))?;

    if matches!(resp.status().as_u16(), 403 | 429) {
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
fn is_valid_version_tag(tag: &str) -> bool {
    let numeric = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_digit());
    let mut parts = tag.trim_start_matches('v').split('.');
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().unwrap_or_default();
    let patch_raw = parts.next().unwrap_or_default();
    let (patch, _) = patch_raw.split_once('-').unwrap_or((patch_raw, ""));
    numeric(major) && numeric(minor) && numeric(patch)
}

/// Compare semver-ish versions. Returns true if `latest` > `current`.
///
/// Strips leading `v` prefix. Falls back to string comparison on parse failure.
fn is_newer(latest: &str, current: &str) -> bool {
    let lt = latest.trim_start_matches('v');
    let cur = current.trim_start_matches('v');

    let parse = |s: &str| -> Vec<u64> {
        s.split('.')
            .filter_map(|p| p.split('-').next().and_then(|n| n.parse().ok()))
            .collect()
    };

    match (parse(lt), parse(cur)) {
        (lt_parts, cur_parts) if !lt_parts.is_empty() && !cur_parts.is_empty() => {
            lt_parts > cur_parts
        }
        _ => lt > cur,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_tag_from_location_absolute() {
        assert_eq!(
            parse_tag_from_location("https://github.com/codecoradev/uteke/releases/tag/v0.19.0"),
            Some("v0.19.0".to_string())
        );
    }

    #[test]
    fn test_parse_tag_from_location_relative() {
        assert_eq!(
            parse_tag_from_location("/codecoradev/uteke/releases/tag/v0.18.1"),
            Some("v0.18.1".to_string())
        );
    }

    #[test]
    fn test_parse_tag_from_location_with_query() {
        assert_eq!(
            parse_tag_from_location(
                "https://github.com/codecoradev/uteke/releases/tag/v0.19.0?utm_source=test"
            ),
            Some("v0.19.0".to_string())
        );
    }

    #[test]
    fn test_parse_tag_from_location_rejects_non_tag() {
        // Not a tag URL at all.
        assert_eq!(
            parse_tag_from_location("https://github.com/codecoradev/uteke/releases"),
            None
        );
        // Tag marker present but segment is not a version.
        assert_eq!(
            parse_tag_from_location("/codecoradev/uteke/releases/tag/latest"),
            None
        );
    }

    #[test]
    fn test_parse_tag_from_location_mirror_last_segment() {
        assert_eq!(
            parse_tag_from_location("https://mirror.example.com/uteke/v0.19.0?sig=abc"),
            Some("v0.19.0".to_string())
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
    fn test_is_newer_true() {
        assert!(is_newer("v0.14.0", "0.13.0"));
        assert!(is_newer("0.14.0", "0.13.0"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(is_newer("v0.13.1", "0.13.0"));
    }

    #[test]
    fn test_is_newer_false() {
        assert!(!is_newer("v0.13.0", "0.13.0"));
        assert!(!is_newer("v0.12.0", "0.13.0"));
        assert!(!is_newer("v0.13.0", "0.13.1"));
    }

    #[test]
    fn test_is_newer_major() {
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(!is_newer("v1.9.9", "2.0.0"));
    }

    #[test]
    fn test_update_info_banner() {
        let info = UpdateInfo {
            latest: "v0.14.0".to_string(),
            current: "0.13.1".to_string(),
        };
        assert!(info.is_update_available());
        assert!(info.banner().contains("v0.14.0"));
        assert!(info.banner().contains("0.13.1"));
    }

    #[test]
    fn test_update_info_no_update() {
        let info = UpdateInfo {
            latest: "v0.13.1".to_string(),
            current: "0.13.1".to_string(),
        };
        assert!(!info.is_update_available());
    }
}
