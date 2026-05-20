//! Drive `RealBd` against a fake `bd` shell script on a controlled PATH.
//!
//! Avoids the global env var dance by passing the absolute path of the
//! stub into `RealBd::at`, which is the production-blessed escape hatch
//! for non-PATH installs.

use std::path::PathBuf;

use hew_core::bd::{BdClient, RealBd};

fn write_stub(dir: &std::path::Path, script: &str) -> PathBuf {
    hew_core::testing::install_executable_stub(dir, "bd", script).unwrap();
    dir.join("bd")
}

#[test]
fn version_round_trips_through_stub() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(
        tmp.path(),
        r#"#!/bin/sh
if [ "$1" = "--version" ]; then
  echo "bd version 9.9.9 (deadbeef)"
  exit 0
fi
echo "unexpected: $@" >&2
exit 2
"#,
    );
    let client = RealBd::at(stub);
    let v = client.version().expect("version");
    assert_eq!(v.semver, "9.9.9");
    assert!(v.raw.contains("deadbeef"));
}

#[test]
fn ready_decodes_real_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(
        tmp.path(),
        r#"#!/bin/sh
if [ "$1" = "ready" ] && [ "$2" = "--json" ]; then
  cat <<'JSON'
[
  {"id":"x-1","title":"first","priority":0,"status":"open","issue_type":"task"},
  {"id":"x-2","title":"second","priority":2,"status":"open","issue_type":"epic","parent":null}
]
JSON
  exit 0
fi
exit 2
"#,
    );
    let client = RealBd::at(stub);
    let tasks = client.ready().expect("ready");
    assert_eq!(tasks.len(), 2);
    assert_eq!(tasks[0].id, "x-1");
    assert_eq!(tasks[1].issue_type, "epic");
}

#[test]
fn stats_decodes_envelope() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(
        tmp.path(),
        r#"#!/bin/sh
if [ "$1" = "stats" ] && [ "$2" = "--json" ]; then
  cat <<'JSON'
{"schema_version":1,"summary":{"total_issues":7,"open_issues":4,"closed_issues":3,"ready_issues":2,"blocked_issues":1}}
JSON
  exit 0
fi
exit 2
"#,
    );
    let client = RealBd::at(stub);
    let s = client.stats().expect("stats");
    assert_eq!(s.total_issues, 7);
    assert_eq!(s.closed_issues, 3);
    assert_eq!(s.ready_issues, 2);
}

#[test]
fn memories_decodes_object() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(
        tmp.path(),
        r#"#!/bin/sh
if [ "$1" = "memories" ] && [ "$2" = "--json" ]; then
  echo '{"alpha":"one","beta":"two"}'
  exit 0
fi
exit 2
"#,
    );
    let client = RealBd::at(stub);
    let m = client.memories().expect("memories");
    assert_eq!(m.len(), 2);
    assert_eq!(m.get("alpha").map(String::as_str), Some("one"));
}

#[test]
fn remember_passes_text_verbatim() {
    let tmp = tempfile::tempdir().unwrap();
    let echo_log = tmp.path().join("calls.log");
    let stub = write_stub(
        tmp.path(),
        &format!(
            r#"#!/bin/sh
echo "$@" >> "{log}"
exit 0
"#,
            log = echo_log.display()
        ),
    );
    let client = RealBd::at(stub);
    client.remember("CONVENTION:test — embedded fact").expect("remember");
    let logged = std::fs::read_to_string(&echo_log).unwrap();
    assert!(logged.starts_with("remember "));
    assert!(logged.contains("CONVENTION:test"));
}

#[test]
fn nonzero_exit_surfaces_stderr() {
    let tmp = tempfile::tempdir().unwrap();
    let stub = write_stub(
        tmp.path(),
        r#"#!/bin/sh
echo "boom" >&2
exit 7
"#,
    );
    let client = RealBd::at(stub);
    let err = client.version().expect_err("must fail");
    let msg = err.to_string();
    assert!(msg.contains("7"), "{msg}");
}
