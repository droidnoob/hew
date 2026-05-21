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

// ──────── --links reader (hew-bhc / ML.4) ────────

/// Stub that returns the JSON memory map from `$BD_STUB_MEMORIES_JSON`
/// for the `memories` verb. Falls through to exit 0 for other verbs
/// so commands like `bd recall` / `bd forget` don't perturb tests
/// that only exercise the read path.
const LINKS_STUB: &str = r#"#!/bin/sh
verb="$1"
case "$verb" in
    memories)
        if [ -n "$BD_STUB_MEMORIES_JSON" ]; then
            printf '%s' "$BD_STUB_MEMORIES_JSON"
        else
            printf '{}'
        fi
        ;;
    *)
        exit 0
        ;;
esac
"#;

fn write_links_stub(dir: &std::path::Path) {
    hew_core::testing::install_executable_stub(dir, "bd", LINKS_STUB).unwrap();
}

/// Fixture: a CONVENTION memory with three outbound explicit LINK
/// rows (one memory-target present, one task-target, one memory-
/// target *missing* from the set → dangling) and one inbound LINK
/// row pointing back at it from `decision-other`.
fn links_fixture() -> &'static str {
    r#"{
        "convention-cli-output": "CONVENTION:never pipe --json through python",
        "decision-review-filing": "DECISION:review findings go to bd as bug/chore tasks",
        "link-conv-to-decision": "LINK:convention-cli-output->relates_to:memory:decision-review-filing",
        "link-conv-to-task":     "LINK:convention-cli-output->relates_to:task:hew-abc",
        "link-conv-to-missing":  "LINK:convention-cli-output->relates_to:memory:totally-absent",
        "link-decision-inbound": "LINK:decision-other->relates_to:memory:convention-cli-output"
    }"#
}

#[test]
fn links_text_shows_outbound_inbound_and_dangling() {
    let tmp = tempfile::tempdir().unwrap();
    write_links_stub(tmp.path());

    let out = hew_in(tmp.path())
        .env("BD_STUB_MEMORIES_JSON", links_fixture())
        .args(["memories", "--links", "convention-cli-output"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();

    assert!(s.contains("Outbound (from convention-cli-output):"), "{s}");
    assert!(s.contains("decision-review-filing"), "outbound memory target missing:\n{s}");
    assert!(s.contains("hew-abc"), "outbound task target missing:\n{s}");
    assert!(s.contains("totally-absent [DANGLING]"), "missing dangling marker:\n{s}");

    assert!(s.contains("Inbound (to convention-cli-output):"), "{s}");
    assert!(s.contains("decision-other"), "inbound source missing:\n{s}");
}

#[test]
fn links_json_matches_documented_schema() {
    let tmp = tempfile::tempdir().unwrap();
    write_links_stub(tmp.path());

    let out = hew_in(tmp.path())
        .env("BD_STUB_MEMORIES_JSON", links_fixture())
        .args(["memories", "--json", "--links", "convention-cli-output"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();

    let v: serde_json::Value = serde_json::from_str(&s).expect("output must be valid JSON");
    assert_eq!(v["key"], "convention-cli-output");

    let outbound = v["outbound"].as_array().expect("outbound must be an array");
    assert_eq!(outbound.len(), 3);
    for r in outbound {
        assert!(r["kind"].is_string());
        assert!(r["to"].is_string());
        assert!(r["dangling"].is_boolean());
    }
    let dangling_count = outbound.iter().filter(|r| r["dangling"] == true).count();
    assert_eq!(dangling_count, 1);

    let inbound = v["inbound"].as_array().expect("inbound must be an array");
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0]["from"], "decision-other");

    let dangling_outbound =
        v["dangling_outbound"].as_array().expect("dangling_outbound must be an array");
    assert_eq!(dangling_outbound.len(), 1);
    assert_eq!(dangling_outbound[0]["to"], "totally-absent");
}

#[test]
fn links_for_missing_key_prints_none_both_sides() {
    let tmp = tempfile::tempdir().unwrap();
    write_links_stub(tmp.path());

    let out = hew_in(tmp.path())
        .env("BD_STUB_MEMORIES_JSON", links_fixture())
        .args(["memories", "--links", "totally-unknown-key"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("Outbound (from totally-unknown-key):"), "{s}");
    assert!(s.contains("Inbound (to totally-unknown-key):"), "{s}");
    assert_eq!(s.matches("(none)").count(), 2, "{s}");
}

#[test]
fn links_picks_up_body_scan_wikilinks() {
    // Verifies the ML.5 body-scanner feeds the reader: a memory
    // body containing `[[other-key]]` surfaces as an outbound edge
    // even with no explicit LINK row.
    let tmp = tempfile::tempdir().unwrap();
    write_links_stub(tmp.path());

    let fixture = r#"{
        "convention-cli-output": "CONVENTION:see [[decision-review-filing]] for filing rules",
        "decision-review-filing": "DECISION:review filings"
    }"#;

    let out = hew_in(tmp.path())
        .env("BD_STUB_MEMORIES_JSON", fixture)
        .args(["memories", "--links", "convention-cli-output"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("decision-review-filing"), "body-scan edge missing:\n{s}");
}

#[test]
fn links_conflicts_with_other_filter_flags() {
    let tmp = tempfile::tempdir().unwrap();
    write_links_stub(tmp.path());

    hew_in(tmp.path())
        .args(["memories", "--links", "foo", "--prefix", "CONVENTION"])
        .assert()
        .failure();
}
