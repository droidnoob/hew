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

// ──────── hew-k2gm: write-target resolution (--global / --project) ────────

/// Builds a hew Command that runs inside a project root (a tempdir with
/// `.beads/` so [`config::discover_project_root`] finds it) and has
/// `HEW_CONFIG` pointed at a user-global path inside that same tempdir
/// (so the test cannot stomp on the host's real config).
fn hew_in_project(user_cfg: &std::path::Path, project_root: &std::path::Path) -> Command {
    let mut c = hew(user_cfg);
    c.current_dir(project_root);
    c
}

fn make_project_root() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    fs::create_dir(tmp.path().join(".beads")).unwrap();
    tmp
}

#[test]
fn set_writes_to_user_when_no_project_file_and_no_flags() {
    // Branch 5: neither flag, no project file → user-global (back-compat).
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "default-runtime", "claude"])
        .assert()
        .success()
        .stdout(contains(user_cfg.to_str().unwrap()));
    let body = fs::read_to_string(&user_cfg).unwrap();
    assert!(body.contains("default_runtime"), "wrote to user-global");
    assert!(!proj.path().join(".hew.toml").exists(), "no project file created");
}

#[test]
fn set_refuses_user_write_when_project_present_no_flags() {
    // Branch 4: project file exists + no flag → refuse with dual-option message.
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(proj.path().join(".hew.toml"), "# placeholder\n").unwrap();

    hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "loop.model.default", "opus-4-7"])
        .assert()
        .failure()
        .stderr(contains("refusing to write to user-global config"))
        .stderr(contains(".hew.toml"))
        .stderr(contains("--project loop.model.default opus-4-7"))
        .stderr(contains("--global  loop.model.default opus-4-7"));
}

#[test]
fn set_global_flag_writes_to_user_when_project_present() {
    // Branch 2: --global wins regardless of project file presence.
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(proj.path().join(".hew.toml"), "# placeholder\n").unwrap();

    hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "--global", "default-runtime", "claude"])
        .assert()
        .success()
        .stdout(contains(user_cfg.to_str().unwrap()));
    let body = fs::read_to_string(&user_cfg).unwrap();
    assert!(body.contains("default_runtime"));
    // Project file is untouched.
    let proj_body = fs::read_to_string(proj.path().join(".hew.toml")).unwrap();
    assert_eq!(proj_body, "# placeholder\n");
}

#[test]
fn set_project_flag_writes_to_project_when_present() {
    // Branch 3a: --project + existing project file → write to that file.
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(proj.path().join(".hew.toml"), "# hew project config\nversion = 1\n").unwrap();

    hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "--project", "loop.fallback_runtime", "codex"])
        .assert()
        .success()
        .stdout(contains(".hew.toml"));

    let proj_body = fs::read_to_string(proj.path().join(".hew.toml")).unwrap();
    assert!(
        proj_body.contains("fallback_runtime = \"codex\""),
        "project file got the new key, body was:\n{proj_body}"
    );
    // User-global untouched (never created here since no --global write happened).
    assert!(!user_cfg.exists(), "user-global stayed untouched");
}

#[test]
fn set_project_flag_creates_project_file_when_absent_with_starter_header() {
    // Branch 3b: --project + no project file → create one with starter
    // header (# hew project config + version = 1).
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");

    hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "--project", "loop.model.default", "claude-opus-4-7"])
        .assert()
        .success();

    let dot = proj.path().join(".hew.toml");
    assert!(dot.exists(), ".hew.toml created");
    let body = fs::read_to_string(&dot).unwrap();
    assert!(body.contains("# hew project config"), "starter header present: {body}");
    assert!(body.contains("version = 1"), "version marker present: {body}");
    assert!(body.contains("default = \"claude-opus-4-7\""), "new key written: {body}");
}

#[test]
fn set_mutually_exclusive_global_and_project_errors_at_clap() {
    // Branch 1: --global + --project → clap conflicts_with rejects upstream.
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "--global", "--project", "default-runtime", "claude"])
        .assert()
        .failure()
        .stderr(contains("cannot be used with"));
}

#[test]
fn set_refusal_message_lists_both_explicit_alternatives() {
    // Acceptance criterion: refusal text matches the plan format exactly
    // — includes both alternatives with the values the user tried.
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(proj.path().join(".hew.toml"), "version = 1\n").unwrap();

    let assert = hew_in_project(&user_cfg, proj.path())
        .args(["config", "set", "branching.strategy", "always"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone()).unwrap();
    // miette reflows the error report into a Unicode-bordered box that
    // wraps long lines. Strip the box prefix (`│ ` / leading whitespace)
    // and collapse internal whitespace so substring matches survive.
    let stripped: String = stderr
        .lines()
        .map(|l| l.trim_start_matches(|c: char| c.is_whitespace() || c == '│'))
        .collect::<Vec<_>>()
        .join(" ");
    let flat: String = stripped.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(flat.contains("--project branching.strategy always"), "stderr:\n{stderr}");
    assert!(flat.contains("--global branching.strategy always"), "stderr:\n{stderr}");
    assert!(flat.contains("commit-shared") || flat.contains("commit- shared"), "stderr:\n{stderr}");
    assert!(flat.contains("personal override"), "stderr:\n{stderr}");
}

#[test]
fn set_help_shows_global_and_project_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("config.toml");
    hew(&cfg)
        .args(["config", "set", "--help"])
        .assert()
        .success()
        .stdout(contains("--global"))
        .stdout(contains("--project"));
}

// ──────── hew-leh9: `hew config show` provenance ────────

/// Build a hew Command rooted in a project dir whose user-global config
/// path is overridden via `HEW_USER_CONFIG` (NOT `HEW_CONFIG`, which is
/// a single-file bypass). Use this for tests that need real layering
/// between user and project files.
fn hew_layered(user_cfg: &std::path::Path, project_root: &std::path::Path) -> Command {
    let mut c = Command::cargo_bin("hew").unwrap();
    c.env_remove("HEW_CONFIG");
    c.env("HEW_USER_CONFIG", user_cfg);
    c.env("NO_COLOR", "1");
    c.env("TERM", "dumb");
    c.current_dir(project_root);
    c
}

#[test]
fn show_lists_sources_in_precedence_order() {
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(&user_cfg, "default_runtime = \"claude\"\n").unwrap();
    fs::write(proj.path().join(".hew.toml"), "[loop]\nfallback_runtime = \"codex\"\n").unwrap();

    let assert = hew_layered(&user_cfg, proj.path()).args(["config", "show"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // sources header in order: user-global before project.
    let user_pos = stdout.find("[user-global]").expect("user-global in output");
    let proj_pos = stdout.find("[project]").expect("project in output");
    assert!(user_pos < proj_pos, "user-global must precede project in:\n{stdout}");
}

#[test]
fn show_attributes_scalar_to_its_source_file() {
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(&user_cfg, "default_runtime = \"claude\"\n").unwrap();
    fs::write(proj.path().join(".hew.toml"), "[loop]\nfallback_runtime = \"codex\"\n").unwrap();

    let assert = hew_layered(&user_cfg, proj.path()).args(["config", "show"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // default-runtime: user-global. loop.fallback_runtime: project.
    let line_runtime =
        stdout.lines().find(|l| l.contains("default-runtime")).expect("default-runtime line");
    assert!(line_runtime.contains("(user-global)"), "line was: {line_runtime}");
    let line_fb = stdout
        .lines()
        .find(|l| l.contains("loop.fallback_runtime"))
        .expect("loop.fallback_runtime line");
    assert!(line_fb.contains("(project)"), "line was: {line_fb}");
}

#[test]
fn show_attributes_array_to_merged_when_both_contributed() {
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(&user_cfg, "[compact]\nexempt = [\"STATUS:user-a\"]\n").unwrap();
    fs::write(proj.path().join(".hew.toml"), "[compact]\nexempt = [\"STATUS:project-b\"]\n")
        .unwrap();

    let assert = hew_layered(&user_cfg, proj.path()).args(["config", "show"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let line = stdout.lines().find(|l| l.contains("compact.exempt")).expect("compact.exempt line");
    assert!(line.contains("(merged)"), "line was: {line}");
}

#[test]
fn show_attributes_default_when_neither_file_set_it() {
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    // No project file. user is also empty.

    let assert = hew_layered(&user_cfg, proj.path()).args(["config", "show"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    let line = stdout.lines().find(|l| l.contains("update-check")).expect("update-check line");
    assert!(line.contains("(default)"), "line was: {line}");
}

#[test]
fn show_env_hew_config_renders_single_source() {
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(&user_cfg, "default_runtime = \"claude\"\n").unwrap();
    // Project file also exists; HEW_CONFIG should bypass both.
    fs::write(proj.path().join(".hew.toml"), "version = 1\n").unwrap();

    let assert = hew_in_project(&user_cfg, proj.path()).args(["config", "show"]).assert().success();
    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Only the env source appears in the sources header.
    assert!(stdout.contains("[env (HEW_CONFIG)]"), "stdout:\n{stdout}");
    assert!(!stdout.contains("[user-global]"), "stdout must not list user-global:\n{stdout}");
    assert!(!stdout.contains("[project]"), "stdout must not list project:\n{stdout}");
    // Keys set in the HEW_CONFIG file attribute to env.
    let line =
        stdout.lines().find(|l| l.contains("default-runtime")).expect("default-runtime line");
    assert!(line.contains("(env)"), "line was: {line}");
}

// ──────── hew-u181: docs/CONFIG.md + CHANGELOG smokes ────────

fn workspace_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is `<workspace>/hew`. Pop one segment.
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn docs_config_md_contains_precedence_table() {
    let body = fs::read_to_string(workspace_root().join("docs/CONFIG.md")).unwrap();
    assert!(body.contains("## Discovery order"), "discovery order section");
    // Precedence table mentions HEW_CONFIG, project, user-global, defaults.
    assert!(body.contains("HEW_CONFIG"), "env source described");
    assert!(body.contains(".hew.toml"), "project file mentioned");
    assert!(body.contains("user-global"), "user-global label present");
}

#[test]
fn docs_config_md_contains_merge_semantics_section() {
    let body = fs::read_to_string(workspace_root().join("docs/CONFIG.md")).unwrap();
    assert!(body.contains("## Merge semantics"), "merge semantics section");
    assert!(body.contains("project wins"), "scalar rule mentioned");
    assert!(body.contains("concat") || body.contains("dedupe"), "array rule mentioned");
}

#[test]
fn changelog_unreleased_has_project_local_config_entry() {
    let body = fs::read_to_string(workspace_root().join("CHANGELOG.md")).unwrap();
    let unreleased = body
        .split("## [Unreleased]")
        .nth(1)
        .and_then(|s| s.split("## [").next())
        .expect("[Unreleased] section");
    assert!(unreleased.contains("hew-c0pa") || unreleased.contains(".hew.toml"));
    assert!(unreleased.contains("project") || unreleased.contains("Project"));
}

#[test]
fn show_json_output_shape_stable() {
    let proj = make_project_root();
    let user_cfg = proj.path().join("user.toml");
    fs::write(&user_cfg, "default_runtime = \"claude\"\n").unwrap();

    let out = hew_layered(&user_cfg, proj.path())
        .args(["--json", "config", "show"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert!(parsed["sources"].is_array(), "json: {parsed}");
    assert!(parsed["keys"].is_object(), "json: {parsed}");
    let runtime = &parsed["keys"]["default-runtime"];
    assert_eq!(runtime["value"], "claude");
    assert_eq!(runtime["source"], "user-global");
    let update_check = &parsed["keys"]["update-check"];
    assert_eq!(update_check["source"], "default");
}
