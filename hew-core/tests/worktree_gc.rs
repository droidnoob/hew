//! Worktree garbage-collection surface: orphan detection + crash-survival
//! semantics. Production wiring (graceful teardown at dispatcher
//! shutdown, `hew loop prune-worktrees` subcommand) lives in `hew/`; this
//! file pins the pure logic in `hew_core::worktree` + `hew_core::loop_log`.
//!
//! Task: hew-kt5q.

use std::collections::HashSet;

use hew_core::loop_log::{
    LOOP_ROOT, RunLog, active_run_ids, run_dir, run_log_path, write_json_atomic,
};
use hew_core::runner::{Run, RunConfig, StopReason};
use hew_core::worktree::{branch_name, list_orphans, worker_path};

/// Stamp `<project>/.hew/loop/<run-id>/run.json` with the given stop
/// reason. `None` keeps the run looking "live".
fn plant_run(project_root: &std::path::Path, run_id: &str, stop: Option<StopReason>) {
    let dir = run_dir(project_root, run_id).expect("run_dir");
    let mut run = Run::new(run_id, "2026-05-30T00:00:00Z", RunConfig::default());
    run.stop_reason = stop;
    let log = RunLog::from_run(&run);
    write_json_atomic(&run_log_path(&dir, None), &log).expect("write run.json");
}

/// Plant a worktree dir under `<wt_root>/<run-id>/<n>/` — no git
/// metadata, just the directory shape `list_orphans` walks.
fn plant_worktree(wt_root: &std::path::Path, run_id: &str, n: u32) {
    std::fs::create_dir_all(worker_path(wt_root, run_id, n)).unwrap();
}

#[test]
fn crash_simulated_leaves_worktrees_on_disk() {
    // A crashed parallel run leaves run.json with stop_reason=None (or
    // missing entirely). The orphan-detection pass treats that run as
    // active and refuses to flag its worktrees — the operator gets to
    // inspect them.
    let project = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let wt_root = wt.path();

    plant_worktree(wt_root, "loop-crashed", 0);
    plant_run(project.path(), "loop-crashed", None);

    let loop_root = project.path().join(LOOP_ROOT);
    let active = active_run_ids(&loop_root).unwrap();
    assert!(active.contains("loop-crashed"), "crashed run must stay in active set");

    let orphans = list_orphans(wt_root, &active).unwrap();
    assert!(orphans.is_empty(), "crashed worktrees must not be auto-orphaned, got {orphans:?}");
    assert!(worker_path(wt_root, "loop-crashed", 0).exists(), "worktree dir survives");
}

#[test]
fn prune_targets_completed_runs_only() {
    // Two planted worktrees: one belongs to a completed run, one to a
    // still-live run. list_orphans must return only the completed one.
    let project = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let wt_root = wt.path();

    plant_worktree(wt_root, "loop-done", 0);
    plant_worktree(wt_root, "loop-live", 0);
    plant_run(project.path(), "loop-done", Some(StopReason::ReadyEmpty));
    plant_run(project.path(), "loop-live", None);

    let loop_root = project.path().join(LOOP_ROOT);
    let active = active_run_ids(&loop_root).unwrap();
    let orphans = list_orphans(wt_root, &active).unwrap();

    let orphan_ids: HashSet<String> = orphans.iter().map(|h| h.run_id.clone()).collect();
    assert!(orphan_ids.contains("loop-done"), "completed run's worktree must be orphan");
    assert!(!orphan_ids.contains("loop-live"), "live run's worktree must NOT be orphan");
}

#[test]
fn unknown_runs_are_orphans() {
    // A worktree whose run-id doesn't appear under .hew/loop/ at all
    // (e.g. project moved, run-dir manually deleted) is unconditionally
    // an orphan: no live process can be driving it.
    let project = tempfile::tempdir().unwrap();
    let wt = tempfile::tempdir().unwrap();
    let wt_root = wt.path();

    plant_worktree(wt_root, "loop-unknown", 2);

    let loop_root = project.path().join(LOOP_ROOT);
    let active = active_run_ids(&loop_root).unwrap();
    assert!(active.is_empty());

    let orphans = list_orphans(wt_root, &active).unwrap();
    assert_eq!(orphans.len(), 1);
    assert_eq!(orphans[0].run_id, "loop-unknown");
    assert_eq!(orphans[0].worker_n, 2);
    assert_eq!(orphans[0].branch, branch_name("loop-unknown", 2));
}
