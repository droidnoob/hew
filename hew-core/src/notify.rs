//! Passive update notification.
//!
//! On every `hew prime` we kick off a background thread that checks
//! GitHub once per 24h for a newer release. The result lands in a
//! tiny cache file (`<cache_dir>/update-available`). The NEXT prime
//! reads the cache and surfaces a one-line notice — no extra wait.
//!
//! The check is best-effort. Network failures are silent; tests can
//! disable the whole feature by setting `HEW_NO_UPDATE_CHECK=1` or
//! the persistent config flag.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{HewError, Result};

const CHECK_INTERVAL_SECS: u64 = 60 * 60 * 24;

#[derive(Debug, Clone)]
pub struct UpdateNotice {
    pub current: String,
    pub latest: String,
}

fn cache_root() -> Result<PathBuf> {
    if let Ok(p) = std::env::var("HEW_CACHE_DIR") {
        return Ok(PathBuf::from(p));
    }
    use etcetera::BaseStrategy;
    let strategy = etcetera::choose_base_strategy()
        .map_err(|e| HewError::Io(std::io::Error::other(e.to_string())))?;
    Ok(strategy.cache_dir().join("hew"))
}

fn marker_path() -> Result<PathBuf> {
    Ok(cache_root()?.join("last-update-check"))
}

fn payload_path() -> Result<PathBuf> {
    Ok(cache_root()?.join("update-available"))
}

/// Read the cached notice, if any. Always non-blocking; errors out
/// only on filesystem failures unrelated to "not present."
pub fn read_cached_notice() -> Result<Option<UpdateNotice>> {
    let path = payload_path()?;
    match std::fs::read_to_string(&path) {
        Ok(s) => parse(&s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(HewError::Io(e)),
    }
}

fn parse(s: &str) -> Result<Option<UpdateNotice>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed: serde_json::Value = serde_json::from_str(trimmed)?;
    let current = parsed.get("current").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let latest = parsed.get("latest").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if current.is_empty() || latest.is_empty() || current == latest {
        return Ok(None);
    }
    Ok(Some(UpdateNotice { current, latest }))
}

/// Schedule a background check if the last one was > 24h ago. Returns
/// immediately. Spawns a thread; the thread silently swallows network
/// errors. Disabled when `HEW_NO_UPDATE_CHECK=1` is set.
pub fn schedule_if_stale(current_version: &'static str) {
    if std::env::var("HEW_NO_UPDATE_CHECK")
        .map(|v| !matches!(v.as_str(), "" | "0" | "false" | "no"))
        .unwrap_or(false)
    {
        return;
    }
    if !is_stale() {
        return;
    }
    std::thread::spawn(move || {
        let _ = check_and_write(current_version);
    });
}

fn is_stale() -> bool {
    let Ok(path) = marker_path() else { return true };
    let Ok(meta) = std::fs::metadata(&path) else { return true };
    let Ok(modified) = meta.modified() else { return true };
    SystemTime::now()
        .duration_since(modified)
        .map(|d| d.as_secs() > CHECK_INTERVAL_SECS)
        .unwrap_or(true)
}

fn check_and_write(current: &str) -> Result<()> {
    let root = cache_root()?;
    std::fs::create_dir_all(&root)?;
    // Touch the marker first so we don't hammer GitHub when offline.
    let marker = marker_path()?;
    std::fs::write(&marker, format!("{}", now_secs()))?;

    let latest = fetch_latest_tag()?;
    let payload =
        serde_json::json!({ "current": current, "latest": latest, "checked_at": now_secs() });
    std::fs::write(payload_path()?, serde_json::to_string(&payload)?)?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Fetch the latest GitHub release tag via the public API.
/// Intentionally minimal — no auth, no retries, no proxy support.
fn fetch_latest_tag() -> Result<String> {
    // Use the std blocking facilities only; we're already on a worker thread.
    let url = "https://api.github.com/repos/droidnoob/hew/releases/latest";
    let body = http_get(url)?;
    let parsed: serde_json::Value = serde_json::from_str(&body)?;
    let tag = parsed.get("tag_name").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    if tag.is_empty() {
        return Err(HewError::Io(std::io::Error::other("no tag_name in GitHub response")));
    }
    Ok(tag.trim_start_matches('v').to_string())
}

/// Tiny HTTP GET — uses `curl` if available so we don't pull a full
/// HTTP client into hew-core. Falls back to "feature disabled" if not.
fn http_get(url: &str) -> Result<String> {
    use std::process::Command;
    let curl = which::which("curl").map_err(|_| {
        HewError::Io(std::io::Error::other(
            "curl not found; passive update check disabled (set HEW_NO_UPDATE_CHECK=1 to silence)",
        ))
    })?;
    let out = Command::new(curl)
        .args([
            "-sSL",
            "--max-time",
            "5",
            "-H",
            "User-Agent: hew/0.1",
            "-H",
            "Accept: application/vnd.github+json",
            url,
        ])
        .output()
        .map_err(HewError::Io)?;
    if !out.status.success() {
        return Err(HewError::Io(std::io::Error::other(format!(
            "curl exit {:?}",
            out.status.code()
        ))));
    }
    String::from_utf8(out.stdout).map_err(|e| HewError::Io(std::io::Error::other(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_payload() {
        let s = r#"{"current":"0.1.0","latest":"0.2.0","checked_at":0}"#;
        let n = parse(s).unwrap().expect("notice");
        assert_eq!(n.current, "0.1.0");
        assert_eq!(n.latest, "0.2.0");
    }

    #[test]
    fn equal_versions_yield_no_notice() {
        let s = r#"{"current":"0.1.0","latest":"0.1.0"}"#;
        assert!(parse(s).unwrap().is_none());
    }

    #[test]
    fn empty_payload_returns_none() {
        assert!(parse("").unwrap().is_none());
        assert!(parse("{}").unwrap().is_none());
    }

    #[test]
    fn read_cached_returns_none_when_no_file() {
        let tmp = tempfile::tempdir().unwrap();
        // SAFETY: tests in this module are not parallel against this env.
        unsafe {
            std::env::set_var("HEW_CACHE_DIR", tmp.path());
        }
        let n = read_cached_notice().unwrap();
        assert!(n.is_none());
    }

    #[test]
    fn schedule_is_a_noop_when_disabled() {
        // SAFETY: test-local env mutation.
        unsafe {
            std::env::set_var("HEW_NO_UPDATE_CHECK", "1");
        }
        schedule_if_stale("0.1.0");
        unsafe {
            std::env::remove_var("HEW_NO_UPDATE_CHECK");
        }
        // No panic, no thread output to assert; existence is the contract.
    }
}
