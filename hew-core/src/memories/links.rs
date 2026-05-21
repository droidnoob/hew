//! `LINK:` row grammar — locked contract for memory cross-references.
//!
//! A LINK row is a single-line, ASCII-only sidecar memory body of the
//! form:
//!
//! ```text
//! LINK:<from-key>->relates_to:<kind>:<to>
//! ```
//!
//! where `<kind>` is `memory` or `task`, `<from-key>` is a memory key
//! (the slugified `[a-z0-9-]+` shape that `compact::slugify` and
//! `tasks::remember` already produce), and `<to>` is either another
//! memory key (when `kind=memory`) or a bd issue id like `hew-abc` or
//! `hew-a3f8.1` (when `kind=task`). Embedded newlines and non-ASCII
//! characters are not part of the grammar.
//!
//! This module is the foundation slice of the hew-dko epic. T1 locks
//! the grammar (this file). T2 adds the parser; downstream tasks add
//! the writer + reader surfaces. Anything that touches LINK rows must
//! agree on the [`LINK_ROW_PATTERN`] regex source defined here.

use std::fmt;

/// Frozen regex source for the LINK: row grammar.
///
/// The pattern is anchored on both ends so partial matches don't slip
/// through. Each capture group is:
///
/// 1. `from` — the originating memory key (`[a-z0-9-]+`, also accepts
///    `.` and `_` for craft-style keys like `craft.fail-fast`).
/// 2. `kind` — `memory` or `task`.
/// 3. `to` — the target reference. Charset matches `from` plus accepts
///    bd subtask dots like `hew-a3f8.1`.
///
/// Kept as `&str` instead of a compiled `Regex` so this module costs
/// nothing at runtime and stays free of a `regex` crate dependency
/// until a later task in the epic decides to adopt one.
pub const LINK_ROW_PATTERN: &str =
    r"^LINK:([a-z0-9._-]+)->relates_to:(memory|task):([a-z0-9._-]+)$";

/// Discriminator for the target side of a LINK row.
///
/// `Memory` points at another memory key; `Task` points at a bd issue
/// id (top-level or subtask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKind {
    Memory,
    Task,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Task => "task",
        }
    }
}

impl fmt::Display for LinkKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parsed representation of a single LINK: row.
///
/// String fields are owned because the source line is typically read
/// once from bd and held longer than the borrow. Round-tripping back
/// to the canonical wire form is T2's problem.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LinkRow {
    pub from: String,
    pub kind: LinkKind,
    pub to: String,
}

/// Parse a single LINK row.
///
/// **Stub.** Always returns `None`. Wired up so callers can take a
/// dependency on the signature before T2 lands the real parser.
pub fn parse_link_row(_line: &str) -> Option<LinkRow> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hand-rolled grammar predicate so the test does not pull in a
    /// regex dependency at T1. Mirrors [`LINK_ROW_PATTERN`] exactly:
    /// reject embedded newlines / non-ASCII up front, then split on
    /// the two structural tokens and validate each piece's charset.
    fn matches_grammar(line: &str) -> bool {
        if !line.is_ascii() || line.contains('\n') {
            return false;
        }
        let rest = match line.strip_prefix("LINK:") {
            Some(r) => r,
            None => return false,
        };
        let (from, after_from) = match rest.split_once("->relates_to:") {
            Some(parts) => parts,
            None => return false,
        };
        let (kind, to) = match after_from.split_once(':') {
            Some(parts) => parts,
            None => return false,
        };
        if !matches!(kind, "memory" | "task") {
            return false;
        }
        is_key_charset(from) && is_key_charset(to)
    }

    fn is_key_charset(s: &str) -> bool {
        !s.is_empty()
            && s.chars().all(|c| {
                c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.')
            })
    }

    #[test]
    fn pattern_is_anchored_and_frozen() {
        // If this constant changes, every downstream LINK consumer
        // (parser, writer, reader) needs a coordinated bump. Pin the
        // exact string so accidental drift fails CI.
        assert_eq!(
            LINK_ROW_PATTERN,
            r"^LINK:([a-z0-9._-]+)->relates_to:(memory|task):([a-z0-9._-]+)$"
        );
    }

    #[test]
    fn accepts_memory_to_memory() {
        assert!(matches_grammar(
            "LINK:convention-cli-output->relates_to:memory:decision-review-filing"
        ));
    }

    #[test]
    fn accepts_memory_to_task_with_bd_id() {
        assert!(matches_grammar("LINK:gotcha-test-counts-drift->relates_to:task:hew-f75"));
        // bd subtask ids carry a dot — must still match.
        assert!(matches_grammar("LINK:decision-auth->relates_to:task:hew-a3f8.1"));
    }

    #[test]
    fn rejects_missing_arrow() {
        assert!(!matches_grammar(
            "LINK:convention-cli-output relates_to:memory:decision-review-filing"
        ));
        assert!(!matches_grammar(
            "LINK:convention-cli-output:relates_to:memory:decision-review-filing"
        ));
    }

    #[test]
    fn rejects_unknown_kind() {
        assert!(!matches_grammar("LINK:foo->relates_to:epic:hew-dko"));
    }

    #[test]
    fn rejects_non_ascii_and_newlines() {
        assert!(!matches_grammar("LINK:café->relates_to:memory:bar"));
        assert!(!matches_grammar("LINK:foo->relates_to:memory:bar\n"));
    }

    #[test]
    fn link_kind_round_trips_string_form() {
        assert_eq!(LinkKind::Memory.as_str(), "memory");
        assert_eq!(LinkKind::Task.as_str(), "task");
        assert_eq!(format!("{}", LinkKind::Memory), "memory");
    }

    #[test]
    fn parse_link_row_is_callable_and_returns_none_for_now() {
        // T1 acceptance: signature exists and is callable. T2 promotes
        // this to an actual parser; until then, every input maps to
        // None so misuse can't silently succeed.
        assert!(parse_link_row("LINK:foo->relates_to:memory:bar").is_none());
        assert!(parse_link_row("definitely not a link row").is_none());
    }
}
