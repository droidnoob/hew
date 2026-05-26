//! Project-stack detection for the `hew loop` per-iter test/lint gate.
//!
//! Pure FS-based: looks at sentinel files in the project root and picks
//! a `(test_cmd, lint_cmd)` pair that matches the detected language. The
//! gate runner in `hew/src/commands/loop_cmd.rs` spawns those commands.
//!
//! Detection order (first match wins):
//!   1. `Cargo.toml`            → Rust   (cargo test + cargo clippy)
//!   2. `pyproject.toml`        → Python (pytest + ruff)
//!      / `pytest.ini` / `tox.ini` / `setup.cfg` / `setup.py`
//!   3. `go.mod`                → Go     (go test + go vet)
//!   4. `package.json`          → Node   (npm test + npm run lint, both
//!      opt-in via package.json scripts)
//!
//! Returns `None` when no sentinel is present — the runner treats that
//! as "no recognized stack, gate skipped (pass)" rather than failing
//! the loop. ENOENT on the spawn itself (tool not installed) is also
//! treated as skip-pass by the runner, so a Python repo without `ruff`
//! installed degrades gracefully instead of trapping the loop.
//!
//! Future: read `[loop]` overrides from `crate::config` so projects can
//! pin `test_cmd` / `lint_cmd` explicitly. Tracked in hew-hhk.

use std::path::Path;

/// Commands to run for the per-iter gate. Each `Vec<String>` is
/// `[program, arg, arg, ...]` — splitting at the binary boundary keeps
/// the spawn `Command::new(spec.test_cmd[0]).args(&spec.test_cmd[1..])`
/// clean and avoids shell interpolation surprises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GateSpec {
    /// Short identifier shown in stderr breadcrumbs (e.g. "rust", "python").
    pub language: &'static str,
    /// Command + args for the test step. Empty vec = skip test step.
    pub test_cmd: Vec<String>,
    /// Command + args for the lint step. Empty vec = skip lint step.
    pub lint_cmd: Vec<String>,
}

impl GateSpec {
    fn new(language: &'static str, test: &[&str], lint: &[&str]) -> Self {
        Self {
            language,
            test_cmd: test.iter().map(|s| s.to_string()).collect(),
            lint_cmd: lint.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Detect the project's gate from sentinel files in `root`.
///
/// Returns `None` if no recognized stack is present. The runner treats
/// `None` as "skip the gate" — no recognized project means there's
/// nothing meaningful to run, and failing the loop on that basis was
/// the bug fix this whole module exists for.
pub fn detect(root: &Path) -> Option<GateSpec> {
    if root.join("Cargo.toml").exists() {
        return Some(GateSpec::new(
            "rust",
            &["cargo", "test", "--quiet"],
            &["cargo", "clippy", "--all-targets", "--", "-D", "warnings"],
        ));
    }

    if is_python_project(root) {
        return Some(GateSpec::new("python", &["pytest", "-q"], &["ruff", "check", "."]));
    }

    if root.join("go.mod").exists() {
        return Some(GateSpec::new("go", &["go", "test", "./..."], &["go", "vet", "./..."]));
    }

    if root.join("package.json").exists() {
        let (has_test, has_lint) = package_json_scripts(&root.join("package.json"));
        let test_cmd: &[&str] = if has_test { &["npm", "test", "--silent"] } else { &[] };
        let lint_cmd: &[&str] = if has_lint { &["npm", "run", "lint", "--silent"] } else { &[] };
        return Some(GateSpec::new("node", test_cmd, lint_cmd));
    }

    None
}

fn is_python_project(root: &Path) -> bool {
    for sentinel in
        ["pyproject.toml", "pytest.ini", "tox.ini", "setup.cfg", "setup.py", "requirements.txt"]
    {
        if root.join(sentinel).exists() {
            return true;
        }
    }
    false
}

/// Cheap, allocation-light scan of `package.json` for `scripts.test` and
/// `scripts.lint` keys. Avoids pulling serde_json into hew-core just for
/// this — a contains-substring check is sufficient because the keys are
/// well-known and we only care about presence, not value validity.
fn package_json_scripts(path: &Path) -> (bool, bool) {
    let Ok(s) = std::fs::read_to_string(path) else {
        return (false, false);
    };
    (s.contains("\"test\""), s.contains("\"lint\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn touch(dir: &Path, name: &str) {
        fs::write(dir.join(name), "").unwrap();
    }

    #[test]
    fn detects_rust_via_cargo_toml() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "Cargo.toml");
        let spec = detect(tmp.path()).expect("rust detected");
        assert_eq!(spec.language, "rust");
        assert_eq!(spec.test_cmd[0], "cargo");
        assert_eq!(spec.lint_cmd[0], "cargo");
    }

    #[test]
    fn detects_python_via_pyproject() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "pyproject.toml");
        let spec = detect(tmp.path()).expect("python detected");
        assert_eq!(spec.language, "python");
        assert_eq!(spec.test_cmd[0], "pytest");
        assert_eq!(spec.lint_cmd[0], "ruff");
    }

    #[test]
    fn detects_python_via_alternate_sentinels() {
        for sentinel in ["pytest.ini", "tox.ini", "setup.cfg", "setup.py", "requirements.txt"] {
            let tmp = TempDir::new().unwrap();
            touch(tmp.path(), sentinel);
            let spec = detect(tmp.path()).unwrap_or_else(|| panic!("python via {sentinel}"));
            assert_eq!(spec.language, "python", "sentinel={sentinel}");
        }
    }

    #[test]
    fn detects_go_via_go_mod() {
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "go.mod");
        let spec = detect(tmp.path()).unwrap();
        assert_eq!(spec.language, "go");
        assert_eq!(spec.test_cmd[..2], ["go".to_string(), "test".to_string()]);
        assert_eq!(spec.lint_cmd[..2], ["go".to_string(), "vet".to_string()]);
    }

    #[test]
    fn detects_node_and_reads_scripts() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("package.json"),
            r#"{ "scripts": { "test": "jest", "lint": "eslint ." } }"#,
        )
        .unwrap();
        let spec = detect(tmp.path()).unwrap();
        assert_eq!(spec.language, "node");
        assert!(!spec.test_cmd.is_empty());
        assert!(!spec.lint_cmd.is_empty());
    }

    #[test]
    fn node_without_lint_script_skips_lint() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), r#"{ "scripts": { "test": "jest" } }"#).unwrap();
        let spec = detect(tmp.path()).unwrap();
        assert!(!spec.test_cmd.is_empty(), "test cmd present");
        assert!(spec.lint_cmd.is_empty(), "no lint script → no lint cmd");
    }

    #[test]
    fn empty_dir_returns_none() {
        let tmp = TempDir::new().unwrap();
        assert!(detect(tmp.path()).is_none());
    }

    #[test]
    fn cargo_toml_wins_over_pyproject() {
        // A polyglot repo with both → rust wins (first match).
        let tmp = TempDir::new().unwrap();
        touch(tmp.path(), "Cargo.toml");
        touch(tmp.path(), "pyproject.toml");
        assert_eq!(detect(tmp.path()).unwrap().language, "rust");
    }
}
