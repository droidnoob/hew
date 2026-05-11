//! Non-interactive mode detection contract.
//!
//! `config` is a stable test target — it always errors with
//! "not yet implemented" regardless of bd state or runtime detection,
//! so it exercises global flag plumbing in isolation.

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
        .args(["--non-interactive", "config", "list"])
        .assert()
        .failure()
        .stderr(contains("not yet implemented"));
}

#[test]
fn json_flag_accepted_globally() {
    hew().args(["--json", "config", "list"]).assert().failure();
}

#[test]
fn quiet_flag_accepted_globally() {
    hew().args(["--quiet", "config", "list"]).assert().failure();
}

#[test]
fn verbose_count_accepted() {
    hew().args(["-vv", "config", "list"]).assert().failure();
}

#[test]
fn ci_env_does_not_break_invocation() {
    hew().env("CI", "true").args(["config", "list"]).assert().failure();
}

#[test]
fn hew_non_interactive_env_does_not_break_invocation() {
    hew().env("HEW_NON_INTERACTIVE", "1").args(["config", "list"]).assert().failure();
}
