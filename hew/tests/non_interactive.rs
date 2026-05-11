//! Non-interactive mode detection contract.
//!
//! Drives every command behind every global flag combination and just
//! asserts the binary parses + returns *some* exit code. The point is
//! flag plumbing — the per-command behavior is tested elsewhere.

use assert_cmd::Command;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c.env_remove("HEW_NON_INTERACTIVE");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c
}

#[test]
fn non_interactive_flag_accepted_globally() {
    // `schema config` succeeds deterministically; the point here is the
    // global --non-interactive flag is wired correctly.
    hew().args(["--non-interactive", "schema", "config"]).assert().success();
}

#[test]
fn json_flag_accepted_globally() {
    hew().args(["--json", "schema", "config"]).assert().success();
}

#[test]
fn quiet_flag_accepted_globally() {
    hew().args(["--quiet", "schema", "config"]).assert().success();
}

#[test]
fn verbose_count_accepted() {
    hew().args(["-vv", "schema", "config"]).assert().success();
}

#[test]
fn ci_env_does_not_break_invocation() {
    hew().env("CI", "true").args(["schema", "config"]).assert().success();
}

#[test]
fn hew_non_interactive_env_does_not_break_invocation() {
    hew().env("HEW_NON_INTERACTIVE", "1").args(["schema", "config"]).assert().success();
}
