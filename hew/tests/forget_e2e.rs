//! `hew forget <KEY>` top-level subcommand. Today it's an ergonomic
//! alias for `hew memories --forget <KEY>` — ML.6 (hew-jem) will
//! extend the same surface with cascade-delete of outbound LINK: rows.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

/// Stub that:
/// - logs every invocation to $BD_STUB_LOG (one line per call)
/// - returns $BD_STUB_MEMORIES_JSON for `bd memories` (or `{}`)
/// - exits 0 for every other verb (forget, etc.)
const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
verb="$1"
case "$verb" in
    memories)
        if [ -n "$BD_STUB_MEMORIES_JSON" ]; then
            printf '%s' "$BD_STUB_MEMORIES_JSON"
        else
            printf '{}'
        fi
        ;;
    *) exit 0 ;;
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

// ──────── ML.6 cascade — purge outbound LINK: rows ────────

/// Fixture: a CONVENTION memory with two outbound LINK sidecars and
/// one *inbound* LINK row pointing back at it from an unrelated
/// source. The cascade should forget the primary + both outbound
/// sidecars and leave the inbound row untouched (dangling).
fn cascade_fixture() -> &'static str {
    r#"{
        "convention-cli-output": "CONVENTION:never pipe --json through python",
        "link-conv-to-decision": "LINK:convention-cli-output->relates_to:memory:decision-review-filing",
        "link-conv-to-task":     "LINK:convention-cli-output->relates_to:task:hew-abc",
        "link-inbound-from-other": "LINK:decision-other->relates_to:memory:convention-cli-output",
        "decision-review-filing": "DECISION:review filings",
        "decision-other":          "DECISION:another decision"
    }"#
}

#[test]
fn forget_cascades_outbound_link_sidecars() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_MEMORIES_JSON", cascade_fixture())
        .args(["forget", "convention-cli-output"])
        .assert()
        .success()
        .stdout(contains("forgot convention-cli-output"))
        .stdout(contains("purged 2 outbound LINK: rows"));

    let log_contents = fs::read_to_string(&log).unwrap();
    // Primary fired first.
    assert!(
        log_contents.contains("forget convention-cli-output"),
        "primary forget missing:\n{log_contents}"
    );
    // Both outbound LINK sidecars also fired.
    assert!(
        log_contents.contains("forget link-conv-to-decision"),
        "outbound memory-link forget missing:\n{log_contents}"
    );
    assert!(
        log_contents.contains("forget link-conv-to-task"),
        "outbound task-link forget missing:\n{log_contents}"
    );
    // The INBOUND LINK row (pointing AT convention-cli-output) must
    // survive — the user policy is that dangling references stay so
    // the next author notices and rewires.
    assert!(
        !log_contents.contains("forget link-inbound-from-other"),
        "inbound LINK row was wrongly forgotten:\n{log_contents}"
    );
}

#[test]
fn forget_ordering_primary_first_then_cascade() {
    // If the primary forget fails, the cascade should never fire.
    // We test ordering by reading the stub log: primary line index
    // must precede every cascade line index.
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_MEMORIES_JSON", cascade_fixture())
        .args(["forget", "convention-cli-output"])
        .assert()
        .success();

    let log_contents = fs::read_to_string(&log).unwrap();
    let primary_idx = log_contents
        .lines()
        .position(|l| l.contains("forget convention-cli-output"))
        .expect("primary forget missing");
    let cascade_a = log_contents
        .lines()
        .position(|l| l.contains("forget link-conv-to-decision"))
        .expect("cascade #1 missing");
    let cascade_b = log_contents
        .lines()
        .position(|l| l.contains("forget link-conv-to-task"))
        .expect("cascade #2 missing");
    assert!(primary_idx < cascade_a, "primary must precede cascade");
    assert!(primary_idx < cascade_b, "primary must precede cascade");
}

#[test]
fn forget_with_no_outbound_links_skips_cascade_noise() {
    // A memory with no outbound LINK sidecars should produce just
    // the single-line confirmation — no spurious "purged 0 rows"
    // summary.
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    let fixture =
        r#"{"foo": "CONVENTION:body", "link-other": "LINK:other->relates_to:memory:bar"}"#;

    let out = hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_MEMORIES_JSON", fixture)
        .args(["forget", "foo"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("forgot foo"), "{s}");
    assert!(!s.contains("purged"), "no cascade summary expected: {s}");

    // The unrelated LINK row (from=other, not from=foo) must NOT have
    // been forgotten as collateral damage.
    let log_contents = fs::read_to_string(&log).unwrap();
    assert!(
        !log_contents.contains("forget link-other"),
        "unrelated LINK row wrongly forgotten:\n{log_contents}"
    );
}

#[test]
fn memories_forget_flag_does_not_cascade() {
    // `hew memories --forget <KEY>` is the lower-level escape hatch:
    // single-key, no cascade. Locks the behavioral distinction.
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let log = tmp.path().join("log");

    hew_in(tmp.path())
        .env("BD_STUB_LOG", &log)
        .env("BD_STUB_MEMORIES_JSON", cascade_fixture())
        .args(["memories", "--forget", "convention-cli-output"])
        .assert()
        .success()
        .stdout(contains("forgot convention-cli-output"));

    let log_contents = fs::read_to_string(&log).unwrap();
    // Only the primary forget — no cascade.
    let forget_count = log_contents.matches("forget convention-cli-output").count();
    assert_eq!(forget_count, 1, "primary forget should fire exactly once:\n{log_contents}");
    assert!(
        !log_contents.contains("forget link-conv-to-decision"),
        "memories --forget must NOT cascade:\n{log_contents}"
    );
}
