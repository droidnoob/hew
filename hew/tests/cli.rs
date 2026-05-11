//! Integration tests for the `hew` binary.
//!
//! Heavy lifting will come once subcommands are implemented; this suite
//! locks the scaffold's CLI surface so future changes don't silently
//! regress it.

use assert_cmd::Command;
use predicates::str::contains;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    // Stable output in tests regardless of host terminal.
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env_remove("HEW_LOG");
    c
}

#[test]
fn version_prints() {
    hew()
        .arg("--version")
        .assert()
        .success()
        .stdout(contains("hew "))
        .stdout(contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_lists_subcommands() {
    hew()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("init"))
        .stdout(contains("prime"))
        .stdout(contains("status"))
        .stdout(contains("doctor"))
        .stdout(contains("config"))
        .stdout(contains("schema"))
        .stdout(contains("update"));
}

#[test]
fn help_shows_global_flags() {
    hew()
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("--non-interactive"))
        .stdout(contains("--json"))
        .stdout(contains("--quiet"))
        .stdout(contains("--output"));
}

#[test]
fn no_args_shows_help_nonzero() {
    // arg_required_else_help → exit code 2, prints usage to stderr.
    hew().assert().failure().code(2);
}

#[test]
fn unknown_subcommand_fails() {
    hew().arg("definitely-not-a-command").assert().failure().code(2);
}

#[test]
fn stub_init_returns_error() {
    // Stubs error cleanly via miette — exit 1, message on stderr.
    hew().arg("init").assert().failure().stderr(contains("not yet implemented"));
}

#[test]
fn stub_prime_requires_skill() {
    // Missing positional arg → clap exits 2.
    hew().arg("prime").assert().failure().code(2);
}

// `hew prime` is exercised end-to-end against a stub `bd` in
// `tests/prime_e2e.rs`. Keeping it out of the generic stub suite
// avoids depending on whatever `bd` happens to be on PATH.
