//! `hew ready` and `hew next` end-to-end via PATH-stubbed bd + git.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

const READY_BODY: &str = r#"[{"id":"h-1","title":"First task","priority":1,"status":"open","issue_type":"task"},{"id":"h-2","title":"Second feature","priority":2,"status":"open","issue_type":"feature"},{"id":"h-3","title":"Bug to fix","priority":2,"status":"open","issue_type":"bug"}]"#;

const EMPTY_BODY: &str = "[]";

const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
case "$1" in
  ready) printf '%s' "$BD_STUB_READY_BODY" ;;
  update|close|note|reopen|q|forget|dep) exit 0 ;;
  *) exit 0 ;;
esac
"#;

fn write_bd_stub(dir: &std::path::Path) {
    hew_core::testing::install_executable_stub(dir, "bd", STUB).unwrap();
}

fn write_git_stub(dir: &std::path::Path, log: &std::path::Path) {
    let stub = format!("#!/bin/sh\necho \"$@\" >> {}\nexit 0\n", log.display());
    hew_core::testing::install_executable_stub(dir, "git", &stub).unwrap();
}

fn hew_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("PATH", dir);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

// ─── hew ready ──────────────────────────────────────────────────────────

#[test]
fn ready_text_lists_all() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .args(["ready"])
        .assert()
        .success()
        .stdout(contains("h-1"))
        .stdout(contains("First task"))
        .stdout(contains("h-2"))
        .stdout(contains("h-3"))
        .stdout(contains("3 ready task(s)"));
}

#[test]
fn ready_empty_text() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", EMPTY_BODY)
        .args(["ready"])
        .assert()
        .success()
        .stdout(contains("(no ready tasks)"));
}

#[test]
fn ready_json_returns_array() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    let out = hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .args(["ready", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v.as_array().unwrap().len(), 3);
    assert_eq!(v[0]["id"], "h-1");
}

#[test]
fn ready_truncates_with_n() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .args(["ready", "--n", "2"])
        .assert()
        .success()
        .stdout(contains("h-1"))
        .stdout(contains("h-2"))
        .stdout(contains("2 ready task(s)").and(contains("h-3").not()));
}

// ─── hew next ───────────────────────────────────────────────────────────

#[test]
fn next_claims_top_and_prints() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("bd.log");

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .env("BD_STUB_LOG", &log)
        .args(["next"])
        .assert()
        .success()
        .stdout(contains("claimed h-1"))
        .stdout(contains("First task"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("update h-1 --claim"), "expected claim call, got:\n{recorded}");
}

#[test]
fn next_no_claim_skips_update() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("bd.log");

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .env("BD_STUB_LOG", &log)
        .args(["next", "--no-claim"])
        .assert()
        .success()
        .stdout(contains("next h-1"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(!recorded.contains("--claim"), "should not have claimed, got:\n{recorded}");
}

#[test]
fn next_empty_queue_text() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", EMPTY_BODY)
        .args(["next"])
        .assert()
        .success()
        .stdout(contains("(no ready tasks)"));
}

#[test]
fn next_empty_queue_json_returns_null_task() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    let out = hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", EMPTY_BODY)
        .args(["next", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(v["task"].is_null());
    assert_eq!(v["claimed"], false);
    assert!(v["branch"].is_null());
}

#[test]
fn next_branch_creates_branch_from_issue_type() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let git_log = tmp.path().join("git.log");
    write_git_stub(tmp.path(), &git_log);

    // Top entry is h-1 (issue_type=task → prefix "feat", title "First task" → "first-task").
    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .args(["next", "--branch"])
        .assert()
        .success()
        .stdout(contains("claimed h-1"))
        .stdout(contains("created branch feat/first-task"));

    let git_recorded = fs::read_to_string(&git_log).unwrap();
    assert!(
        git_recorded.contains("checkout -b feat/first-task"),
        "expected branch creation; got:\n{git_recorded}"
    );
}

#[test]
fn next_branch_respects_prefix_and_slug_overrides() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let git_log = tmp.path().join("git.log");
    write_git_stub(tmp.path(), &git_log);

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .args(["next", "--branch", "--prefix", "fix", "--slug", "explicit slug"])
        .assert()
        .success()
        .stdout(contains("created branch fix/explicit-slug"));
}

#[test]
fn next_branch_skipped_when_no_claim() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let git_log = tmp.path().join("git.log");
    write_git_stub(tmp.path(), &git_log);

    hew_in(tmp.path())
        .env("BD_STUB_READY_BODY", READY_BODY)
        .args(["next", "--no-claim", "--branch"])
        .assert()
        .success()
        .stdout(contains("next h-1").and(contains("created branch").not()));

    // git stub should never have been called.
    assert!(!git_log.exists() || fs::read_to_string(&git_log).unwrap().is_empty());
}
