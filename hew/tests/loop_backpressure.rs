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
use hew_core::runtime::{RuntimeSpawner, SpawnFailureClass, SpawnOpts, SpawnOutcome};

use hew::commands::loop_cmd::{
    Args, GateRunner, StaticGateRunner, Worker, run_loop_with, run_worker_loop,
};
use hew_core::config::LoopModelConfig;
use hew_core::runtime::FallbackConfig;

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
        _opts: &SpawnOpts,
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
            failure_class: SpawnFailureClass::Success,
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
        strict: true,
        interactive: false,
        unattended: false,
        runtime: "claude".into(),
        stop_file: None,
        dry_run: false,
        skill: "hew-execute".into(),
        fallback_runtime: None,
        fallback_cooldown_iters: None,
        jobs: 1,
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

    run_loop_with(
        &ctx(),
        args_one_iter(),
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

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
        std::fs::read_to_string(iter_log_path(&dir, None, 1)).expect("read iter-001.json");
    let log: IterLog = serde_json::from_str(&iter_log_body).expect("parse iter log");
    assert_eq!(log.outcome.as_deref(), Some("backpressure_fail"));
}

/// Spawner used by `unattended_resolves_deferred_via_memory_lookup`:
/// during spawn, writes a `DEFERRED:<topic>` memory through the shared
/// bd handle. No commits made (no rollback needed for this test).
#[derive(Debug)]
struct DeferredWritingSpawner {
    bd: std::sync::Arc<SharedMemoriesBd>,
    topic: String,
}

impl RuntimeSpawner for DeferredWritingSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _allowed_tools: &[String],
        _opts: &SpawnOpts,
    ) -> hew_core::error::Result<SpawnOutcome> {
        self.bd
            .remember(&format!("DEFERRED:{} — agent is unsure", self.topic))
            .expect("remember DEFERRED");
        Ok(SpawnOutcome {
            success: true,
            closed_task: Some("hew-fake".into()),
            tokens: TokenSpend::default(),
            stderr_tail: String::new(),
            raw_text: "closed hew-fake — synthetic\n".into(),
            failure_class: SpawnFailureClass::Success,
        })
    }
}

/// Mock bd with a shared (Arc) interior so a spawner thread can also
/// write memories during the iter. Returns one ready task + a seeded
/// DECISION memory the resolver should find via `memory_lookup`.
#[derive(Debug)]
struct SharedMemoriesBd {
    ready: Vec<ReadyTask>,
    memories: std::sync::Mutex<BTreeMap<String, String>>,
}

impl SharedMemoriesBd {
    fn with_seed(seed: BTreeMap<String, String>, ready: Vec<ReadyTask>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self { ready, memories: std::sync::Mutex::new(seed) })
    }
}

impl BdClient for SharedMemoriesBd {
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
        Ok(self.memories.lock().unwrap().clone())
    }
    fn remember(&self, text: &str) -> HewResult<()> {
        let mut mems = self.memories.lock().unwrap();
        let id = format!("auto-{}", mems.len());
        mems.insert(id, text.to_string());
        Ok(())
    }
    fn run_raw(&self, _: &[&OsStr]) -> HewResult<BdOutput> {
        Ok(BdOutput { stdout: String::new(), stderr: String::new() })
    }
}

#[test]
fn unattended_resolves_deferred_via_memory_lookup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    // Seed bd with a DECISION memory whose body mentions the topic.
    // The resolver should find this via BdDecisionContext::memory_lookup.
    let mut seed = BTreeMap::new();
    seed.insert(
        "seed-1".to_string(),
        "DECISION:auth-strategy — use OAuth2 PKCE per RFC 7636".to_string(),
    );
    let bd = SharedMemoriesBd::with_seed(
        seed,
        vec![ReadyTask {
            id: "hew-test".into(),
            title: "synthetic ready task".into(),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }],
    );
    let spawner = DeferredWritingSpawner { bd: bd.clone(), topic: "auth-strategy".into() };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let mut args = args_one_iter();
    args.unattended = true;

    run_loop_with(
        &ctx(),
        args,
        &*bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    let final_mems = bd.memories().unwrap();
    let new_decisions: Vec<&String> = final_mems
        .iter()
        .filter(|(k, v)| k.as_str() != "seed-1" && v.starts_with("DECISION:auth-strategy"))
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        new_decisions.len(),
        1,
        "expected exactly one new DECISION:auth-strategy memory, got memories={final_mems:?}",
    );
    assert!(
        new_decisions[0].contains("seed-1"),
        "decision body should cite the prior memory id: {}",
        new_decisions[0],
    );

    // The DEFERRED the spawner wrote should still be in memory (we
    // never delete on Decided — we just add a DECISION alongside).
    assert!(
        final_mems.values().any(|v| v.starts_with("DEFERRED:auth-strategy")),
        "DEFERRED memory should remain alongside the new DECISION",
    );

    // iter-001.json should record the topic in `decisions`.
    let runs_root = repo.join(".hew/loop");
    let run_id = std::fs::read_dir(&runs_root)
        .expect("loop runs dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("loop-"))
        .expect("run-id dir");
    let dir = run_dir(&repo, &run_id).expect("resolve run-dir");
    let log: IterLog = serde_json::from_str(
        &std::fs::read_to_string(iter_log_path(&dir, None, 1)).expect("read iter-001.json"),
    )
    .expect("parse iter log");
    assert_eq!(log.decisions, vec!["auth-strategy".to_string()]);
}

#[test]
fn without_unattended_deferred_is_left_alone() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let mut seed = BTreeMap::new();
    seed.insert(
        "seed-1".to_string(),
        "DECISION:auth-strategy — use OAuth2 PKCE per RFC 7636".to_string(),
    );
    let bd = SharedMemoriesBd::with_seed(
        seed,
        vec![ReadyTask {
            id: "hew-test".into(),
            title: "synthetic ready task".into(),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }],
    );
    let spawner = DeferredWritingSpawner { bd: bd.clone(), topic: "auth-strategy".into() };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    // unattended = false (default).
    run_loop_with(
        &ctx(),
        args_one_iter(),
        &*bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    let final_mems = bd.memories().unwrap();
    let new_decisions = final_mems.values().filter(|v| v.starts_with("DECISION:")).count();
    // Only the seed DECISION remains; no resolver pass happened.
    assert_eq!(
        new_decisions, 1,
        "without --unattended, no new DECISION memory should be filed, got: {final_mems:?}",
    );
    assert!(
        final_mems.values().any(|v| v.starts_with("DEFERRED:auth-strategy")),
        "DEFERRED memory should remain (operator review)",
    );
}

/// Bd whose ready set is mutable. Used to simulate `hew task close`
/// happening inside the spawn — the task disappears from `ready()`
/// after the spawner mutates the inner Vec.
#[derive(Debug)]
struct MutableReadyBd {
    ready: std::sync::Mutex<Vec<ReadyTask>>,
    remembered: std::sync::Mutex<Vec<String>>,
}

impl MutableReadyBd {
    fn with(ready: Vec<ReadyTask>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            ready: std::sync::Mutex::new(ready),
            remembered: std::sync::Mutex::new(Vec::new()),
        })
    }
    fn remove_ready(&self, id: &str) {
        self.ready.lock().unwrap().retain(|t| t.id != id);
    }
}

impl BdClient for MutableReadyBd {
    fn version(&self) -> HewResult<BdVersion> {
        Ok(BdVersion { raw: "test 1.0.0".into(), semver: "1.0.0".into() })
    }
    fn ready(&self) -> HewResult<Vec<ReadyTask>> {
        Ok(self.ready.lock().unwrap().clone())
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
        self.remembered.lock().unwrap().push(text.to_string());
        Ok(())
    }
    fn run_raw(&self, _: &[&OsStr]) -> HewResult<BdOutput> {
        Ok(BdOutput { stdout: String::new(), stderr: String::new() })
    }
}

/// Spawner that closes the primed task via the shared bd handle and
/// returns a SpawnOutcome WITHOUT the literal `closed <id>` marker in
/// either `closed_task` or `raw_text`. Exercises the out-of-band
/// closure detection (hew-7tp).
#[derive(Debug)]
struct SilentlyClosingSpawner {
    bd: std::sync::Arc<MutableReadyBd>,
    task_id: String,
}

impl RuntimeSpawner for SilentlyClosingSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _allowed_tools: &[String],
        _opts: &SpawnOpts,
    ) -> hew_core::error::Result<SpawnOutcome> {
        self.bd.remove_ready(&self.task_id);
        Ok(SpawnOutcome {
            success: true,
            closed_task: None,
            tokens: TokenSpend { input: 5, output: 3, cache_read: 0, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: "Done; added the struct and tests.".into(),
            failure_class: SpawnFailureClass::Success,
        })
    }
}

/// Spawner that closes whichever ready task is at the front of the
/// queue. Used to drive a multi-iter loop where each iter promotes
/// the next task.
#[derive(Debug)]
struct DrainingSpawner {
    bd: std::sync::Arc<MutableReadyBd>,
}

impl RuntimeSpawner for DrainingSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _allowed_tools: &[String],
        _opts: &SpawnOpts,
    ) -> hew_core::error::Result<SpawnOutcome> {
        let head = self.bd.ready.lock().unwrap().first().map(|t| t.id.clone());
        if let Some(id) = head {
            self.bd.remove_ready(&id);
        }
        Ok(SpawnOutcome {
            success: true,
            closed_task: None,
            tokens: TokenSpend { input: 1, output: 1, cache_read: 0, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: "done".into(),
            failure_class: SpawnFailureClass::Success,
        })
    }
}

#[test]
fn prompt_prefix_hash_is_stable_across_iters() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let bd = MutableReadyBd::with(vec![
        ReadyTask {
            id: "hew-one".into(),
            title: "first".into(),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        },
        ReadyTask {
            id: "hew-two".into(),
            title: "second".into(),
            description: String::new(),
            priority: 2,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        },
    ]);
    let spawner = DrainingSpawner { bd: bd.clone() };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let mut args = args_one_iter();
    args.max_iter = Some(2);

    run_loop_with(
        &ctx(),
        args,
        &*bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    let runs_root = repo.join(".hew/loop");
    let run_id = std::fs::read_dir(&runs_root)
        .expect("loop runs dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("loop-"))
        .expect("run-id dir");
    let dir = run_dir(&repo, &run_id).expect("resolve run-dir");
    let log1: IterLog = serde_json::from_str(
        &std::fs::read_to_string(iter_log_path(&dir, None, 1)).expect("read iter-001.json"),
    )
    .expect("parse");
    let log2: IterLog = serde_json::from_str(
        &std::fs::read_to_string(iter_log_path(&dir, None, 2)).expect("read iter-002.json"),
    )
    .expect("parse");
    let h1 = log1.prompt_prefix_hash.as_deref().expect("iter-001 has prefix hash");
    let h2 = log2.prompt_prefix_hash.as_deref().expect("iter-002 has prefix hash");
    assert_eq!(
        h1, h2,
        "prefix_hash must be byte-stable across iters — got {h1} vs {h2}; per-iter content leaked into the cacheable prefix",
    );
}

#[test]
fn out_of_band_closure_promotes_no_close_to_closed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let task_id = "hew-silent".to_string();
    let bd = MutableReadyBd::with(vec![ReadyTask {
        id: task_id.clone(),
        title: "silent close".into(),
        description: String::new(),
        priority: 1,
        status: "open".into(),
        issue_type: "task".into(),
        parent: None,
    }]);
    let spawner = SilentlyClosingSpawner { bd: bd.clone(), task_id: task_id.clone() };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    run_loop_with(
        &ctx(),
        args_one_iter(),
        &*bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    let runs_root = repo.join(".hew/loop");
    let run_id = std::fs::read_dir(&runs_root)
        .expect("loop runs dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("loop-"))
        .expect("run-id dir");
    let dir = run_dir(&repo, &run_id).expect("resolve run-dir");
    let log: IterLog = serde_json::from_str(
        &std::fs::read_to_string(iter_log_path(&dir, None, 1)).expect("read iter-001.json"),
    )
    .expect("parse iter log");
    assert_eq!(
        log.outcome.as_deref(),
        Some("closed"),
        "expected NoClose to be promoted to Closed after the task left the ready set",
    );
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

    run_loop_with(
        &ctx(),
        args_one_iter(),
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    assert_ne!(
        head_sha(&repo),
        initial_sha,
        "expected the iter commit to remain (Pass verdict does not reset)"
    );
    assert!(bd.remembered.borrow().is_empty(), "Pass verdict should not emit memories");
}

/// Spawner whose outcome is scripted per-call. Each `spawn` consumes
/// the next entry; if the script is exhausted the last entry is
/// reused. Used by the cooldown integration test to flip a mock
/// primary from `RuntimeError` → `Success` without touching git.
#[derive(Debug)]
struct ScriptedSpawner {
    label: &'static str,
    outcomes: RefCell<Vec<SpawnOutcome>>,
    fallback_default: SpawnOutcome,
    calls: RefCell<u32>,
}

impl ScriptedSpawner {
    fn new(
        label: &'static str,
        outcomes: Vec<SpawnOutcome>,
        fallback_default: SpawnOutcome,
    ) -> Self {
        Self { label, outcomes: RefCell::new(outcomes), fallback_default, calls: RefCell::new(0) }
    }
}

impl RuntimeSpawner for ScriptedSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _allowed_tools: &[String],
        _opts: &SpawnOpts,
    ) -> hew_core::error::Result<SpawnOutcome> {
        *self.calls.borrow_mut() += 1;
        let mut q = self.outcomes.borrow_mut();
        let out = if q.is_empty() { self.fallback_default.clone() } else { q.remove(0) };
        eprintln!(
            "scripted spawner `{}` call → success={} class={:?}",
            self.label, out.success, out.failure_class
        );
        Ok(out)
    }
}

fn ok_outcome(closed: Option<&str>) -> SpawnOutcome {
    SpawnOutcome {
        success: true,
        closed_task: closed.map(|s| s.to_string()),
        tokens: TokenSpend::default(),
        stderr_tail: String::new(),
        raw_text: closed.map(|s| format!("closed {s} — synthetic\n")).unwrap_or_default(),
        failure_class: SpawnFailureClass::Success,
    }
}

fn rate_limit_outcome() -> SpawnOutcome {
    SpawnOutcome {
        success: false,
        closed_task: None,
        tokens: TokenSpend::default(),
        stderr_tail: "synthetic 429\n".into(),
        raw_text: String::new(),
        failure_class: SpawnFailureClass::RuntimeError(
            hew_core::runtime::RuntimeErrorKind::RateLimit,
        ),
    }
}

/// Cooldown end-to-end: a primary that errors with rate-limit on its
/// first call (then would succeed) routes to the fallback for
/// `cooldown_iters=3` iters before retrying the primary. Asserts the
/// per-iter `runtime_used` + `cooldown_engaged` log fields trace the
/// expected sequence.
#[test]
fn cooldown_routes_to_fallback_for_n_iters_then_retries_primary() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    // No git repo needed — every iter is non-erroring on the fallback
    // path (the gate is a static pass and the spawner never touches
    // the worktree). git_head_sha will fail and log a breadcrumb;
    // that's the expected non-git tolerance branch.

    let primary = ScriptedSpawner::new(
        "primary",
        vec![rate_limit_outcome()],
        ok_outcome(Some("hew-primary-success")),
    );
    let fallback_spawner =
        ScriptedSpawner::new("fallback", Vec::new(), ok_outcome(Some("hew-fallback-iter")));

    // Five identical ready tasks; bd.ready() returns the head each
    // iter and out-of-band detection promotes NoClose→Closed when
    // the task drops from the ready set. Because nothing actually
    // closes tasks here, every iter logs as `no_close` — that's fine
    // for the cooldown-routing assertions below.
    let ready: Vec<ReadyTask> = (0..5)
        .map(|i| ReadyTask {
            id: format!("hew-r{i}"),
            title: format!("synthetic ready {i}"),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        })
        .collect();
    let bd = CapturingBd { ready, remembered: RefCell::new(Vec::new()) };

    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let args = Args {
        max_iter: Some(5),
        until_empty: false,
        budget_tokens: None,
        budget_wall: None,
        strict: true,
        interactive: false,
        unattended: false,
        runtime: "claude".into(),
        stop_file: None,
        dry_run: false,
        skill: "hew-execute".into(),
        fallback_runtime: Some("codex".into()),
        fallback_cooldown_iters: Some(3),
        jobs: 1,
    };
    let fallback_cfg =
        FallbackConfig { runtime: Some(hew_core::runtime::RuntimeKind::Codex), cooldown_iters: 3 };

    run_loop_with(
        &ctx(),
        args,
        &bd,
        Some(&primary),
        Some(&fallback_spawner),
        fallback_cfg,
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    // Find the run dir.
    let runs_root = repo.join(".hew/loop");
    let run_id = std::fs::read_dir(&runs_root)
        .expect("loop runs dir")
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|n| n.starts_with("loop-"))
        .expect("run-id dir");
    let dir = run_dir(&repo, &run_id).expect("resolve run-dir");

    let load = |n: u32| -> IterLog {
        let body = std::fs::read_to_string(iter_log_path(&dir, None, n)).expect("read iter log");
        serde_json::from_str(&body).expect("parse iter log")
    };
    let iters: Vec<IterLog> = (1..=5).map(load).collect();

    // iter 1: primary, errors, cooldown engages.
    assert_eq!(iters[0].runtime_used.as_deref(), Some("claude"));
    assert!(iters[0].cooldown_engaged, "primary error should engage cooldown");
    assert_eq!(iters[0].outcome.as_deref(), Some("runtime_error"));

    // iters 2..=4: fallback drains the window.
    for (idx, log) in iters[1..=3].iter().enumerate() {
        assert_eq!(
            log.runtime_used.as_deref(),
            Some("codex"),
            "iter {} should route to fallback",
            idx + 2,
        );
        assert!(log.cooldown_engaged, "iter {} cooldown should still be engaged", idx + 2);
    }

    // iter 5: primary retry — cooldown exits on success.
    assert_eq!(iters[4].runtime_used.as_deref(), Some("claude"));
    assert!(!iters[4].cooldown_engaged, "successful primary retry should exit cooldown");

    // Sanity: spawner call counts. Primary should be invoked exactly
    // twice (iter 1 + iter 5 retry); fallback exactly three times
    // (iters 2, 3, 4).
    assert_eq!(*primary.calls.borrow(), 2, "primary call count");
    assert_eq!(*fallback_spawner.calls.borrow(), 3, "fallback call count");
}

/// Gate runner that captures every `working_dir` it is invoked with.
/// Used to assert per-worker backpressure scoping for hew-j4x.
#[derive(Debug, Default)]
struct RecordingGateRunner {
    calls: std::sync::Mutex<Vec<PathBuf>>,
    check: GateCheck,
}

impl RecordingGateRunner {
    fn passing() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            check: GateCheck { tests_passed: true, lint_passed: true, ..Default::default() },
        }
    }
}

impl GateRunner for RecordingGateRunner {
    fn run_gate(&self, working_dir: &Path) -> GateCheck {
        self.calls.lock().unwrap().push(working_dir.to_path_buf());
        self.check.clone()
    }
}

/// hew-j4x: `run_worker_loop` must invoke the backpressure gate against
/// `worker.worktree_dir`, not the dispatcher's ambient project root.
/// A future per-worker dispatcher (hew-9m5) trusts this contract to keep
/// parallel workers' test/lint runs scoped to disjoint worktrees.
#[test]
fn gate_is_called_with_worker_worktree_dir() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path().join("wt");
    let log_dir = tmp.path().join("logs");
    std::fs::create_dir_all(&worktree).expect("mkdir worktree");
    std::fs::create_dir_all(&log_dir).expect("mkdir logs");

    git(&worktree, &["init", "-q", "-b", "main"]);
    std::fs::write(worktree.join("README.md"), b"seed\n").unwrap();
    git(&worktree, &["add", "README.md"]);
    git(&worktree, &["commit", "-q", "-m", "seed"]);

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
    let spawner = CommitMakingSpawner { repo_dir: worktree.clone() };
    let gate = RecordingGateRunner::passing();

    let args = args_one_iter();
    let skill = hew_core::skills::find("hew-execute").expect("hew-execute skill present");
    let allowed = hew_core::allowed_tools::for_skill("hew-execute");
    let worker = Worker {
        id: 0,
        worktree_dir: worktree.clone(),
        branch: "loop/test/w0".into(),
        worker_n: None,
        log_dir: log_dir.clone(),
    };
    let stop_path = log_dir.join(".stop");

    run_worker_loop(
        &ctx(),
        &args,
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &worker,
        &skill,
        "",
        "loop-test",
        &allowed,
        &stop_path,
    )
    .expect("worker loop runs");

    let calls = gate.calls.lock().unwrap();
    assert_eq!(calls.len(), 1, "expected exactly one gate invocation per iter, got {calls:?}");
    assert_eq!(
        calls[0], worktree,
        "gate must run against worker.worktree_dir, not the dispatcher's ambient cwd",
    );
}

/// hew-j4x: the single-worker fast path must keep its prior behavior —
/// `run_loop_with` constructs a `Worker` with `worktree_dir =
/// project_root`, so the gate is invoked at the project root just like
/// before the per-worker plumbing landed.
#[test]
fn gate_falls_back_to_project_root_when_unspecified() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

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
    let gate = RecordingGateRunner::passing();

    run_loop_with(
        &ctx(),
        args_one_iter(),
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("loop runs");

    let calls = gate.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], repo, "single-worker path should pass project_root to the gate");
}

/// `run_worker_loop` must target `worker.worktree_dir` for every git
/// call, and write iter logs under `worker.log_dir` — not whatever
/// the dispatcher's `project_root` happened to be. Exercises the
/// per-worker contract that the future parallel dispatcher relies on.
#[test]
fn run_worker_loop_uses_worker_worktree_for_git_calls() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let worktree = tmp.path().join("wt");
    let log_dir = tmp.path().join("logs");
    std::fs::create_dir_all(&worktree).expect("mkdir worktree");
    std::fs::create_dir_all(&log_dir).expect("mkdir logs");

    git(&worktree, &["init", "-q", "-b", "main"]);
    std::fs::write(worktree.join("README.md"), b"seed\n").unwrap();
    git(&worktree, &["add", "README.md"]);
    git(&worktree, &["commit", "-q", "-m", "seed"]);
    let initial_sha = head_sha(&worktree);

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
    let spawner = CommitMakingSpawner { repo_dir: worktree.clone() };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let args = args_one_iter();
    let skill = hew_core::skills::find("hew-execute").expect("hew-execute skill present");
    let allowed = hew_core::allowed_tools::for_skill("hew-execute");
    let worker = Worker {
        id: 0,
        worktree_dir: worktree.clone(),
        branch: "loop/test/w0".into(),
        worker_n: None,
        log_dir: log_dir.clone(),
    };
    let stop_path = log_dir.join(".stop");

    let outcome = run_worker_loop(
        &ctx(),
        &args,
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &worker,
        &skill,
        "",
        "loop-test",
        &allowed,
        &stop_path,
    )
    .expect("worker loop runs");

    // Spawner committed inside `worktree`; HEAD must have advanced.
    assert_ne!(head_sha(&worktree), initial_sha, "expected commit in worker worktree");

    // Iter log must land under worker.log_dir, NOT under any other
    // ambient project root.
    let iter_log = log_dir.join("iter-001.json");
    assert!(iter_log.exists(), "iter-001.json must live under worker.log_dir");
    let body = std::fs::read_to_string(&iter_log).expect("read iter log");
    let log: IterLog = serde_json::from_str(&body).expect("parse iter log");
    assert_eq!(log.task_id.as_deref(), Some("hew-test"));
    // The returned outcome mirrors the on-disk run.
    assert_eq!(outcome.iter_logs.len(), 1);
    assert_eq!(outcome.run.iters.len(), 1);
}

#[test]
fn jobs_default_is_1() {
    // Pins the clap default + the test-fixture default the rest of the
    // loop suite shares. Together with `loop_run_help_documents_jobs_flag`
    // in tests/cli.rs this is the user-facing contract.
    let args = args_one_iter();
    assert_eq!(args.jobs, 1);
}

#[test]
fn jobs_1_uses_serial_fast_path() {
    // jobs=1 must keep the byte-identical N=1 layout: iter logs at the
    // run-dir root (no worker-N subdir) and a manifest whose `jobs`
    // field is 1. Indirectly proves run_loop_serial was taken — the
    // parallel path always writes worker-N/ subdirs + Manifest::jobs
    // == args.jobs (see `jobs_2_uses_dispatcher_path`).
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let bd = CapturingBd { ready: vec![], remembered: RefCell::new(Vec::new()) };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let mut args = args_one_iter();
    args.jobs = 1;
    args.dry_run = true;

    run_loop_with(
        &ctx(),
        args,
        &bd,
        None,
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("serial loop runs");

    let loop_root = repo.join(".hew/loop");
    let entry = std::fs::read_dir(&loop_root)
        .expect("read loop root")
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .expect("one run dir");
    let run_dir = entry.path();

    assert!(
        !run_dir.join("worker-0").exists(),
        "serial fast path must not create worker-N subdirs"
    );
    let manifest_path = run_dir.join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json should be written");
    let body = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let m: serde_json::Value = serde_json::from_str(&body).expect("parse manifest");
    assert_eq!(m["jobs"].as_u64(), Some(1), "serial path Manifest.jobs == 1");
}

#[test]
fn jobs_2_uses_dispatcher_path() {
    // Inverse of `jobs_1_uses_serial_fast_path`: --jobs 2 must invoke
    // the Dispatcher path, evidenced by Manifest::jobs == 2.
    let tmp = tempfile::tempdir().expect("tempdir");
    let repo = tmp.path().to_path_buf();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), b"seed\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "seed"]);

    let bd = CapturingBd { ready: vec![], remembered: RefCell::new(Vec::new()) };
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let mut args = args_one_iter();
    args.jobs = 2;
    args.dry_run = true;

    run_loop_with(
        &ctx(),
        args,
        &bd,
        None,
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("parallel loop runs (dry-run)");

    let loop_root = repo.join(".hew/loop");
    let entry = std::fs::read_dir(&loop_root)
        .expect("read loop root")
        .filter_map(|e| e.ok())
        .find(|e| e.path().is_dir())
        .expect("one run dir");
    let run_dir = entry.path();

    let manifest_path = run_dir.join("manifest.json");
    assert!(manifest_path.exists(), "manifest.json should be written");
    let body = std::fs::read_to_string(&manifest_path).expect("read manifest");
    let m: serde_json::Value = serde_json::from_str(&body).expect("parse manifest");
    assert_eq!(m["jobs"].as_u64(), Some(2), "parallel path Manifest.jobs == args.jobs");
}
