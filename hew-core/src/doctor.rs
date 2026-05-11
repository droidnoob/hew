//! Project health diagnostics.
//!
//! Each check returns a `CheckResult`; the caller decides how to render.
//! Doctor does not auto-fix on its own — `--fix` is wired by the binary
//! and only enables fixes for checks that opt into safe self-repair.

use std::path::Path;

use crate::bd::BdClient;
use crate::install;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub name: &'static str,
    pub severity: Severity,
    pub message: String,
}

impl CheckResult {
    fn ok(name: &'static str, msg: impl Into<String>) -> Self {
        Self { name, severity: Severity::Ok, message: msg.into() }
    }
    fn warn(name: &'static str, msg: impl Into<String>) -> Self {
        Self { name, severity: Severity::Warn, message: msg.into() }
    }
    fn fail(name: &'static str, msg: impl Into<String>) -> Self {
        Self { name, severity: Severity::Fail, message: msg.into() }
    }
}

pub struct DoctorReport {
    pub checks: Vec<CheckResult>,
    pub fixed: Vec<String>,
}

impl DoctorReport {
    pub fn overall(&self) -> Severity {
        if self.checks.iter().any(|c| c.severity == Severity::Fail) {
            Severity::Fail
        } else if self.checks.iter().any(|c| c.severity == Severity::Warn) {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
}

/// Run every check. `project_root` is the directory to inspect.
/// `fix` enables safe self-repair on checks that support it.
pub fn run(client: &dyn BdClient, project_root: &Path, fix: bool) -> DoctorReport {
    let mut checks = Vec::new();
    let mut fixed = Vec::new();

    checks.push(check_bd(client));
    checks.push(check_beads_dir(project_root));
    let (gi_check, gi_fixed) = check_gitignore(project_root, fix);
    checks.push(gi_check);
    if let Some(line) = gi_fixed {
        fixed.push(line);
    }
    checks.push(check_runtime_markers(project_root));
    checks.push(check_skills_layout(project_root));

    DoctorReport { checks, fixed }
}

fn check_bd(client: &dyn BdClient) -> CheckResult {
    match client.version() {
        Ok(v) => CheckResult::ok("bd", format!("v{} on PATH", v.semver)),
        Err(_) => CheckResult::fail("bd", "binary not found on PATH; install Beads"),
    }
}

fn check_beads_dir(project_root: &Path) -> CheckResult {
    if project_root.join(".beads").exists() {
        CheckResult::ok("beads", ".beads/ present")
    } else {
        CheckResult::fail("beads", ".beads/ missing — run `hew init`")
    }
}

/// Returns the check plus, when --fix is set and we actually wrote, a one-line summary.
fn check_gitignore(project_root: &Path, fix: bool) -> (CheckResult, Option<String>) {
    let gi = project_root.join(".gitignore");
    let body = std::fs::read_to_string(&gi).unwrap_or_default();
    let listed = body.lines().any(|l| l.trim() == ".beads/" || l.trim() == ".beads");

    if listed {
        return (CheckResult::ok("gitignore", ".beads/ ignored"), None);
    }

    if !fix {
        return (
            CheckResult::warn(
                "gitignore",
                ".beads/ not in .gitignore — add it, or re-run with --fix",
            ),
            None,
        );
    }

    match install::ensure_beads_gitignored(project_root) {
        Ok(true) => (
            CheckResult::ok("gitignore", ".beads/ added to .gitignore (fix applied)"),
            Some("added .beads/ to .gitignore".into()),
        ),
        Ok(false) => (CheckResult::ok("gitignore", ".beads/ already ignored"), None),
        Err(e) => (CheckResult::fail("gitignore", format!("could not update: {e}")), None),
    }
}

fn check_runtime_markers(project_root: &Path) -> CheckResult {
    let found = install::detect_runtimes(project_root);
    if found.is_empty() {
        CheckResult::warn(
            "runtime",
            "no agent runtime detected (.claude/.cursor/.codex/.windsurf missing)",
        )
    } else {
        let names: Vec<&str> = found.iter().map(|r| r.as_str()).collect();
        CheckResult::ok("runtime", format!("detected: {}", names.join(", ")))
    }
}

fn check_skills_layout(project_root: &Path) -> CheckResult {
    let hew_root = project_root.join(".claude").join("skills").join("hew");
    if !hew_root.exists() {
        return CheckResult::warn(
            "skills",
            ".claude/skills/hew/ not present — run `hew init --runtime=claude`",
        );
    }
    let must_have = ["SKILL.md", "core/hew-execute.md", "core/hew-plan.md"];
    let missing: Vec<&str> =
        must_have.iter().filter(|p| !hew_root.join(p).exists()).copied().collect();
    if missing.is_empty() {
        CheckResult::ok("skills", "core skill files present")
    } else {
        CheckResult::warn("skills", format!("missing: {}", missing.join(", ")))
    }
}

pub fn render_text(r: &DoctorReport) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let _ = writeln!(s, "hew doctor");
    let _ = writeln!(s, "──────────────────────────────────");
    for c in &r.checks {
        let mark = match c.severity {
            Severity::Ok => "✓",
            Severity::Warn => "⚠",
            Severity::Fail => "✗",
        };
        let _ = writeln!(s, "  {mark} {:<10}  {}", c.name, c.message);
    }
    if !r.fixed.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "Applied fixes:");
        for line in &r.fixed {
            let _ = writeln!(s, "  • {line}");
        }
    }
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Overall: {}",
        match r.overall() {
            Severity::Ok => "ok",
            Severity::Warn => "warnings",
            Severity::Fail => "fail",
        }
    );
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::ffi::OsStr;

    use crate::bd::{BdOutput, BdVersion, ReadyTask, StatsSummary};
    use crate::error::{HewError, Result};

    #[derive(Debug)]
    struct FakeBd {
        ok: bool,
    }
    impl BdClient for FakeBd {
        fn version(&self) -> Result<BdVersion> {
            if self.ok {
                Ok(BdVersion { raw: "bd version 1.0.0".into(), semver: "1.0.0".into() })
            } else {
                Err(HewError::BdNotFound)
            }
        }
        fn ready(&self) -> Result<Vec<ReadyTask>> {
            Ok(vec![])
        }
        fn stats(&self) -> Result<StatsSummary> {
            Ok(StatsSummary::default())
        }
        fn prime_raw(&self) -> Result<String> {
            Ok(String::new())
        }
        fn memories(&self) -> Result<BTreeMap<String, String>> {
            Ok(BTreeMap::new())
        }
        fn remember(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, _: &[&OsStr]) -> Result<BdOutput> {
            Ok(BdOutput { stdout: String::new(), stderr: String::new() })
        }
    }

    #[test]
    fn bd_missing_is_a_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let r = run(&FakeBd { ok: false }, tmp.path(), false);
        assert_eq!(r.overall(), Severity::Fail);
    }

    #[test]
    fn beads_dir_missing_is_a_fail() {
        let tmp = tempfile::tempdir().unwrap();
        let r = run(&FakeBd { ok: true }, tmp.path(), false);
        assert_eq!(r.overall(), Severity::Fail);
    }

    #[test]
    fn warns_without_fix_when_gitignore_missing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".beads")).unwrap();
        std::fs::create_dir(tmp.path().join(".claude")).unwrap();
        let r = run(&FakeBd { ok: true }, tmp.path(), false);
        assert_eq!(r.overall(), Severity::Warn);
        let gi = r.checks.iter().find(|c| c.name == "gitignore").unwrap();
        assert_eq!(gi.severity, Severity::Warn);
    }

    #[test]
    fn fix_applies_gitignore_update() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".beads")).unwrap();
        std::fs::create_dir(tmp.path().join(".claude")).unwrap();
        let r = run(&FakeBd { ok: true }, tmp.path(), true);
        assert!(r.fixed.iter().any(|f| f.contains(".beads/")));
        let body = std::fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(body.contains(".beads/"));
    }

    #[test]
    fn all_green_passes() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join(".beads")).unwrap();
        std::fs::create_dir(tmp.path().join(".claude")).unwrap();
        std::fs::write(tmp.path().join(".gitignore"), ".beads/\n").unwrap();
        // Lay down the skills tree manually to satisfy the layout check.
        for sub in ["core", "brownfield", "optional", "custom"] {
            std::fs::create_dir_all(
                tmp.path().join(".claude").join("skills").join("hew").join(sub),
            )
            .unwrap();
        }
        let hew_root = tmp.path().join(".claude").join("skills").join("hew");
        std::fs::write(hew_root.join("SKILL.md"), "x").unwrap();
        std::fs::write(hew_root.join("core").join("hew-execute.md"), "x").unwrap();
        std::fs::write(hew_root.join("core").join("hew-plan.md"), "x").unwrap();

        let r = run(&FakeBd { ok: true }, tmp.path(), false);
        assert_eq!(r.overall(), Severity::Ok, "{:?}", r.checks);
    }
}
