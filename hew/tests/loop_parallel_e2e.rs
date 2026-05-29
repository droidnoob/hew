//! End-to-end integration tests for the parallel `hew loop` path
//! (`--jobs >= 2`). Exercises the dispatcher + per-worker worktree +
//! merge_back surface against a synthetic bd graph + in-process mock
//! spawner. Hermetic: every test isolates its `~/.hew/wt/` by setting
//! `HOME` to a tempdir, and the project repo lives in another tempdir.
//!
//! Task: hew-d5gd.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

use hew_core::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
use hew_core::ctx::{Ctx, OutputMode};
use hew_core::error::Result as HewResult;
use hew_core::prompt::AssembledPrompt;
use hew_core::runner::TokenSpend;
use hew_core::runtime::{
    FallbackConfig, RuntimeSpawner, SpawnFailureClass, SpawnOpts, SpawnOutcome,
};

use hew::commands::loop_cmd::{Args, StaticGateRunner, run_loop_with};
use hew_core::backpressure::GateCheck;
use hew_core::config::LoopModelConfig;

/// Process-wide lock for HOME mutation. Tests in this binary may run on
/// separate threads; serializing them keeps the env swap safe.
static HOME_LOCK: Mutex<()> = Mutex::new(());

struct HomeGuard {
    _lock: std::sync::MutexGuard<'static, ()>,
    prev: Option<std::ffi::OsString>,
    _tmp: tempfile::TempDir,
}

impl HomeGuard {
    fn install() -> (Self, PathBuf) {
        let lock = HOME_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let prev = std::env::var_os("HOME");
        let tmp = tempfile::tempdir().expect("home tempdir");
        let home = tmp.path().to_path_buf();
        // SAFETY: lock serializes mutation within this test binary; no
        // other thread reads HOME concurrently here.
        unsafe { std::env::set_var("HOME", &home) };
        // Scrub git env that the host (pre-commit hook, parent shell)
        // may have leaked in — `git worktree add` otherwise fights the
        // ambient GIT_INDEX_FILE / GIT_DIR pointing at the outer repo.
        for v in [
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
            unsafe { std::env::remove_var(v) };
        }
        // Production `RealGit` invocations during merge_back create commits
        // (`merge --no-ff`); without an identity those fail with "Please tell
        // me who you are" before any conflict is detected, masking real
        // conflict reporting. Ubuntu runners have no system git identity, so
        // seed one into this HOME tempdir.
        std::fs::write(
            home.join(".gitconfig"),
            b"[user]\n\tname = hew-test\n\temail = hew-test@example.com\n",
        )
        .expect("write .gitconfig");
        (Self { _lock: lock, prev, _tmp: tmp }, home)
    }
}

impl Drop for HomeGuard {
    fn drop(&mut self) {
        // SAFETY: still under HOME_LOCK; restore the prior value or
        // remove the override.
        unsafe {
            match self.prev.take() {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
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

fn ctx() -> Ctx {
    Ctx::new(true, OutputMode::Text, true, 0)
}

fn ready_task(id: &str) -> ReadyTask {
    ReadyTask {
        id: id.into(),
        title: format!("synthetic {id}"),
        description: String::new(),
        priority: 1,
        status: "open".into(),
        issue_type: "task".into(),
        parent: None,
    }
}

fn args_parallel(jobs: u32) -> Args {
    Args {
        max_iter: Some(8),
        until_empty: true,
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
        jobs,
    }
}

/// In-memory bd whose `ready()` set + `q`-allocated ids are protected
/// by a Mutex so the spawner can mutate state mid-iter. Tracks every
/// `bd q` and `bd update --description` so merge-conflict bug-task
/// filing is observable.
#[derive(Debug)]
struct SharedBd {
    ready: Mutex<Vec<ReadyTask>>,
    next_id: Mutex<u32>,
    new_task_titles: Mutex<Vec<String>>,
    new_task_bodies: Mutex<Vec<String>>,
}

impl SharedBd {
    fn with(ready: Vec<ReadyTask>) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            ready: Mutex::new(ready),
            next_id: Mutex::new(0),
            new_task_titles: Mutex::new(Vec::new()),
            new_task_bodies: Mutex::new(Vec::new()),
        })
    }

    fn pop_ready(&self) -> Option<ReadyTask> {
        let mut g = self.ready.lock().unwrap();
        if g.is_empty() { None } else { Some(g.remove(0)) }
    }

    fn ready_len(&self) -> usize {
        self.ready.lock().unwrap().len()
    }
}

impl BdClient for SharedBd {
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
    fn remember(&self, _text: &str) -> HewResult<()> {
        Ok(())
    }
    fn run_raw(&self, args: &[&OsStr]) -> HewResult<BdOutput> {
        // `bd update <id> --claim` — dispatcher's atomic claim. Remove
        // id from ready to mirror real bd's "claimed tasks leave the
        // ready queue" semantics.
        if args.len() == 3 && args[0] == OsStr::new("update") && args[2] == OsStr::new("--claim") {
            let id = args[1].to_string_lossy().to_string();
            self.ready.lock().unwrap().retain(|t| t.id != id);
            return Ok(BdOutput { stdout: String::new(), stderr: String::new() });
        }
        // `bd q <title> ...` — task creation; record + emit a new id.
        if args.first() == Some(&OsStr::new("q")) {
            let title = args.get(1).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            self.new_task_titles.lock().unwrap().push(title);
            let mut next = self.next_id.lock().unwrap();
            *next += 1;
            let id = format!("hew-bug-{}", *next);
            return Ok(BdOutput { stdout: format!("{id}\n"), stderr: String::new() });
        }
        // `bd update <id> --description <body>` — capture body for the
        // merge-conflict assertion. We also accept `--body-file`.
        if args.first() == Some(&OsStr::new("update")) {
            if let Some(pos) = args.iter().position(|a| *a == OsStr::new("--description"))
                && let Some(body) = args.get(pos + 1)
            {
                self.new_task_bodies.lock().unwrap().push(body.to_string_lossy().to_string());
            }
            return Ok(BdOutput { stdout: String::new(), stderr: String::new() });
        }
        Ok(BdOutput { stdout: String::new(), stderr: String::new() })
    }
}

/// Spawner that closes the head of `bd.ready` on every call and reports
/// it as `closed`. No worktree side-effects. Used for test 1.
#[derive(Debug)]
struct DrainingMockSpawner {
    bd: std::sync::Arc<SharedBd>,
    calls: Mutex<u32>,
}

impl DrainingMockSpawner {
    fn new(bd: std::sync::Arc<SharedBd>) -> Self {
        Self { bd, calls: Mutex::new(0) }
    }
    fn call_count(&self) -> u32 {
        *self.calls.lock().unwrap()
    }
}

impl RuntimeSpawner for DrainingMockSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _tools: &[String],
        _opts: &SpawnOpts,
    ) -> HewResult<SpawnOutcome> {
        *self.calls.lock().unwrap() += 1;
        let closed = self.bd.pop_ready().map(|t| t.id);
        let raw = closed.as_deref().map(|id| format!("closed {id} — mock")).unwrap_or_default();
        Ok(SpawnOutcome {
            success: true,
            closed_task: closed,
            tokens: TokenSpend { input: 1, output: 1, cache_read: 0, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: raw,
            failure_class: SpawnFailureClass::Success,
        })
    }
}

/// Spawner that writes a *colliding* line to `<conflict_file>` in the
/// next undiscovered worktree under `<wt_root>/<run-id>/<n>/`, commits
/// it on the worker's branch, then closes the head of `bd.ready`. Used
/// for test 2.
#[derive(Debug)]
struct ConflictingSpawner {
    bd: std::sync::Arc<SharedBd>,
    wt_root: PathBuf,
    conflict_file: String,
    call_idx: Mutex<u32>,
}

impl ConflictingSpawner {
    fn new(bd: std::sync::Arc<SharedBd>, wt_root: PathBuf, conflict_file: &str) -> Self {
        Self { bd, wt_root, conflict_file: conflict_file.into(), call_idx: Mutex::new(0) }
    }
}

impl RuntimeSpawner for ConflictingSpawner {
    fn spawn(
        &self,
        _prompt: &AssembledPrompt,
        _tools: &[String],
        _opts: &SpawnOpts,
    ) -> HewResult<SpawnOutcome> {
        let n = {
            let mut g = self.call_idx.lock().unwrap();
            let v = *g;
            *g += 1;
            v
        };
        // Resolve which worker dir this call corresponds to. The
        // dispatcher created `<wt_root>/<run-id>/{0,1,...}` before the
        // serial worker loop began, so we just discover the (single)
        // run-id dir and pick `n`.
        let run_dir = std::fs::read_dir(&self.wt_root)
            .expect("wt_root populated")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.is_dir())
            .expect("run-id subdir");
        let wt = run_dir.join(n.to_string());
        let body = format!("worker {n} version line\n");
        std::fs::write(wt.join(&self.conflict_file), body).unwrap();
        git(&wt, &["add", &self.conflict_file]);
        git(&wt, &["commit", "-q", "-m", &format!("w{n} change")]);

        let closed = self.bd.pop_ready().map(|t| t.id);
        let raw = closed.as_deref().map(|id| format!("closed {id} — mock")).unwrap_or_default();
        Ok(SpawnOutcome {
            success: true,
            closed_task: closed,
            tokens: TokenSpend { input: 1, output: 1, cache_read: 0, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: raw,
            failure_class: SpawnFailureClass::Success,
        })
    }
}

fn read_manifest(repo: &Path) -> serde_json::Value {
    let loop_root = repo.join(".hew/loop");
    let run_dir = std::fs::read_dir(&loop_root)
        .expect("loop root")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .expect("one run dir");
    let body = std::fs::read_to_string(run_dir.join("manifest.json")).expect("manifest");
    serde_json::from_str(&body).expect("parse manifest")
}

#[test]
fn e2e_parallel_jobs_2_with_mock_spawner() {
    if !git_available() {
        eprintln!("git not on PATH, skipping");
        return;
    }
    let (_home, home_dir) = HomeGuard::install();
    let wt_root = home_dir.join(".hew").join("wt");

    let repo_tmp = tempfile::tempdir().expect("repo tempdir");
    let repo = repo_tmp.path().to_path_buf();
    seed_repo(&repo);

    let bd = SharedBd::with((1..=4).map(|i| ready_task(&format!("hew-r{i}"))).collect());
    let spawner = DrainingMockSpawner::new(bd.clone());
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    run_loop_with(
        &ctx(),
        args_parallel(2),
        &*bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        &repo,
    )
    .expect("parallel loop runs");

    // All four tasks must be gone from the ready set — two via the
    // dispatcher's atomic claim and two via the spawner's mock close.
    assert_eq!(bd.ready_len(), 0, "all 4 ready tasks should have left the queue");
    // Spawner ran at least twice (one call per worker iter that found
    // a ready task). With current serial-per-worker dispatch this is
    // exactly the count of remaining tasks after dispatcher claims (=2)
    // — pin it loosely as `>= 2` so a future concurrent dispatcher
    // doesn't break this test.
    assert!(
        spawner.call_count() >= 2,
        "expected spawner to be invoked at least twice, got {}",
        spawner.call_count(),
    );

    // Graceful shutdown (hew-kt5q): both worker branches merged cleanly
    // onto launch HEAD, so the dispatcher's post-merge teardown removed
    // each worker's worktree. The run-id dir itself is removed by
    // `worktree::prune` once empty.
    let surviving_run_dirs: Vec<PathBuf> = std::fs::read_dir(&wt_root)
        .map(|it| {
            it.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(
        surviving_run_dirs.is_empty(),
        "expected no surviving worktree dirs after clean merge, got {surviving_run_dirs:?}",
    );

    // Manifest pins the per-run shape downstream tooling consumes.
    let m = read_manifest(&repo);
    assert_eq!(m["jobs"].as_u64(), Some(2));
    let workers = m["workers"].as_array().expect("workers array");
    assert_eq!(workers.len(), 2, "manifest must list 2 workers");
}

#[test]
fn e2e_parallel_merge_conflict_files_bug_task() {
    if !git_available() {
        eprintln!("git not on PATH, skipping");
        return;
    }
    let (_home, home_dir) = HomeGuard::install();
    let wt_root = home_dir.join(".hew").join("wt");

    let repo_tmp = tempfile::tempdir().expect("repo tempdir");
    let repo = repo_tmp.path().to_path_buf();
    seed_repo(&repo);

    // Four ready tasks so the dispatcher's pre-claim (2) still leaves
    // 2 in the queue for the workers' iters to consume. Without the
    // surplus the workers would find an empty ready set and exit
    // before committing the colliding file.
    let bd = SharedBd::with((1..=4).map(|i| ready_task(&format!("hew-c{i}"))).collect());
    let spawner = ConflictingSpawner::new(bd.clone(), wt_root.clone(), "data.txt");
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let mut args = args_parallel(2);
    // Each worker should run exactly one iter (write + commit) so both
    // worker branches actually carry a colliding commit. Without this
    // cap a worker with an empty ready set would no-op without a
    // commit, and merge_back would land both empties cleanly.
    args.max_iter = Some(1);
    args.until_empty = false;

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
    .expect("parallel loop runs");

    // Exactly one [merge-conflict] bug task: first worker branch merges
    // cleanly onto launch HEAD, second collides on `data.txt`.
    let titles = bd.new_task_titles.lock().unwrap().clone();
    let conflict_titles: Vec<&String> =
        titles.iter().filter(|t| t.starts_with("[merge-conflict]")).collect();
    assert_eq!(
        conflict_titles.len(),
        1,
        "expected exactly one [merge-conflict] bug task, got {titles:?}",
    );

    // The filed bug body mentions `data.txt`.
    let bodies = bd.new_task_bodies.lock().unwrap().clone();
    assert!(
        bodies.iter().any(|b| b.contains("data.txt")),
        "expected a bug-task body referencing the conflicting file, got {bodies:?}",
    );

    // Worker 1's worktree must survive on disk — its branch conflicted
    // on `data.txt` and the operator needs the worktree to resolve. Per
    // hew-kt5q's graceful teardown, worker 0's worktree (which merged
    // cleanly first) is pruned on the way out; the run-id dir remains
    // only because at least one child (worker-1) is still on disk.
    let runs_under_wt: Vec<PathBuf> = std::fs::read_dir(&wt_root)
        .expect("wt_root exists")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(runs_under_wt.len(), 1);
    let run_dir = &runs_under_wt[0];
    assert!(
        !run_dir.join("0").exists(),
        "worker 0 worktree (clean merge) should be pruned by graceful teardown"
    );
    assert!(run_dir.join("1").is_dir(), "worker 1 worktree (conflict) must remain on disk");
}
