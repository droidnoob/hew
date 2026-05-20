//! `hew task <verb>` end-to-end via a PATH-stubbed bd binary.
//!
//! The stub script dispatches on the first arg (`show`, `list`, ...) and
//! either writes the matching `BD_STUB_<VERB>_BODY` env var to stdout or
//! records its argv to `BD_STUB_LOG`. Sub-dispatch (`dep add`, `dep tree`)
//! uses the second arg.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
# Record argv for assertions.
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi

verb="$1"
case "$verb" in
    show)
        printf '%s' "$BD_STUB_SHOW_BODY"
        ;;
    list)
        printf '%s' "$BD_STUB_LIST_BODY"
        ;;
    children)
        printf '%s' "$BD_STUB_CHILDREN_BODY"
        ;;
    search)
        printf '%s' "$BD_STUB_SEARCH_BODY"
        ;;
    q)
        printf '%s\n' "$BD_STUB_Q_BODY"
        ;;
    update|close|reopen|note|forget|dep|recall|acceptance)
        # Side-effect verbs — empty stdout, log captures argv.
        exit 0
        ;;
    *)
        echo "stub: unhandled verb $verb" >&2
        exit 1
        ;;
esac
"#;

fn write_bd_stub(dir: &std::path::Path) {
    hew_core::testing::install_executable_stub(dir, "bd", STUB).unwrap();
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

fn issue_json(id: &str, title: &str, status: &str, closed_at: &str) -> String {
    format!(
        r#"{{"id":"{id}","title":"{title}","description":"body for {id}","status":"{status}","priority":2,"issue_type":"task","closed_at":"{closed_at}","close_reason":null,"parent":null}}"#
    )
}

// ─── show ───────────────────────────────────────────────────────────────

#[test]
fn show_text_includes_title_and_status() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!("[{}]", issue_json("hew-1", "do the thing", "open", ""));

    hew_in(tmp.path())
        .env("BD_STUB_SHOW_BODY", &body)
        .args(["task", "show", "hew-1"])
        .assert()
        .success()
        .stdout(contains("hew-1"))
        .stdout(contains("do the thing"))
        .stdout(contains("status:"))
        .stdout(contains("open"))
        .stdout(contains("body for hew-1"));
}

#[test]
fn show_appends_children_section_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!("[{}]", issue_json("hew-epic", "Epic L3", "open", ""));
    let kids = format!(
        "[{},{}]",
        issue_json("hew-epic.1", "L3.1 first slice", "closed", "2026-05-12T00:00:00Z"),
        issue_json("hew-epic.2", "L3.2 second slice", "open", ""),
    );

    hew_in(tmp.path())
        .env("BD_STUB_SHOW_BODY", &body)
        .env("BD_STUB_CHILDREN_BODY", &kids)
        .args(["task", "show", "hew-epic"])
        .assert()
        .success()
        .stdout(contains("CHILDREN (1/2 complete)"))
        .stdout(contains("hew-epic.1"))
        .stdout(contains("L3.1 first slice"))
        .stdout(contains("hew-epic.2"))
        .stdout(contains("L3.2 second slice"));
}

#[test]
fn show_no_children_flag_suppresses_section_even_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!("[{}]", issue_json("hew-epic", "Epic", "open", ""));
    let kids = format!("[{}]", issue_json("hew-epic.1", "child", "open", ""));

    hew_in(tmp.path())
        .env("BD_STUB_SHOW_BODY", &body)
        .env("BD_STUB_CHILDREN_BODY", &kids)
        .args(["task", "show", "hew-epic", "--no-children"])
        .assert()
        .success()
        .stdout(contains("hew-epic"))
        .stdout(predicates::str::contains("CHILDREN").not());
}

#[test]
fn show_json_includes_children_array_when_present() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!("[{}]", issue_json("hew-epic", "Epic", "open", ""));
    let kids = format!("[{}]", issue_json("hew-epic.1", "child", "open", ""));

    let out = hew_in(tmp.path())
        .env("BD_STUB_SHOW_BODY", &body)
        .env("BD_STUB_CHILDREN_BODY", &kids)
        .args(["task", "show", "hew-epic", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed["id"], "hew-epic");
    let children = parsed["children"].as_array().expect("children array present");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0]["id"], "hew-epic.1");
}

#[test]
fn show_json_round_trips_through_task_summary() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!("[{}]", issue_json("hew-1", "x", "closed", "2026-05-12T00:00:00Z"));

    let out = hew_in(tmp.path())
        .env("BD_STUB_SHOW_BODY", &body)
        .args(["task", "show", "hew-1", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed["id"], "hew-1");
    assert_eq!(parsed["status"], "closed");
    assert_eq!(parsed["issue_type"], "task");
}

// ─── list ───────────────────────────────────────────────────────────────

#[test]
fn list_passes_filters_to_bd() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_LIST_BODY", "[]")
        .args([
            "task",
            "list",
            "--status",
            "open,in_progress",
            "--type",
            "task",
            "--parent",
            "hew-4az",
            "--n",
            "5",
        ])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("--status open,in_progress"), "{recorded}");
    assert!(recorded.contains("--type task"), "{recorded}");
    assert!(recorded.contains("--parent hew-4az"), "{recorded}");
    assert!(recorded.contains("--limit 5"), "{recorded}");
    assert!(!recorded.contains("--reverse"), "{recorded}");
}

#[test]
fn list_head_flips_to_oldest_first() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_LIST_BODY", "[]")
        .args(["task", "list", "--head"])
        .assert()
        .success();

    assert!(fs::read_to_string(&log).unwrap().contains("--reverse"));
}

#[test]
fn list_renders_rows_in_text_mode() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!(
        "[{},{}]",
        issue_json("hew-1", "first task", "open", ""),
        issue_json("hew-2", "second task", "closed", "2026-05-12T00:00:00Z"),
    );

    hew_in(tmp.path())
        .env("BD_STUB_LIST_BODY", &body)
        .args(["task", "list"])
        .assert()
        .success()
        .stdout(contains("first task"))
        .stdout(contains("second task"))
        .stdout(contains("2 task(s)"));
}

// ─── claim / close / reopen ─────────────────────────────────────────────

#[test]
fn claim_sends_update_claim_flag() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");
    let body = format!("[{}]", issue_json("hew-1", "title", "in_progress", ""));

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_SHOW_BODY", &body)
        .args(["task", "claim", "hew-1"])
        .assert()
        .success()
        .stdout(contains("claimed hew-1"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.lines().any(|l| l == "update hew-1 --claim"), "{recorded}");
}

#[test]
fn close_passes_reason_verbatim() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["task", "close", "hew-1", "--reason", "shipped via abc123"])
        .assert()
        .success()
        .stdout(contains("closed hew-1"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("close hew-1 -r shipped via abc123"), "{recorded}");
}

#[test]
fn close_with_rule_tag_prepends_to_reason() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["task", "close", "hew-1", "--reason", "scope creep", "--type", "2"])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("[Rule 2] scope creep"), "{recorded}");
}

#[test]
fn reopen_sends_id_only_without_reason() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["task", "reopen", "hew-1"])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.trim(), "reopen hew-1");
}

// ─── new ────────────────────────────────────────────────────────────────

#[test]
fn new_captures_id_from_bd_q() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .env("BD_STUB_Q_BODY", "hew-9zz")
        .args(["task", "new", "--title", "Add login", "--type", "task"])
        .assert()
        .success()
        .stdout(contains("hew-9zz"));
}

#[test]
fn new_with_description_calls_update() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_Q_BODY", "hew-9zz")
        .args(["task", "new", "--title", "Add login", "--description", "OAuth flow + cookies"])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("q Add login"), "{recorded}");
    assert!(recorded.contains("update hew-9zz --description OAuth flow + cookies"), "{recorded}");
}

#[test]
fn new_with_parent_chases_update() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_Q_BODY", "hew-9zz")
        .args(["task", "new", "--title", "Sub", "--parent", "hew-4az"])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("update hew-9zz --parent hew-4az"), "{recorded}");
}

// ─── children / note / search ───────────────────────────────────────────

#[test]
fn children_lists_one_level() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let body = format!("[{}]", issue_json("hew-1.1", "child", "open", ""));

    hew_in(tmp.path())
        .env("BD_STUB_CHILDREN_BODY", &body)
        .args(["task", "children", "hew-1"])
        .assert()
        .success()
        .stdout(contains("hew-1.1"))
        .stdout(contains("child"));
}

#[test]
fn note_sends_text() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["task", "note", "hew-1", "saw a flake"])
        .assert()
        .success()
        .stdout(contains("note added"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.trim(), "note hew-1 saw a flake");
}

#[test]
fn search_includes_status_all_and_json() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");
    let body = format!("[{}]", issue_json("hew-1", "auth bug", "open", ""));

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_SEARCH_BODY", &body)
        .args(["task", "search", "auth"])
        .assert()
        .success()
        .stdout(contains("auth bug"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("search auth"), "{recorded}");
    assert!(recorded.contains("--status all"), "{recorded}");
}

// ─── update ─────────────────────────────────────────────────────────────

#[test]
fn update_passes_title_and_description_to_bd() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args([
            "task",
            "update",
            "hew-1",
            "--title",
            "new title",
            "--description",
            "rewritten body",
        ])
        .assert()
        .success()
        .stdout(contains("updated hew-1"))
        .stdout(contains("2 fields"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("update hew-1"), "{recorded}");
    assert!(recorded.contains("--title new title"), "{recorded}");
    assert!(recorded.contains("--description rewritten body"), "{recorded}");
}

#[test]
fn update_with_description_file_routes_to_body_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");
    let spec = tmp.path().join("spec.md");
    fs::write(&spec, "new spec body").unwrap();

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["task", "update", "hew-1", "--description-file", spec.to_str().unwrap()])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("--body-file"), "{recorded}");
    assert!(recorded.contains(spec.to_str().unwrap()), "{recorded}");
}

#[test]
fn update_errors_when_no_fields_provided() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["task", "update", "hew-1"])
        .assert()
        .failure()
        .stderr(contains("no fields to update"));
}

#[test]
fn update_rejects_description_and_description_file_together() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["task", "update", "hew-1", "--description", "x", "--description-file", "/tmp/y"])
        .assert()
        .failure();
}

// ─── help ───────────────────────────────────────────────────────────────

#[test]
fn task_help_lists_all_verbs() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["task", "--help"])
        .assert()
        .success()
        .stdout(contains("show"))
        .stdout(contains("list"))
        .stdout(contains("claim"))
        .stdout(contains("close"))
        .stdout(contains("new"))
        .stdout(contains("reopen"))
        .stdout(contains("children"))
        .stdout(contains("note"))
        .stdout(contains("search"))
        .stdout(contains("update"));
}
