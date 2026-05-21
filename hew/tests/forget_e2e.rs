//! `hew forget <KEY>` top-level subcommand. Today it's an ergonomic
//! alias for `hew memories --forget <KEY>` — ML.6 (hew-jem) will
//! extend the same surface with cascade-delete of outbound LINK: rows.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
verb="$1"
case "$verb" in
    forget) exit 0 ;;
    *)      exit 0 ;;
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
fn forget_invokes_bd_forget_and_prints_confirmation() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .args(["forget", "convention-cli-output"])
        .assert()
        .success()
        .stdout(contains("forgot convention-cli-output"));

    assert!(
        fs::read_to_string(&log).unwrap().contains("forget convention-cli-output"),
        "bd forget verb missing from stub log"
    );
}

#[test]
fn forget_requires_a_key_argument() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path()).args(["forget"]).assert().failure();
}

#[test]
fn forget_quiet_suppresses_confirmation() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    let out = hew_in(tmp.path())
        .args(["--quiet", "forget", "some-key"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    assert!(out.is_empty(), "quiet mode must suppress stdout, got: {out:?}");
}
