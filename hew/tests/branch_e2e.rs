//! `hew branch new` end-to-end with a PATH-stubbed git binary.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::str::contains;

fn write_git_stub(dir: &std::path::Path, body: &str) {
    let path = dir.join("git");
    fs::write(&path, body).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
}

fn hew_with_path(path_dir: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("PATH", path_dir);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.env("HEW_NO_UPDATE_CHECK", "1");
    c.env("HEW_NON_INTERACTIVE", "1");
    c.env_remove("HEW_LOG");
    c.env_remove("CI");
    c
}

#[test]
fn new_creates_branch_via_checkout_dash_b() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("args.log");
    write_git_stub(tmp.path(), &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display()));

    hew_with_path(tmp.path())
        .args(["branch", "new", "--prefix", "feat", "--slug", "Add Auth"])
        .assert()
        .success()
        .stdout(contains("feat/add-auth"));

    let recorded = fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.trim(), "checkout -b feat/add-auth");
}

#[test]
fn new_with_from_passes_base() {
    let tmp = tempfile::tempdir().unwrap();
    let log = tmp.path().join("args.log");
    write_git_stub(tmp.path(), &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display()));

    hew_with_path(tmp.path())
        .args(["branch", "new", "--prefix", "fix", "--slug", "bug 123", "--from", "origin/main"])
        .assert()
        .success();

    let recorded = fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.trim(), "checkout -b fix/bug-123 origin/main");
}

#[test]
fn new_rejects_unknown_prefix() {
    let tmp = tempfile::tempdir().unwrap();
    write_git_stub(tmp.path(), "#!/bin/sh\nexit 0\n");

    hew_with_path(tmp.path())
        .args(["branch", "new", "--prefix", "feature", "--slug", "x"])
        .assert()
        .failure()
        .stderr(contains("feature"))
        .stderr(contains("feat"));
}

#[test]
fn new_rejects_empty_slug() {
    let tmp = tempfile::tempdir().unwrap();
    write_git_stub(tmp.path(), "#!/bin/sh\nexit 0\n");

    hew_with_path(tmp.path())
        .args(["branch", "new", "--prefix", "feat", "--slug", "✨"])
        .assert()
        .failure()
        .stderr(contains("slugifies to empty"));
}

#[test]
fn new_surfaces_git_failure() {
    let tmp = tempfile::tempdir().unwrap();
    write_git_stub(
        tmp.path(),
        "#!/bin/sh\necho 'fatal: A branch named feat/foo already exists' >&2\nexit 128\n",
    );

    hew_with_path(tmp.path())
        .args(["branch", "new", "--prefix", "feat", "--slug", "foo"])
        .assert()
        .failure()
        .stderr(contains("already"))
        .stderr(contains("128"));
}

#[test]
fn new_errors_when_git_missing() {
    // PATH points to a dir with no `git`.
    let tmp = tempfile::tempdir().unwrap();
    hew_with_path(tmp.path())
        .args(["branch", "new", "--prefix", "feat", "--slug", "x"])
        .assert()
        .failure()
        .stderr(contains("git"));
}

#[test]
fn config_set_branching_strategy_validates() {
    let cfg_tmp = tempfile::tempdir().unwrap();
    let cfg = cfg_tmp.path().join("config.toml");
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", &cfg);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.args(["config", "set", "branching.strategy", "epic"]).assert().success();

    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", &cfg);
    c.args(["config", "get", "branching.strategy"]).assert().success().stdout(contains("epic"));

    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", &cfg);
    c.args(["config", "set", "branching.strategy", "weekly"]).assert().failure();
}

#[test]
fn config_set_research_default_validates() {
    let cfg_tmp = tempfile::tempdir().unwrap();
    let cfg = cfg_tmp.path().join("config.toml");

    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", &cfg);
    c.args(["config", "set", "research.default", "auto-skip"]).assert().success();

    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", &cfg);
    c.args(["config", "get", "research.default"]).assert().success().stdout(contains("auto-skip"));

    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", &cfg);
    c.args(["config", "set", "research.default", "maybe"]).assert().failure();
}
