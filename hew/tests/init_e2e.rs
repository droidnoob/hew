//! `hew init` end-to-end against a fake `bd` and an isolated project dir.

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
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
    // Isolate config writes — init persists choices to ~/.config/hew/config.toml
    // from IV.4 onward; tests must not stomp the user's real config.
    c.env("HEW_CONFIG", project.join("hew-test-config.toml"));
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
        .stdout(contains("Setup complete"))
        .stdout(contains("runtime           claude"));

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
fn init_persists_git_track_to_config() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--git-track"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains("git_track = true"), "config:\n{cfg_body}");
}

#[test]
fn init_no_git_repo_forces_git_track_false() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    // No .git/ — even with no flag, git_track must end up false and .beads/ gitignored.

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success();

    let gi = fs::read_to_string(project.path().join(".gitignore")).unwrap_or_default();
    assert!(gi.contains(".beads/"), ".beads/ should be gitignored when no .git/:\n{gi}");
    let cfg_body =
        fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap_or_default();
    assert!(cfg_body.contains("git_track = false"), "config:\n{cfg_body}");
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
fn init_banner_absent_in_non_interactive_runs() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("Carve code, not chaos.").not());
}

#[test]
fn init_summary_panel_renders_all_rows() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("Setup complete"))
        .stdout(contains("runtime           claude"))
        .stdout(contains("branching         epic"))
        .stdout(contains("optional skills   deps=ask"))
        .stdout(contains("require tests     no"))
        .stdout(contains("review cadence    off"));
}

#[test]
fn init_quiet_suppresses_panel_keeps_one_liner() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--quiet"])
        .assert()
        .success()
        .stdout(contains("hew installed for claude"))
        .stdout(contains("Setup complete").not());
}

#[test]
fn init_advanced_defaults_preserved_when_not_set() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains(r#"default = "ask""#), "config:\n{cfg_body}");
    assert!(cfg_body.contains("after_n_tasks = 0"), "config:\n{cfg_body}");
    assert!(cfg_body.contains("after_epic = false"), "config:\n{cfg_body}");
}

#[test]
fn init_research_default_flag_persists() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args([
            "init",
            "--non-interactive",
            "--runtime",
            "claude",
            "--research-default",
            "auto-run",
        ])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains(r#"default = "auto-run""#), "config:\n{cfg_body}");
}

#[test]
fn init_review_cadence_flags_persist() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args([
            "init",
            "--non-interactive",
            "--runtime",
            "claude",
            "--review-after-n",
            "5",
            "--review-after-epic",
        ])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains("after_n_tasks = 5"), "config:\n{cfg_body}");
    assert!(cfg_body.contains("after_epic = true"), "config:\n{cfg_body}");
}

#[test]
fn init_require_tests_default_false() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains("require = false"), "config:\n{cfg_body}");
}

#[test]
fn init_require_tests_flag_persists_true() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--require-tests"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains("require = true"), "config:\n{cfg_body}");
}

#[test]
fn init_optional_skills_default_to_ask() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains(r#"deps = "ask""#), "config:\n{cfg_body}");
    assert!(cfg_body.contains(r#"research = "ask""#), "config:\n{cfg_body}");
    assert!(cfg_body.contains(r#"security = "ask""#), "config:\n{cfg_body}");
}

#[test]
fn init_optional_skills_flags_persist() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args([
            "init",
            "--non-interactive",
            "--runtime",
            "claude",
            "--deps",
            "yes",
            "--security",
            "no",
        ])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains(r#"deps = "yes""#), "config:\n{cfg_body}");
    assert!(cfg_body.contains(r#"security = "no""#), "config:\n{cfg_body}");
    assert!(cfg_body.contains(r#"research = "ask""#), "config:\n{cfg_body}");
}

#[test]
fn init_optional_skills_reject_invalid_value() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--deps", "maybe"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn init_branching_defaults_to_epic_non_interactive() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains(r#"strategy = "epic""#), "config:\n{cfg_body}");
}

#[test]
fn init_branching_flag_persists() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--branching", "none"])
        .assert()
        .success();

    let cfg_body = fs::read_to_string(project.path().join("hew-test-config.toml")).unwrap();
    assert!(cfg_body.contains(r#"strategy = "none""#), "config:\n{cfg_body}");
}

#[test]
fn init_branching_rejects_invalid_value() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--branching", "weekly"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn init_project_type_defaults_to_new_in_empty_dir() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("Next: /hew:new-project"));
}

#[test]
fn init_project_type_detects_existing_codebase() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join("src")).unwrap();
    fs::write(project.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("Next: /hew:scan to map this codebase"));
}

#[test]
fn init_project_type_flag_overrides_detection() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join("src")).unwrap(); // would auto-detect existing

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude", "--project-type", "new"])
        .assert()
        .success()
        .stdout(contains("Next: /hew:new-project"));
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
fn init_errors_clearly_when_bd_missing_and_no_installer_available() {
    // Empty stub dir → no bd, no brew, no curl on PATH. hew init now
    // *requires* Beads, so it must surface a clear message naming the
    // installers it tried.
    let empty = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), empty.path())
        .args(["init", "--non-interactive"])
        .assert()
        .failure()
        .stderr(contains("Beads is required"))
        .stderr(contains("brew"))
        .stderr(contains("curl"));
}

#[test]
fn init_prints_beads_status_lines() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("beads: ✓ on PATH"))
        .stdout(contains("beads: ✓ task graph initialised in .beads/"));
}

#[test]
fn init_skips_beads_init_message_when_already_initialised() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join(".beads")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("beads: ✓ on PATH"))
        .stdout(contains("task graph initialised").not());
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

#[test]
fn init_warns_when_git_missing_in_non_interactive() {
    // PATH points only at the bd stub — no git, no anything else.
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stderr(contains("`git` not on PATH"))
        .stderr(contains("auto-branching will be skipped"));
}

#[test]
fn init_runs_git_init_when_repo_absent() {
    // git stub that creates .git/ on `init --quiet` (mimics real git init).
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    let git_stub = r#"#!/bin/sh
PATH="/usr/bin:/bin:$PATH"
case "$1" in
  --version) echo "git version 0.0-stub"; exit 0 ;;
  -C)
    target="$2"
    if [ "$3" = "init" ]; then
      mkdir -p "$target/.git"
      exit 0
    fi
    ;;
esac
exit 0
"#;
    let git_path = stub_dir.path().join("git");
    fs::write(&git_path, git_stub).unwrap();
    let mut perms = fs::metadata(&git_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&git_path, perms).unwrap();

    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("git: ✓ on PATH"))
        .stdout(contains("git: ✓ initialised repo"));

    // Resolve symlinks (/tmp -> /private/tmp on macOS) so the path the stub
    // wrote into matches the path we're checking.
    let resolved = fs::canonicalize(project.path()).unwrap();
    assert!(resolved.join(".git").exists(), ".git/ should exist after init");
}

#[test]
fn init_skips_git_init_when_repo_present() {
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    // Stub that explodes if called with `init` — we want to verify we skip.
    let git_stub = r#"#!/bin/sh
case "$1" in
  --version) exit 0 ;;
  -C)
    if [ "$3" = "init" ]; then
      echo "should not run init on existing repo" >&2
      exit 17
    fi
    ;;
esac
exit 0
"#;
    let git_path = stub_dir.path().join("git");
    fs::write(&git_path, git_stub).unwrap();
    let mut perms = fs::metadata(&git_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&git_path, perms).unwrap();

    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();
    fs::create_dir(project.path().join(".git")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stdout(contains("git: ✓ on PATH"))
        .stdout(contains("git: ✓ initialised repo").not());
}

#[test]
fn init_does_not_warn_when_git_present() {
    // PATH includes both bd and git stubs.
    let stub_dir = tempfile::tempdir().unwrap();
    install_stub(stub_dir.path(), BD_STUB_OK);
    // Drop a git stub that exits 0 on --version so RealGit::is_available() finds it.
    let git_path = stub_dir.path().join("git");
    fs::write(&git_path, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&git_path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&git_path, perms).unwrap();

    let project = tempfile::tempdir().unwrap();
    fs::create_dir(project.path().join(".claude")).unwrap();

    hew_with_stub(project.path(), stub_dir.path())
        .args(["init", "--non-interactive", "--runtime", "claude"])
        .assert()
        .success()
        .stderr(predicates::str::contains("`git` not on PATH").not());
}
