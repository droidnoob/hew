//! `hew loop run --runtime=<x>` clap-level acceptance / rejection.
//!
//! Pinned to the surface added by hew-g8i: claude + codex parse,
//! anything else is rejected at clap with the valid-values list.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

#[test]
fn loop_runtime_rejects_cursor_at_clap() {
    hew()
        .args(["loop", "run", "--runtime=cursor", "--dry-run", "--max-iter", "1"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("claude").and(contains("codex")));
}

#[test]
fn loop_runtime_rejects_bogus_at_clap() {
    hew()
        .args(["loop", "run", "--runtime=bogus", "--dry-run", "--max-iter", "1"])
        .assert()
        .failure()
        .code(2);
}

/// `--runtime=codex` must pass clap. We don't have bd in the test
/// process's working dir, so the command may still fail downstream
/// (bd discover) — but it must NOT exit via clap (code 2) and must
/// NOT print the legacy "unsupported runtime" guard.
#[test]
fn loop_runtime_codex_passes_clap() {
    let out = hew()
        .args(["loop", "run", "--runtime=codex", "--dry-run", "--max-iter", "1"])
        .assert()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("unsupported runtime"),
        "legacy v1 guard must be gone, stderr was: {stderr}"
    );
    if let Some(code) = out.status.code() {
        assert_ne!(code, 2, "clap usage error: stderr={stderr}");
    }
}
