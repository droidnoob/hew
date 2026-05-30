//! End-of-run test verification for `hew loop`.
//!
//! The verify step runs once after the last iter (and after merge-back
//! on `--jobs >= 2`) and before the final `run.json` write. It proves
//! the *final stacked state* compiles + passes its declared tests so
//! merge-back / PR creation isn't shipping a green-by-construction
//! pipeline that breaks in CI.
//!
//! Conditional on **both**:
//!   1. A test command resolves (CLI override > config override >
//!      [`crate::gate::detect`] auto-detect of project-authored
//!      signals).
//!   2. The user opted in via `loop.end_of_run.verify_tests = true`
//!      or `hew loop run --verify-tests`.
//!
//! Failure surfaces in `hew loop summary` and writes a
//! `STATUS:loop-verify-failed:<run-id>` memory; it does **not** unwind
//! closed tasks. Per `DECISION:loop-parallel-overlap-policy`,
//! conflicts on merge-back already file `[merge-conflict]` bugs;
//! verify-tests is the next-layer safety net for "final state is
//! actually green".

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use wait_timeout::ChildExt;

use crate::gate::GateSpec;
use crate::process::spawn_with_etxtbsy_retry;

/// Outcome of one verify-tests invocation. Persisted in `run.json` as
/// `Run.verify_outcome` and re-rendered by `hew loop summary`.
///
/// Stays a small enum so adding outcomes later (e.g. `Cancelled` if
/// the user ctrl-Cs the verify step itself) is a non-breaking append.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VerifyOutcome {
    /// Command exited 0. `command` is the rendered argv for the
    /// summary line; `duration_secs` is wall-clock spent.
    Passed { command: String, duration_secs: u64 },
    /// Command exited non-zero. `stderr_tail` is the last ~2 KiB of
    /// merged stdout/stderr for the failure breadcrumb.
    Failed { command: String, exit_code: i32, duration_secs: u64, stderr_tail: String },
    /// Verify was opt-in-true but no command resolved (no CLI override,
    /// no config override, `gate::detect` returned empty test_cmd).
    /// Also covers the `verify_tests = false` path so a single field
    /// captures every non-run case.
    Skipped { reason: String },
    /// Wall-clock budget elapsed before the command finished. The
    /// child was killed.
    TimedOut { command: String, budget_secs: u64 },
}

impl VerifyOutcome {
    /// True iff the run should exit non-zero on this outcome. Wrapper
    /// scripts / CI branches on this.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::TimedOut { .. })
    }

    /// One-line label for `hew loop summary` output.
    pub fn summary_line(&self) -> String {
        match self {
            Self::Passed { command, duration_secs } => {
                format!("passed ({}s, {})", duration_secs, command)
            }
            Self::Failed { command, exit_code, duration_secs, .. } => {
                format!("failed (exit {}, {}s, {})", exit_code, duration_secs, command)
            }
            Self::Skipped { reason } => format!("skipped ({})", reason),
            Self::TimedOut { command, budget_secs } => {
                format!("timed out (> {}s, {})", budget_secs, command)
            }
        }
    }
}

/// Resolve the verify command for this run. Precedence:
///
/// 1. `cli_override` — `--verify-command="..."` on `hew loop run`.
/// 2. `config_override` — `loop.end_of_run.verify_command` in hew config.
/// 3. `gate.test_cmd` — project-authored signals (`justfile`,
///    `Makefile`, `package.json`). Already detected by the caller; we
///    don't re-walk the filesystem.
///
/// Returns `None` when nothing resolves — caller skips with a
/// `no_command_resolved` reason.
pub fn resolve_command(
    cli_override: Option<&str>,
    config_override: Option<&str>,
    gate: &GateSpec,
) -> Option<Vec<String>> {
    if let Some(raw) = cli_override.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(split_command(raw));
    }
    if let Some(raw) = config_override.map(str::trim).filter(|s| !s.is_empty()) {
        return Some(split_command(raw));
    }
    if !gate.test_cmd.is_empty() {
        return Some(gate.test_cmd.clone());
    }
    None
}

/// Whitespace-split a user-supplied command string. We deliberately
/// avoid shell-style quoting parsing here — operators wanting that
/// shape pass the command through their shell instead. Mirrors
/// `gate::detect`'s naive vector shape.
fn split_command(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(str::to_string).collect()
}

/// Spawn the resolved verify command under `budget` and capture its
/// output. The command runs in `working_dir` (the project root or, on
/// parallel runs, the launch HEAD — caller decides). Combined
/// stdout+stderr is written byte-for-byte to `log_path` and the last
/// ~2 KiB of stderr returned in [`VerifyOutcome::Failed`].
///
/// Pure-ish: only side effects are subprocess spawn + writing the
/// log file. Callers persist [`VerifyOutcome`] into `run.json`.
pub fn run_verify(
    command: &[String],
    working_dir: &Path,
    log_path: &Path,
    budget: Duration,
) -> VerifyOutcome {
    let rendered = command.join(" ");
    if command.is_empty() {
        return VerifyOutcome::Skipped { reason: "empty command".into() };
    }
    let program = &command[0];
    let args: Vec<&OsStr> = command[1..].iter().map(OsStr::new).collect();

    let mut cmd = Command::new(program);
    cmd.args(&args)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let start = Instant::now();
    let mut child = match spawn_with_etxtbsy_retry(&mut cmd) {
        Ok(c) => c,
        Err(e) => {
            return VerifyOutcome::Failed {
                command: rendered,
                exit_code: -1,
                duration_secs: 0,
                stderr_tail: format!("spawn failed: {e}"),
            };
        }
    };

    let status = match child.wait_timeout(budget) {
        Ok(Some(s)) => s,
        Ok(None) => {
            let _ = child.kill();
            let _ = child.wait();
            return VerifyOutcome::TimedOut { command: rendered, budget_secs: budget.as_secs() };
        }
        Err(e) => {
            return VerifyOutcome::Failed {
                command: rendered,
                exit_code: -1,
                duration_secs: start.elapsed().as_secs(),
                stderr_tail: format!("wait failed: {e}"),
            };
        }
    };

    let duration_secs = start.elapsed().as_secs();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(mut s) = child.stdout.take() {
        use std::io::Read;
        let _ = s.read_to_end(&mut stdout);
    }
    if let Some(mut s) = child.stderr.take() {
        use std::io::Read;
        let _ = s.read_to_end(&mut stderr);
    }

    // Best-effort log write. A failure here is not load-bearing — the
    // outcome record is the durable signal.
    let mut combined = Vec::with_capacity(stdout.len() + stderr.len() + 16);
    combined.extend_from_slice(b"=== stdout ===\n");
    combined.extend_from_slice(&stdout);
    combined.extend_from_slice(b"\n=== stderr ===\n");
    combined.extend_from_slice(&stderr);
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(log_path, &combined);

    if status.success() {
        VerifyOutcome::Passed { command: rendered, duration_secs }
    } else {
        VerifyOutcome::Failed {
            command: rendered,
            exit_code: status.code().unwrap_or(-1),
            duration_secs,
            stderr_tail: tail_bytes(&stderr, 2048),
        }
    }
}

fn tail_bytes(bytes: &[u8], cap: usize) -> String {
    if bytes.len() <= cap {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    String::from_utf8_lossy(&bytes[bytes.len() - cap..]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gate_with(cmd: &[&str]) -> GateSpec {
        GateSpec { test_cmd: cmd.iter().map(|s| s.to_string()).collect(), lint_cmd: Vec::new() }
    }

    #[test]
    fn resolve_prefers_cli_over_config_over_gate() {
        let gate = gate_with(&["just", "test"]);
        let r = resolve_command(Some("cargo test"), Some("make test"), &gate);
        assert_eq!(r, Some(vec!["cargo".into(), "test".into()]));
    }

    #[test]
    fn resolve_falls_through_to_config_when_cli_empty() {
        let gate = gate_with(&["just", "test"]);
        let r = resolve_command(Some(""), Some("make test"), &gate);
        assert_eq!(r, Some(vec!["make".into(), "test".into()]));
    }

    #[test]
    fn resolve_falls_through_to_gate_when_overrides_absent() {
        let gate = gate_with(&["just", "test"]);
        let r = resolve_command(None, None, &gate);
        assert_eq!(r, Some(vec!["just".into(), "test".into()]));
    }

    #[test]
    fn resolve_returns_none_when_nothing_set() {
        let gate = GateSpec::default();
        assert!(resolve_command(None, None, &gate).is_none());
    }

    #[test]
    fn resolve_trims_whitespace_only_strings_as_empty() {
        let gate = gate_with(&["just", "test"]);
        let r = resolve_command(Some("   "), None, &gate);
        assert_eq!(r, Some(vec!["just".into(), "test".into()]));
    }

    #[cfg(unix)]
    #[test]
    fn run_verify_passed_records_command_and_duration() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("verify.log");
        let out = run_verify(&["true".into()], tmp.path(), &log, Duration::from_secs(5));
        match out {
            VerifyOutcome::Passed { command, .. } => assert_eq!(command, "true"),
            other => panic!("expected Passed, got {other:?}"),
        }
        assert!(log.exists(), "log file should be written");
    }

    #[cfg(unix)]
    #[test]
    fn run_verify_failed_captures_exit_code_and_stderr_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("verify.log");
        let out = run_verify(
            &["sh".into(), "-c".into(), "echo boom 1>&2; exit 3".into()],
            tmp.path(),
            &log,
            Duration::from_secs(5),
        );
        match out {
            VerifyOutcome::Failed { exit_code, stderr_tail, .. } => {
                assert_eq!(exit_code, 3);
                assert!(stderr_tail.contains("boom"), "stderr tail: {stderr_tail}");
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn run_verify_timeout_kills_child_and_reports_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("verify.log");
        let out = run_verify(
            &["sh".into(), "-c".into(), "sleep 5".into()],
            tmp.path(),
            &log,
            Duration::from_millis(200),
        );
        match out {
            VerifyOutcome::TimedOut { budget_secs, .. } => assert_eq!(budget_secs, 0),
            other => panic!("expected TimedOut, got {other:?}"),
        }
    }

    #[test]
    fn run_verify_empty_command_skips() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("verify.log");
        let out = run_verify(&[], tmp.path(), &log, Duration::from_secs(1));
        assert!(matches!(out, VerifyOutcome::Skipped { .. }));
    }

    #[test]
    fn run_verify_spawn_failure_returns_failed_with_negative_exit_code() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("verify.log");
        let out = run_verify(
            &["this-binary-does-not-exist-xyz".into()],
            tmp.path(),
            &log,
            Duration::from_secs(1),
        );
        match out {
            VerifyOutcome::Failed { exit_code, stderr_tail, .. } => {
                assert_eq!(exit_code, -1);
                assert!(stderr_tail.contains("spawn failed"));
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn verify_outcome_serde_round_trip_passed() {
        let v = VerifyOutcome::Passed { command: "cargo test".into(), duration_secs: 22 };
        let s = serde_json::to_string(&v).unwrap();
        let back: VerifyOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn verify_outcome_serde_round_trip_failed_has_tail() {
        let v = VerifyOutcome::Failed {
            command: "cargo test".into(),
            exit_code: 101,
            duration_secs: 5,
            stderr_tail: "thread 'main' panicked".into(),
        };
        let s = serde_json::to_string(&v).unwrap();
        let back: VerifyOutcome = serde_json::from_str(&s).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn verify_outcome_is_failure_classifies_correctly() {
        assert!(!VerifyOutcome::Passed { command: "x".into(), duration_secs: 1 }.is_failure());
        assert!(!VerifyOutcome::Skipped { reason: "off".into() }.is_failure());
        assert!(
            VerifyOutcome::Failed {
                command: "x".into(),
                exit_code: 1,
                duration_secs: 0,
                stderr_tail: String::new(),
            }
            .is_failure()
        );
        assert!(VerifyOutcome::TimedOut { command: "x".into(), budget_secs: 1 }.is_failure());
    }

    #[test]
    fn summary_line_renders_each_variant() {
        let p = VerifyOutcome::Passed { command: "cargo test".into(), duration_secs: 22 };
        assert!(p.summary_line().contains("passed"));
        assert!(p.summary_line().contains("cargo test"));
        let f = VerifyOutcome::Failed {
            command: "cargo test".into(),
            exit_code: 1,
            duration_secs: 5,
            stderr_tail: String::new(),
        };
        assert!(f.summary_line().contains("failed"));
        let s = VerifyOutcome::Skipped { reason: "no command".into() };
        assert!(s.summary_line().contains("skipped"));
        let t = VerifyOutcome::TimedOut { command: "x".into(), budget_secs: 600 };
        assert!(t.summary_line().contains("timed out"));
    }

    #[test]
    fn tail_bytes_under_cap_returns_all() {
        assert_eq!(tail_bytes(b"hello", 100), "hello");
    }

    #[test]
    fn tail_bytes_over_cap_returns_suffix() {
        let b = b"abcdefghij";
        assert_eq!(tail_bytes(b, 4), "ghij");
    }
}
