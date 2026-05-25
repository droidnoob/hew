//! `hew statusline` end-to-end via a PATH-stubbed bd binary.
//!
//! Covers the three formats, the empty-stdin tolerance, the malformed-
//! JSON tolerance, and the silent-exit when bd isn't on PATH.

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

/// A bd stub that:
/// - Reports a sane version (so RealBd::discover succeeds).
/// - Returns a small stats summary so `prime::resume` thinks the graph
///   is initialized.
/// - Returns empty list/children/memories so the rest of the pipeline
///   short-circuits to defaults — we only assert on render shape here.
const STUB_PRESENT: &str = r#"#!/bin/sh
verb="$1"
case "$verb" in
    --version)
        printf 'bd version 1.0.3 (test)\n'
        ;;
    stats)
        printf '{"schema_version":1,"summary":{"total_issues":10,"closed_issues":5,"open_issues":5,"in_progress_issues":0,"ready_issues":3,"blocked_issues":2}}'
        ;;
    ready)
        printf '[]'
        ;;
    memories)
        printf '{}'
        ;;
    list)
        printf '[]'
        ;;
    children)
        printf '[]'
        ;;
    show)
        printf '{"id":"x","title":"x","status":"open","priority":2,"issue_type":"task","closed_at":"","close_reason":null,"parent":null,"description":""}'
        ;;
    prime)
        printf '{}'
        ;;
    *)
        exit 0
        ;;
esac
"#;

fn write_stub(dir: &std::path::Path, body: &str) {
    hew_core::testing::install_executable_stub(dir, "bd", body).unwrap();
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
fn compact_format_shape() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path())
        .args(["statusline", "--compact"])
        .assert()
        .success()
        // `<label> <bar> N%`
        .stdout(contains("%"))
        // No phase word — that's medium+.
        .stdout(contains("executing").not())
        .stdout(contains("planning").not())
        .stdout(contains("verifying").not());
}

#[test]
fn medium_default_includes_phase() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path())
        .args(["statusline"])
        .assert()
        .success()
        // No STATUS markers present in the stub → Planning phase.
        .stdout(contains("planning"));
}

#[test]
fn full_format_appends_user_segment() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path())
        .env("USER", "alice")
        .args(["statusline", "--full"])
        .assert()
        .success()
        .stdout(contains("alice"));
}

#[test]
fn exits_zero_with_empty_stdout_when_bd_not_on_path() {
    // No bd stub written → which::which("bd") fails → silent exit.
    let tmp = tempfile::tempdir().unwrap();

    hew_in(tmp.path()).args(["statusline"]).assert().success().stdout("");
}

#[test]
fn tolerates_empty_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path()).args(["statusline", "--compact"]).write_stdin("").assert().success();
}

#[test]
fn tolerates_malformed_json_on_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path())
        .args(["statusline", "--compact"])
        .write_stdin("{not valid json at all")
        .assert()
        .success();
}

#[test]
fn composes_with_claude_prefix_when_session_json_on_stdin() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path())
        .env("NO_COLOR", "1")
        .args(["statusline", "--compact"])
        .write_stdin(
            r#"{"model":{"display_name":"Opus 4.7"},"workspace":{"current_dir":"/tmp/proj"}}"#,
        )
        .assert()
        .success()
        // Claude prefix present.
        .stdout(contains("Opus 4.7"))
        .stdout(contains("/tmp/proj"))
        // Composed with the `||` separator.
        .stdout(contains("||"))
        // hew segment still rendered (percent suffix).
        .stdout(contains("%"));
}

#[test]
fn bare_flag_skips_claude_prefix_even_with_session_json() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    hew_in(tmp.path())
        .env("NO_COLOR", "1")
        .args(["statusline", "--compact", "--bare"])
        .write_stdin(r#"{"model":{"display_name":"Opus"},"workspace":{"current_dir":"/tmp/x"}}"#)
        .assert()
        .success()
        .stdout(contains("Opus").not())
        .stdout(contains("||").not())
        .stdout(contains("%"));
}

#[test]
fn width_is_clamped_not_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    write_stub(tmp.path(), STUB_PRESENT);

    // width=0 → clamped to 1 in the pure render fn; CLI must not error.
    hew_in(tmp.path()).args(["statusline", "--compact", "--width", "0"]).assert().success();
    // width=9999 → clamped to 80.
    hew_in(tmp.path()).args(["statusline", "--compact", "--width", "9999"]).assert().success();
}
