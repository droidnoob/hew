//! Craft-principle soft warnings for `hew-guard`.
//!
//! `hew-guard` is the pre-close sanity gate. The seven hard checks in
//! its skill body (debug statements, secrets, lint, tests, etc.) are
//! enforced by the agent. This module adds a parallel layer of *soft*
//! signals driven by the project's chosen craft principles
//! (`CONVENTION:craft.<id>` memories) and `[craft]` / `[testing]` config.
//!
//! Per `DECISION:craft-enforcement` (memory), warnings here NEVER block
//! `bd close` on their own. The one current promotion path is
//! `testing.require = true`, which lifts a "missing-tests" warning from
//! [`Severity::Warn`] to [`Severity::Fail`] — and even then it's the
//! executor's choice whether to refuse the close.
//!
//! [`craft_warnings`] is a pure function: caller passes in the diff text,
//! the memories map, and the loaded [`Config`]. No bd / git calls. This
//! makes the heuristics trivially unit-testable on synthetic diffs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::Config;

/// Severity of a single craft warning. `Warn` is the default; `Fail`
/// is reserved for promotions driven by explicit per-rule config
/// (currently only `testing.require = true`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Severity {
    Warn,
    Fail,
}

/// One soft warning surfaced by `hew-guard`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CraftWarning {
    /// Stable rule id (e.g. `"missing-tests"`, `"function-length"`,
    /// `"duplication"`). Useful for filtering / silencing.
    pub rule: String,
    pub severity: Severity,
    /// Path as it appears in the diff (post-rename, without the `b/`).
    pub file: String,
    /// 1-based line within the post-image, when known.
    pub line: Option<u32>,
    /// Human-readable explanation of what triggered.
    pub message: String,
    /// One short sentence on how to silence: config flip, memory removal,
    /// or a `CONVENTION:` carve-out.
    pub silence: String,
}

/// Compute craft soft-warnings for a staged diff.
///
/// `memories` is the raw `bd memories --json` map: keys are slugs, values
/// are the full memory bodies (the `CONVENTION:craft.<id>` membership is
/// derived from `value.trim().starts_with("CONVENTION:craft.")`).
///
/// `diff` is unified `git diff` output. Empty diff → empty result.
///
/// Heuristics:
///
/// 1. **missing-tests** — every changed source file lacking a co-changed
///    test sibling earns a warning. `testing.require=true` promotes to
///    [`Severity::Fail`]. Always-on (doesn't require a craft memory).
/// 2. **function-length** — gated on `cfg.craft.max_function_lines > 0`.
///    Within each post-image hunk, detects function definitions per
///    language and counts the trailing run of added lines until the next
///    sibling-or-shallower def. Lines over threshold → warning.
/// 3. **duplication** — gated on a `CONVENTION:craft.dry` memory being
///    present. Flags any run of ≥5 consecutive non-trivial added lines
///    that appears in two distinct files (or twice in the same file).
pub fn craft_warnings(
    memories: &BTreeMap<String, String>,
    diff: &str,
    cfg: &Config,
) -> Vec<CraftWarning> {
    let parsed = parse_diff(diff);
    let mut out = Vec::new();

    out.extend(check_missing_tests(&parsed, cfg));

    if cfg.craft.max_function_lines > 0 {
        out.extend(check_function_length(&parsed, cfg.craft.max_function_lines));
    }

    if has_dry_memory(memories) {
        out.extend(check_duplication(&parsed));
    }

    out
}

// ────────────────────────────────────────────────────────────────────────────
// Heuristic 1: missing tests
// ────────────────────────────────────────────────────────────────────────────

fn check_missing_tests(parsed: &ParsedDiff, cfg: &Config) -> Vec<CraftWarning> {
    let severity = if cfg.testing.require { Severity::Fail } else { Severity::Warn };
    let silence = if cfg.testing.require {
        "set `testing.require = false` to demote to warning, or co-change a test for this file"
    } else {
        "co-change a test file alongside the source, or add `CONVENTION:tests-exempt` if this path is glue/config"
    };

    let any_test_changed = parsed.files.iter().any(|f| looks_like_test_path(&f.path));

    let mut out = Vec::new();
    for f in &parsed.files {
        if !is_behavior_changing_source(f) {
            continue;
        }
        if looks_like_test_path(&f.path) {
            continue;
        }
        if any_test_changed && has_sibling_test(&f.path, parsed) {
            continue;
        }
        out.push(CraftWarning {
            rule: "missing-tests".to_string(),
            severity,
            file: f.path.clone(),
            line: None,
            message: format!(
                "behavior-changing file `{}` has no co-changed test in this diff",
                f.path
            ),
            silence: silence.to_string(),
        });
    }
    out
}

fn looks_like_test_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.contains("/tests/")
        || p.contains("/test/")
        || p.starts_with("tests/")
        || p.starts_with("test/")
        || p.ends_with("_test.go")
        || p.ends_with("_test.py")
        || p.ends_with(".test.ts")
        || p.ends_with(".test.tsx")
        || p.ends_with(".test.js")
        || p.ends_with(".spec.ts")
        || p.ends_with(".spec.js")
        || p.contains("test_")
        || p.ends_with("/conftest.py")
        // Rust convention: tests live inline in `mod tests` or under `tests/`.
        || (p.ends_with(".rs") && p.contains("/tests/"))
}

fn is_behavior_changing_source(f: &ParsedFile) -> bool {
    if is_excluded_path(&f.path) {
        return false;
    }
    if !is_source_language(&f.path) {
        return false;
    }
    f.added_lines
        .iter()
        .any(|l| !l.trimmed_content.is_empty() && !is_comment_only(&l.trimmed_content, &f.path))
}

fn is_excluded_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".md")
        || p.ends_with(".txt")
        || p.ends_with(".toml")
        || p.ends_with(".yaml")
        || p.ends_with(".yml")
        || p.ends_with(".json")
        || p.ends_with(".lock")
        || p.ends_with(".css")
        || p.ends_with(".scss")
        || p.ends_with(".html")
        || p.contains("/generated/")
        || p.contains(".generated.")
}

fn is_source_language(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".py")
        || p.ends_with(".rs")
        || p.ends_with(".ts")
        || p.ends_with(".tsx")
        || p.ends_with(".js")
        || p.ends_with(".jsx")
        || p.ends_with(".go")
        || p.ends_with(".rb")
        || p.ends_with(".java")
        || p.ends_with(".kt")
}

fn is_comment_only(line: &str, path: &str) -> bool {
    let l = line.trim_start();
    if l.is_empty() {
        return true;
    }
    let p = path.to_ascii_lowercase();
    if p.ends_with(".py") || p.ends_with(".rb") {
        return l.starts_with('#');
    }
    if p.ends_with(".rs")
        || p.ends_with(".ts")
        || p.ends_with(".tsx")
        || p.ends_with(".js")
        || p.ends_with(".jsx")
        || p.ends_with(".go")
        || p.ends_with(".java")
        || p.ends_with(".kt")
    {
        return l.starts_with("//") || l.starts_with("/*") || l.starts_with('*');
    }
    false
}

fn has_sibling_test(source_path: &str, parsed: &ParsedDiff) -> bool {
    let stem = file_stem(source_path);
    if stem.is_empty() {
        return false;
    }
    parsed.files.iter().any(|f| {
        if !looks_like_test_path(&f.path) {
            return false;
        }
        let other_stem = file_stem(&f.path);
        other_stem.contains(stem) || stem.contains(other_stem.trim_start_matches("test_"))
    })
}

fn file_stem(path: &str) -> &str {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.split('.').next().unwrap_or(base)
}

// ────────────────────────────────────────────────────────────────────────────
// Heuristic 2: function length
// ────────────────────────────────────────────────────────────────────────────

fn check_function_length(parsed: &ParsedDiff, threshold: u32) -> Vec<CraftWarning> {
    let mut out = Vec::new();
    for f in &parsed.files {
        if is_excluded_path(&f.path) || !is_source_language(&f.path) {
            continue;
        }
        for func in detect_functions(f) {
            if func.length > threshold {
                out.push(CraftWarning {
                    rule: "function-length".to_string(),
                    severity: Severity::Warn,
                    file: f.path.clone(),
                    line: Some(func.start_line),
                    message: format!(
                        "function `{}` spans {} added lines (threshold: {})",
                        func.name, func.length, threshold
                    ),
                    silence: "raise `craft.max_function_lines`, split the function, or set `craft.max_function_lines = 0` to disable".to_string(),
                });
            }
        }
    }
    out
}

struct DetectedFn {
    name: String,
    start_line: u32,
    length: u32,
}

fn detect_functions(f: &ParsedFile) -> Vec<DetectedFn> {
    let mut out = Vec::new();
    let lines: Vec<&AddedLine> = f.added_lines.iter().collect();
    let mut i = 0;
    while i < lines.len() {
        if let Some((name, def_indent)) = parse_function_def(&lines[i].trimmed_content, &f.path) {
            let start_line = lines[i].new_line;
            let mut end = i + 1;
            while end < lines.len() {
                let body_trim = lines[end].trimmed_content.as_str();
                if body_trim.is_empty() {
                    end += 1;
                    continue;
                }
                let indent = leading_spaces(&lines[end].raw_content);
                // Stop when we dedent to the def's indent (or shallower)
                // AND the line isn't a continuation brace.
                if indent <= def_indent && !is_continuation(body_trim) {
                    break;
                }
                end += 1;
            }
            let length = (end - i) as u32;
            out.push(DetectedFn { name, start_line, length });
            i = end;
        } else {
            i += 1;
        }
    }
    out
}

fn leading_spaces(s: &str) -> usize {
    s.chars().take_while(|c| *c == ' ' || *c == '\t').count()
}

fn is_continuation(s: &str) -> bool {
    let t = s.trim();
    matches!(t, "}" | "})" | "});" | "} else {" | "} else if {" | ");" | ")")
}

fn parse_function_def(line: &str, path: &str) -> Option<(String, usize)> {
    let p = path.to_ascii_lowercase();
    let indent = leading_spaces(line);
    let trimmed = line.trim_start();

    if p.ends_with(".py") {
        // def foo(...):    or    async def foo(...):
        let rest = trimmed.strip_prefix("def ").or_else(|| trimmed.strip_prefix("async def "))?;
        let name = rest.split('(').next()?.trim();
        if name.is_empty() {
            return None;
        }
        return Some((name.to_string(), indent));
    }
    if p.ends_with(".rs") {
        // fn foo(   or   pub fn foo(   or   pub(crate) async fn foo(
        let after = trimmed.find("fn ")?;
        // Guard against e.g. `let fn_name = ...` — must be word-boundary.
        if after > 0 {
            let prev = trimmed.as_bytes()[after - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' {
                return None;
            }
        }
        let rest = &trimmed[after + 3..];
        let name = rest.split(|c: char| c == '(' || c == '<' || c.is_whitespace()).next()?;
        if name.is_empty() {
            return None;
        }
        return Some((name.to_string(), indent));
    }
    if p.ends_with(".ts") || p.ends_with(".tsx") || p.ends_with(".js") || p.ends_with(".jsx") {
        // function foo(...)   or   const foo = (...) =>   or   foo(...) {
        if let Some(rest) = trimmed.strip_prefix("function ") {
            let name = rest.split(|c: char| c == '(' || c == '<' || c.is_whitespace()).next()?;
            if !name.is_empty() {
                return Some((name.to_string(), indent));
            }
        }
        if let Some(rest) = trimmed.strip_prefix("export function ") {
            let name = rest.split(|c: char| c == '(' || c == '<' || c.is_whitespace()).next()?;
            if !name.is_empty() {
                return Some((name.to_string(), indent));
            }
        }
        if let Some(rest) = trimmed.strip_prefix("async function ") {
            let name = rest.split(|c: char| c == '(' || c == '<' || c.is_whitespace()).next()?;
            if !name.is_empty() {
                return Some((name.to_string(), indent));
            }
        }
        // const foo = (...) =>  /  let foo = function (
        for prefix in ["const ", "let ", "var "] {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                let name = rest.split([' ', '=', ':']).next()?;
                if !name.is_empty() && (rest.contains("=>") || rest.contains("function")) {
                    return Some((name.to_string(), indent));
                }
            }
        }
        return None;
    }
    if p.ends_with(".go") {
        // func Foo(...)   or   func (r *T) Foo(...)
        let rest = trimmed.strip_prefix("func ")?;
        let after_recv =
            rest.trim_start_matches(|c: char| c != ')').trim_start_matches(')').trim_start();
        let head = if rest.starts_with('(') { after_recv } else { rest };
        let name = head.split(|c: char| c == '(' || c.is_whitespace()).next()?;
        if name.is_empty() {
            return None;
        }
        return Some((name.to_string(), indent));
    }

    None
}

// ────────────────────────────────────────────────────────────────────────────
// Heuristic 3: duplication
// ────────────────────────────────────────────────────────────────────────────

const DUP_WINDOW: usize = 5;
const DUP_MIN_LINE_LEN: usize = 20;

fn check_duplication(parsed: &ParsedDiff) -> Vec<CraftWarning> {
    // Collect (file_idx, line_idx) for each non-trivial added line, keyed
    // by trimmed content. A window of DUP_WINDOW consecutive trimmed
    // contents is the unit of duplication.
    #[derive(Default)]
    struct Block<'a> {
        windows: Vec<(usize, u32, Vec<&'a str>)>, // (file_idx, start_line, window)
    }

    let mut block = Block::default();
    for (fi, f) in parsed.files.iter().enumerate() {
        if is_excluded_path(&f.path) {
            continue;
        }
        let added: Vec<&AddedLine> = f
            .added_lines
            .iter()
            .filter(|l| {
                l.trimmed_content.len() >= DUP_MIN_LINE_LEN
                    && !is_comment_only(&l.trimmed_content, &f.path)
            })
            .collect();
        if added.len() < DUP_WINDOW {
            continue;
        }
        // Slide a window of size DUP_WINDOW over the contiguous non-trivial
        // added lines. We require strict line adjacency to avoid matching
        // pieces of distant code.
        let mut i = 0;
        while i + DUP_WINDOW <= added.len() {
            let mut contiguous = true;
            for k in 1..DUP_WINDOW {
                if added[i + k].new_line != added[i + k - 1].new_line + 1 {
                    contiguous = false;
                    break;
                }
            }
            if contiguous {
                let window: Vec<&str> =
                    (0..DUP_WINDOW).map(|k| added[i + k].trimmed_content.as_str()).collect();
                block.windows.push((fi, added[i].new_line, window));
            }
            i += 1;
        }
    }

    // Find any pair of windows with identical content from different
    // locations. To stay deterministic, emit one warning per duplicated
    // run on the second occurrence.
    let mut out = Vec::new();
    let mut seen: BTreeMap<Vec<&str>, (usize, u32)> = BTreeMap::new();
    for (fi, ln, win) in &block.windows {
        if let Some((other_fi, other_ln)) = seen.get(win) {
            let f = &parsed.files[*fi];
            let other = &parsed.files[*other_fi];
            out.push(CraftWarning {
                rule: "duplication".to_string(),
                severity: Severity::Warn,
                file: f.path.clone(),
                line: Some(*ln),
                message: format!(
                    "{}-line block duplicates `{}`:{} (DRY)",
                    DUP_WINDOW, other.path, other_ln
                ),
                silence: "extract a shared helper, or remove the `CONVENTION:craft.dry` memory to disable this check".to_string(),
            });
        } else {
            seen.insert(win.clone(), (*fi, *ln));
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────────────
// Memory helpers
// ────────────────────────────────────────────────────────────────────────────

fn has_dry_memory(memories: &BTreeMap<String, String>) -> bool {
    memories.values().any(|v| v.trim().starts_with("CONVENTION:craft.dry"))
}

// ────────────────────────────────────────────────────────────────────────────
// Minimal unified-diff parser
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct ParsedDiff {
    files: Vec<ParsedFile>,
}

#[derive(Debug)]
struct ParsedFile {
    path: String,
    added_lines: Vec<AddedLine>,
}

#[derive(Debug, Clone)]
struct AddedLine {
    new_line: u32,
    /// Original content with original leading whitespace preserved.
    raw_content: String,
    /// `raw_content.trim_start().trim_end().to_string()` for cheap compare.
    trimmed_content: String,
}

fn parse_diff(diff: &str) -> ParsedDiff {
    let mut out = ParsedDiff::default();
    let mut current: Option<ParsedFile> = None;
    let mut new_line: u32 = 0;

    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            // Flush the previous file.
            if let Some(f) = current.take() {
                out.files.push(f);
            }
            let path = rest.trim().trim_start_matches("b/").to_string();
            let path = if path == "/dev/null" { String::new() } else { path };
            current = Some(ParsedFile { path, added_lines: Vec::new() });
            new_line = 0;
            continue;
        }
        if line.starts_with("--- ") || line.starts_with("diff --git ") || line.starts_with("index ")
        {
            continue;
        }
        if let Some(rest) = line.strip_prefix("@@") {
            // Format: @@ -old,len +new,len @@ optional trailing context
            // We only need the new-side start.
            if let Some(plus) = rest.split_whitespace().find(|s| s.starts_with('+')) {
                let body = &plus[1..];
                let start: u32 = body.split(',').next().unwrap_or("0").parse().unwrap_or(0);
                new_line = start.saturating_sub(1);
            }
            continue;
        }
        let Some(file) = current.as_mut() else { continue };
        if file.path.is_empty() {
            continue;
        }
        if let Some(content) = line.strip_prefix('+') {
            if line.starts_with("+++") {
                continue;
            }
            new_line += 1;
            let trimmed = content.trim().to_string();
            file.added_lines.push(AddedLine {
                new_line,
                raw_content: content.to_string(),
                trimmed_content: trimmed,
            });
        } else if line.starts_with('-') {
            // deletion — don't advance new_line
        } else if let Some(stripped) = line.strip_prefix(' ') {
            // context line — advance
            let _ = stripped;
            new_line += 1;
        }
    }

    if let Some(f) = current.take() {
        out.files.push(f);
    }
    out
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg_with(testing_require: bool, max_fn: u32) -> Config {
        let mut c = Config::default();
        c.testing.require = testing_require;
        c.craft.max_function_lines = max_fn;
        c
    }

    fn dry_memory() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("craft-dry".into(), "CONVENTION:craft.dry — don't repeat yourself".into());
        m
    }

    // ──────── parser ────────

    #[test]
    fn parser_extracts_added_lines_with_correct_line_numbers() {
        let diff = "\
diff --git a/src/foo.py b/src/foo.py
index 111..222 100644
--- a/src/foo.py
+++ b/src/foo.py
@@ -1,3 +1,5 @@
 unchanged
+added line one
+added line two
 still unchanged
+third add
";
        let p = parse_diff(diff);
        assert_eq!(p.files.len(), 1);
        let f = &p.files[0];
        assert_eq!(f.path, "src/foo.py");
        assert_eq!(f.added_lines.len(), 3);
        assert_eq!(f.added_lines[0].new_line, 2);
        assert_eq!(f.added_lines[1].new_line, 3);
        assert_eq!(f.added_lines[2].new_line, 5);
    }

    #[test]
    fn parser_handles_multiple_files() {
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -0,0 +1,1 @@
+from b import x
diff --git a/b.py b/b.py
--- a/b.py
+++ b/b.py
@@ -0,0 +1,1 @@
+x = 1
";
        let p = parse_diff(diff);
        assert_eq!(p.files.len(), 2);
        assert_eq!(p.files[0].path, "a.py");
        assert_eq!(p.files[1].path, "b.py");
    }

    // ──────── missing-tests ────────

    #[test]
    fn missing_tests_warns_when_source_changes_without_test() {
        let diff = "\
diff --git a/src/auth.py b/src/auth.py
--- a/src/auth.py
+++ b/src/auth.py
@@ -1,0 +1,2 @@
+def login(user):
+    return user.token
";
        let memories = BTreeMap::new();
        let warnings = craft_warnings(&memories, diff, &cfg_with(false, 0));
        let m = warnings.iter().find(|w| w.rule == "missing-tests").expect("missing-tests fires");
        assert_eq!(m.severity, Severity::Warn);
        assert_eq!(m.file, "src/auth.py");
    }

    #[test]
    fn missing_tests_promotes_to_fail_when_testing_required() {
        let diff = "\
diff --git a/src/auth.py b/src/auth.py
--- a/src/auth.py
+++ b/src/auth.py
@@ -1,0 +1,1 @@
+def login(user): return user.token
";
        let warnings = craft_warnings(&BTreeMap::new(), diff, &cfg_with(true, 0));
        let m = warnings.iter().find(|w| w.rule == "missing-tests").unwrap();
        assert_eq!(m.severity, Severity::Fail);
        assert!(m.silence.contains("testing.require"));
    }

    #[test]
    fn missing_tests_silent_when_test_co_changed() {
        let diff = "\
diff --git a/src/auth.py b/src/auth.py
--- a/src/auth.py
+++ b/src/auth.py
@@ -1,0 +1,1 @@
+def login(user): return user.token
diff --git a/tests/test_auth.py b/tests/test_auth.py
--- a/tests/test_auth.py
+++ b/tests/test_auth.py
@@ -1,0 +1,1 @@
+def test_login(): pass
";
        let warnings = craft_warnings(&BTreeMap::new(), diff, &cfg_with(false, 0));
        assert!(warnings.iter().all(|w| w.rule != "missing-tests"), "got: {warnings:?}");
    }

    #[test]
    fn missing_tests_skips_excluded_paths() {
        let diff = "\
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1,0 +1,1 @@
+# heading
diff --git a/Cargo.toml b/Cargo.toml
--- a/Cargo.toml
+++ b/Cargo.toml
@@ -1,0 +1,1 @@
+version = \"1.0\"
";
        let warnings = craft_warnings(&BTreeMap::new(), diff, &cfg_with(false, 0));
        assert!(warnings.is_empty());
    }

    #[test]
    fn missing_tests_skips_comment_only_additions() {
        let diff = "\
diff --git a/src/foo.py b/src/foo.py
--- a/src/foo.py
+++ b/src/foo.py
@@ -1,0 +1,2 @@
+# just a comment
+
";
        let warnings = craft_warnings(&BTreeMap::new(), diff, &cfg_with(false, 0));
        assert!(warnings.is_empty(), "got: {warnings:?}");
    }

    // ──────── function-length ────────

    #[test]
    fn function_length_disabled_when_threshold_zero() {
        let diff = long_python_fn_diff(50);
        let warnings = craft_warnings(&BTreeMap::new(), &diff, &cfg_with(false, 0));
        assert!(warnings.iter().all(|w| w.rule != "function-length"));
    }

    #[test]
    fn function_length_warns_over_threshold() {
        let diff = long_python_fn_diff(30);
        let warnings = craft_warnings(&BTreeMap::new(), &diff, &cfg_with(false, 10));
        let w =
            warnings.iter().find(|w| w.rule == "function-length").expect("function-length fires");
        assert_eq!(w.severity, Severity::Warn);
        assert!(w.message.contains("login"));
        assert_eq!(w.file, "src/auth.py");
    }

    #[test]
    fn function_length_under_threshold_silent() {
        let diff = long_python_fn_diff(5);
        let warnings = craft_warnings(&BTreeMap::new(), &diff, &cfg_with(false, 20));
        assert!(warnings.iter().all(|w| w.rule != "function-length"));
    }

    #[test]
    fn function_length_detects_rust_fn() {
        let mut body = String::from(
            "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -0,0 +1,15 @@
+fn process(input: &str) -> String {
",
        );
        for i in 0..14 {
            body.push_str(&format!("+    let step_{i} = {i};\n"));
        }
        let warnings = craft_warnings(&BTreeMap::new(), &body, &cfg_with(false, 5));
        let w = warnings.iter().find(|w| w.rule == "function-length").unwrap();
        assert!(w.message.contains("process"), "msg: {}", w.message);
    }

    fn long_python_fn_diff(body_lines: usize) -> String {
        let mut s = String::from(
            "\
diff --git a/src/auth.py b/src/auth.py
--- a/src/auth.py
+++ b/src/auth.py
@@ -0,0 +1,",
        );
        s.push_str(&format!("{} @@\n", body_lines + 1));
        s.push_str("+def login(user):\n");
        for i in 0..body_lines {
            s.push_str(&format!("+    step_{i} = {i}\n"));
        }
        s
    }

    // ──────── duplication ────────

    #[test]
    fn duplication_fires_only_when_dry_memory_present() {
        let diff = dup_diff();
        let no_mem = craft_warnings(&BTreeMap::new(), &diff, &cfg_with(false, 0));
        assert!(no_mem.iter().all(|w| w.rule != "duplication"));

        let with_mem = craft_warnings(&dry_memory(), &diff, &cfg_with(false, 0));
        let d = with_mem.iter().find(|w| w.rule == "duplication").expect("fires");
        assert!(d.message.contains("DRY"));
        assert_eq!(d.severity, Severity::Warn);
    }

    #[test]
    fn duplication_silent_when_blocks_differ() {
        let diff = "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -0,0 +1,5 @@
+row_one_value_alpha_distinct
+row_two_value_alpha_distinct
+row_three_value_alpha_distinct
+row_four_value_alpha_distinct
+row_five_value_alpha_distinct
diff --git a/b.py b/b.py
--- a/b.py
+++ b/b.py
@@ -0,0 +1,5 @@
+different_row_one_completely
+different_row_two_completely
+different_row_three_completely
+different_row_four_completely
+different_row_five_completely
";
        let warnings = craft_warnings(&dry_memory(), diff, &cfg_with(false, 0));
        assert!(warnings.iter().all(|w| w.rule != "duplication"));
    }

    fn dup_diff() -> String {
        // Same 5-line block in two files, ≥20 chars per line.
        let block = "\
+config_step_one_with_padding
+config_step_two_with_padding
+config_step_three_with_padding
+config_step_four_with_padding
+config_step_five_with_padding\n";
        format!(
            "\
diff --git a/a.py b/a.py
--- a/a.py
+++ b/a.py
@@ -0,0 +1,5 @@
{block}\
diff --git a/b.py b/b.py
--- a/b.py
+++ b/b.py
@@ -0,0 +1,5 @@
{block}"
        )
    }

    // ──────── overall contract ────────

    #[test]
    fn empty_diff_yields_no_warnings() {
        let warnings = craft_warnings(&BTreeMap::new(), "", &cfg_with(true, 50));
        assert!(warnings.is_empty());
    }

    #[test]
    fn warnings_default_to_warn_severity() {
        // Default config: testing.require=false, max_function_lines=0.
        // Only missing-tests can fire; must be Warn.
        let diff = "\
diff --git a/src/x.py b/src/x.py
--- a/src/x.py
+++ b/src/x.py
@@ -0,0 +1,1 @@
+def x(): return 1
";
        let warnings = craft_warnings(&BTreeMap::new(), diff, &Config::default());
        for w in &warnings {
            assert_eq!(w.severity, Severity::Warn, "rule {} should be Warn", w.rule);
        }
    }

    #[test]
    fn missing_tests_recognizes_pytest_layout() {
        let diff = "\
diff --git a/src/auth.py b/src/auth.py
--- a/src/auth.py
+++ b/src/auth.py
@@ -0,0 +1,1 @@
+def login(): pass
diff --git a/tests/auth/test_auth.py b/tests/auth/test_auth.py
--- a/tests/auth/test_auth.py
+++ b/tests/auth/test_auth.py
@@ -0,0 +1,1 @@
+def test_login(): pass
";
        let warnings = craft_warnings(&BTreeMap::new(), diff, &cfg_with(false, 0));
        assert!(warnings.iter().all(|w| w.rule != "missing-tests"));
    }
}
