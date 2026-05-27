//! Project-authored gate signals for the `hew loop` per-iter check.
//!
//! Reads `(test_cmd, lint_cmd)` from signals the project owner already
//! wrote — `Makefile` targets, `justfile` recipes, `package.json`
//! scripts. We do *not* infer commands from language sentinels
//! (`Cargo.toml`, `pyproject.toml`, etc.) because guessing the right
//! invocation per language is presumptuous: `pytest -q` vs `uv run
//! pytest tests/` vs `nox -s tests` is the project owner's call, not
//! ours.
//!
//! Lookup order (first match per step wins):
//!   1. `justfile` recipe named `test` / `lint`     → `just test`, `just lint`
//!   2. `Makefile` target named `test` / `lint`     → `make test`, `make lint`
//!   3. `package.json` script named `test` / `lint` → `npm test`, `npm run lint`
//!
//! If no signal is found for a step, that step is skipped (treated as
//! pass with a stderr breadcrumb in the runner). If the project wants
//! a gate, it adds a `test` target/recipe/script. If it doesn't, the
//! loop runs without one — that's the correct default given the
//! agent inside the loop can run whatever checks it wants directly
//! via Bash anyway.
//!
//! A future config override (`[loop.gate] test_cmd = "..."` in hew
//! config) is tracked in hew-hhk and will take precedence over these
//! signals once landed.

use std::path::{Path, PathBuf};

/// Commands to run for the per-iter gate. Each `Vec<String>` is
/// `[program, arg, arg, ...]`. Empty vec = skip that step (pass).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GateSpec {
    pub test_cmd: Vec<String>,
    pub lint_cmd: Vec<String>,
}

impl GateSpec {
    /// True iff at least one step is wired. The runner uses this to
    /// emit a "no gate signals found" breadcrumb when both are empty.
    pub fn has_any(&self) -> bool {
        !self.test_cmd.is_empty() || !self.lint_cmd.is_empty()
    }
}

/// Resolve the gate for `root` from project-authored signals.
///
/// Always returns a `GateSpec` (possibly empty). Empty vec on a step
/// means "no signal for this step, skip it" — the runner treats that
/// as pass with a breadcrumb, not as a failure.
pub fn detect(root: &Path) -> GateSpec {
    let mut spec = GateSpec::default();

    if let Some(path) = first_existing(root, &["justfile", ".justfile", "Justfile"]) {
        let recipes = parse_justfile(&path);
        if recipes.contains(&"test".to_string()) {
            spec.test_cmd = vec!["just".into(), "test".into()];
        }
        if recipes.contains(&"lint".to_string()) {
            spec.lint_cmd = vec!["just".into(), "lint".into()];
        }
    }

    if !spec.has_any()
        && let Some(path) = first_existing(root, &["Makefile", "makefile", "GNUmakefile"])
    {
        let targets = parse_makefile(&path);
        if spec.test_cmd.is_empty() && targets.contains(&"test".to_string()) {
            spec.test_cmd = vec!["make".into(), "test".into()];
        }
        if spec.lint_cmd.is_empty() && targets.contains(&"lint".to_string()) {
            spec.lint_cmd = vec!["make".into(), "lint".into()];
        }
    }

    if !spec.has_any() && root.join("package.json").exists() {
        let (has_test, has_lint) = package_json_scripts(&root.join("package.json"));
        if spec.test_cmd.is_empty() && has_test {
            spec.test_cmd = vec!["npm".into(), "test".into(), "--silent".into()];
        }
        if spec.lint_cmd.is_empty() && has_lint {
            spec.lint_cmd = vec!["npm".into(), "run".into(), "lint".into(), "--silent".into()];
        }
    }

    spec
}

fn first_existing(root: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        let p = root.join(name);
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// Extract Makefile target names. Matches lines of the form
/// `<name>:` or `<name>: deps...` at column 0, excluding pattern
/// rules (`%:`), variable assignments (`FOO := bar`), and the
/// `.PHONY` declaration itself. Best-effort — bad Makefiles just
/// produce a sparser set of detected targets.
fn parse_makefile(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut targets = Vec::new();
    for line in content.lines() {
        if line.is_empty() || line.starts_with('\t') || line.starts_with('#') {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        if line.contains(":=") || line.contains("::=") || line.contains("?=") || line.contains("+=")
        {
            continue;
        }
        let name = line[..colon].trim();
        if name.is_empty() || name.starts_with('.') || name.starts_with('%') || name.contains(' ') {
            continue;
        }
        targets.push(name.to_string());
    }
    targets
}

/// Extract justfile recipe names. Matches lines of the form `name:`
/// or `name args:` at column 0. Skips assignments (`foo := "bar"`),
/// comments, and indented lines (recipe bodies).
fn parse_justfile(path: &Path) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut recipes = Vec::new();
    for line in content.lines() {
        if line.is_empty() || line.starts_with([' ', '\t', '#', '@']) {
            continue;
        }
        let Some(colon) = line.find(':') else { continue };
        if line.contains(":=") {
            continue;
        }
        // Recipe header may be `name` or `name arg1 arg2`. Pull the
        // first whitespace-delimited token before the colon.
        let head = &line[..colon];
        let Some(name) = head.split_whitespace().next() else { continue };
        if name.starts_with('_') {
            // Private recipes — skip, but they could still be `test` etc.
            // if explicitly prefixed; leaving them out is the conservative call.
            continue;
        }
        recipes.push(name.to_string());
    }
    recipes
}

/// Cheap presence-check for `scripts.test` / `scripts.lint` in
/// `package.json`. Contains-substring is good enough — these keys are
/// well-known and we only need presence, not value validation.
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

    #[test]
    fn empty_dir_returns_empty_spec() {
        let tmp = TempDir::new().unwrap();
        let spec = detect(tmp.path());
        assert!(!spec.has_any());
    }

    #[test]
    fn detects_makefile_test_and_lint() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Makefile"), "test:\n\tcargo test\n\nlint:\n\tcargo clippy\n")
            .unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd, vec!["make", "test"]);
        assert_eq!(spec.lint_cmd, vec!["make", "lint"]);
    }

    #[test]
    fn makefile_with_only_test_target() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Makefile"), "test:\n\tpytest\n").unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd, vec!["make", "test"]);
        assert!(spec.lint_cmd.is_empty());
    }

    #[test]
    fn makefile_ignores_assignments_and_pattern_rules() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("Makefile"),
            "CC := gcc\n%.o: %.c\n\t$(CC) -c $<\ntest:\n\techo hi\n",
        )
        .unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd, vec!["make", "test"]);
    }

    #[test]
    fn detects_justfile() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("justfile"), "test:\n    pytest\n\nlint:\n    ruff check .\n")
            .unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd, vec!["just", "test"]);
        assert_eq!(spec.lint_cmd, vec!["just", "lint"]);
    }

    #[test]
    fn justfile_recipe_with_args() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("justfile"), "test filter='':\n    pytest -k {{filter}}\n")
            .unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd, vec!["just", "test"]);
    }

    #[test]
    fn justfile_wins_over_makefile_when_both_present() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("justfile"), "test:\n    pytest\n").unwrap();
        fs::write(tmp.path().join("Makefile"), "test:\n\techo make\n").unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd, vec!["just", "test"]);
    }

    #[test]
    fn detects_package_json_scripts() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("package.json"),
            r#"{ "scripts": { "test": "jest", "lint": "eslint ." } }"#,
        )
        .unwrap();
        let spec = detect(tmp.path());
        assert_eq!(spec.test_cmd[..2], ["npm".to_string(), "test".to_string()]);
        assert_eq!(spec.lint_cmd[..3], ["npm".to_string(), "run".to_string(), "lint".to_string()]);
    }

    #[test]
    fn package_json_without_lint_script_skips_lint() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("package.json"), r#"{ "scripts": { "test": "jest" } }"#).unwrap();
        let spec = detect(tmp.path());
        assert!(!spec.test_cmd.is_empty());
        assert!(spec.lint_cmd.is_empty());
    }

    #[test]
    fn cargo_project_without_makefile_returns_empty() {
        // No more language-table inference. Pure Cargo project with no
        // Makefile/justfile/package.json → no gate. The agent runs
        // checks directly via Bash inside the iter.
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let spec = detect(tmp.path());
        assert!(!spec.has_any(), "Cargo-only project should not auto-gate");
    }
}
