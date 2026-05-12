//! Branch naming helpers shared by `hew branch new` and (future)
//! `hew-execute` first-claim auto-branching.
//!
//! - [`PREFIXES`] is the locked conventional set (see DECISION:branch-prefixes).
//! - [`slugify`] normalizes user-supplied text into a safe ref component.
//! - [`build_branch_name`] composes `<prefix>/<slug>` with validation.

use crate::error::{HewError, Result};

/// Conventional-commit prefixes accepted by `hew branch new`.
/// Matches the commit-type list in `hew-execute` step 8.
pub const PREFIXES: &[&str] =
    &["feat", "fix", "chore", "docs", "refactor", "perf", "test", "style"];

pub fn is_valid_prefix(p: &str) -> bool {
    PREFIXES.contains(&p)
}

/// Normalize text into a git-ref-safe slug.
///
/// Rules: lowercase; ASCII alphanumerics kept; spaces/underscores/dots/slashes
/// collapse to `-`; everything else dropped; runs of `-` collapsed; leading
/// and trailing `-` trimmed.
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            Some(ch.to_ascii_lowercase())
        } else if matches!(ch, ' ' | '_' | '.' | '/' | '\\' | '\t') || ch == '-' {
            Some('-')
        } else {
            None
        };
        match mapped {
            Some('-') if !prev_dash && !out.is_empty() => {
                out.push('-');
                prev_dash = true;
            }
            Some('-') => {}
            Some(c) => {
                out.push(c);
                prev_dash = false;
            }
            None => {}
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Build `<prefix>/<slug>` after validating both.
///
/// Errors:
/// - `MissingFlag { flag: "prefix (..." }` if the prefix isn't in [`PREFIXES`].
/// - `MissingFlag { flag: "slug (..." }` if `slugify(slug)` produces empty.
pub fn build_branch_name(prefix: &str, slug_raw: &str) -> Result<String> {
    if !is_valid_prefix(prefix) {
        return Err(HewError::MissingFlag {
            flag: format!("prefix (unknown: `{prefix}`; expected one of: {})", PREFIXES.join(", ")),
        });
    }
    let slug = slugify(slug_raw);
    if slug.is_empty() {
        return Err(HewError::MissingFlag {
            flag: format!("slug (input `{slug_raw}` slugifies to empty)"),
        });
    }
    Ok(format!("{prefix}/{slug}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Add Auth"), "add-auth");
        assert_eq!(slugify("  spaced  out  "), "spaced-out");
        assert_eq!(slugify("UPPER_case.thing"), "upper-case-thing");
        assert_eq!(slugify("emoji-✨-stripped"), "emoji-stripped");
        assert_eq!(slugify("multi---dash"), "multi-dash");
        assert_eq!(slugify("trailing---"), "trailing");
        assert_eq!(slugify("---leading"), "leading");
        assert_eq!(slugify("a/b/c"), "a-b-c");
    }

    #[test]
    fn slugify_empty_on_only_garbage() {
        assert_eq!(slugify(""), "");
        assert_eq!(slugify("✨✨"), "");
        assert_eq!(slugify("---"), "");
    }

    #[test]
    fn prefix_validation() {
        for p in PREFIXES {
            assert!(is_valid_prefix(p), "{p}");
        }
        assert!(!is_valid_prefix("feature"));
        assert!(!is_valid_prefix("hotfix"));
        assert!(!is_valid_prefix(""));
    }

    #[test]
    fn build_branch_name_happy() {
        assert_eq!(build_branch_name("feat", "Add Auth").unwrap(), "feat/add-auth");
        assert_eq!(build_branch_name("fix", "bug #123").unwrap(), "fix/bug-123");
    }

    #[test]
    fn build_branch_name_unknown_prefix() {
        let err = build_branch_name("feature", "x").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("feature"), "{msg}");
    }

    #[test]
    fn build_branch_name_empty_slug() {
        let err = build_branch_name("feat", "✨").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("slugifies to empty"), "{msg}");
    }
}
