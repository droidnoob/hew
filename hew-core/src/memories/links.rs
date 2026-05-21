//! `LINK:` row grammar + parser + writer + index.
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
//! This module is the pure-logic core of the hew-dko epic. The CLI
//! wrapper (later task) reads `bd.memories()` and feeds the result
//! into [`read_links`]; readers / scanners / `--json` consumers go
//! through the resulting [`LinkIndex`]. No I/O or bd calls live here.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

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
/// Kept as `&str` instead of a compiled `Regex` so the module stays
/// free of a `regex` crate dependency; [`parse_link_row`] hand-rolls
/// the same grammar against this string.
pub const LINK_ROW_PATTERN: &str =
    r"^LINK:([a-z0-9._-]+)->relates_to:(memory|task):([a-z0-9._-]+)$";

/// Discriminator for the target side of a LINK row.
///
/// `Memory` points at another memory key; `Task` points at a bd issue
/// id (top-level or subtask).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
/// once from bd and held longer than the borrow. [`format_link_row`]
/// renders this back to the canonical wire form.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LinkRow {
    pub from: String,
    pub kind: LinkKind,
    pub to: String,
}

/// Parse a single LINK row.
///
/// Returns `None` for any input that does not match the grammar
/// exactly — wrong prefix, wrong separators, unknown kind, embedded
/// newline, non-ASCII char, or out-of-charset key bytes. Surrounding
/// whitespace is not trimmed; LINK rows are stored as bd memory bodies
/// and the body is what reaches us verbatim.
pub fn parse_link_row(line: &str) -> Option<LinkRow> {
    // Reject anything that can't be ASCII per the grammar before doing
    // any structural work — saves the rest of the parser from caring.
    if !line.is_ascii() || line.contains('\n') {
        return None;
    }
    let rest = line.strip_prefix("LINK:")?;
    let (from, after_from) = rest.split_once("->relates_to:")?;
    let (kind_str, to) = after_from.split_once(':')?;
    let kind = match kind_str {
        "memory" => LinkKind::Memory,
        "task" => LinkKind::Task,
        _ => return None,
    };
    if !is_key_charset(from) || !is_key_charset(to) {
        return None;
    }
    Some(LinkRow { from: from.to_string(), kind, to: to.to_string() })
}

/// Render a [`LinkRow`] back to the canonical wire form. Always
/// round-trips with [`parse_link_row`] for any [`LinkRow`] that was
/// produced by the parser.
pub fn format_link_row(row: &LinkRow) -> String {
    format!("LINK:{}->relates_to:{}:{}", row.from, row.kind.as_str(), row.to)
}

/// Thin wrapper for callers (the binary's `hew remember --related`
/// path) that have a `(from, kind, to)` triple but not a built
/// [`LinkRow`]. Identical to `format_link_row(&LinkRow { from, kind, to })`.
pub fn build_link_row_body(from: &str, kind: LinkKind, to: &str) -> String {
    format!("LINK:{}->relates_to:{}:{}", from, kind.as_str(), to)
}

/// Bidirectional index over a memory set's LINK: rows.
///
/// Built by [`read_links`]. Stores each unique `LinkRow` once in
/// [`Self::rows`] and keeps `BTreeMap<key, Vec<row-index>>` views for
/// outbound / inbound lookups so we never clone a row twice. The set
/// of memory keys observed at construction is retained so
/// [`Self::dangling`] can flag memory-kind targets that point at
/// missing keys.
#[derive(Debug, Default, Clone)]
pub struct LinkIndex {
    rows: Vec<LinkRow>,
    by_from: BTreeMap<String, Vec<usize>>,
    by_to: BTreeMap<String, Vec<usize>>,
    present_keys: BTreeSet<String>,
}

impl LinkIndex {
    /// Borrowed view of every unique LINK row, in insertion order.
    pub fn all(&self) -> &[LinkRow] {
        &self.rows
    }

    /// Every row whose `from` equals `key`.
    pub fn outbound(&self, key: &str) -> Vec<&LinkRow> {
        self.by_from
            .get(key)
            .map(|idxs| idxs.iter().map(|i| &self.rows[*i]).collect())
            .unwrap_or_default()
    }

    /// Every row whose `to` equals `key`.
    pub fn inbound(&self, key: &str) -> Vec<&LinkRow> {
        self.by_to
            .get(key)
            .map(|idxs| idxs.iter().map(|i| &self.rows[*i]).collect())
            .unwrap_or_default()
    }

    /// Rows whose target memory is missing from the input set.
    ///
    /// Only `LinkKind::Memory` targets are checked — task-kind targets
    /// would require a bd query to validate, which this pure module
    /// cannot do.
    pub fn dangling(&self) -> Vec<&LinkRow> {
        self.rows
            .iter()
            .filter(|r| r.kind == LinkKind::Memory && !self.present_keys.contains(&r.to))
            .collect()
    }
}

/// Walk `(key, body)` pairs, parse every body whose body is a LINK
/// row, and return a [`LinkIndex`].
///
/// Bodies that don't parse are silently ignored — this is the only
/// sensible policy for a memory store that mixes LINK rows with
/// freeform `CONVENTION:` / `GOTCHA:` text. Identical LINK rows
/// surfaced from multiple bodies are deduped; the first occurrence
/// wins for indexing order.
pub fn read_links<K, B>(memories: &[(K, B)]) -> LinkIndex
where
    K: AsRef<str>,
    B: AsRef<str>,
{
    let mut idx = LinkIndex::default();
    let mut seen: BTreeSet<LinkRow> = BTreeSet::new();
    for (k, _) in memories {
        idx.present_keys.insert(k.as_ref().to_string());
    }
    for (_, body) in memories {
        let Some(row) = parse_link_row(body.as_ref()) else {
            continue;
        };
        if !seen.insert(row.clone()) {
            continue;
        }
        let pos = idx.rows.len();
        idx.by_from.entry(row.from.clone()).or_default().push(pos);
        idx.by_to.entry(row.to.clone()).or_default().push(pos);
        idx.rows.push(row);
    }
    idx
}

fn is_key_charset(s: &str) -> bool {
    !s.is_empty()
        && s.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
}

// Implementing PartialOrd/Ord on LinkRow so we can put it in a BTreeSet
// for the dedupe pass in read_links. Order is purely structural — by
// (from, kind, to) tuple — and exists only for set membership; no
// caller should depend on this being a meaningful sort key.
impl PartialOrd for LinkRow {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LinkRow {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.from.as_str(), self.kind.as_str(), self.to.as_str()).cmp(&(
            other.from.as_str(),
            other.kind.as_str(),
            other.to.as_str(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ──────── grammar pin ────────

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

    // ──────── parse + format ────────

    #[test]
    fn parse_accepts_memory_to_memory() {
        let row =
            parse_link_row("LINK:convention-cli-output->relates_to:memory:decision-review-filing")
                .expect("valid");
        assert_eq!(row.from, "convention-cli-output");
        assert_eq!(row.kind, LinkKind::Memory);
        assert_eq!(row.to, "decision-review-filing");
    }

    #[test]
    fn parse_accepts_memory_to_task_with_bd_id() {
        let row = parse_link_row("LINK:gotcha-test-counts-drift->relates_to:task:hew-f75")
            .expect("valid");
        assert_eq!(row.kind, LinkKind::Task);
        assert_eq!(row.to, "hew-f75");
    }

    #[test]
    fn parse_accepts_subtask_id_with_dot() {
        let row = parse_link_row("LINK:decision-auth->relates_to:task:hew-a3f8.1").expect("valid");
        assert_eq!(row.to, "hew-a3f8.1");
    }

    #[test]
    fn parse_rejects_missing_arrow() {
        assert!(
            parse_link_row("LINK:convention-cli-output relates_to:memory:decision-review-filing")
                .is_none()
        );
        assert!(
            parse_link_row("LINK:convention-cli-output:relates_to:memory:decision-review-filing")
                .is_none()
        );
    }

    #[test]
    fn parse_rejects_unknown_kind() {
        assert!(parse_link_row("LINK:foo->relates_to:epic:hew-dko").is_none());
    }

    #[test]
    fn parse_rejects_non_ascii_and_newlines() {
        assert!(parse_link_row("LINK:café->relates_to:memory:bar").is_none());
        assert!(parse_link_row("LINK:foo->relates_to:memory:bar\n").is_none());
        assert!(parse_link_row("LINK:foo\n->relates_to:memory:bar").is_none());
    }

    #[test]
    fn parse_rejects_empty_keys() {
        assert!(parse_link_row("LINK:->relates_to:memory:bar").is_none());
        assert!(parse_link_row("LINK:foo->relates_to:memory:").is_none());
    }

    #[test]
    fn parse_rejects_uppercase_in_keys() {
        // Grammar is lowercase-only; uppercase in keys is a common
        // copy-paste mistake and must fail loudly.
        assert!(parse_link_row("LINK:FOO->relates_to:memory:bar").is_none());
    }

    #[test]
    fn format_round_trips_memory_kind() {
        let row = LinkRow {
            from: "convention-cli-output".into(),
            kind: LinkKind::Memory,
            to: "decision-review-filing".into(),
        };
        let s = format_link_row(&row);
        assert_eq!(s, "LINK:convention-cli-output->relates_to:memory:decision-review-filing");
        assert_eq!(parse_link_row(&s), Some(row));
    }

    #[test]
    fn format_round_trips_task_kind_with_subtask_dot() {
        let row =
            LinkRow { from: "decision-auth".into(), kind: LinkKind::Task, to: "hew-a3f8.1".into() };
        let s = format_link_row(&row);
        assert_eq!(s, "LINK:decision-auth->relates_to:task:hew-a3f8.1");
        assert_eq!(parse_link_row(&s), Some(row));
    }

    #[test]
    fn format_round_trips_five_fixtures() {
        let fixtures = [
            "LINK:a->relates_to:memory:b",
            "LINK:convention-errors->relates_to:memory:decision-error-shape",
            "LINK:gotcha-flake->relates_to:task:hew-xyz",
            "LINK:research-axoupdater-0-10->relates_to:task:hew-lv2",
            "LINK:craft.fail-fast->relates_to:memory:convention-errors",
        ];
        for f in fixtures {
            let row = parse_link_row(f).unwrap_or_else(|| panic!("must parse: {f}"));
            assert_eq!(format_link_row(&row), f, "round-trip mismatch for {f}");
        }
    }

    // ──────── LinkIndex: outbound / inbound ────────

    fn link(from: &str, kind: LinkKind, to: &str) -> LinkRow {
        LinkRow { from: from.into(), kind, to: to.into() }
    }

    fn link_body(from: &str, kind: LinkKind, to: &str) -> String {
        format_link_row(&link(from, kind, to))
    }

    #[test]
    fn read_links_outbound_one_to_many() {
        let mems: Vec<(&str, String)> = vec![
            ("convention-errors", link_body("convention-errors", LinkKind::Memory, "decision-a")),
            ("convention-errors-2", link_body("convention-errors", LinkKind::Memory, "decision-b")),
            ("decision-a", "freeform body — not a link row".to_string()),
            ("decision-b", "another freeform body".to_string()),
        ];
        let idx = read_links(&mems);
        let out = idx.outbound("convention-errors");
        assert_eq!(out.len(), 2, "got: {out:?}");
        let targets: Vec<&str> = out.iter().map(|r| r.to.as_str()).collect();
        assert!(targets.contains(&"decision-a"));
        assert!(targets.contains(&"decision-b"));
        // Source that owns no outbound rows returns empty.
        assert!(idx.outbound("decision-a").is_empty());
    }

    #[test]
    fn read_links_outbound_ignores_freeform_bodies() {
        let mems: Vec<(&str, &str)> = vec![
            ("k1", "This is just a regular memory body, no LINK here."),
            ("k2", "LINK:k1->relates_to:memory:k3"),
            ("k3", "Another freeform memory."),
        ];
        let idx = read_links(&mems);
        assert_eq!(idx.all().len(), 1);
        assert_eq!(idx.outbound("k1").len(), 1);
    }

    #[test]
    fn read_links_inbound_many_to_one() {
        let mems: Vec<(&str, String)> = vec![
            ("k1", link_body("k1", LinkKind::Memory, "hub")),
            ("k2", link_body("k2", LinkKind::Memory, "hub")),
            ("k3", link_body("k3", LinkKind::Task, "hew-abc")),
            ("hub", "freeform hub body".to_string()),
        ];
        let idx = read_links(&mems);
        let inbound = idx.inbound("hub");
        assert_eq!(inbound.len(), 2);
        let froms: Vec<&str> = inbound.iter().map(|r| r.from.as_str()).collect();
        assert!(froms.contains(&"k1"));
        assert!(froms.contains(&"k2"));
        // Task targets show up in inbound too.
        assert_eq!(idx.inbound("hew-abc").len(), 1);
    }

    #[test]
    fn read_links_inbound_empty_for_unreferenced_key() {
        let mems: Vec<(&str, String)> = vec![
            ("a", link_body("a", LinkKind::Memory, "b")),
            ("b", "freeform".to_string()),
            ("c", "freeform".to_string()),
        ];
        let idx = read_links(&mems);
        assert!(idx.inbound("c").is_empty());
        assert!(idx.inbound("not-even-a-memory").is_empty());
    }

    // ──────── LinkIndex: dangling ────────

    #[test]
    fn dangling_flags_missing_memory_target() {
        let mems: Vec<(&str, String)> = vec![
            ("present-from", link_body("present-from", LinkKind::Memory, "missing-target")),
            ("present-from-2", link_body("present-from-2", LinkKind::Memory, "present-target")),
            ("present-target", "freeform body".to_string()),
        ];
        let idx = read_links(&mems);
        let dangling = idx.dangling();
        assert_eq!(dangling.len(), 1);
        assert_eq!(dangling[0].to, "missing-target");
    }

    #[test]
    fn dangling_does_not_flag_task_kind_targets() {
        // Task targets can't be validated by this pure module; only
        // memory-kind targets count as "dangling" when missing.
        let mems: Vec<(&str, String)> = vec![
            ("from-a", link_body("from-a", LinkKind::Task, "hew-unknown-id")),
            ("from-b", link_body("from-b", LinkKind::Task, "hew-also-unknown")),
        ];
        let idx = read_links(&mems);
        assert!(idx.dangling().is_empty(), "task-kind misses must not be reported");
    }

    #[test]
    fn dangling_is_empty_when_all_memory_targets_present() {
        let mems: Vec<(&str, String)> =
            vec![("a", link_body("a", LinkKind::Memory, "b")), ("b", "freeform".to_string())];
        let idx = read_links(&mems);
        assert!(idx.dangling().is_empty());
    }

    // ──────── dedupe + all() ────────

    #[test]
    fn read_links_dedupes_identical_rows_across_memories() {
        // Same LINK body cached under two different memory keys —
        // the index should record it once, surface it once in
        // outbound/inbound/all().
        let body = link_body("from-x", LinkKind::Memory, "to-y");
        let mems: Vec<(&str, String)> = vec![
            ("memo-1", body.clone()),
            ("memo-2", body.clone()),
            ("to-y", "freeform".to_string()),
        ];
        let idx = read_links(&mems);
        assert_eq!(idx.all().len(), 1, "duplicate LINK rows must be deduped");
        assert_eq!(idx.outbound("from-x").len(), 1);
        assert_eq!(idx.inbound("to-y").len(), 1);
    }

    // ──────── serde / sanity ────────

    #[test]
    fn link_kind_serde_round_trips_lowercase_strings() {
        let s = serde_json::to_string(&LinkKind::Memory).unwrap();
        assert_eq!(s, "\"memory\"");
        let back: LinkKind = serde_json::from_str("\"task\"").unwrap();
        assert_eq!(back, LinkKind::Task);
    }

    #[test]
    fn link_row_serde_round_trip() {
        let row = link("decision-auth", LinkKind::Task, "hew-a3f8.1");
        let json = serde_json::to_string(&row).unwrap();
        let back: LinkRow = serde_json::from_str(&json).unwrap();
        assert_eq!(back, row);
    }

    #[test]
    fn build_link_row_body_matches_format_link_row() {
        let row = link("decision-auth", LinkKind::Task, "hew-abc");
        assert_eq!(build_link_row_body(&row.from, row.kind, &row.to), format_link_row(&row));
    }

    #[test]
    fn link_kind_display_matches_as_str() {
        assert_eq!(format!("{}", LinkKind::Memory), "memory");
        assert_eq!(format!("{}", LinkKind::Task), "task");
        assert_eq!(LinkKind::Memory.as_str(), "memory");
        assert_eq!(LinkKind::Task.as_str(), "task");
    }
}
