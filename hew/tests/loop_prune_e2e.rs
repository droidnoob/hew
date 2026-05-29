//! `hew loop prune-worktrees` end-to-end. Plants an orphan worktree
//! under an isolated `HOME=<tempdir>/.hew/wt/<run-id>/<n>/`, a matching
//! "completed" `<project>/.hew/loop/<run-id>/run.json`, then invokes the
//! subcommand and asserts the dry-run lists the orphan and `--apply`
//! removes it.
//!
//! Task: hew-kt5q.

use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

use assert_cmd::Command as AssertCmd;
use hew_core::loop_log::{RunLog, run_dir, run_log_path, write_json_atomic};
use hew_core::runner::{Run, RunConfig, StopReason};
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

/// Process-wide lock for HOME mutation; this binary also has other
/// HOME-touching tests so we keep the env swap serial.
static HOME_LOCK: Mutex<()> = Mutex::new(());

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn seed_repo(repo: &Path) {
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(repo, &["add", "README.md"]);
    git(repo, &["commit", "-q", "-m", "seed"]);
}

fn plant_completed_run(project_root: &Path, run_id: &str) {
    let dir = run_dir(project_root, run_id).unwrap();
    let mut run = Run::new(run_id, "2026-05-30T00:00:00Z", RunConfig::default());
    run.stop_reason = Some(StopReason::ReadyEmpty);
    let log = RunLog::from_run(&run);
    write_json_atomic(&run_log_path(&dir, None), &log).unwrap();
}

fn hew_in(repo: &Path, home: &Path) -> AssertCmd {
    let mut c = AssertCmd::cargo_bin("hew").unwrap();
    c.current_dir(repo);
    c.env("HOME", home);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

#[test]
fn prune_subcommand_dry_run_lists_without_removing() {
    if !git_available() {
        eprintln!("git not on PATH, skipping");
        return;
    }
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path());

    // Plant an orphan: completed run + worktree dir on disk under HOME.
    plant_completed_run(repo.path(), "loop-orphan-dry");
    let wt_dir = home.path().join(".hew/wt/loop-orphan-dry/0");
    std::fs::create_dir_all(&wt_dir).unwrap();

    hew_in(repo.path(), home.path())
        .args(["loop", "prune-worktrees"])
        .assert()
        .success()
        .stdout(contains("dry-run").and(contains("loop-orphan-dry")));

    assert!(wt_dir.exists(), "dry-run must NOT remove the worktree dir");
}

#[test]
fn prune_subcommand_apply_removes_orphans_only() {
    if !git_available() {
        eprintln!("git not on PATH, skipping");
        return;
    }
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path());

    // Orphan: run.json stop_reason=ReadyEmpty.
    plant_completed_run(repo.path(), "loop-orphan-apply");
    let orphan_wt = home.path().join(".hew/wt/loop-orphan-apply/0");
    std::fs::create_dir_all(&orphan_wt).unwrap();

    // Live run: run.json with no stop_reason — must survive.
    let live_dir = run_dir(repo.path(), "loop-live").unwrap();
    let live_run = Run::new("loop-live", "2026-05-30T00:00:00Z", RunConfig::default());
    write_json_atomic(&run_log_path(&live_dir, None), &RunLog::from_run(&live_run)).unwrap();
    let live_wt = home.path().join(".hew/wt/loop-live/0");
    std::fs::create_dir_all(&live_wt).unwrap();

    hew_in(repo.path(), home.path())
        .args(["loop", "prune-worktrees", "--apply"])
        .assert()
        .success()
        .stdout(contains("pruned 1"));

    assert!(!orphan_wt.exists(), "orphan worktree must be removed");
    assert!(live_wt.exists(), "live run's worktree must survive");
}

#[test]
fn prune_subcommand_reports_when_nothing_to_do() {
    if !git_available() {
        eprintln!("git not on PATH, skipping");
        return;
    }
    let _g = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());

    let home = tempfile::tempdir().unwrap();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path());

    hew_in(repo.path(), home.path())
        .args(["loop", "prune-worktrees"])
        .assert()
        .success()
        .stdout(contains("no orphan worktrees"));
}
