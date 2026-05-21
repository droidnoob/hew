//! `hew remember` end-to-end via a PATH-stubbed bd binary.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
verb="$1"
case "$verb" in
    remember|forget)
        exit 0
        ;;
    recall)
        # Used by --recall tests; echoes the stub body.
        printf '%s' "$BD_STUB_RECALL_BODY"
        ;;
    *)
        exit 0
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

#[test]
fn type_convention_prepends_upper_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["remember", "--type", "convention", "tabs not spaces"])
        .assert()
        .success()
        .stdout(contains("remembered"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("remember CONVENTION:tabs not spaces"), "{recorded}");
}

#[test]
fn type_accepts_mixed_case_and_normalises() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["remember", "--type", "Decision", "use opus 4.7"])
        .assert()
        .success();

    assert!(fs::read_to_string(&log).unwrap().contains("DECISION:use opus 4.7"));
}

#[test]
fn type_rejects_unknown_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["remember", "--type", "review", "body here"])
        .assert()
        .failure()
        .stderr(contains("review"));
}

#[test]
fn body_with_known_prefix_is_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["remember", "--type", "convention", "CONVENTION:already prefixed"])
        .assert()
        .failure()
        .stderr(contains("already starts with"));
}

#[test]
fn raw_skips_validation() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["remember", "--raw", "WEIRD:custom prefix"])
        .assert()
        .success();

    assert!(fs::read_to_string(&log).unwrap().contains("remember WEIRD:custom prefix"));
}

#[test]
fn key_is_passed_through() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["remember", "--type", "status", "scan done", "--key", "scan-marker"])
        .assert()
        .success()
        .stdout(contains("scan-marker"));

    assert!(
        fs::read_to_string(&log).unwrap().contains("remember STATUS:scan done --key scan-marker")
    );
}

#[test]
fn missing_type_without_raw_errors() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path()).args(["remember", "bare body no type"]).assert().failure();
}

// ──────── --related / --related-task sidecars (hew-utn / ML.3) ────────

#[test]
fn related_emits_memory_link_sidecars() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args([
            "remember",
            "--type",
            "gotcha",
            "lesson body",
            "--key",
            "foo",
            "--related",
            "bar",
            "--related",
            "baz",
        ])
        .assert()
        .success()
        .stdout(contains("emitted 2 LINK: sidecar memories"));

    let log_contents = fs::read_to_string(&log).unwrap();
    // Primary write happens FIRST per DECISION:compact-safety.
    let primary_line = log_contents
        .lines()
        .position(|l| l.contains("remember GOTCHA:lesson body --key foo"))
        .expect("primary write missing from log");
    let bar_line = log_contents
        .lines()
        .position(|l| l.contains("remember LINK:foo->relates_to:memory:bar"))
        .expect("bar LINK row missing");
    let baz_line = log_contents
        .lines()
        .position(|l| l.contains("remember LINK:foo->relates_to:memory:baz"))
        .expect("baz LINK row missing");
    assert!(primary_line < bar_line, "primary must precede LINK sidecars");
    assert!(primary_line < baz_line);
}

#[test]
fn related_task_emits_task_link_sidecar() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args([
            "remember",
            "--type",
            "decision",
            "do the thing",
            "--key",
            "decision-thing",
            "--related-task",
            "hew-abc.1",
        ])
        .assert()
        .success();

    assert!(
        fs::read_to_string(&log)
            .unwrap()
            .contains("remember LINK:decision-thing->relates_to:task:hew-abc.1"),
        "task LINK row missing"
    );
}

#[test]
fn related_mixed_with_related_task_in_one_invocation() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args([
            "remember",
            "--type",
            "convention",
            "rule body",
            "--key",
            "convention-rule",
            "--related",
            "convention-other",
            "--related-task",
            "hew-xyz",
        ])
        .assert()
        .success()
        .stdout(contains("emitted 2 LINK: sidecar memories"));

    let log_contents = fs::read_to_string(&log).unwrap();
    assert!(log_contents.contains("LINK:convention-rule->relates_to:memory:convention-other"));
    assert!(log_contents.contains("LINK:convention-rule->relates_to:task:hew-xyz"));
}

#[test]
fn related_requires_explicit_key() {
    // clap `requires = "key"` should reject --related when --key is
    // absent. Without --key, the `<from>` side of the LINK row would
    // be whatever slug bd auto-derives — non-deterministic and not
    // useful as an edge target.
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["remember", "--type", "gotcha", "body", "--related", "some-key"])
        .assert()
        .failure();
}

#[test]
fn related_rejects_uppercase_target() {
    // Front-door validation should fail BEFORE the primary write
    // succeeds — turning "missing edges" into a clean error.
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["remember", "--type", "gotcha", "body", "--key", "foo", "--related", "BAD-KEY"])
        .assert()
        .failure();

    // Log should be empty — primary write never fired.
    let log_contents = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !log_contents.contains("remember GOTCHA:body"),
        "primary write fired despite invalid --related: {log_contents}"
    );
}

#[test]
fn related_rejects_empty_string_target() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["remember", "--type", "gotcha", "body", "--key", "foo", "--related", ""])
        .assert()
        .failure();
}
