//! Regression tests for hew-s9mb: the serial (`--jobs=1`) loop path must
//! honor the run's `Scope::Epics` filter at task selection, matching the
//! `Dispatcher::dispatch_tick` behavior on the parallel path.
//!
//! Before the fix, `run_worker_loop_with_scope` polled `bd.ready()` and
//! grabbed the first task without consulting `cfg.scope` — so a
//! `--scope=epics --epics=<id>` run on `--jobs=1` would happily claim
//! any unrelated ready bug.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;

use hew_core::backpressure::GateCheck;
use hew_core::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
use hew_core::config::{LoopModelConfig, LoopPlannerConfig};
use hew_core::ctx::{Ctx, OutputMode};
use hew_core::error::Result as HewResult;
use hew_core::runtime::FallbackConfig;
use hew_core::scope::Scope;
use hew_core::{allowed_tools, skills};

use hew::commands::loop_cmd::{Args, StaticGateRunner, Worker, run_worker_loop_with_scope};

/// BdClient that exposes a fixed ready list plus a per-parent children
/// map for `bd children <id>` lookups (used by
/// `scope::resolve_descendants` when filtering for `Scope::Epics`).
#[derive(Debug)]
struct ScopedBd {
    ready: Vec<ReadyTask>,
    children: BTreeMap<String, String>,
    calls: RefCell<Vec<Vec<String>>>,
}

impl ScopedBd {
    fn new(ready: Vec<ReadyTask>) -> Self {
        Self { ready, children: BTreeMap::new(), calls: RefCell::new(Vec::new()) }
    }
    fn with_children(mut self, parent: &str, kids: &[&ReadyTask]) -> Self {
        let body = kids
            .iter()
            .map(|t| {
                format!(
                    r#"{{"id":"{}","title":"{}","description":"","status":"open","priority":{},"issue_type":"task","closed_at":"","close_reason":null,"parent":"{}"}}"#,
                    t.id, t.title, t.priority, parent,
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        self.children.insert(parent.to_string(), format!("[{body}]"));
        self
    }
}

impl BdClient for ScopedBd {
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
    fn remember(&self, _text: &str) -> HewResult<()> {
        Ok(())
    }
    fn run_raw(&self, args: &[&OsStr]) -> HewResult<BdOutput> {
        let captured: Vec<String> = args.iter().map(|a| a.to_string_lossy().to_string()).collect();
        self.calls.borrow_mut().push(captured.clone());
        if captured.first().map(|s| s.as_str()) == Some("children") {
            let parent = captured.get(1).cloned().unwrap_or_default();
            let body = self.children.get(&parent).cloned().unwrap_or_else(|| "[]".into());
            return Ok(BdOutput { stdout: body, stderr: String::new() });
        }
        Ok(BdOutput { stdout: String::new(), stderr: String::new() })
    }
}

fn ctx() -> Ctx {
    Ctx { interactive: false, output: OutputMode::Text, quiet: true, verbose: 0 }
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
        dry_run: true,
        skill: "hew-execute".into(),
        fallback_runtime: None,
        fallback_cooldown_iters: None,
        jobs: 1,
        scope: None,
        epics: Vec::new(),
        epic: Vec::new(),
        no_planner: false,
        planner_budget: None,
        planner_runtime: None,
        verify_tests: false,
        no_verify_tests: false,
        verify_command: None,
    }
}

fn ready_task(id: &str, title: &str) -> ReadyTask {
    ReadyTask {
        id: id.into(),
        title: title.into(),
        description: String::new(),
        priority: 2,
        status: "open".into(),
        issue_type: "task".into(),
        parent: None,
    }
}

fn worker(log_dir: &Path) -> Worker {
    Worker {
        id: 0,
        worktree_dir: log_dir.to_path_buf(),
        branch: String::new(),
        log_dir: log_dir.to_path_buf(),
        worker_n: None,
    }
}

/// The reproducer from hew-s9mb: ready set contains an unrelated bug
/// (`hew-zt4z`) plus the in-scope child (`hew-ja44`). The serial loop,
/// running under `--scope=epics --epics=hew-c0pa`, must claim the child
/// and ignore the unrelated bug — before the fix it picked the unrelated
/// task because the serial path's `bd.ready()` poll skipped the filter.
#[test]
fn e2e_serial_scope_epics_filters_out_unrelated_ready_tasks() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let log_dir = tmp.path().to_path_buf();

    let unrelated = ready_task("hew-zt4z", "unrelated bug");
    let child = ready_task("hew-ja44", "in-scope entry child");
    let bd = ScopedBd::new(vec![unrelated.clone(), child.clone()])
        .with_children("hew-c0pa", &[&child])
        .with_children("hew-ja44", &[]);

    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let args = args_one_iter();
    let skill = skills::find(&args.skill).expect("hew-execute skill present");
    let allowed = allowed_tools::for_skill(&args.skill);
    let stop = log_dir.join(".stop");

    let outcome = run_worker_loop_with_scope(
        &ctx(),
        &args,
        &bd,
        None,
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        LoopPlannerConfig::default(),
        &gate,
        &worker(&log_dir),
        &skill,
        "",
        "loop-test-scope",
        &allowed,
        &stop,
        Scope::Epics { epic_ids: vec!["hew-c0pa".into()] },
    )
    .expect("serial worker loop runs");

    assert_eq!(outcome.run.iters.len(), 1, "exactly one iter under max_iter=1");
    let iter = &outcome.run.iters[0];
    assert_eq!(
        iter.task_id.as_deref(),
        Some("hew-ja44"),
        "serial path must respect Scope::Epics — claimed the wrong task",
    );
    assert_ne!(
        iter.task_id.as_deref(),
        Some("hew-zt4z"),
        "serial path leaked outside scope: claimed unrelated ready bug",
    );
}

/// When the only ready tasks are outside the epic's descendant set, the
/// scoped serial loop must stop with `ReadyEmpty` rather than spawning
/// against any of them.
#[test]
fn serial_loop_skips_bd_ready_tasks_outside_scope_epic_descendants() {
    use hew_core::runner::StopReason;

    let tmp = tempfile::tempdir().expect("tempdir");
    let log_dir = tmp.path().to_path_buf();

    let stranger = ready_task("hew-stranger", "outside the epic");
    let bd = ScopedBd::new(vec![stranger]).with_children("hew-c0pa", &[]);
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let args = args_one_iter();
    let skill = skills::find(&args.skill).expect("hew-execute skill present");
    let allowed = allowed_tools::for_skill(&args.skill);
    let stop = log_dir.join(".stop");

    let outcome = run_worker_loop_with_scope(
        &ctx(),
        &args,
        &bd,
        None,
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        LoopPlannerConfig::default(),
        &gate,
        &worker(&log_dir),
        &skill,
        "",
        "loop-test-scope-empty",
        &allowed,
        &stop,
        Scope::Epics { epic_ids: vec!["hew-c0pa".into()] },
    )
    .expect("serial worker loop runs");

    assert!(outcome.run.iters.is_empty(), "no iter should run when scope filter empties the queue");
    assert!(
        matches!(outcome.run.stop_reason, Some(StopReason::ReadyEmpty)),
        "expected StopReason::ReadyEmpty, got {:?}",
        outcome.run.stop_reason,
    );
}

/// Sanity: `Scope::Ready` (the legacy default) keeps the pre-fix
/// behavior — any bd-ready task is fair game. Locks the no-regression
/// promise the bug ticket explicitly calls out ("loop_scope_e2e 7/7 still
/// pass" — those exercise argv-contract; this exercises actual selection).
#[test]
fn serial_loop_scope_ready_still_claims_any_ready_task() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let log_dir = tmp.path().to_path_buf();

    let only = ready_task("hew-anything", "any old task");
    let bd = ScopedBd::new(vec![only]);
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });

    let args = args_one_iter();
    let skill = skills::find(&args.skill).expect("hew-execute skill present");
    let allowed = allowed_tools::for_skill(&args.skill);
    let stop = log_dir.join(".stop");

    let outcome = run_worker_loop_with_scope(
        &ctx(),
        &args,
        &bd,
        None,
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        LoopPlannerConfig::default(),
        &gate,
        &worker(&log_dir),
        &skill,
        "",
        "loop-test-scope-ready",
        &allowed,
        &stop,
        Scope::Ready,
    )
    .expect("serial worker loop runs");

    assert_eq!(outcome.run.iters.len(), 1);
    assert_eq!(outcome.run.iters[0].task_id.as_deref(), Some("hew-anything"));
}
