//! Thin, typed wrapper around the `bd` (Beads) CLI.
//!
//! Design notes:
//! - Always pass arguments as `OsString`; never shell-interpolate.
//! - Always `.stdin(Stdio::null())` — `bd` will otherwise wait on a TTY.
//! - `RealBd` resolves the binary once via `which` and caches the path.
//! - All callers go through `BdClient` so tests can inject a fake.
//! - JSON shapes are decoded permissively (extra fields ignored) so a
//!   newer `bd` does not break us.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::error::{HewError, Result};

/// Soft default timeout for any single `bd` invocation.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BdVersion {
    pub raw: String,
    pub semver: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ReadyTask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub priority: u8,
    #[serde(default)]
    pub status: String,
    #[serde(default, rename = "issue_type")]
    pub issue_type: String,
    #[serde(default)]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct StatsSummary {
    #[serde(default)]
    pub total_issues: u64,
    #[serde(default)]
    pub open_issues: u64,
    #[serde(default)]
    pub closed_issues: u64,
    #[serde(default)]
    pub in_progress_issues: u64,
    #[serde(default)]
    pub blocked_issues: u64,
    #[serde(default)]
    pub ready_issues: u64,
    #[serde(default)]
    pub deferred_issues: u64,
    #[serde(default)]
    pub epics_eligible_for_closure: u64,
}

#[derive(Debug, Deserialize)]
struct StatsEnvelope {
    summary: StatsSummary,
}

/// Outcome of a `bd` invocation.
pub struct BdOutput {
    pub stdout: String,
    pub stderr: String,
}

/// Abstraction over the `bd` binary so tests can drop in a fake.
pub trait BdClient: std::fmt::Debug {
    fn version(&self) -> Result<BdVersion>;
    fn ready(&self) -> Result<Vec<ReadyTask>>;
    fn stats(&self) -> Result<StatsSummary>;
    fn prime_raw(&self) -> Result<String>;
    fn memories(&self) -> Result<std::collections::BTreeMap<String, String>>;
    fn remember(&self, text: &str) -> Result<()>;
    fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput>;
}

/// Default implementation that shells out to the real `bd` binary.
#[derive(Debug, Clone)]
pub struct RealBd {
    path: PathBuf,
    timeout: Duration,
}

impl RealBd {
    /// Resolve `bd` on `PATH` once. Errors with `HewError::BdNotFound` if missing.
    pub fn discover() -> Result<Self> {
        let path = which::which("bd").map_err(|_| HewError::BdNotFound)?;
        Ok(Self { path, timeout: DEFAULT_TIMEOUT })
    }

    /// Explicit path — used by tests that put a stub on PATH.
    pub fn at(path: PathBuf) -> Self {
        Self { path, timeout: DEFAULT_TIMEOUT }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn run(&self, args: &[&OsStr]) -> Result<BdOutput> {
        debug!(bd = %self.path.display(), ?args, "running bd");

        let mut cmd = Command::new(&self.path);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        let status = match child.wait_timeout(self.timeout)? {
            Some(s) => s,
            None => {
                // Soft kill, then hard.
                let _ = child.kill();
                let _ = child.wait();
                return Err(HewError::BdNonZero {
                    code: -1,
                    stderr: format!("`bd` timed out after {:?}", self.timeout),
                });
            }
        };

        // After wait_timeout we still need stdio.
        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut s) = child.stdout.take() {
            use std::io::Read;
            s.read_to_string(&mut stdout)?;
        }
        if let Some(mut s) = child.stderr.take() {
            use std::io::Read;
            s.read_to_string(&mut stderr)?;
        }

        if !status.success() {
            return Err(HewError::BdNonZero {
                code: status.code().unwrap_or(-1),
                stderr: if stderr.is_empty() { stdout.clone() } else { stderr },
            });
        }
        Ok(BdOutput { stdout, stderr })
    }
}

// `wait_timeout::ChildExt` extends `std::process::Child::wait_timeout`.
use wait_timeout::ChildExt;

impl BdClient for RealBd {
    fn version(&self) -> Result<BdVersion> {
        let out = self.run(&[OsStr::new("--version")])?;
        Ok(parse_version(&out.stdout))
    }

    fn ready(&self) -> Result<Vec<ReadyTask>> {
        let out = self.run(&[OsStr::new("ready"), OsStr::new("--json")])?;
        let parsed: Vec<ReadyTask> = serde_json::from_str(out.stdout.trim())?;
        Ok(parsed)
    }

    fn stats(&self) -> Result<StatsSummary> {
        let out = self.run(&[OsStr::new("stats"), OsStr::new("--json")])?;
        let env: StatsEnvelope = serde_json::from_str(out.stdout.trim())?;
        Ok(env.summary)
    }

    fn prime_raw(&self) -> Result<String> {
        let out = self.run(&[OsStr::new("prime")])?;
        Ok(out.stdout)
    }

    fn memories(&self) -> Result<std::collections::BTreeMap<String, String>> {
        let out = self.run(&[OsStr::new("memories"), OsStr::new("--json")])?;
        // bd interleaves metadata like `schema_version: 1` with string entries.
        // Decode permissively then keep only string-valued keys.
        let raw: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_str(out.stdout.trim())?;
        Ok(raw.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect())
    }

    fn remember(&self, text: &str) -> Result<()> {
        let text_os = OsString::from(text);
        self.run(&[OsStr::new("remember"), text_os.as_os_str()])?;
        Ok(())
    }

    fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
        self.run(args)
    }
}

/// Parse `bd version X.Y.Z (sha)` into structured form. Falls back to raw on weirdness.
fn parse_version(raw: &str) -> BdVersion {
    let raw = raw.trim().to_string();
    let semver = raw
        .strip_prefix("bd version ")
        .and_then(|s| s.split_whitespace().next())
        .unwrap_or(&raw)
        .to_string();
    BdVersion { raw, semver }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_version_string() {
        let v = parse_version("bd version 1.0.3 (1b2dd2cb)\n");
        assert_eq!(v.semver, "1.0.3");
        assert!(v.raw.contains("1b2dd2cb"));
    }

    #[test]
    fn parses_version_fallback() {
        let v = parse_version("garbage");
        assert_eq!(v.semver, "garbage");
    }

    #[test]
    fn ready_task_decodes_minimal() {
        let json = r#"[{"id":"x-1","title":"hi"}]"#;
        let v: Vec<ReadyTask> = serde_json::from_str(json).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id, "x-1");
    }

    #[test]
    fn stats_envelope_decodes() {
        let json = r#"{"schema_version":1,"summary":{"total_issues":3,"open_issues":2,"closed_issues":1}}"#;
        let env: StatsEnvelope = serde_json::from_str(json).unwrap();
        assert_eq!(env.summary.total_issues, 3);
        assert_eq!(env.summary.closed_issues, 1);
    }

    #[test]
    fn discover_errors_cleanly_when_bd_missing() {
        // Sanitised PATH so `bd` cannot be found.
        let saved = std::env::var_os("PATH");
        // Safety: tests in this crate are not run in parallel against PATH on purpose;
        // `cargo test` defaults to threaded but we only mutate inside this single test.
        unsafe {
            std::env::set_var("PATH", "");
        }
        let r = RealBd::discover();
        if let Some(p) = saved {
            unsafe {
                std::env::set_var("PATH", p);
            }
        }
        assert!(matches!(r, Err(HewError::BdNotFound)));
    }
}
