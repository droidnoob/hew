//! `hew schema` end-to-end.

use assert_cmd::Command;

fn hew() -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c
}

#[test]
fn schema_prime_emits_valid_jsonschema() {
    let out = hew().args(["schema", "prime"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "PrimeOutput");
    assert_eq!(parsed["type"], "object");
    assert!(parsed["properties"]["schema_version"].is_object());
    assert!(parsed["properties"]["skill_instructions"].is_object());
    assert!(parsed["$schema"].is_string());
}

#[test]
fn schema_config_emits_valid_jsonschema() {
    let out = hew().args(["schema", "config"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "Config");
    assert!(parsed["properties"]["update_check"].is_object());
}

#[test]
fn schema_rejects_unknown_target() {
    hew().args(["schema", "bogus"]).assert().failure().code(2);
}
