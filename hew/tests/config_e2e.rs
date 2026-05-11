//! `hew config` end-to-end with HEW_CONFIG pointed at a tempdir.

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

fn hew(cfg_path: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("HEW_CONFIG", cfg_path);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c
}

#[test]
fn list_shows_defaults_when_no_file_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg)
        .args(["config", "list"])
        .assert()
        .success()
        .stdout(contains("update-check"))
        .stdout(contains("default-runtime"))
        .stdout(contains("optional-skills.deps"));
}

#[test]
fn set_then_get_round_trips() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg).args(["config", "set", "default-runtime", "claude"]).assert().success();
    hew(&cfg)
        .args(["config", "get", "default-runtime"])
        .assert()
        .success()
        .stdout(contains("claude"));
    // Verify it actually wrote a real TOML file.
    let body = fs::read_to_string(&cfg).unwrap();
    assert!(body.contains("default_runtime"));
}

#[test]
fn set_bool_accepts_friendly_aliases() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg).args(["config", "set", "update-check", "off"]).assert().success();
    hew(&cfg).args(["config", "get", "update-check"]).assert().success().stdout(contains("false"));
}

#[test]
fn set_unknown_key_fails() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg)
        .args(["config", "set", "bogus-key", "x"])
        .assert()
        .failure()
        .stderr(contains("bogus-key"));
}

#[test]
fn reset_restores_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg).args(["config", "set", "update-check", "false"]).assert().success();
    hew(&cfg).args(["config", "reset"]).assert().success();
    hew(&cfg).args(["config", "get", "update-check"]).assert().success().stdout(contains("true"));
}

#[test]
fn json_list_emits_object() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    let out =
        hew(&cfg).args(["--json", "config", "list"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(parsed["update-check"].is_string());
}

#[test]
fn path_prints_config_location() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg).args(["config", "path"]).assert().success().stdout(contains(cfg.to_str().unwrap()));
}
