//! `hew epic <verb>` end-to-end via a PATH-stubbed bd binary.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

// Stub looks up responses in files under $BD_STUB_FIXTURES (one file per
// id). File names use the id verbatim. Empty/missing → empty stdout.
const STUB: &str = r#"#!/bin/sh
if [ -n "$BD_STUB_LOG" ]; then
    printf '%s\n' "$*" >> "$BD_STUB_LOG"
fi
verb="$1"
case "$verb" in
    show)
        f="$BD_STUB_FIXTURES/show-$2.json"
        [ -f "$f" ] && /bin/cat "$f"
        ;;
    children)
        f="$BD_STUB_FIXTURES/children-$2.json"
        if [ -f "$f" ]; then /bin/cat "$f"; else printf '[]'; fi
        ;;
    close)
        exit 0
        ;;
    *)
        exit 0
        ;;
esac
"#;

fn seed(fixtures: &std::path::Path, kind: &str, id: &str, body: &str) {
    fs::create_dir_all(fixtures).unwrap();
    fs::write(fixtures.join(format!("{kind}-{id}.json")), body).unwrap();
}

fn write_bd_stub(dir: &std::path::Path) {
    let path = dir.join("bd");
    fs::write(&path, STUB).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
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

fn issue_json(id: &str, title: &str, status: &str, reason: Option<&str>) -> String {
    let reason_field = match reason {
        Some(r) => format!("\"{r}\""),
        None => "null".into(),
    };
    format!(
        r#"{{"id":"{id}","title":"{title}","description":"epic body for {id}","status":"{status}","priority":2,"issue_type":"task","closed_at":"","close_reason":{reason_field},"parent":null}}"#
    )
}

fn epic_show(id: &str, title: &str) -> String {
    format!(
        r#"[{{"id":"{id}","title":"{title}","description":"epic body here","status":"in_progress","priority":2,"issue_type":"epic","closed_at":"","close_reason":null,"parent":null}}]"#
    )
}

// ─── show ───────────────────────────────────────────────────────────────

fn fx(tmp: &std::path::Path) -> std::path::PathBuf {
    tmp.join("fixtures")
}

#[test]
fn show_stitches_epic_and_children() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    seed(&f, "show", "hew-1", &epic_show("hew-1", "the epic"));
    seed(
        &f,
        "children",
        "hew-1",
        &format!(
            "[{},{}]",
            issue_json("hew-1.1", "child a", "open", None),
            issue_json("hew-1.2", "child b", "closed", Some("done")),
        ),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .args(["epic", "show", "hew-1"])
        .assert()
        .success()
        .stdout(contains("hew-1"))
        .stdout(contains("the epic"))
        .stdout(contains("epic body here"))
        .stdout(contains("child_count:  2"))
        .stdout(contains("hew-1.1"))
        .stdout(contains("hew-1.2"));
}

#[test]
fn show_json_round_trips_through_epic_summary() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    seed(&f, "show", "hew-1", &epic_show("hew-1", "the epic"));
    seed(&f, "children", "hew-1", &format!("[{}]", issue_json("hew-1.1", "a", "open", None)));

    let out = hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .args(["epic", "show", "hew-1", "--json"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let parsed: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(parsed["id"], "hew-1");
    assert_eq!(parsed["child_count"], 1);
    assert_eq!(parsed["children"][0]["id"], "hew-1.1");
}

#[test]
fn tree_walks_two_levels() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    seed(&f, "show", "hew-1", &format!("[{}]", issue_json("hew-1", "root", "open", None)));
    seed(&f, "show", "hew-1.1", &format!("[{}]", issue_json("hew-1.1", "kid", "open", None)));
    seed(&f, "show", "hew-1.1.1", &format!("[{}]", issue_json("hew-1.1.1", "grand", "open", None)));
    seed(&f, "children", "hew-1", &format!("[{}]", issue_json("hew-1.1", "kid", "open", None)));
    seed(
        &f,
        "children",
        "hew-1.1",
        &format!("[{}]", issue_json("hew-1.1.1", "grand", "open", None)),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .args(["epic", "tree", "hew-1", "--depth", "2"])
        .assert()
        .success()
        .stdout(contains("hew-1"))
        .stdout(contains("hew-1.1"))
        .stdout(predicates::str::contains("hew-1.1.1").not());
}

#[test]
fn close_refuses_when_child_is_open() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    seed(
        &f,
        "children",
        "hew-1",
        &format!("[{}]", issue_json("hew-1.1", "open kid", "open", None)),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .args(["epic", "close", "hew-1"])
        .assert()
        .failure()
        .stderr(contains("still open"));
}

#[test]
fn close_force_overrides_open_children() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    let log = tmp.path().join("log");
    seed(
        &f,
        "children",
        "hew-1",
        &format!("[{}]", issue_json("hew-1.1", "open kid", "open", None)),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .env("BD_STUB_LOG", &log)
        .args(["epic", "close", "hew-1", "--force", "--reason", "abandoned"])
        .assert()
        .success();

    assert!(fs::read_to_string(&log).unwrap().contains("close hew-1 -r abandoned"));
}

#[test]
fn close_succeeds_when_all_children_closed() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    let log = tmp.path().join("log");
    seed(
        &f,
        "children",
        "hew-1",
        &format!("[{}]", issue_json("hew-1.1", "kid", "closed", Some("done"))),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .env("BD_STUB_LOG", &log)
        .args(["epic", "close", "hew-1", "--reason", "shipped via abc"])
        .assert()
        .success();

    assert!(fs::read_to_string(&log).unwrap().contains("close hew-1 -r shipped via abc"));
}

#[test]
fn audit_flags_thin_close_reasons() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    seed(
        &f,
        "children",
        "hew-1",
        &format!(
            "[{},{}]",
            issue_json("hew-1.1", "thin kid", "closed", Some("done")),
            issue_json(
                "hew-1.2",
                "good kid",
                "closed",
                Some("Shipped via abc123 — added tests for X")
            ),
        ),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .args(["epic", "audit", "hew-1"])
        .assert()
        .success()
        .stdout(contains("hew-1.1"))
        .stdout(contains("thin close_reason"))
        .stdout(predicates::str::contains("hew-1.2").not());
}

#[test]
fn summary_one_line_per_child() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());
    let f = fx(tmp.path());
    seed(
        &f,
        "children",
        "hew-1",
        &format!(
            "[{},{}]",
            issue_json("hew-1.1", "done one", "closed", Some("done")),
            issue_json("hew-1.2", "todo one", "open", None),
        ),
    );

    hew_in(tmp.path())
        .env("BD_STUB_FIXTURES", &f)
        .args(["epic", "summary", "hew-1"])
        .assert()
        .success()
        .stdout(contains("✓ hew-1.1 done one"))
        .stdout(contains("○ hew-1.2 todo one"));
}

#[test]
fn epic_help_lists_all_verbs() {
    let tmp = tempfile::tempdir().unwrap();
    write_bd_stub(tmp.path());

    hew_in(tmp.path())
        .args(["epic", "--help"])
        .assert()
        .success()
        .stdout(contains("show"))
        .stdout(contains("tree"))
        .stdout(contains("close"))
        .stdout(contains("audit"))
        .stdout(contains("summary"));
}
