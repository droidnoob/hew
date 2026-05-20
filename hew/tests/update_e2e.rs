//! `hew update` end-to-end. We focus on `--local`: the binary
//! self-updater path (`run_sync`) hits GitHub and is exercised in
//! release CI, not here.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

fn hew_in(dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.current_dir(dir);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

#[test]
fn update_local_errors_when_no_runtime_detected() {
    let tmp = tempfile::tempdir().unwrap();

    hew_in(tmp.path())
        .args(["update", "--local"])
        .assert()
        .failure()
        .stderr(contains("no runtime markers found"))
        .stderr(contains("hew init"));
}

#[test]
fn update_local_refreshes_claude_runtime() {
    let tmp = tempfile::tempdir().unwrap();
    // Pre-seed the Claude marker so `install::detect_runtimes` matches.
    fs::create_dir_all(tmp.path().join(".claude")).unwrap();

    hew_in(tmp.path())
        .args(["update", "--local"])
        .assert()
        .success()
        .stdout(contains("refreshing skills for claude"))
        .stdout(contains("Refreshed"))
        .stdout(contains("Binary stayed at"));

    // Sanity: the install actually wrote SKILL.md under .claude/skills/hew/.
    let skill_md = tmp.path().join(".claude/skills/hew/SKILL.md");
    assert!(skill_md.exists(), "expected SKILL.md to be written");
}

#[test]
fn update_help_documents_local_independence() {
    let tmp = tempfile::tempdir().unwrap();
    hew_in(tmp.path())
        .args(["update", "--help"])
        .assert()
        .success()
        .stdout(contains("--local"))
        .stdout(contains("skips the binary self-updater"));
}
