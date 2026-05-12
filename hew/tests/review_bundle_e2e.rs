//! `hew review-bundle` end-to-end against PATH-stubbed bd + git.
//!
//! The stub bd recognises only the calls review-bundle actually makes:
//! `bd list --status=closed --sort=closed --limit 0 --json`,
//! `bd show <id> --json`, `bd memories --json`. Anything else echoes
//! "unhandled" to stderr and exits 2.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

// Stubs locate fixtures via HEW_STUB_DIR env var. PATH-lookup obscures $0,
// so we don't rely on the stub's own directory.

const BD_STUB: &str = r#"#!/bin/sh
DIR="$HEW_STUB_DIR"
case "$1" in
  list)
    /bin/cat "$DIR/list-closed.json"
    exit 0
    ;;
  show)
    if [ -f "$DIR/show-$2.json" ]; then
      /bin/cat "$DIR/show-$2.json"
      exit 0
    fi
    echo "stub: no fixture for show $2" >&2
    exit 1
    ;;
  memories)
    if [ -f "$DIR/memories.json" ]; then
      /bin/cat "$DIR/memories.json"
    else
      echo "{}"
    fi
    exit 0
    ;;
  remember)
    echo "$2" >> "$DIR/remembered.log"
    exit 0
    ;;
  *)
    echo "stub bd: unhandled: $*" >&2
    exit 2
    ;;
esac
"#;

const GIT_STUB: &str = r#"#!/bin/sh
DIR="$HEW_STUB_DIR"
case "$1" in
  rev-list)
    if [ -f "$DIR/rev-list.out" ]; then
      /bin/cat "$DIR/rev-list.out"
    fi
    exit 0
    ;;
  diff)
    if [ -f "$DIR/diff.out" ]; then
      /bin/cat "$DIR/diff.out"
    fi
    exit 0
    ;;
  rev-parse)
    if [ -f "$DIR/rev-parse-ok" ]; then
      ACCEPT="$(/bin/cat "$DIR/rev-parse-ok")"
      if [ "$3" = "$ACCEPT" ]; then
        exit 0
      fi
    fi
    exit 128
    ;;
  *)
    exit 0
    ;;
esac
"#;

fn install_stub(dir: &Path, name: &str, script: &str) {
    let path = dir.join(name);
    fs::write(&path, script).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn write_fixture(dir: &Path, name: &str, body: &str) {
    fs::write(dir.join(name), body).unwrap();
}

fn hew(stub_dir: &Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("PATH", stub_dir);
    c.env("HEW_STUB_DIR", stub_dir);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    // Force a known config so review.batch_size is predictable.
    c.env("HEW_CONFIG", stub_dir.join("config.toml"));
    c
}

fn three_closed_tasks_newest_first() -> &'static str {
    r#"[
        {"id":"t-3","title":"third","status":"closed","priority":2,"issue_type":"task","closed_at":"2026-05-12T13:00:00Z","close_reason":"r3","parent":null},
        {"id":"t-2","title":"second","status":"closed","priority":2,"issue_type":"task","closed_at":"2026-05-12T12:00:00Z","close_reason":"r2","parent":null},
        {"id":"t-1","title":"first","status":"closed","priority":2,"issue_type":"task","closed_at":"2026-05-12T11:00:00Z","close_reason":"r1","parent":null}
    ]"#
}

#[test]
fn default_scope_uses_batch_size_from_config() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", three_closed_tasks_newest_first());
    write_fixture(dir.path(), "memories.json", "{}");
    write_fixture(dir.path(), "rev-list.out", "abc123\n");
    write_fixture(dir.path(), "diff.out", "(stub diff body)\n");
    // batch_size = 2
    write_fixture(
        dir.path(),
        "config.toml",
        "update_check = true\n[review]\nafter_n_tasks = 0\nafter_epic = false\nbatch_size = 2\n",
    );

    let out = hew(dir.path()).arg("review-bundle").assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["scope"]["kind"], "last_n");
    assert_eq!(v["scope"]["n"], 2);
    assert_eq!(v["closed_tasks"].as_array().unwrap().len(), 2);
    // Oldest first: t-2 then t-3.
    assert_eq!(v["closed_tasks"][0]["id"], "t-2");
    assert_eq!(v["closed_tasks"][1]["id"], "t-3");
    assert_eq!(v["diff_base"], "abc123");
}

#[test]
fn n_flag_overrides_config() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", three_closed_tasks_newest_first());
    write_fixture(dir.path(), "memories.json", "{}");

    hew(dir.path())
        .args(["review-bundle", "--n", "1"])
        .assert()
        .success()
        .stdout(contains("\"n\": 1"));
}

#[test]
fn since_epic_id_is_classified_as_epic_scope() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", three_closed_tasks_newest_first());
    write_fixture(dir.path(), "memories.json", "{}");
    write_fixture(
        dir.path(),
        "show-e-1.json",
        r#"[{"id":"e-1","title":"epic-1","description":"body","status":"closed","issue_type":"epic","closed_at":"2026-05-12T14:00:00Z"}]"#,
    );

    hew(dir.path())
        .args(["review-bundle", "--since", "e-1"])
        .assert()
        .success()
        .stdout(contains("\"kind\": \"epic\""))
        .stdout(contains("\"id\": \"e-1\""));
}

#[test]
fn since_task_id_is_classified_as_task_scope() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", three_closed_tasks_newest_first());
    write_fixture(dir.path(), "memories.json", "{}");
    write_fixture(
        dir.path(),
        "show-t-2.json",
        r#"[{"id":"t-2","title":"second","status":"closed","issue_type":"task","closed_at":"2026-05-12T12:00:00Z"}]"#,
    );

    let out = hew(dir.path())
        .args(["review-bundle", "--since", "t-2"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["scope"]["kind"], "task");
    assert_eq!(v["scope"]["id"], "t-2");
    // closed_at >= 12:00 → t-2, t-3 (oldest first).
    let ids: Vec<&str> =
        v["closed_tasks"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(ids, vec!["t-2", "t-3"]);
}

#[test]
fn since_git_ref_is_classified_as_git_scope() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    // bd show on the ref fails (no fixture); git rev-parse accepts.
    write_fixture(dir.path(), "rev-parse-ok", "HEAD~3");
    write_fixture(dir.path(), "diff.out", "(git diff body)");
    write_fixture(dir.path(), "memories.json", "{}");
    write_fixture(dir.path(), "list-closed.json", "[]");

    let out = hew(dir.path())
        .args(["review-bundle", "--since", "HEAD~3"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["scope"]["kind"], "git_ref");
    assert_eq!(v["scope"]["rev"], "HEAD~3");
    assert_eq!(v["diff_base"], "HEAD~3");
    // GitRef scope explicitly does not return closed_tasks.
    assert!(v["closed_tasks"].as_array().unwrap().is_empty());
}

#[test]
fn unknown_since_errors_with_clear_message() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", "[]");
    write_fixture(dir.path(), "memories.json", "{}");

    hew(dir.path())
        .args(["review-bundle", "--since", "totally-not-a-ref"])
        .assert()
        .failure()
        .stderr(contains("matches no bd issue"));
}

#[test]
fn memories_are_filtered_to_review_relevant_prefixes() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", three_closed_tasks_newest_first());
    write_fixture(
        dir.path(),
        "memories.json",
        r#"{
            "schema_version": 1,
            "k-c": "CONVENTION:foo — bar",
            "k-b": "BOUNDARY:auth — public",
            "k-s": "SECURITY:csrf — required",
            "k-d": "DECISION:other — irrelevant",
            "k-r": "STATUS:review:2026-05-12T11:30:00Z"
        }"#,
    );

    let out = hew(dir.path()).arg("review-bundle").assert().success().get_output().stdout.clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(v["memories"]["conventions"].as_array().unwrap().len(), 1);
    assert_eq!(v["memories"]["boundaries"].as_array().unwrap().len(), 1);
    assert_eq!(v["memories"]["security"].as_array().unwrap().len(), 1);
    assert_eq!(v["last_review_at"], "2026-05-12T11:30:00Z");
}

#[test]
fn json_output_is_valid() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);
    write_fixture(dir.path(), "list-closed.json", three_closed_tasks_newest_first());
    write_fixture(dir.path(), "memories.json", "{}");

    let out = hew(dir.path())
        .args(["review-bundle", "--n", "1"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    // Must parse as valid JSON.
    let _v: serde_json::Value = serde_json::from_slice(&out).unwrap();
}

#[test]
fn since_and_n_are_mutually_exclusive() {
    let dir = tempfile::tempdir().unwrap();
    install_stub(dir.path(), "bd", BD_STUB);
    install_stub(dir.path(), "git", GIT_STUB);

    hew(dir.path())
        .args(["review-bundle", "--since", "x", "--n", "3"])
        .assert()
        .failure()
        // clap's standard conflict message
        .stderr(contains("cannot be used with"));
}

#[test]
fn schema_review_bundle_emits_jsonschema() {
    // Standalone — no stubs needed for `hew schema`.
    let dir = tempfile::tempdir().unwrap();
    let out = hew(dir.path())
        .args(["schema", "review-bundle"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
    // JSON-Schema documents typically expose either `$schema` or `properties`.
    assert!(v.get("$schema").is_some() || v.get("properties").is_some());
}
