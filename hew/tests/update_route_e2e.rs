//! `hew update` install-source routing.
//!
//! `HEW_INSTALL_SOURCE` is the testable override; we put stub `brew` /
//! `cargo` / `hew` binaries on PATH and assert they were invoked
//! (or not) with the expected argv.

use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

fn write_stub(dir: &std::path::Path, name: &str, log: &std::path::Path) {
    // Record argv to `log`, exit 0.
    let body = format!("#!/bin/sh\necho \"{name}: $@\" >> {}\nexit 0\n", log.display());
    hew_core::testing::install_executable_stub(dir, name, &body).unwrap();
}

fn write_failing_stub(dir: &std::path::Path, name: &str, code: i32) {
    let body = format!("#!/bin/sh\nexit {code}\n");
    hew_core::testing::install_executable_stub(dir, name, &body).unwrap();
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
fn brew_source_shells_out_to_brew_upgrade() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("calls.log");

    write_stub(tmp.path(), "brew", &log);
    // Stub `hew` so the post-upgrade re-exec records its argv too.
    write_stub(tmp.path(), "hew", &log);

    hew_in(tmp.path())
        .env("HEW_INSTALL_SOURCE", "brew")
        .args(["update", "--no-refresh"])
        .assert()
        .success()
        .stderr(contains("detected install source = brew"))
        .stdout(contains("brew upgrade hew"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("brew: upgrade hew"), "expected brew call; got:\n{recorded}");
    // --no-refresh should suppress the re-exec into the hew stub.
    assert!(
        !recorded.contains("hew: update --local"),
        "should not have re-exec'd; got:\n{recorded}"
    );
}

#[test]
fn cargo_source_shells_out_to_cargo_install() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("calls.log");

    write_stub(tmp.path(), "cargo", &log);

    hew_in(tmp.path())
        .env("HEW_INSTALL_SOURCE", "cargo")
        .args(["update", "--no-refresh"])
        .assert()
        .success()
        .stderr(contains("detected install source = cargo"))
        .stdout(contains("cargo install"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(
        recorded.contains("cargo: install --git https://github.com/droidnoob/hew hew --force"),
        "expected cargo invocation; got:\n{recorded}"
    );
}

#[test]
fn dev_source_refuses_to_self_upgrade() {
    let tmp = tempfile::tempdir().unwrap();

    hew_in(tmp.path())
        .env("HEW_INSTALL_SOURCE", "dev")
        .args(["update"])
        .assert()
        .failure()
        .stderr(contains("dev build").or(contains("target/")));
}

#[test]
fn brew_failure_surfaces_with_manual_hint() {
    let tmp = tempfile::tempdir().unwrap();
    write_failing_stub(tmp.path(), "brew", 1);

    hew_in(tmp.path())
        .env("HEW_INSTALL_SOURCE", "brew")
        .args(["update", "--no-refresh"])
        .assert()
        .failure()
        .stderr(contains("brew upgrade hew").and(contains("Install or upgrade manually")));
}

#[test]
fn auto_refreshes_skills_after_upgrade_when_runtime_present() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("calls.log");

    // Create a .claude/ marker so detect_runtimes sees Claude installed.
    fs::create_dir(tmp.path().join(".claude")).unwrap();

    write_stub(tmp.path(), "brew", &log);
    write_stub(tmp.path(), "hew", &log);

    hew_in(tmp.path())
        .current_dir(tmp.path())
        .env("HEW_INSTALL_SOURCE", "brew")
        .args(["update"])
        .assert()
        .success()
        .stdout(contains("refreshing project skill files"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(
        recorded.contains("hew: update --local"),
        "expected re-exec of `hew update --local`; got:\n{recorded}"
    );
}

#[test]
fn auto_refresh_skipped_when_no_runtime_in_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("calls.log");

    write_stub(tmp.path(), "brew", &log);
    write_stub(tmp.path(), "hew", &log);

    hew_in(tmp.path())
        .current_dir(tmp.path())
        .env("HEW_INSTALL_SOURCE", "brew")
        .args(["update"])
        .assert()
        .success()
        .stdout(contains("No runtime markers in cwd"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert!(
        !recorded.contains("hew: update --local"),
        "should not have re-exec'd; got:\n{recorded}"
    );
}
