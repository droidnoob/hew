//! `hew loop summary` end-to-end coverage for the in-flight states.
//!
//! Plants a run-dir on disk WITHOUT a `run.json` (mirroring the
//! window between dispatcher start and end of iter 1) and asserts the
//! command renders the degraded "in flight" view instead of erroring
//! on `read run.json: No such file or directory`.
//!
//! Task: hew-cn2y.

use std::path::Path;

use assert_cmd::Command as AssertCmd;
use hew_core::loop_log::{IterLog, ensure_worker_dir, iter_log_path, run_dir, write_json_atomic};
use hew_core::runner::{Iter, IterOutcome, TokenSpend};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

fn hew_in(repo: &Path) -> AssertCmd {
    let mut c = AssertCmd::cargo_bin("hew").unwrap();
    c.current_dir(repo);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

#[test]
fn summary_renders_in_flight_view_when_run_dir_is_empty() {
    // Serial path: run-dir exists, no run.json, no iter logs.
    let repo = tempfile::tempdir().unwrap();
    let _ = run_dir(repo.path(), "loop-empty-start").unwrap();

    hew_in(repo.path())
        .args(["loop", "summary", "--run-id", "loop-empty-start"])
        .assert()
        .success()
        .stdout(contains("loop-empty-start").and(contains("in flight")))
        .stdout(contains("No such file").not())
        .stderr(contains("No such file").not());
}

#[test]
fn summary_renders_parallel_in_flight_view_when_worker_dirs_present() {
    let repo = tempfile::tempdir().unwrap();
    let dir = run_dir(repo.path(), "loop-par-inflight").unwrap();
    ensure_worker_dir(&dir, 0).unwrap();
    ensure_worker_dir(&dir, 1).unwrap();

    hew_in(repo.path())
        .args(["loop", "summary", "--run-id", "loop-par-inflight"])
        .assert()
        .success()
        .stdout(contains("across 2 workers"))
        .stdout(contains("worker-0 (running)"))
        .stdout(contains("worker-1 (running)"))
        .stdout(contains("No such file").not());
}

#[test]
fn summary_renders_partial_iters_when_run_json_missing_but_iters_present() {
    let repo = tempfile::tempdir().unwrap();
    let dir = run_dir(repo.path(), "loop-iters-only").unwrap();
    let mut it = Iter::new(1, "2026-05-30T00:00:00Z");
    it.outcome = Some(IterOutcome::Closed);
    it.cost = TokenSpend { input: 200, output: 100, cache_read: 0, cache_create: 0 };
    let log = IterLog::from_iter(&it, None, Vec::new(), Vec::new());
    write_json_atomic(&iter_log_path(&dir, None, 1), &log).unwrap();

    hew_in(repo.path())
        .args(["loop", "summary", "--run-id", "loop-iters-only"])
        .assert()
        .success()
        .stdout(contains("in flight"))
        .stdout(contains("partial:"))
        .stdout(contains("300 token"));
}

#[test]
fn summary_errors_when_run_id_truly_missing() {
    // Genuinely missing run-dir must still surface a clear error — the
    // in-flight path only applies when the dir exists but has no run.json.
    let repo = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(repo.path().join(".hew/loop")).unwrap();

    hew_in(repo.path())
        .args(["loop", "summary", "--run-id", "loop-does-not-exist"])
        .assert()
        .failure()
        .stderr(contains("run-dir not found"));
}
