//! Pure line-math for diff intersection.
//!
//! NOT feature-gated. TS.2 layer: given a slice of `Symbol`s extracted
//! from a file plus the set of line ranges a diff touched, return the
//! symbols whose definitions overlap the changed lines.
//!
//! No tree-sitter dependency — these helpers only touch `std::ops::Range`
//! and `super::Symbol`. They compile and test under default features.

use std::ops::Range;

use super::Symbol;

/// True iff the two half-open ranges share at least one line.
///
/// Empty ranges (where `start >= end`) never overlap anything — `git diff`
/// doesn't emit empty hunks and an empty symbol span is malformed, so the
/// `false` branch is the right safety default.
pub fn line_ranges_overlap(a: &Range<u32>, b: &Range<u32>) -> bool {
    if a.start >= a.end || b.start >= b.end {
        return false;
    }
    a.start < b.end && b.start < a.end
}

/// Return clones of every symbol whose `line_range` overlaps at least one
/// range in `changed_ranges`. Input order is preserved and each symbol
/// appears at most once even if multiple changed ranges hit it.
///
/// Dedup is by reference position in the input slice, not by value — two
/// extracted symbols that happen to compare equal stay separate.
pub fn changed_symbols(extracted: &[Symbol], changed_ranges: &[Range<u32>]) -> Vec<Symbol> {
    let mut out = Vec::new();
    for sym in extracted {
        if changed_ranges.iter().any(|r| line_ranges_overlap(&sym.line_range, r)) {
            out.push(sym.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::treesitter::SymbolKind;

    fn sym(name: &str, line_range: Range<u32>) -> Symbol {
        Symbol { name: name.to_string(), kind: SymbolKind::Function, byte_range: 0..0, line_range }
    }

    #[test]
    fn line_ranges_overlap_empty_a_returns_false() {
        assert!(!line_ranges_overlap(&(10..10), &(5..20)));
    }

    #[test]
    fn line_ranges_overlap_empty_b_returns_false() {
        assert!(!line_ranges_overlap(&(5..20), &(10..10)));
    }

    #[test]
    fn line_ranges_overlap_exact_boundary_inclusive_start() {
        // Half-open: a ends at 20, b starts at 20 — no shared line.
        assert!(!line_ranges_overlap(&(10..20), &(20..30)));
        assert!(!line_ranges_overlap(&(20..30), &(10..20)));
    }

    #[test]
    fn line_ranges_overlap_one_inside_other() {
        assert!(line_ranges_overlap(&(10..20), &(12..15)));
        assert!(line_ranges_overlap(&(12..15), &(10..20)));
    }

    #[test]
    fn line_ranges_overlap_partial_overlap() {
        assert!(line_ranges_overlap(&(10..20), &(15..25)));
        assert!(line_ranges_overlap(&(15..25), &(10..20)));
    }

    #[test]
    fn line_ranges_overlap_disjoint() {
        assert!(!line_ranges_overlap(&(10..20), &(30..40)));
        assert!(!line_ranges_overlap(&(30..40), &(10..20)));
    }

    #[test]
    fn changed_symbols_empty_changes_returns_empty() {
        let extracted = vec![sym("foo", 10..20), sym("bar", 30..40)];
        assert!(changed_symbols(&extracted, &[]).is_empty());
    }

    #[test]
    fn changed_symbols_no_overlap_returns_empty() {
        let extracted = vec![sym("foo", 10..20), sym("bar", 30..40)];
        let changes = vec![100..110, 200..210];
        assert!(changed_symbols(&extracted, &changes).is_empty());
    }

    #[test]
    fn changed_symbols_multi_range_diff_collects_all_overlapping() {
        let extracted = vec![sym("a", 10..20), sym("b", 30..40), sym("c", 50..60)];
        let changes = vec![15..16, 55..56];
        let got = changed_symbols(&extracted, &changes);
        assert_eq!(got.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["a", "c"]);
    }

    #[test]
    fn changed_symbols_dedupes_symbol_hit_by_two_ranges() {
        let extracted = vec![sym("a", 10..20)];
        let changes = vec![11..12, 15..16, 18..19];
        let got = changed_symbols(&extracted, &changes);
        assert_eq!(got.len(), 1, "symbol must appear once even with three hits");
        assert_eq!(got[0].name, "a");
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // intentional: one "whole-file" hunk
    fn changed_symbols_preserves_input_order() {
        let extracted = vec![sym("z", 50..60), sym("a", 10..20), sym("m", 30..40)];
        let changes = vec![0..1000];
        let got = changed_symbols(&extracted, &changes);
        assert_eq!(got.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), vec!["z", "a", "m"]);
    }

    #[test]
    #[allow(clippy::single_range_in_vec_init)] // intentional: one "whole-file" hunk
    fn changed_symbols_full_file_change_returns_all() {
        let extracted = vec![sym("a", 10..20), sym("b", 30..40), sym("c", 50..60)];
        let changes = vec![1..1000];
        let got = changed_symbols(&extracted, &changes);
        assert_eq!(got.len(), 3);
    }
}
