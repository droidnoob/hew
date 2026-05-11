//! `prime::build` driven against a `RealBd` pointed at a stub script.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use hew_core::bd::RealBd;
use hew_core::prime;

fn write_stub(dir: &std::path::Path, script: &str) -> PathBuf {
    let path = dir.join("bd");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    path
}

const STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "bd version 1.0.3 (deadbeef)"; exit 0 ;;
  ready)
    cat <<'JSON'
[{"id":"x-1","title":"first","priority":0,"status":"open","issue_type":"task"}]
JSON
    exit 0 ;;
  stats)
    cat <<'JSON'
{"schema_version":1,"summary":{"total_issues":4,"open_issues":3,"closed_issues":1,"ready_issues":1,"blocked_issues":2,"in_progress_issues":0}}
JSON
    exit 0 ;;
  memories)
    cat <<'JSON'
{"a":"CONVENTION:errors — wrap","b":"STATUS:plan:complete — 2026-05-11T15:00:00","c":"Backend: FastAPI"}
JSON
    exit 0 ;;
esac
exit 2
"#;

#[test]
fn build_assembles_expected_shape() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_stub(tmp.path(), STUB);
    let client = RealBd::at(path);

    let out = prime::build(&client, "execute").expect("build");

    assert_eq!(out.schema_version, 1);
    assert_eq!(out.skill, "hew-execute");
    assert!(out.project.beads_initialized);
    assert_eq!(out.project.bd_version.as_deref(), Some("1.0.3"));

    // Memory bucketing
    assert_eq!(out.memories.conventions.len(), 1);
    assert_eq!(out.memories.factual.len(), 1);

    // STATUS parsed out into status map, prerequisites met.
    assert!(out.status.get("plan").map(|s| s.complete).unwrap_or(false));
    assert!(out.prerequisites.met);
    assert!(out.prerequisites.missing.is_empty());

    // Task summary lifted from stats.
    assert_eq!(out.tasks.total, 4);
    assert_eq!(out.tasks.done, 1);
    assert_eq!(out.tasks.ready, 1);
    assert_eq!(out.tasks.ready_list.len(), 1);

    // Skill body present.
    assert!(out.skill_instructions.contains("hew-execute"));
}

#[test]
fn build_short_circuits_when_prerequisite_missing() {
    let stub_no_status = STUB.replace("STATUS:plan:complete — 2026-05-11T15:00:00", "noise");
    let tmp = tempfile::tempdir().unwrap();
    let path = write_stub(tmp.path(), &stub_no_status);
    let client = RealBd::at(path);

    let out = prime::build(&client, "execute").expect("build");
    assert!(!out.prerequisites.met);
    assert_eq!(out.prerequisites.missing, vec!["plan"]);
}

#[test]
fn build_rejects_unknown_skill() {
    let tmp = tempfile::tempdir().unwrap();
    let path = write_stub(tmp.path(), STUB);
    let client = RealBd::at(path);
    let err = prime::build(&client, "nope").expect_err("must fail");
    assert!(err.to_string().contains("nope"));
}

#[test]
fn build_tolerates_bd_failures_for_optional_calls() {
    // bd that 500s on every call should still produce a valid (empty-ish) output.
    let tmp = tempfile::tempdir().unwrap();
    let path = write_stub(
        tmp.path(),
        r#"#!/bin/sh
echo "boom" >&2
exit 9
"#,
    );
    let client = RealBd::at(path);
    let out = prime::build(&client, "plan").expect("build still succeeds");
    assert_eq!(out.tasks.total, 0);
    assert!(out.memories.conventions.is_empty());
}
