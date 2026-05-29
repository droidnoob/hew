//! Integration test for hew-6et: per-iter `resolve_model` output must
//! reach the spawner through `SpawnOpts::model_override`. Builds a
//! fixture ready task with a `<!-- hew:model=opus -->` description,
//! drives one dry-run iter against a [`MockSpawner`], and asserts the
//! mock saw the model name verbatim.

use std::collections::BTreeMap;
use std::ffi::OsStr;

use hew_core::backpressure::GateCheck;
use hew_core::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
use hew_core::config::LoopModelConfig;
use hew_core::ctx::{Ctx, OutputMode};
use hew_core::error::Result as HewResult;
use hew_core::runtime::{FallbackConfig, MockSpawner, SpawnOutcome};

use hew::commands::loop_cmd::{Args, StaticGateRunner, run_loop_with};

#[derive(Debug)]
struct StaticBd {
    ready: Vec<ReadyTask>,
}

impl BdClient for StaticBd {
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
    fn remember(&self, _: &str) -> HewResult<()> {
        Ok(())
    }
    fn run_raw(&self, _: &[&OsStr]) -> HewResult<BdOutput> {
        Ok(BdOutput { stdout: String::new(), stderr: String::new() })
    }
}

fn args_one_dry_iter() -> Args {
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
        // dry_run skips the auto-gate + git-head capture so the test
        // doesn't need a temporary repo. The spawner itself is still
        // invoked because it's passed in directly to run_loop_with.
        dry_run: true,
        skill: "hew-execute".into(),
        fallback_runtime: None,
        fallback_cooldown_iters: None,
        jobs: 1,
    }
}

fn ctx() -> Ctx {
    Ctx::new(true, OutputMode::Text, true, 0)
}

#[test]
fn description_model_tag_threads_into_spawn_opts() {
    let bd = StaticBd {
        ready: vec![ReadyTask {
            id: "hew-fake".into(),
            title: "synthetic ready task".into(),
            description: "body with <!-- hew:model=opus --> tag".into(),
            priority: 2,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }],
    };
    let spawner = MockSpawner::new(SpawnOutcome::default());
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });
    // tempdir for project_root keeps the run-dir writes isolated.
    let tmp = tempfile::tempdir().expect("tempdir");

    run_loop_with(
        &ctx(),
        args_one_dry_iter(),
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        tmp.path(),
    )
    .expect("loop runs");

    let last = spawner.last_opts.borrow();
    let opts = last.as_ref().expect("MockSpawner should have recorded SpawnOpts for the iter");
    assert_eq!(
        opts.model_override.as_deref(),
        Some("opus"),
        "expected description tag `<!-- hew:model=opus -->` to thread into SpawnOpts::model_override",
    );
}

#[test]
fn no_annotation_no_config_leaves_model_override_none() {
    let bd = StaticBd {
        ready: vec![ReadyTask {
            id: "hew-plain".into(),
            title: "plain ready task".into(),
            description: "no tag here".into(),
            priority: 2,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }],
    };
    let spawner = MockSpawner::new(SpawnOutcome::default());
    let gate =
        StaticGateRunner(GateCheck { tests_passed: true, lint_passed: true, ..Default::default() });
    let tmp = tempfile::tempdir().expect("tempdir");

    run_loop_with(
        &ctx(),
        args_one_dry_iter(),
        &bd,
        Some(&spawner),
        None,
        FallbackConfig::default(),
        LoopModelConfig::default(),
        &gate,
        tmp.path(),
    )
    .expect("loop runs");

    let last = spawner.last_opts.borrow();
    let opts = last.as_ref().expect("spawn opts recorded");
    assert!(
        opts.model_override.is_none(),
        "empty config + un-annotated task should leave model_override = None, got {:?}",
        opts.model_override,
    );
}
