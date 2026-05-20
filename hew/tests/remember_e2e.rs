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
