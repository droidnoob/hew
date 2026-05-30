//! `hew loop graph` end-to-end. Plants a tiny 2-iter run-dir with a
//! batch-plan + run.json and asserts the subcommand renders mermaid by
//! default + writes to `--output` when requested.
//!
//! Task: hew-m7lq.

use std::path::Path;

use assert_cmd::Command as AssertCmd;
use hew_core::batch_plan::{BatchPlan, BatchSource, SCHEMA_VERSION};
use hew_core::loop_log::{IterLog, RunLog, run_dir, run_log_path, write_json_atomic};
use hew_core::runner::TokenSpend;
use predicates::str::contains;

fn write_iter(dir: &Path, n: u32, task: &str, started: &str, ended: &str) {
    let log = IterLog {
        number: n,
        task_id: Some(task.into()),
        started_at: started.into(),
        ended_at: Some(ended.into()),
        outcome: Some("closed".into()),
        prompt_prefix_hash: None,
        cost: TokenSpend { input: 100, output: 50, cache_read: 0, cache_create: 0 },
        decisions: Vec::new(),
        deferred: Vec::new(),
        tool_calls: Vec::new(),
        stderr_tail: None,
        symbols_touched: Vec::new(),
        runtime_used: None,
        cooldown_engaged: false,
        model: None,
    };
    write_json_atomic(&dir.join(format!("iter-{n:03}.json")), &log).unwrap();
}

fn write_run(dir: &Path, id: &str) {
    let rl = RunLog {
        id: id.into(),
        started_at: "2026-05-30T00:00:00Z".into(),
        last_updated_at: "2026-05-30T00:01:00Z".into(),
        iter_count: 2,
        cumulative_tokens: 300,
        stop_reason: Some("ready_empty".into()),
        max_iter: None,
        strict: false,
        interactive: false,
        scope: None,
        verify_outcome: None,
    };
    write_json_atomic(&run_log_path(dir, None), &rl).unwrap();
}

fn plant_run(project_root: &Path, run_id: &str) {
    let dir = run_dir(project_root, run_id).unwrap();
    write_iter(&dir, 1, "hew-a", "2026-05-30T00:00:00Z", "2026-05-30T00:00:10Z");
    write_iter(&dir, 2, "hew-b", "2026-05-30T00:00:10Z", "2026-05-30T00:00:20Z");
    hew_core::batch_plan::write(
        &dir,
        &BatchPlan {
            schema_version: SCHEMA_VERSION,
            iter_number: 2,
            task_ids: vec!["hew-b".into()],
            source: BatchSource::Agent,
            reason: None,
            created_at: "2026-05-30T00:00:09Z".into(),
            planner_tokens: None,
        },
    )
    .unwrap();
    write_run(&dir, run_id);
}

#[test]
fn cli_loop_graph_renders_latest_run_to_stdout_as_mermaid() {
    let tmp = tempfile::tempdir().unwrap();
    plant_run(tmp.path(), "loop-graph-e2e");

    AssertCmd::cargo_bin("hew")
        .unwrap()
        .current_dir(tmp.path())
        .args(["loop", "graph"])
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .assert()
        .success()
        .stdout(contains("flowchart TD"))
        .stdout(contains("iter1 -. agent .-> iter2"));
}

#[test]
fn cli_loop_graph_writes_to_output_file_when_provided() {
    let tmp = tempfile::tempdir().unwrap();
    plant_run(tmp.path(), "loop-graph-e2e-out");
    let out_path = tmp.path().join("graph.md");

    AssertCmd::cargo_bin("hew")
        .unwrap()
        .current_dir(tmp.path())
        .args(["loop", "graph", "--out", out_path.to_str().unwrap()])
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .assert()
        .success()
        .stdout(contains("wrote"));

    let body = std::fs::read_to_string(&out_path).unwrap();
    // .md output wraps mermaid in a fenced block.
    assert!(body.starts_with("```mermaid\n"), "body: {body}");
    assert!(body.contains("flowchart TD"));
    assert!(body.trim_end().ends_with("```"));
}

#[test]
fn cli_loop_graph_supports_dot_and_ascii_formats() {
    let tmp = tempfile::tempdir().unwrap();
    plant_run(tmp.path(), "loop-graph-e2e-fmt");

    AssertCmd::cargo_bin("hew")
        .unwrap()
        .current_dir(tmp.path())
        .args(["loop", "graph", "--format", "dot"])
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .assert()
        .success()
        .stdout(contains("digraph loop"))
        .stdout(contains("iter1 -> iter2"));

    AssertCmd::cargo_bin("hew")
        .unwrap()
        .current_dir(tmp.path())
        .args(["loop", "graph", "--format", "ascii"])
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .assert()
        .success()
        .stdout(contains("run: loop-graph-e2e-fmt"));
}

#[test]
fn cli_loop_graph_all_aggregates_multiple_runs() {
    let tmp = tempfile::tempdir().unwrap();
    plant_run(tmp.path(), "loop-graph-all-aaa");
    plant_run(tmp.path(), "loop-graph-all-bbb");

    AssertCmd::cargo_bin("hew")
        .unwrap()
        .current_dir(tmp.path())
        .args(["loop", "graph", "--all"])
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .assert()
        .success()
        .stdout(contains("subgraph loop_graph_all_aaa"))
        .stdout(contains("subgraph loop_graph_all_bbb"));
}

#[test]
fn cli_loop_graph_errors_when_run_dir_missing() {
    let tmp = tempfile::tempdir().unwrap();
    // No runs planted.

    AssertCmd::cargo_bin("hew")
        .unwrap()
        .current_dir(tmp.path())
        .args(["loop", "graph"])
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .assert()
        .failure();
}
