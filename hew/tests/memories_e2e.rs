//! `hew memories --recall/--forget` end-to-end. The existing
//! prefix/grep/research filter paths are covered indirectly by other
//! e2e tests that exercise them via real bd; this file focuses on the
//! new single-key recall/forget surface.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
verb="$1"
case "$verb" in
    recall)
        if [ -n "$BD_STUB_RECALL_BODY" ]; then
            printf '%s' "$BD_STUB_RECALL_BODY"
            exit 0
        else
            printf 'No memory with key "%s"\n' "$2" >&2
            exit 1
        fi
        ;;
    forget)
        exit 0
        ;;
    memories)
        # `bd memories --json` returns an object. The dispatch in
        # hew::commands::memories pulls this path only when neither
        # --recall nor --forget is set.
        printf '{}'
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
fn recall_prints_body_only() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .env("BD_STUB_RECALL_BODY", "CONVENTION:hello there")
        .args(["memories", "--recall", "some-key"])
        .assert()
        .success()
        .stdout(contains("CONVENTION:hello there"))
        // no "(N memories)" footer
        .stdout(predicates::str::contains("memories\n").not());
}

#[test]
fn recall_missing_key_fails_clearly() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        // BD_STUB_RECALL_BODY unset → stub emits the "No memory with key" stderr.
        .args(["memories", "--recall", "absent"])
        .assert()
        .failure()
        .stderr(contains("no memory with key"));
}

#[test]
fn forget_invokes_bd_forget() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["memories", "--forget", "scratch"])
        .assert()
        .success()
        .stdout(contains("forgot scratch"));

    assert!(fs::read_to_string(&log).unwrap().contains("forget scratch"));
}

#[test]
fn recall_and_prefix_are_mutually_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["memories", "--recall", "k", "--prefix", "CONVENTION"])
        .assert()
        .failure();
}

#[test]
fn recall_and_forget_are_mutually_exclusive() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path()).args(["memories", "--recall", "k", "--forget", "k"]).assert().failure();
}
