//! `hew doctor` end-to-end.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::str::contains;

const BD_OK: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "bd version 1.0.3"; exit 0 ;;
esac
exit 0
"#;

fn install_bd(dir: &std::path::Path, script: &str) {
    let path = dir.join("bd");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn hew(project: &std::path::Path, stub_dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.current_dir(project);
    c.env("PATH", stub_dir);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c
}

fn scaffold_healthy(project: &std::path::Path) {
    fs::create_dir(project.join(".beads")).unwrap();
    fs::create_dir_all(project.join(".claude/skills/hew/core")).unwrap();
    fs::write(project.join(".claude/skills/hew/SKILL.md"), "x").unwrap();
    fs::write(project.join(".claude/skills/hew/core/hew-plan.md"), "x").unwrap();
    fs::write(project.join(".claude/skills/hew/core/hew-execute.md"), "x").unwrap();
    fs::write(project.join(".gitignore"), ".beads/\n").unwrap();
}

#[test]
fn doctor_passes_when_everything_is_set_up() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_bd(stub_dir.path(), BD_OK);
    let project = tempfile::tempdir().unwrap();
    scaffold_healthy(project.path());

    hew(project.path(), stub_dir.path())
        .arg("doctor")
        .assert()
        .success()
        .stdout(contains("✓ bd"))
        .stdout(contains("✓ beads"))
        .stdout(contains("✓ gitignore"))
        .stdout(contains("✓ runtime"))
        .stdout(contains("✓ skills"))
        .stdout(contains("Overall: ok"));
}

#[test]
fn doctor_fails_when_bd_missing() {
    let empty = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    scaffold_healthy(project.path());

    hew(project.path(), empty.path())
        .arg("doctor")
        .assert()
        .failure()
        .stdout(contains("✗ bd"))
        .stdout(contains("Overall: fail"));
}

#[test]
fn doctor_fix_adds_gitignore_entry() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_bd(stub_dir.path(), BD_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".beads")).unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew(project.path(), stub_dir.path())
        .args(["doctor", "--fix"])
        .assert()
        .success()
        .stdout(contains("fix applied"));

    let gi = fs::read_to_string(project.path().join(".gitignore")).unwrap();
    assert!(gi.contains(".beads/"));
}

#[test]
fn doctor_json_emits_overall_and_checks() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_bd(stub_dir.path(), BD_OK);
    let project = tempfile::tempdir().unwrap();
    scaffold_healthy(project.path());

    let out = hew(project.path(), stub_dir.path())
        .args(["--json", "doctor"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["overall"], "ok");
    assert!(parsed["checks"].as_array().unwrap().len() >= 5);
}
