//! Parse `git diff --unified=0` hunk headers into line ranges.
//!
//! Pure line math, not feature-gated. The companion to
//! `treesitter::diff::changed_symbols`: hunk headers in, half-open line
//! ranges out, ready to intersect with extracted symbols.
//!
//! Format reminder (man git-diff):
//!
//! ```text
//! @@ -<old_start>[,<old_count>] +<new_start>[,<new_count>] @@
//! ```
//!
//! When `<count>` is omitted it defaults to 1; when it's 0 the hunk
//! represents a pure insertion or deletion anchored after `<start>`.
//! We only care about the "new" side (`+` ranges) — that's what maps
//! onto the *current* file's symbols.

use std::ops::Range;

/// Walk every line of `diff_text`, picking out hunk headers and
/// returning each one's "new" side as a half-open `Range<u32>` in
/// 1-based line numbers.
///
/// Skips anything that isn't a hunk header. Returns an empty Vec when
/// the input has no hunks (e.g. a pure rename, an empty diff).
pub fn parse_changed_ranges(diff_text: &str) -> Vec<Range<u32>> {
    diff_text.lines().filter_map(parse_hunk_header).collect()
}

/// Parse a single `@@ -... +<start>[,<count>] @@ ...` line. Returns
/// `None` for anything that doesn't start with `@@`, malformed offsets,
/// or zero-count pure-deletion hunks (which don't add lines on the new
/// side and so have no symbols to intersect).
fn parse_hunk_header(line: &str) -> Option<Range<u32>> {
    let rest = line.strip_prefix("@@ ")?;
    // Find the `+` token. We don't care about the `-` side.
    let plus = rest.find('+')?;
    let after_plus = &rest[plus + 1..];
    // The `+<start>[,<count>] @@ ...` chunk ends at the next space.
    let end = after_plus.find(' ').unwrap_or(after_plus.len());
    let spec = &after_plus[..end];
    let (start_s, count_s) = match spec.split_once(',') {
        Some((s, c)) => (s, c),
        None => (spec, "1"),
    };
    let start: u32 = start_s.parse().ok()?;
    let count: u32 = count_s.parse().ok()?;
    if count == 0 {
        // Pure deletion — no new-side lines to scope symbols against.
        return None;
    }
    Some(start..start + count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_hunk_with_explicit_counts() {
        let diff = "@@ -10,5 +20,5 @@ some context\n unchanged\n";
        assert_eq!(parse_changed_ranges(diff), vec![20..25]);
    }

    #[test]
    fn omitted_count_defaults_to_one() {
        let diff = "@@ -1 +5 @@\n line\n";
        assert_eq!(parse_changed_ranges(diff), vec![5..6]);
    }

    #[test]
    fn zero_count_pure_deletion_is_skipped() {
        let diff = "@@ -3,5 +2,0 @@\n";
        assert!(parse_changed_ranges(diff).is_empty());
    }

    #[test]
    fn multiple_hunks_collect_all() {
        let diff = "@@ -1,3 +1,3 @@\n line\n@@ -10,2 +12,4 @@\n line2\n";
        assert_eq!(parse_changed_ranges(diff), vec![1..4, 12..16]);
    }

    #[test]
    fn non_hunk_lines_skipped() {
        let diff = "diff --git a/foo b/foo\nindex abc..def 100644\n--- a/foo\n+++ b/foo\n@@ -1,1 +1,1 @@\n";
        assert_eq!(parse_changed_ranges(diff), vec![1..2]);
    }

    #[test]
    fn malformed_header_returns_none() {
        assert!(parse_hunk_header("@@ -xx +yy @@").is_none());
        assert!(parse_hunk_header("not a header").is_none());
        assert!(parse_hunk_header("@@ -1,1 @@").is_none()); // no `+` side
    }

    #[test]
    fn empty_diff_returns_empty() {
        assert!(parse_changed_ranges("").is_empty());
    }
}
