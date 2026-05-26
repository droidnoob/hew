//! Integration test for hew-h06: the backpressure gate must roll the
//! worktree back to the pre-iter HEAD on Fail, set the iter's outcome
//! to `BackpressureFail`, and file a `STATUS:loop-iter-failed:` memory.
//!
//! Uses a synthetic git worktree + mock spawner (which advances HEAD
//! mid-iter so the reset has something to undo) + canned failing gate
//! to exercise the rollback path end to end.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use hew_core::backpressure::GateCheck;
use hew_core::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
use hew_core::ctx::{Ctx, OutputMode};
use hew_core::error::Result as HewResult;
use hew_core::loop_log::{IterLog, iter_log_path, run_dir};
use hew_core::prompt::AssembledPrompt;
use hew_core::runner::TokenSpend;
use hew_core::runtime::{RuntimeSpawner, SpawnOutcome};

use hew::commands::loop_cmd::{Args, StaticGateRunner, run_loop_with};

/// Spawner that creates a second commit in `repo_dir` to simulate the
/// agent making changes during the iter, then returns a `Closed`
/// outcome with a fabricated closed_task id.
#[derive(Debug)]
struct CommitMakingSpawner {
    repo_dir: PathBuf,
}

impl RuntimeSpawner for CommitMakingSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _allowed_tools: &[String],
    ) -> hew_core::error::Result<SpawnOutcome> {
        std::fs::write(self.repo_dir.join("iter-marker.txt"), b"iter\n")
            .expect("write iter marker");
        git(&self.repo_dir, &["add", "iter-marker.txt"]);
        git(&self.repo_dir, &["commit", "-m", "iter commit"]);
        Ok(SpawnOutcome {
            success: true,
            closed_task: Some("hew-fake".into()),
            tokens: TokenSpend { input: 10, output: 5, cache_read: 0, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: "closed hew-fake — synthetic\n".into(),
        })
    }
}

#[derive(Debug)]
struct CapturingBd {
    ready: Vec<ReadyTask>,
    remembered: RefCell<Vec<String>>,
}

impl BdClient for CapturingBd {
    fn version(&self) -> HewResult<BdVersion> {
        Ok(BdVersion { raw: "test 1.0.0".into(), semver: "1.0.0".into() })
    }
    fn ready(&self) -> HewResult<Vec<ReadyTask>> {
        Ok(self.ready.clone())
    }
    fn stats(&self) -> HewResult<StatsSummary> {
        Ok(StatsSummary::default())
    }
    fn prime_raw(&self) -> HewResult<String> {
        Ok(String::new())
    }
    fn memories(&self) -> HewResult<BTreeMap<String, String>> {
        Ok(BTreeMap::new())
    }
    fn remember(&self, text: &str) -> HewResult<()> {
        self.remembered.borrow_mut().push(text.to_string());
        Ok(())
    }
    fn run_raw(&self, _: &[&OsStr]) -> HewResult<BdOutput> {
        Ok(BdOutput { stdout: String::new(), stderr: String::new() })
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_AUTHOR_NAME", "test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("run git");
    assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
}

fn head_sha(dir: &Path) -> String {
    let out = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("rev-parse");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn ctx() -> Ctx {
    Ctx::new(true, OutputMode::Text, true, 0)
}

fn args_one_iter() -> Args {
    Args {
        max_iter: Some(1),
        until_empty: false,
        budget_tokens: None,
        budget_wall: None,
        research_budget: hew_core::runner::ResearchBudget { web: 5, fetch: 3 },
        strict: true,
        interactive: false,
        runtime: "claude".into(),
        stop_file: None,
        dry_run: false,
        skill: "hew-execute".into(),
    }
}

#[test]
fn gate_fail_reverts_iter_commits_and_files_status_memory() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);
    let initial_sha = head_sha(&repo);

    let bd = CapturingBd {
        ready: vec![ReadyTask {
            id: "hew-test".into(),
            title: "synthetic ready task".into(),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }],
        remembered: RefCell::new(Vec::new()),
    };
    let spawner = CommitMakingSpawner { repo_dir: repo.clone() };
    let gate = StaticGateRunner(GateCheck {
        tests_passed: true,
        lint_passed: false,
        ..Default::default()
    });

    run_loop_with(&ctx(), args_one_iter(), &bd, Some(&spawner), &gate, &repo).expect("loop runs");

    assert_eq!(head_sha(&repo), initial_sha, "expected HEAD to be rolled back to the pre-iter sha");

    let remembered = bd.remembered.borrow();
    assert!(
        remembered.iter().any(|m| m.starts_with("STATUS:loop-iter-failed:")),
        "expected a STATUS:loop-iter-failed memory, got {remembered:?}",
    );

    let runs_root = repo.join(".hew/loop");
    let run_id = std::fs::read_dir(&runs_root)
        .expect("loop runs dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("loop-"))
        .expect("run-id dir");
    let dir = run_dir(&repo, &run_id).expect("resolve run-dir");
    let iter_log_body =
        std::fs::read_to_string(iter_log_path(&dir, 1)).expect("read iter-001.json");
    let log: IterLog = serde_json::from_str(&iter_log_body).expect("parse iter log");
    assert_eq!(log.outcome.as_deref(), Some("backpressure_fail"));
}

#[test]
fn gate_pass_keeps_iter_commit() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);
    let initial_sha = head_sha(&repo);

    let bd = CapturingBd {
        ready: vec![ReadyTask {
            id: "hew-test".into(),
            title: "synthetic ready task".into(),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }],
        remembered: RefCell::new(Vec::new()),
    };
    let spawner = CommitMakingSpawner { repo_dir: repo.clone() };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    run_loop_with(&ctx(), args_one_iter(), &bd, Some(&spawner), &gate, &repo).expect("loop runs");

    assert_ne!(
        head_sha(&repo),
        initial_sha,
        "expected the iter commit to remain (Pass verdict does not reset)"
    );
    assert!(bd.remembered.borrow().is_empty(), "Pass verdict should not emit memories");
}
