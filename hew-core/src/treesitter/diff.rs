//! Pure line-math for diff intersection (skeleton — TS.1).
//!
//! NOT feature-gated. Lives here so TS.2 can land the intersection logic
//! without dragging the tree-sitter feature into default test runs.
//! Symbol-aware intersection happens in TS.3+; this module only knows
//! about line ranges.

/// A closed, 1-based line range `[start, end]`. Matches `git diff` line
/// semantics — `end >= start` always holds for non-empty ranges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct LineRange {
    pub start: u32,
    pub end: u32,
}

impl LineRange {
    /// True iff `self` and `other` share at least one line.
    #[allow(dead_code)]
    pub fn overlaps(self, other: LineRange) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}
