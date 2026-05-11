//! End-to-end: invoke the `hew` binary with PATH pointing at a fake `bd`.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "bd version 1.0.3"; exit 0 ;;
  ready) echo '[{"id":"x-1","title":"only","priority":0,"status":"open","issue_type":"task"}]'; exit 0 ;;
  stats) echo '{"schema_version":1,"summary":{"total_issues":1,"open_issues":1,"closed_issues":0,"ready_issues":1,"blocked_issues":0,"in_progress_issues":0}}'; exit 0 ;;
  memories) echo '{"k":"CONVENTION:errors — wrap"}'; exit 0 ;;
esac
exit 2
"#;

fn make_stub_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bd");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(STUB.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    tmp
}

#[test]
fn prime_emits_valid_json_to_stdout() {
    let stub_dir = make_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "execute"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("prime output must be valid JSON");

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["skill"], "hew-execute");
    assert_eq!(parsed["tasks"]["ready"], 1);
    assert_eq!(parsed["memories"]["conventions"].as_array().unwrap().len(), 1);
    assert!(parsed["skill_instructions"].as_str().unwrap().contains("hew-execute"));
}

#[test]
fn prime_pretty_flag_indents_output() {
    let stub_dir = make_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "plan", "--pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\n  "), "pretty output should be indented:\n{text}");
}

#[test]
fn prime_errors_when_bd_missing() {
    // Empty PATH → bd not found → miette diagnostic on stderr, exit 1.
    Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", "")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "execute"])
        .assert()
        .failure()
        .stderr(contains("`bd` binary not found"));
}

#[test]
fn prime_errors_on_unknown_skill() {
    let stub_dir = make_stub_dir();
    Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "definitely-not-a-skill"])
        .assert()
        .failure()
        .stderr(contains("definitely-not-a-skill"));
}
