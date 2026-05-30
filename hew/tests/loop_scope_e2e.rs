//! `hew loop run --scope / --epics / --epic` CLI-surface acceptance.
//!
//! Covers the resolution policy of hew-xhhw:
//! - argv > picker > non-interactive error;
//! - `--scope=ready` is the legacy default and runs without prompting;
//! - `--scope=epics --epics=<csv>` reaches resolve without prompting;
//! - missing flags in non-interactive mode emit `MissingFlag`;
//! - disallowed combinations (`--scope=ready --epics=X`) error.
//!
//! Picker UX itself (interactive `inquire` prompts) is intentionally
//! NOT exercised here — driving inquire from tests requires a faked
//! terminal we don't ship. The unit tests in `commands/loop_cmd.rs`
//! cover the `resolve_scope` branches directly against a stub `BdClient`.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

/// `--scope=bogus` is rejected at clap (exit 2). Pins the ValueEnum so
/// future additions stay deliberate.
#[test]
fn scope_rejects_bogus_at_clap() {
    hew()
        .args(["loop", "run", "--scope=bogus", "--dry-run", "--max-iter", "1"])
        .assert()
        .failure()
        .code(2)
        .stderr(contains("ready").and(contains("epics")));
}

/// `--scope=ready` parses and clears the non-interactive MissingFlag
/// guard. Downstream may still fail (no bd in the test cwd) — the
/// assertion is just that we *didn't* exit on the MissingFlag path.
#[test]
fn scope_ready_flag_no_picker() {
    let out = hew()
        .args(["loop", "run", "--scope=ready", "--dry-run", "--max-iter", "1"])
        .assert()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("missing required value in non-interactive mode: --scope"),
        "should NOT trip the scope MissingFlag with --scope=ready, stderr={stderr}",
    );
    assert!(
        !stderr.contains("missing required value in non-interactive mode: --epics"),
        "should NOT trip the epics MissingFlag with --scope=ready, stderr={stderr}",
    );
}

/// `--scope=epics --epics=<id>` reaches past the MissingFlag guard
/// without prompting. The actual bd lookup may still fail (no bd in
/// the test cwd) but the failure isn't the MissingFlag path.
#[test]
fn scope_epics_with_epics_flag_no_picker() {
    let out = hew()
        .args([
            "loop",
            "run",
            "--scope=epics",
            "--epics=hew-doesnotexist",
            "--dry-run",
            "--max-iter",
            "1",
        ])
        .assert()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("missing required value in non-interactive mode: --scope"),
        "should NOT trip scope MissingFlag, stderr={stderr}",
    );
    assert!(
        !stderr.contains("missing required value in non-interactive mode: --epics"),
        "should NOT trip epics MissingFlag with --epics= passed, stderr={stderr}",
    );
}

/// `--scope=epics` with no `--epics` in non-interactive mode emits the
/// epics MissingFlag. Agents calling agents MUST be explicit.
#[test]
fn scope_epics_no_epics_non_interactive_errors() {
    hew()
        .args(["loop", "run", "--scope=epics", "--dry-run", "--max-iter", "1"])
        .assert()
        .failure()
        .stderr(contains("missing required value in non-interactive mode: --epics"));
}

/// `hew loop run` with no `--scope` argv on a non-interactive runner
/// emits the scope MissingFlag.
#[test]
fn scope_omitted_non_interactive_errors() {
    hew()
        .args(["loop", "run", "--dry-run", "--max-iter", "1"])
        .assert()
        .failure()
        .stderr(contains("missing required value in non-interactive mode: --scope"));
}

/// `--scope=ready --epics=<id>` is a contradiction. Reject at resolve
/// time before any iter spawns.
#[test]
fn scope_ready_with_epics_argv_errors() {
    hew()
        .args(["loop", "run", "--scope=ready", "--epics=hew-6az", "--dry-run", "--max-iter", "1"])
        .assert()
        .failure()
        .stderr(contains("--scope=ready does not accept --epics"));
}

/// `--scope` and `--epics` show up in `--help` so the explicit-argv
/// path is discoverable.
#[test]
fn scope_flags_are_in_help() {
    let out = hew().args(["loop", "run", "--help"]).assert().get_output().clone();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("--scope"), "help missing --scope:\n{stdout}");
    assert!(stdout.contains("--epics"), "help missing --epics:\n{stdout}");
    assert!(stdout.contains("--epic "), "help missing --epic (singular):\n{stdout}");
}
