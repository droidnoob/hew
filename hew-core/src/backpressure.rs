//! Per-iter test/lint gate for `hew loop`.
//!
//! After the spawner returns a closed task, the runner runs the
//! project's test and lint commands. The results feed into [`evaluate`]
//! along with the run's `--strict` flag; this module decides whether
//! the iter is a [`Verdict::Pass`], a [`Verdict::WarnOnly`] (logged but
//! the iter stands), or a [`Verdict::Fail`] (the runner reverts the
//! iter's commits and files a `STATUS:loop-iter-failed` memory).
//!
//! Side effects (running cargo, doing `git reset --hard <pre-iter>`)
//! live in the runner glue. This module is pure: same `GateCheck` +
//! `strict` produce the same `Verdict`. See epic hew-gr1.

/// Snapshot of the project's gate state for one iter. The runner
/// gathers the booleans by shelling out to test/lint commands and
/// reading craft signals out of hew config.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GateCheck {
    /// Test suite ran and exited 0.
    pub tests_passed: bool,
    /// Test suite skipped (config has `testing.require=false`).
    pub tests_skipped: bool,
    /// Lint/clippy ran and exited 0.
    pub lint_passed: bool,
    /// Lint was skipped (no lint command configured).
    pub lint_skipped: bool,
    /// `craft.warn_on_unused` fired (unused imports / dead code).
    pub craft_warn_unused: bool,
    /// `craft.testing` warning (missing tests for new code).
    pub craft_warn_testing: bool,
}

/// Outcome of [`evaluate`]. The runner turns this into git ops and a
/// log row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// All checks clean.
    Pass,
    /// Soft signals only — log the warnings, keep the iter.
    WarnOnly(Vec<String>),
    /// Hard failure — revert the iter and log the reasons.
    Fail(Vec<String>),
}

impl Verdict {
    /// Whether the runner should revert the iter's commits.
    pub fn is_fail(&self) -> bool {
        matches!(self, Verdict::Fail(_))
    }

    /// Human-readable reasons attached to this verdict, if any.
    pub fn reasons(&self) -> &[String] {
        match self {
            Verdict::Pass => &[],
            Verdict::WarnOnly(r) | Verdict::Fail(r) => r,
        }
    }
}

/// Apply the gate rules. Hard failures (test or lint failed when not
/// skipped) always Fail regardless of strict. Craft warnings Fail only
/// when `strict` is true; otherwise they degrade to WarnOnly.
pub fn evaluate(check: &GateCheck, strict: bool) -> Verdict {
    let mut hard_reasons = Vec::new();
    if !check.tests_skipped && !check.tests_passed {
        hard_reasons.push("tests failed".to_string());
    }
    if !check.lint_skipped && !check.lint_passed {
        hard_reasons.push("lint failed".to_string());
    }

    let mut soft_reasons = Vec::new();
    if check.craft_warn_unused {
        soft_reasons.push("craft.warn_on_unused: unused imports / dead code".to_string());
    }
    if check.craft_warn_testing {
        soft_reasons.push("craft.testing: missing tests for new code".to_string());
    }

    if !hard_reasons.is_empty() {
        hard_reasons.extend(soft_reasons);
        return Verdict::Fail(hard_reasons);
    }

    if soft_reasons.is_empty() {
        return Verdict::Pass;
    }

    if strict { Verdict::Fail(soft_reasons) } else { Verdict::WarnOnly(soft_reasons) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clean() -> GateCheck {
        GateCheck { tests_passed: true, lint_passed: true, ..Default::default() }
    }

    #[test]
    fn clean_check_passes() {
        assert_eq!(evaluate(&clean(), true), Verdict::Pass);
        assert_eq!(evaluate(&clean(), false), Verdict::Pass);
    }

    #[test]
    fn test_failure_fails_regardless_of_strict() {
        let c = GateCheck { tests_passed: false, lint_passed: true, ..Default::default() };
        match evaluate(&c, false) {
            Verdict::Fail(r) => assert!(r.iter().any(|s| s.contains("tests"))),
            other => panic!("expected Fail, got {other:?}"),
        }
        match evaluate(&c, true) {
            Verdict::Fail(r) => assert!(r.iter().any(|s| s.contains("tests"))),
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn lint_failure_fails_regardless_of_strict() {
        let c = GateCheck { tests_passed: true, lint_passed: false, ..Default::default() };
        assert!(evaluate(&c, false).is_fail());
        assert!(evaluate(&c, true).is_fail());
    }

    #[test]
    fn skipped_tests_dont_count_as_failure() {
        let c = GateCheck { tests_skipped: true, lint_passed: true, ..Default::default() };
        assert_eq!(evaluate(&c, true), Verdict::Pass);
    }

    #[test]
    fn skipped_lint_dont_count_as_failure() {
        let c = GateCheck { tests_passed: true, lint_skipped: true, ..Default::default() };
        assert_eq!(evaluate(&c, true), Verdict::Pass);
    }

    #[test]
    fn craft_warnings_are_warn_only_without_strict() {
        let c = GateCheck {
            tests_passed: true,
            lint_passed: true,
            craft_warn_unused: true,
            ..Default::default()
        };
        match evaluate(&c, false) {
            Verdict::WarnOnly(r) => assert!(r.iter().any(|s| s.contains("warn_on_unused"))),
            other => panic!("expected WarnOnly, got {other:?}"),
        }
    }

    #[test]
    fn craft_warnings_are_failures_under_strict() {
        let c = GateCheck {
            tests_passed: true,
            lint_passed: true,
            craft_warn_unused: true,
            craft_warn_testing: true,
            ..Default::default()
        };
        match evaluate(&c, true) {
            Verdict::Fail(r) => {
                assert!(r.iter().any(|s| s.contains("warn_on_unused")));
                assert!(r.iter().any(|s| s.contains("testing")));
            }
            other => panic!("expected Fail under strict, got {other:?}"),
        }
    }

    #[test]
    fn hard_failure_includes_soft_reasons_too() {
        let c = GateCheck {
            tests_passed: false,
            lint_passed: true,
            craft_warn_unused: true,
            ..Default::default()
        };
        match evaluate(&c, false) {
            Verdict::Fail(r) => {
                assert!(r.iter().any(|s| s.contains("tests")));
                assert!(r.iter().any(|s| s.contains("warn_on_unused")));
            }
            other => panic!("expected Fail, got {other:?}"),
        }
    }

    #[test]
    fn verdict_is_fail_only_for_fail_variant() {
        assert!(!Verdict::Pass.is_fail());
        assert!(!Verdict::WarnOnly(vec!["x".into()]).is_fail());
        assert!(Verdict::Fail(vec!["x".into()]).is_fail());
    }

    #[test]
    fn verdict_reasons_returns_attached_strings() {
        assert!(Verdict::Pass.reasons().is_empty());
        let warn = Verdict::WarnOnly(vec!["a".into(), "b".into()]);
        assert_eq!(warn.reasons().len(), 2);
        let fail = Verdict::Fail(vec!["c".into()]);
        assert_eq!(fail.reasons()[0], "c");
    }
}
