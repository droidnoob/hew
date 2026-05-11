//! `hew init` end-to-end against a fake `bd` and an isolated project dir.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

const BD_STUB_OK: &str = r#"#!/bin/sh
# Pretend bd init succeeded; create the .beads/ marker so re-runs short-circuit.
if [ "$1" = "init" ]; then
  mkdir -p .beads
  echo "stub-bd init ok"
  exit 0
fi
echo "stub-bd unhandled: $@" >&2
exit 2
"#;

fn install_stub(dir: &Path, script: &str) {
    let path = dir.join("bd");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(script.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn hew_with_stub(project: &Path, stub_dir: &Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.current_dir(project);
    c.env("PATH", stub_dir);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env_remove("HEW_LOG");
    // Force non-interactive so inquire never engages even if a TTY is somehow visible.
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env("CI", "true");
    c
}

#[test]
fn init_claude_writes_full_layout_and_gitignores_beads() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);

    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive"])
        .assert()
        .success()
        .stdout(contains("hew installed for claude"));

    let hew_root = project.path().join(".claude").join("skills").join("hew");
    assert!(hew_root.join("SKILL.md").exists());
    assert!(hew_root.join("core").join("hew-execute.md").exists());
    assert!(hew_root.join("brownfield").join("hew-scan.md").exists());
    assert!(hew_root.join("optional").join("hew-quick.md").exists());
    assert!(hew_root.join("custom").exists());

    let gitignore = fs::read_to_string(project.path().join(".gitignore")).unwrap();
    assert!(gitignore.contains(".beads/"), "expected .beads/ in .gitignore:\n{gitignore}");
}

#[test]
fn init_git_track_flag_skips_gitignore() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--git-track"])
        .assert()
        .success();

    let gi = project.path().join(".gitignore");
    let body = fs::read_to_string(&gi).unwrap_or_default();
    assert!(!body.contains(".beads/"), "git-track must not add .beads/:\n{body}");
}

#[test]
fn init_runtime_flag_overrides_detection() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    // No runtime markers — would normally fail in non-interactive mode without --runtime.

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "generic"])
        .assert()
        .success();

    assert!(project.path().join("CLAUDE.md").exists());
}

#[test]
fn init_non_interactive_without_runtime_fails_loudly() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap(); // no runtime markers

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive"])
        .assert()
        .failure()
        .stderr(contains("runtime"));
}

#[test]
fn init_errors_when_bd_missing() {
    let empty = tempfile::tempdir().unwrap();
    // Empty stub_dir → no bd on PATH.
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), empty.path())
        .args(["init", "--non-interactive"])
        .assert()
        .failure()
        .stderr(contains("`bd` binary not found"));
}

#[test]
fn init_is_idempotent_when_beads_already_exists() {
    let stub_dir = tempfile::tempdir().unwrap();
    // Stub fails noisily if `bd init` is called. Idempotence means we skip it.
    install_stub(
        stub_dir.path(),
        r#"#!/bin/sh
if [ "$1" = "init" ]; then
  echo "should not be called" >&2
  exit 99
fi
exit 0
"#,
    );

    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join(".beads")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive"])
        .assert()
        .success();
}

#[test]
fn init_unknown_runtime_value_rejected_by_clap() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--runtime", "fake-runtime"])
        .assert()
        .failure()
        .code(2);
}
