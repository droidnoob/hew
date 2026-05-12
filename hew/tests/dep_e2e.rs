//! `hew dep <verb>` end-to-end via a PATH-stubbed bd binary.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
verb="$1"
sub="$2"
case "$verb" in
    dep)
        if [ "$sub" = "tree" ]; then
            printf '%s' "$BD_STUB_TREE_BODY"
        fi
        exit 0
        ;;
    list)
        printf '%s' "$BD_STUB_LIST_BODY"
        ;;
    *)
        exit 0
        ;;
esac
"#;

fn write_bd_stub(dir: &std::path::Path) {
    let path = dir.join("bd");
    fs::write(&path, STUB).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
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

#[test]
fn add_sends_dep_add_argv() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["dep", "add", "hew-1", "--on", "hew-2"])
        .assert()
        .success()
        .stdout(contains("hew-1 now depends on hew-2"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("dep add hew-1 hew-2"), "{recorded}");
}

#[test]
fn add_requires_on_flag() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path()).args(["dep", "add", "hew-1"]).assert().failure();
}

#[test]
fn remove_sends_dep_remove_argv() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["dep", "remove", "hew-1", "hew-2"])
        .assert()
        .success();

    assert!(fs::read_to_string(&log).unwrap().contains("dep remove hew-1 hew-2"));
}

#[test]
fn tree_text_renders_indented_nodes() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = r#"{"id":"hew-1","title":"root","status":"in_progress","children":[
        {"id":"hew-2","title":"child","status":"open","children":[]}
    ]}"#;

    hew_in(tmp.path())
        .env("BD_STUB_TREE_BODY", body)
        .args(["dep", "tree", "hew-1"])
        .assert()
        .success()
        .stdout(contains("hew-1"))
        .stdout(contains("root"))
        .stdout(contains("hew-2"))
        .stdout(contains("child"));
}

#[test]
fn tree_depth_truncates_children_in_json_mode() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = r#"{"id":"a","children":[
        {"id":"b","children":[
            {"id":"c","children":[]}
        ]}
    ]}"#;

    let out = hew_in(tmp.path())
        .env("BD_STUB_TREE_BODY", body)
        .args(["dep", "tree", "a", "--depth", "2", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let b = &parsed["children"][0];
    assert_eq!(b["id"], "b");
    assert!(b["children"].as_array().unwrap().is_empty(), "{}", parsed);
}

#[test]
fn blocked_lists_status_blocked_tasks() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");
    let body = r#"[{"id":"hew-1","title":"stuck","status":"blocked","priority":2,"issue_type":"task","closed_at":""}]"#;

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_LIST_BODY", body)
        .args(["dep", "blocked"])
        .assert()
        .success()
        .stdout(contains("stuck"))
        .stdout(contains("1 blocked"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("--status blocked"), "{recorded}");
}

#[test]
fn dep_help_lists_all_verbs() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["dep", "--help"])
        .assert()
        .success()
        .stdout(contains("add"))
        .stdout(contains("remove"))
        .stdout(contains("tree"))
        .stdout(contains("blocked"));
}
