//! Non-interactive mode detection contract.
//!
//! `update` is still stub-only (tracked separately) — a stable target for
//! exercising global flag plumbing without depending on bd / runtime
//! detection / disk state.

use assert_cmd::Command;
use predicates::str::contains;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c.env_remove("HEW_NON_INTERACTIVE");
    c
}

#[test]
fn non_interactive_flag_accepted_globally() {
    hew()
        .args(["--non-interactive", "update"])
        .assert()
        .failure()
        .stderr(contains("not yet implemented"));
}

#[test]
fn json_flag_accepted_globally() {
    hew().args(["--json", "update"]).assert().failure();
}

#[test]
fn quiet_flag_accepted_globally() {
    hew().args(["--quiet", "update"]).assert().failure();
}

#[test]
fn verbose_count_accepted() {
    hew().args(["-vv", "update"]).assert().failure();
}

#[test]
fn ci_env_does_not_break_invocation() {
    hew().env("CI", "true").args(["update"]).assert().failure();
}

#[test]
fn hew_non_interactive_env_does_not_break_invocation() {
    hew().env("HEW_NON_INTERACTIVE", "1").args(["update"]).assert().failure();
}
