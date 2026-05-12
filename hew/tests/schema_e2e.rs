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

#[test]
fn schema_task_emits_task_summary() {
    let out = hew().args(["schema", "task"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "TaskSummary");
    assert!(parsed["properties"]["id"].is_object());
    assert!(parsed["properties"]["status"].is_object());
    assert!(parsed["properties"]["close_reason"].is_object());
}

#[test]
fn schema_epic_emits_epic_summary() {
    let out = hew().args(["schema", "epic"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "EpicSummary");
    assert!(parsed["properties"]["child_count"].is_object());
    assert!(parsed["properties"]["children"].is_object());
}

#[test]
fn schema_task_list_filter_emits_filter_args() {
    let out =
        hew().args(["schema", "task-list-filter"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "TaskListFilter");
    assert!(parsed["properties"]["status"].is_object());
}

#[test]
fn schema_new_task_emits_args() {
    let out = hew().args(["schema", "new-task"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "NewTaskArgs");
    assert!(parsed["properties"]["title"].is_object());
}

#[test]
fn schema_stacks_emits_stack_table() {
    let out = hew().args(["schema", "stacks"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "StackTable");
    assert!(parsed["properties"]["stacks"].is_object());
}

#[test]
fn schema_craft_principles_emits_craft_table() {
    let out =
        hew().args(["schema", "craft-principles"]).assert().success().get_output().stdout.clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(parsed["title"], "CraftTable");
    assert!(parsed["properties"]["principles"].is_object());
}
