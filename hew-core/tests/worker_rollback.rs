//! Integration test for `hew_core::git::reset_hard_in` — proves the
//! per-worker rollback scope (`git -C <worktree>` semantics) only
//! touches the named worktree, never siblings.
//!
//! Runs against the real `git` binary. Skips silently when git is
//! unavailable (CI image without git, sandboxed environment); on any
//! supported platform the assertion holds.

use std::path::Path;
use std::process::Command;

use hew_core::git::{RealGit, reset_hard_in};

fn git_in(dir: &Path, args: &[&str]) -> String {
    // Scrub any inherited git env from the test parent (the pre-commit
    // hook runs us inside its own git context — `GIT_DIR` / `GIT_INDEX_FILE`
    // leak would point our isolated repo at the wrong files).
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env("GIT_AUTHOR_NAME", "hew-test")
        .env("GIT_AUTHOR_EMAIL", "hew@test.local")
        .env("GIT_COMMITTER_NAME", "hew-test")
        .env("GIT_COMMITTER_EMAIL", "hew@test.local");
    let out = cmd.output().expect("git invocation");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

#[test]
fn worker_rollback_only_resets_own_worktree() {
    if !RealGit::is_available() {
        eprintln!("git not on PATH, skipping");
        return;
    }
    // Scrub inherited git env in-process so the `reset_hard_in` call
    // (which goes through `RealGit` without per-invocation env scrubbing)
    // doesn't pick up `GIT_DIR` from a host pre-commit hook and operate
    // on the wrong repo. Safe here because the integration-test binary
    // only runs this single test.
    for var in [
        "GIT_DIR",
        "GIT_INDEX_FILE",
        "GIT_WORK_TREE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_CONFIG",
        "GIT_CONFIG_GLOBAL",
        "GIT_CONFIG_SYSTEM",
    ] {
        // SAFETY: single-test binary, no parallel test threads observe the env.
        unsafe { std::env::remove_var(var) };
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();

    // Bootstrap a single-commit repo on `main`.
    git_in(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README"), "base\n").unwrap();
    git_in(&repo, &["add", "README"]);
    git_in(&repo, &["commit", "-q", "-m", "base"]);
    let base_sha = git_in(&repo, &["rev-parse", "HEAD"]);

    // Two sibling worktrees, each on its own branch off `main`.
    let wt0 = tmp.path().join("wt0");
    let wt1 = tmp.path().join("wt1");
    git_in(&repo, &["worktree", "add", "-b", "loop/test/w0", wt0.to_str().unwrap(), &base_sha]);
    git_in(&repo, &["worktree", "add", "-b", "loop/test/w1", wt1.to_str().unwrap(), &base_sha]);

    // Each worker commits an iter on top of base.
    std::fs::write(wt0.join("a.txt"), "from worker 0\n").unwrap();
    git_in(&wt0, &["add", "a.txt"]);
    git_in(&wt0, &["commit", "-q", "-m", "iter from w0"]);
    let w0_iter_sha = git_in(&wt0, &["rev-parse", "HEAD"]);

    std::fs::write(wt1.join("b.txt"), "from worker 1\n").unwrap();
    git_in(&wt1, &["add", "b.txt"]);
    git_in(&wt1, &["commit", "-q", "-m", "iter from w1"]);
    let w1_iter_sha_before = git_in(&wt1, &["rev-parse", "HEAD"]);

    assert_ne!(w0_iter_sha, base_sha);
    assert_ne!(w1_iter_sha_before, base_sha);
    assert_ne!(w0_iter_sha, w1_iter_sha_before);

    // Roll back w0 only. w1 must be untouched.
    let git = RealGit::discover().expect("git discovered");
    reset_hard_in(&git, &wt0, &base_sha).expect("reset");

    let w0_after = git_in(&wt0, &["rev-parse", "HEAD"]);
    let w1_after = git_in(&wt1, &["rev-parse", "HEAD"]);

    assert_eq!(w0_after, base_sha, "w0 rolled back to base");
    assert_eq!(w1_after, w1_iter_sha_before, "w1's HEAD must be unchanged by w0's rollback");
    assert!(!wt0.join("a.txt").exists(), "w0's iter file gone after reset --hard");
    assert!(wt1.join("b.txt").exists(), "w1's iter file survives");
}
