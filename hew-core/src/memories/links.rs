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
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
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

/// Where an indexed link came from.
///
/// `Explicit` = a memory body that *is* a LINK: row (written by
/// `hew remember --related` or by hand). `BodyScan` = a `[[memory-key]]`
/// or `#bd-task` reference extracted from the body of a non-LINK
/// memory by [`scan_body_refs`]. Surface so callers can choose how
/// strict to be — e.g. audit views show both; a strict graph reader
/// can filter to `Explicit` only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum LinkSource {
    #[default]
    Explicit,
    BodyScan,
}

impl LinkSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Explicit => "explicit",
            Self::BodyScan => "body-scan",
        }
    }
}

/// Bidirectional index over a memory set's LINK: rows.
///
/// Built by [`read_links`] or [`read_links_with_body_scan`]. Stores
/// each unique `LinkRow` once in [`Self::rows`] alongside a parallel
/// `sources` vector recording where each row came from. The set of
/// memory keys observed at construction is retained so
/// [`Self::dangling`] can flag memory-kind targets that point at
/// missing keys.
///
/// Dedupe policy: if the same `(from, kind, to)` triple is surfaced
/// by both an explicit LINK: row and a body-scan reference,
/// `Explicit` wins — the explicit annotation is more authoritative.
#[derive(Debug, Default, Clone)]
pub struct LinkIndex {
    rows: Vec<LinkRow>,
    sources: Vec<LinkSource>,
    by_from: BTreeMap<String, Vec<usize>>,
    by_to: BTreeMap<String, Vec<usize>>,
    present_keys: BTreeSet<String>,
}

impl LinkIndex {
    /// Borrowed view of every unique LINK row, in insertion order.
    pub fn all(&self) -> &[LinkRow] {
        &self.rows
    }

    /// Provenance tags parallel to [`Self::all`].
    pub fn sources(&self) -> &[LinkSource] {
        &self.sources
    }

    /// Provenance of `row`. Returns `Explicit` for unknown rows so
    /// the caller doesn't have to disambiguate `None` vs explicit.
    pub fn source_of(&self, row: &LinkRow) -> LinkSource {
        self.rows
            .iter()
            .position(|r| r == row)
            .and_then(|i| self.sources.get(i).copied())
            .unwrap_or(LinkSource::Explicit)
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

    /// Insert `row` with its provenance. Returns `true` if the row
    /// was inserted, `false` if it merged into an existing row.
    /// `Explicit` upgrades any prior `BodyScan` entry for the same
    /// `(from, kind, to)` triple.
    fn insert(&mut self, row: LinkRow, source: LinkSource) -> bool {
        if let Some(pos) = self.rows.iter().position(|r| r == &row) {
            if matches!(self.sources[pos], LinkSource::BodyScan)
                && matches!(source, LinkSource::Explicit)
            {
                self.sources[pos] = LinkSource::Explicit;
            }
            return false;
        }
        let pos = self.rows.len();
        self.by_from.entry(row.from.clone()).or_default().push(pos);
        self.by_to.entry(row.to.clone()).or_default().push(pos);
        self.rows.push(row);
        self.sources.push(source);
        true
    }
}

/// Walk `(key, body)` pairs, parse every body that *is* a LINK row,
/// and return a [`LinkIndex`]. Bodies that don't parse are silently
/// ignored — this is the only sensible policy for a memory store
/// that mixes LINK rows with freeform `CONVENTION:` / `GOTCHA:` text.
/// Identical LINK rows surfaced from multiple bodies are deduped.
///
/// Equivalent to `read_links_with_body_scan(memories, false)`.
pub fn read_links<K, B>(memories: &[(K, B)]) -> LinkIndex
where
    K: AsRef<str>,
    B: AsRef<str>,
{
    read_links_with_body_scan(memories, false)
}

/// Same as [`read_links`], but when `scan_bodies = true`, every
/// non-LINK memory body is also scanned for `[[memory-key]]` and
/// `#prefix-id` references (see [`scan_body_refs`]). Body-derived
/// rows enter the index tagged `LinkSource::BodyScan`; explicit
/// LINK rows enter as `LinkSource::Explicit`. When both sources
/// surface the same edge, `Explicit` wins.
pub fn read_links_with_body_scan<K, B>(memories: &[(K, B)], scan_bodies: bool) -> LinkIndex
where
    K: AsRef<str>,
    B: AsRef<str>,
{
    let mut idx = LinkIndex::default();
    for (k, _) in memories {
        idx.present_keys.insert(k.as_ref().to_string());
    }
    // Pass 1: explicit LINK rows. Walk first so the BodyScan pass can
    // notice already-explicit edges and skip clobbering them.
    for (_, body) in memories {
        let Some(row) = parse_link_row(body.as_ref()) else {
            continue;
        };
        idx.insert(row, LinkSource::Explicit);
    }
    if scan_bodies {
        for (k, body) in memories {
            // Don't re-scan a body that *is* a LINK row — pass 1 took it.
            if parse_link_row(body.as_ref()).is_some() {
                continue;
            }
            for row in scan_body_refs(k.as_ref(), body.as_ref()) {
                idx.insert(row, LinkSource::BodyScan);
            }
        }
    }
    idx
}

/// Scan an arbitrary memory body for inline cross-references and
/// turn each into a directed [`LinkRow`] from `from` → target.
///
/// Two reference forms are recognized:
///
/// 1. `[[memory-key]]` — wikilink to another memory. Yields a
///    `LinkRow { from, kind: Memory, to: memory-key }`. The key
///    inside the brackets must match the LINK row charset
///    (`[a-z0-9._-]+`); uppercase, spaces, or punctuation kill the
///    match.
/// 2. `#prefix-id` — bd task reference (e.g. `#hew-abc`, `#hew-a3f8.1`,
///    `#bd-xyz`). Must start at a word boundary (start of string or
///    after whitespace / common punctuation). The prefix must be
///    lowercase letters followed by `-` and at least one charset
///    byte.
///
/// Escaping: a backslash directly before `[[` (i.e. `\[[k]]`)
/// suppresses the wikilink match. Tasks have no escape syntax — if
/// you need a literal `#hew-abc` in prose, write it in a code span
/// (this scanner doesn't try to understand code spans; the trade-off
/// is intentional simplicity).
///
/// Duplicates inside a single body are folded — `[[foo]] ... [[foo]]`
/// yields one `LinkRow`. The full reader-level dedupe across bodies
/// happens in [`LinkIndex::insert`].
pub fn scan_body_refs(from: &str, body: &str) -> Vec<LinkRow> {
    // The reference grammars are ASCII-only on both sides, but the
    // surrounding body can contain non-ASCII text (em-dashes, smart
    // quotes, …). Walk raw bytes — UTF-8 continuation bytes are >127
    // and never match `[` or `#`, so multi-byte chars are just
    // skipped past safely.
    let mut out: Vec<LinkRow> = Vec::new();
    let mut seen: BTreeSet<(LinkKind, String)> = BTreeSet::new();

    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // ── `[[memory-key]]`
        if i + 1 < bytes.len() && bytes[i] == b'[' && bytes[i + 1] == b'[' {
            // Backslash-escape: `\[[k]]` is ignored.
            let escaped = i > 0 && bytes[i - 1] == b'\\';
            if escaped {
                i += 2;
                continue;
            }
            let start = i + 2;
            if let Some(rel) = find_closer(bytes, start, b"]]") {
                let key = &body[start..start + rel];
                if !key.is_empty() && is_key_charset(key) {
                    let target = key.to_string();
                    if seen.insert((LinkKind::Memory, target.clone())) {
                        out.push(LinkRow {
                            from: from.to_string(),
                            kind: LinkKind::Memory,
                            to: target,
                        });
                    }
                }
                i = start + rel + 2;
                continue;
            }
            i += 2;
            continue;
        }
        // ── `#prefix-id` — must start at a word boundary
        if bytes[i] == b'#' {
            let at_boundary = i == 0
                || matches!(
                    bytes[i - 1],
                    b' ' | b'\t' | b'\n' | b'\r' | b'(' | b'[' | b',' | b'.' | b':' | b';'
                );
            if at_boundary {
                let start = i + 1;
                let end = scan_task_ref(bytes, start);
                if end > start {
                    let id = &body[start..end];
                    if looks_like_task_id(id) {
                        let target = id.to_string();
                        if seen.insert((LinkKind::Task, target.clone())) {
                            out.push(LinkRow {
                                from: from.to_string(),
                                kind: LinkKind::Task,
                                to: target,
                            });
                        }
                        i = end;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    out
}

/// Return the byte-offset (relative to `start`) of `needle` within
/// `bytes[start..]`, or `None` if not found.
fn find_closer(bytes: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || start + needle.len() > bytes.len() {
        return None;
    }
    let mut i = start;
    while i + needle.len() <= bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            return Some(i - start);
        }
        i += 1;
    }
    None
}

/// Walk the byte slice starting at `start`, returning the end index
/// of a contiguous task-ref body (`[a-z0-9._-]+`). Trailing `.`/`-`/`_`
/// bytes are *not* consumed — they're almost always sentence
/// punctuation (`#bd-99.` ends a sentence; the `.` is not part of the
/// id). Subtask dots like `hew-a3f8.1` are preserved because the
/// `1` after the dot stays inside the charset.
fn scan_task_ref(bytes: &[u8], start: usize) -> usize {
    let mut i = start;
    while i < bytes.len()
        && (bytes[i].is_ascii_lowercase()
            || bytes[i].is_ascii_digit()
            || matches!(bytes[i], b'-' | b'_' | b'.'))
    {
        i += 1;
    }
    while i > start && matches!(bytes[i - 1], b'-' | b'_' | b'.') {
        i -= 1;
    }
    i
}

/// A bd-style task id is `<prefix>-<suffix>` where prefix is one or
/// more lowercase letters and suffix has at least one charset byte.
/// `#x` alone, or `#hew` with no hyphen, doesn't qualify.
fn looks_like_task_id(s: &str) -> bool {
    let Some((prefix, suffix)) = s.split_once('-') else {
        return false;
    };
    !prefix.is_empty()
        && !suffix.is_empty()
        && prefix.bytes().all(|b| b.is_ascii_lowercase())
        && suffix.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
        })
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

    // ──────── body scanner (ML.5) ────────

    #[test]
    fn scan_finds_wikilink_to_memory() {
        let refs = scan_body_refs("from-key", "see [[decision-auth]] for the reason");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].from, "from-key");
        assert_eq!(refs[0].kind, LinkKind::Memory);
        assert_eq!(refs[0].to, "decision-auth");
    }

    #[test]
    fn scan_finds_task_ref_with_subtask_dot() {
        let refs = scan_body_refs(
            "convention-cli-output",
            "rule of thumb — see #hew-a3f8.1 for the example",
        );
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].kind, LinkKind::Task);
        assert_eq!(refs[0].to, "hew-a3f8.1");
    }

    #[test]
    fn scan_finds_multiple_refs_in_one_body() {
        let body = "Background in [[convention-errors]]. Originally raised in #hew-xyz; \
                    superseded by #bd-99.";
        let refs = scan_body_refs("from", body);
        assert_eq!(refs.len(), 3);
        let kinds: Vec<LinkKind> = refs.iter().map(|r| r.kind).collect();
        assert!(kinds.contains(&LinkKind::Memory));
        assert!(kinds.contains(&LinkKind::Task));
        let tos: Vec<&str> = refs.iter().map(|r| r.to.as_str()).collect();
        assert!(tos.contains(&"convention-errors"));
        assert!(tos.contains(&"hew-xyz"));
        assert!(tos.contains(&"bd-99"));
    }

    #[test]
    fn scan_dedupes_within_a_single_body() {
        // The within-body dedupe in scan_body_refs returns each unique
        // target once even when the body mentions it twice. (The full
        // cross-body dedupe lives in LinkIndex::insert.)
        let refs = scan_body_refs(
            "from",
            "first [[foo]] and then [[foo]] again, plus #hew-abc and #hew-abc",
        );
        assert_eq!(refs.len(), 2, "got: {refs:?}");
    }

    #[test]
    fn scan_respects_backslash_escape_on_wikilink() {
        let refs = scan_body_refs("from", "literal: \\[[not-a-ref]] but [[real-ref]] is one");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to, "real-ref");
    }

    #[test]
    fn scan_rejects_malformed_refs() {
        // Single brackets, uppercase keys, empty brackets, hashes
        // without a hyphen, hashes mid-word — all must be ignored.
        let refs =
            scan_body_refs("from", "[only-one] [[]] [[UPPER]] not#hew-abc # hew-abc #x #nohypen-");
        // The trailing `-` in `#nohypen-` makes the suffix empty after
        // split_once('-'), so `looks_like_task_id` rejects it. The
        // `#x` ref has no hyphen at all. `not#hew-abc` lacks a word
        // boundary. `# hew-abc` has a space after `#`.
        assert!(refs.is_empty(), "got unexpected refs: {refs:?}");
    }

    #[test]
    fn scan_handles_task_ref_at_start_of_body() {
        // i == 0 is a word boundary.
        let refs = scan_body_refs("from", "#hew-first thing at the start");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to, "hew-first");
    }

    #[test]
    fn scan_finds_refs_in_body_with_non_ascii_surroundings() {
        // Real memory bodies routinely contain em-dashes, smart
        // quotes, and accented words. Refs themselves are ASCII by
        // charset; the scanner must walk past non-ASCII bytes without
        // rejecting the whole body.
        let refs = scan_body_refs("from", "Café — see [[decision-auth]] for context");
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].to, "decision-auth");
    }

    // ──────── reader integration: explicit + scanned merge ────────

    #[test]
    fn read_links_with_body_scan_merges_explicit_and_scanned() {
        // memory `convention-foo` body contains a [[decision-bar]]
        // wikilink (body-scanned). A separate explicit LINK row says
        // `convention-foo -> decision-baz`. The merged index should
        // expose BOTH outbound edges from `convention-foo`.
        let mems: Vec<(&str, String)> = vec![
            ("convention-foo", "rule — see [[decision-bar]] for why".to_string()),
            ("explicit-edge", link_body("convention-foo", LinkKind::Memory, "decision-baz")),
            ("decision-bar", "freeform".to_string()),
            ("decision-baz", "freeform".to_string()),
        ];
        let idx = read_links_with_body_scan(&mems, true);
        let out = idx.outbound("convention-foo");
        assert_eq!(out.len(), 2, "got: {out:?}");
        let tos: Vec<&str> = out.iter().map(|r| r.to.as_str()).collect();
        assert!(tos.contains(&"decision-bar"), "body-scan target missing");
        assert!(tos.contains(&"decision-baz"), "explicit target missing");
    }

    #[test]
    fn explicit_link_wins_over_body_scan_on_dedupe() {
        // Same edge surfaces both as body-scan [[decision-bar]] and
        // explicit LINK row. The merged index keeps one row, tagged
        // Explicit.
        let mems: Vec<(&str, String)> = vec![
            ("foo", "see [[bar]] for context".to_string()),
            ("explicit-edge", link_body("foo", LinkKind::Memory, "bar")),
            ("bar", "freeform".to_string()),
        ];
        let idx = read_links_with_body_scan(&mems, true);
        assert_eq!(idx.all().len(), 1);
        assert_eq!(idx.source_of(&idx.all()[0]), LinkSource::Explicit);
    }

    #[test]
    fn body_scan_disabled_by_default_in_read_links() {
        let mems: Vec<(&str, &str)> = vec![("foo", "see [[bar]] for context"), ("bar", "freeform")];
        let idx = read_links(&mems);
        assert!(idx.all().is_empty(), "read_links must not body-scan");
    }

    #[test]
    fn body_scan_source_tag_propagates() {
        let mems: Vec<(&str, &str)> = vec![("foo", "see [[bar]] for context"), ("bar", "freeform")];
        let idx = read_links_with_body_scan(&mems, true);
        assert_eq!(idx.all().len(), 1);
        assert_eq!(idx.source_of(&idx.all()[0]), LinkSource::BodyScan);
    }

    #[test]
    fn link_source_serde_kebab_case() {
        assert_eq!(serde_json::to_string(&LinkSource::Explicit).unwrap(), "\"explicit\"");
        assert_eq!(serde_json::to_string(&LinkSource::BodyScan).unwrap(), "\"body-scan\"");
        let back: LinkSource = serde_json::from_str("\"body-scan\"").unwrap();
        assert_eq!(back, LinkSource::BodyScan);
    }

    #[test]
    fn link_kind_display_matches_as_str() {
        assert_eq!(format!("{}", LinkKind::Memory), "memory");
        assert_eq!(format!("{}", LinkKind::Task), "task");
        assert_eq!(LinkKind::Memory.as_str(), "memory");
        assert_eq!(LinkKind::Task.as_str(), "task");
    }
}
