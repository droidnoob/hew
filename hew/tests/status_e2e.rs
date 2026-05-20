//! `hew status` end-to-end against a stub `bd`.

use assert_cmd::Command;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "bd version 1.2.3"; exit 0 ;;
  ready) echo '[{"id":"t-1","title":"first","priority":0,"status":"open","issue_type":"task"},{"id":"t-2","title":"second","priority":1,"status":"open","issue_type":"task"}]'; exit 0 ;;
  stats) echo '{"schema_version":1,"summary":{"total_issues":10,"open_issues":7,"closed_issues":3,"ready_issues":2,"blocked_issues":5,"in_progress_issues":1}}'; exit 0 ;;
  memories) echo '{"a":"CONVENTION:errors — wrap","b":"CONVENTION:services — DI","c":"STATUS:plan:complete — 2026-05-11T15:00:00","d":"BOUNDARY: POST /users"}'; exit 0 ;;
esac
exit 2
"#;

fn make_stub() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    hew_core::testing::install_executable_stub(tmp.path(), "bd", STUB).unwrap();
    tmp
}

fn hew(stub: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("PATH", stub);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c
}

#[test]
fn status_renders_human_text_by_default() {
    let stub = make_stub();
    hew(stub.path())
        .arg("status")
        .assert()
        .success()
        .stdout(contains("hew status"))
        .stdout(contains("Phases"))
        .stdout(contains("Tasks"))
        .stdout(contains("Memories"))
        .stdout(contains("v1.2.3"))
        .stdout(contains("10 total"))
        .stdout(contains("✓ plan"))
        .stdout(contains("○ scan"));
}

#[test]
fn status_json_emits_valid_json() {
    let stub = make_stub();
    let out =
        hew(stub.path()).args(["--json", "status"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["bd_version"], "1.2.3");
    assert_eq!(parsed["tasks"]["total"], 10);
    assert_eq!(parsed["memories"]["conventions"], 2);
}

#[test]
fn status_errors_when_bd_missing() {
    let empty = tempfile::tempdir().unwrap();
    hew(empty.path()).arg("status").assert().failure().stderr(contains("`bd` binary not found"));
}
