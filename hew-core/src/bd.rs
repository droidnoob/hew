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

#[derive(Debug, Clone, Deserialize, Serialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, Deserialize, Serialize, Default, schemars::JsonSchema)]
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

    /// Run `bd <args>` and write stdout directly to `out_path`. Use this for
    /// queries whose stdout can exceed the OS pipe buffer (`bd list --json
    /// --limit=0`, `bd prime`). The default impl falls back to `run_raw` for
    /// mocks; production [`RealBd`] overrides with file-descriptor redirection
    /// to dodge the pipe entirely.
    fn run_to_file(&self, args: &[&OsStr], out_path: &std::path::Path) -> Result<()> {
        let out = self.run_raw(args)?;
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(out_path, out.stdout.as_bytes())?;
        Ok(())
    }
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
        // GOTCHA: this read-after-wait pattern deadlocks when stdout exceeds
        // the OS pipe buffer (~16KB macOS, ~64KB Linux). Small commands like
        // `bd remember`, `bd show <id>`, `bd version` are safe; queries that
        // can return large JSON (`bd list`, `bd prime`) must go through
        // `run_to_file` instead.
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

    /// Run `bd <args>`, capture stdout via a `<sys-tmp>/.hew/<label>-…<ext>`
    /// file, read it back, and clean up. Use for queries whose output can
    /// exceed the OS pipe buffer (~16KB macOS / ~64KB Linux).
    fn read_via_temp(&self, args: &[&OsStr], label: &str, ext: &str) -> Result<String> {
        let path = hew_temp_path(label, ext);
        self.run_to_file_inner(args, &path)?;
        let body = std::fs::read_to_string(&path)?;
        // Best-effort cleanup. A leftover file under .hew/ is harmless.
        let _ = std::fs::remove_file(&path);
        Ok(body)
    }

    /// Run `bd <args>` with stdout redirected to `out_path` (created/truncated).
    /// No pipes are involved on stdout, so this is safe for arbitrarily large
    /// outputs (`bd list --json --limit=0`, `bd prime`, etc.).
    fn run_to_file_inner(&self, args: &[&OsStr], out_path: &std::path::Path) -> Result<()> {
        debug!(bd = %self.path.display(), ?args, out = %out_path.display(), "running bd (file)");

        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::File::create(out_path)?;

        let mut cmd = Command::new(&self.path);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::from(file)).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        let status = match child.wait_timeout(self.timeout)? {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HewError::BdNonZero {
                    code: -1,
                    stderr: format!("`bd` timed out after {:?}", self.timeout),
                });
            }
        };

        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            use std::io::Read;
            s.read_to_string(&mut stderr)?;
        }

        if !status.success() {
            return Err(HewError::BdNonZero {
                code: status.code().unwrap_or(-1),
                stderr: if stderr.is_empty() {
                    format!("`bd {}` failed", args_to_display(args))
                } else {
                    stderr
                },
            });
        }
        Ok(())
    }
}

/// `<system tmpdir>/.hew/<label>-<pid>-<nanos>.<ext>` — unique per process
/// and call site. The parent dir is auto-created on first use. Files persist
/// under `.hew/` to aid debugging when a query goes sideways; they are
/// cheap to ignore.
pub(crate) fn hew_temp_path(label: &str, ext: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    std::env::temp_dir().join(".hew").join(format!("{label}-{pid}-{nanos}.{ext}"))
}

fn args_to_display(args: &[&OsStr]) -> String {
    args.iter().map(|a| a.to_string_lossy().into_owned()).collect::<Vec<_>>().join(" ")
}

// `wait_timeout::ChildExt` extends `std::process::Child::wait_timeout`.
use wait_timeout::ChildExt;

impl BdClient for RealBd {
    fn version(&self) -> Result<BdVersion> {
        let out = self.run(&[OsStr::new("--version")])?;
        Ok(parse_version(&out.stdout))
    }

    fn ready(&self) -> Result<Vec<ReadyTask>> {
        // ready output grows with the unblocked-task count; route through
        // a temp file to avoid pipe-buffer deadlocks on big graphs.
        let body =
            self.read_via_temp(&[OsStr::new("ready"), OsStr::new("--json")], "bd-ready", "json")?;
        let parsed: Vec<ReadyTask> = serde_json::from_str(body.trim())?;
        Ok(parsed)
    }

    fn stats(&self) -> Result<StatsSummary> {
        let out = self.run(&[OsStr::new("stats"), OsStr::new("--json")])?;
        let env: StatsEnvelope = serde_json::from_str(out.stdout.trim())?;
        Ok(env.summary)
    }

    fn prime_raw(&self) -> Result<String> {
        // prime output can be tens of KB on mature projects.
        self.read_via_temp(&[OsStr::new("prime")], "bd-prime", "json")
    }

    fn memories(&self) -> Result<std::collections::BTreeMap<String, String>> {
        // Memory store grows over a project's lifetime; route via temp file.
        let body = self.read_via_temp(
            &[OsStr::new("memories"), OsStr::new("--json")],
            "bd-memories",
            "json",
        )?;
        // bd interleaves metadata like `schema_version: 1` with string entries.
        // Decode permissively then keep only string-valued keys.
        let raw: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_str(body.trim())?;
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

    fn run_to_file(&self, args: &[&OsStr], out_path: &std::path::Path) -> Result<()> {
        self.run_to_file_inner(args, out_path)
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
