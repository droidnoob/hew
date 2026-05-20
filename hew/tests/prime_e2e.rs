//! End-to-end: invoke the `hew` binary with PATH pointing at a fake `bd`.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use assert_cmd::Command;
use predicates::str::contains;

const STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "bd version 1.0.3"; exit 0 ;;
  ready) echo '[{"id":"x-1","title":"only","priority":0,"status":"open","issue_type":"task"}]'; exit 0 ;;
  stats) echo '{"schema_version":1,"summary":{"total_issues":1,"open_issues":1,"closed_issues":0,"ready_issues":1,"blocked_issues":0,"in_progress_issues":0}}'; exit 0 ;;
  memories) echo '{"k":"CONVENTION:errors — wrap"}'; exit 0 ;;
esac
exit 2
"#;

fn make_stub_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bd");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(STUB.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    tmp
}

#[test]
fn prime_emits_valid_json_to_stdout() {
    let stub_dir = make_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "execute"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("prime output must be valid JSON");

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["skill"], "hew-execute");
    assert_eq!(parsed["tasks"]["ready"], 1);
    assert_eq!(parsed["memories"]["conventions"].as_array().unwrap().len(), 1);
    assert!(parsed["skill_instructions"].as_str().unwrap().contains("hew-execute"));
}

#[test]
fn prime_pretty_flag_indents_output() {
    let stub_dir = make_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "plan", "--pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\n  "), "pretty output should be indented:\n{text}");
}

#[test]
fn prime_errors_when_bd_missing() {
    // Empty PATH → bd not found → miette diagnostic on stderr, exit 1.
    Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", "")
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "execute"])
        .assert()
        .failure()
        .stderr(contains("`bd` binary not found"));
}

#[test]
fn prime_errors_on_unknown_skill() {
    let stub_dir = make_stub_dir();
    Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .args(["prime", "definitely-not-a-skill"])
        .assert()
        .failure()
        .stderr(contains("definitely-not-a-skill"));
}

const RESUME_STUB: &str = r#"#!/bin/sh
case "$1" in
  --version) echo "bd version 1.0.3"; exit 0 ;;
  ready) echo '[]'; exit 0 ;;
  stats) echo '{"schema_version":1,"summary":{"total_issues":1,"open_issues":1,"closed_issues":0,"ready_issues":0,"blocked_issues":0,"in_progress_issues":1}}'; exit 0 ;;
  list) echo '[{"id":"hew-99z","title":"in-flight task","description":"first line of body","status":"in_progress","priority":1,"issue_type":"feature","assignee":"droidnoob"}]'; exit 0 ;;
  memories) echo '{"ck-old":"CHECKPOINT:2026-05-10T08:00 — earlier work","ck-new":"CHECKPOINT:2026-05-12T14:30 — newer work; in flight: refresh rotation","conv":"CONVENTION:errors — wrap","dec":"DECISION:errors-as-types","got":"GOTCHA:pipe-deadlock","fb":"FEEDBACK:no-json-piping","status-scan":"STATUS:scan:complete — 2026-05-12T07:54:13Z"}'; exit 0 ;;
esac
exit 2
"#;

fn make_resume_stub_dir() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("bd");
    let mut f = fs::File::create(&path).unwrap();
    f.write_all(RESUME_STUB.as_bytes()).unwrap();
    let mut perms = fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&path, perms).unwrap();
    tmp
}

#[test]
fn prime_resume_emits_skill_agnostic_json() {
    let stub_dir = make_resume_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("HEW_NON_INTERACTIVE", "1")
        // Pin config to defaults so the test isn't sensitive to the
        // dev machine's `~/.config/hew/config.toml`. A nonexistent path
        // falls back to `Config::default()` per `config::load_from`.
        .env("HEW_CONFIG", "/nonexistent/hew-config.toml")
        .env_remove("HEW_LOG")
        .env_remove("CI")
        .args(["prime", "resume", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let text = String::from_utf8(out).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(text.trim()).expect("resume output must be valid JSON");

    assert_eq!(parsed["schema_version"], 1);
    // No skill_instructions, no skill, no prerequisites in resume mode.
    assert!(parsed.get("skill_instructions").is_none());
    assert!(parsed.get("skill").is_none());
    assert!(parsed.get("prerequisites").is_none());

    // STATUS:scan still parsed out of memories.
    assert_eq!(parsed["status"]["scan"]["complete"], true);

    // Latest checkpoint is the newer one.
    assert_eq!(parsed["latest_checkpoint"]["key"], "ck-new");
    assert_eq!(parsed["latest_checkpoint"]["timestamp"], "2026-05-12T14:30");
    assert!(parsed["latest_checkpoint"]["body"].as_str().unwrap().contains("refresh rotation"));

    // In-progress task surfaced (claimed task body in the agent's first turn).
    let claimed = parsed["in_progress"].as_array().expect("in_progress array");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0]["id"], "hew-99z");
    assert_eq!(claimed[0]["title"], "in-flight task");

    // First-class memory buckets — DECISION/GOTCHA/FEEDBACK no longer
    // buried in `factual`.
    assert_eq!(parsed["memories"]["decisions"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["memories"]["gotchas"].as_array().unwrap().len(), 1);
    assert_eq!(parsed["memories"]["feedback"].as_array().unwrap().len(), 1);
    let factual: Vec<&str> = parsed["memories"]["factual"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|v| v.as_str())
        .collect();
    assert!(
        !factual.iter().any(|s| s.starts_with("DECISION:")
            || s.starts_with("GOTCHA:")
            || s.starts_with("FEEDBACK:")),
        "DECISION/GOTCHA/FEEDBACK must not leak into factual; got: {factual:?}"
    );
}

#[test]
fn prime_resume_pretty_flag_indents_output() {
    let stub_dir = make_resume_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("HEW_NON_INTERACTIVE", "1")
        // Pin config to defaults so the test isn't sensitive to the
        // dev machine's `~/.config/hew/config.toml`. A nonexistent path
        // falls back to `Config::default()` per `config::load_from`.
        .env("HEW_CONFIG", "/nonexistent/hew-config.toml")
        .env_remove("HEW_LOG")
        .env_remove("CI")
        .args(["prime", "resume", "--pretty"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\n  "), "pretty output should be indented:\n{text}");
    // --pretty implies --json: output must parse as JSON.
    serde_json::from_str::<serde_json::Value>(text.trim())
        .expect("--pretty must imply --json and emit valid JSON");
}

#[test]
fn prime_resume_defaults_to_plaintext() {
    let stub_dir = make_resume_stub_dir();
    let out = Command::cargo_bin("hew")
        .unwrap()
        .env("PATH", stub_dir.path())
        .env("NO_COLOR", "1")
        .env("TERM", "dumb")
        .env("HEW_NO_UPDATE_CHECK", "1")
        .env("HEW_NON_INTERACTIVE", "1")
        // Pin config to defaults so the test isn't sensitive to the
        // dev machine's `~/.config/hew/config.toml`. A nonexistent path
        // falls back to `Config::default()` per `config::load_from`.
        .env("HEW_CONFIG", "/nonexistent/hew-config.toml")
        .env_remove("HEW_LOG")
        .env_remove("CI")
        .args(["prime", "resume"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();
    // Plaintext, not JSON.
    assert!(
        serde_json::from_str::<serde_json::Value>(text.trim()).is_err(),
        "default `prime resume` must be plaintext, got JSON:\n{text}"
    );
    assert!(text.contains("hew resume"), "missing header:\n{text}");
    assert!(text.contains("Phases"), "missing Phases section:\n{text}");
    assert!(text.contains("Tasks"), "missing Tasks section:\n{text}");
    assert!(text.contains("Memories"), "missing Memories section:\n{text}");
    assert!(
        text.contains("Project config — read as standing instructions"),
        "missing project-config section:\n{text}"
    );
    // Default branching strategy is `epic`; that line should appear.
    assert!(text.contains("auto-create a feature branch"), "missing branching line:\n{text}");
    // In-progress section renders with the claimed task's title + body.
    assert!(text.contains("Claimed (in-flight)"), "missing in-progress section:\n{text}");
    assert!(text.contains("hew-99z"), "missing claimed task id:\n{text}");
    assert!(text.contains("first line of body"), "missing claimed task body:\n{text}");
    // First-class buckets surface in the counts line.
    assert!(text.contains("1 DECISION"), "missing DECISION count:\n{text}");
    assert!(text.contains("1 GOTCHA"), "missing GOTCHA count:\n{text}");
    assert!(text.contains("1 FEEDBACK"), "missing FEEDBACK count:\n{text}");
    assert!(text.contains("Latest CHECKPOINT"), "missing checkpoint section:\n{text}");
    // CHECKPOINT body content should be surfaced.
    assert!(text.contains("refresh rotation"), "checkpoint body content missing:\n{text}");
    // STATUS phase still rendered.
    assert!(text.contains("✓ scan"), "scan phase mark missing:\n{text}");
}
