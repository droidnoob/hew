//! `hew remember --type=<prefix> "<body>"` — write a memory with an
//! enforced allowlist. The `--raw` escape hatch skips validation for
//! migrations. `--from-file <path>` reads a JSON array of entries for
//! bulk insert.

use std::path::PathBuf;

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::error::HewError;
use hew_core::memories::links::{LinkKind, build_link_row_body};
use hew_core::memories::suggest::{Suggestion, rank_related};
use hew_core::tasks::{self, MEMORY_PREFIXES, validate_memory_type};
use serde::{Deserialize, Serialize};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Memory type — one of: convention, boundary, security, audit,
    /// decision, status, gotcha, feedback, project, milestone, roadmap,
    /// research, dep, factual. The canonical UPPER prefix is prepended
    /// to the body before write.
    #[arg(long = "type", conflicts_with_all = ["raw", "from_file"])]
    pub kind: Option<String>,

    /// Bare body. With `--type=foo`, written as `FOO:<body>`. With
    /// `--raw`, written verbatim. Omit when using `--from-file`.
    #[arg(conflicts_with = "from_file")]
    pub body: Option<String>,

    /// Optional explicit key for upsert-by-key semantics.
    #[arg(long, conflicts_with = "from_file")]
    pub key: Option<String>,

    /// Skip allowlist validation; write `body` verbatim. Use only for
    /// migrations or temporarily-unknown prefixes.
    #[arg(long, conflicts_with = "from_file")]
    pub raw: bool,

    /// Bulk insert: path to a JSON array of memory entries. Each entry
    /// is `{ "type": "<kind>", "body": "<text>", "key": "<opt>", "raw": false }`.
    /// `type` is required unless `raw=true`. Fail-fast on first invalid
    /// entry (the entry index is included in the error).
    #[arg(long = "from-file", value_name = "PATH")]
    pub from_file: Option<PathBuf>,

    /// Emit a `LINK:<key>->relates_to:memory:<related>` sidecar memory
    /// after the primary write. Repeatable. Requires `--key` so the
    /// `<from>` side of the link is deterministic. The related memory
    /// key must match the LINK row charset (`[a-z0-9._-]+`).
    #[arg(long = "related", value_name = "KEY", requires = "key", conflicts_with = "from_file")]
    pub related: Vec<String>,

    /// Emit a `LINK:<key>->relates_to:task:<id>` sidecar memory after
    /// the primary write. Repeatable. Requires `--key` so the `<from>`
    /// side of the link is deterministic. The task id must match the
    /// LINK row charset (`[a-z0-9._-]+`, e.g. `hew-abc` or `hew-abc.1`).
    #[arg(
        long = "related-task",
        value_name = "ID",
        requires = "key",
        conflicts_with = "from_file"
    )]
    pub related_task: Vec<String>,

    /// Suppress the interactive "these look related — link any?"
    /// prompt that fires (in interactive mode) after the body is
    /// captured but before the primary write. The prompt is already
    /// suppressed under `--non-interactive` / CI / non-TTY stderr;
    /// this flag lets an interactive user opt out too.
    #[arg(long = "no-suggest", conflicts_with = "from_file")]
    pub no_suggest: bool,

    /// Cap on the number of related-memory suggestions shown by
    /// the interactive prompt. Default 3. `0` is equivalent to
    /// `--no-suggest`.
    #[arg(
        long = "suggest-top",
        value_name = "N",
        default_value_t = 3,
        conflicts_with = "from_file"
    )]
    pub suggest_top: usize,
}

/// One entry in a bulk-insert JSON array.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BulkEntry {
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default)]
    pub raw: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;

    if let Some(path) = args.from_file.as_deref() {
        let raw = std::fs::read_to_string(path)
            .map_err(|e| miette::miette!("reading {}: {}", path.display(), e))?;
        let entries: Vec<BulkEntry> = serde_json::from_str(&raw).map_err(|e| {
            miette::miette!("parsing {} as JSON array of entries: {}", path.display(), e)
        })?;
        let n = entries.len();
        run_bulk(&bd, &entries)?;
        if !ctx.quiet {
            println!("remembered {n}");
        }
        return Ok(());
    }

    let body = args.body.ok_or_else(|| HewError::MissingFlag { flag: "body".to_string() })?;

    let payload = if args.raw {
        body
    } else {
        let kind = args.kind.as_deref().ok_or_else(|| HewError::MissingFlag {
            flag: format!("type (one of: {})", MEMORY_PREFIXES.join(", ")),
        })?;
        let upper = validate_memory_type(kind)?;
        if body_already_has_known_prefix(&body) {
            return Err(miette::miette!(
                "body already starts with a known prefix — pass either `--type=<x>` with a bare \
                 body, or `--raw` with the full prefixed string (got: {:?})",
                body.chars().take(40).collect::<String>(),
            ));
        }
        format!("{upper}:{}", body)
    };

    // ML.9 suggestion pass — interactive only, opt-out via
    // --no-suggest. Requires --key (same precondition as --related)
    // because suggested targets feed into the LINK: write path.
    // Under --non-interactive / CI / non-TTY this branch is silent
    // per CONVENTION:cli-non-interactive.
    let mut related = args.related.clone();
    if ctx.interactive && !args.no_suggest && args.suggest_top > 0 && args.key.is_some() {
        let picks = interactive_suggest_picks(&bd, &payload, args.suggest_top, &related)?;
        for k in picks {
            if !related.contains(&k) {
                related.push(k);
            }
        }
    }

    // Validate LINK targets BEFORE the primary write so a malformed
    // --related / --related-task value fails the whole command
    // without leaving a stranded primary memory in bd.
    for v in &related {
        validate_link_target("--related", v)?;
    }
    for v in &args.related_task {
        validate_link_target("--related-task", v)?;
    }

    tasks::remember(&bd, &payload, args.key.as_deref())?;
    if !ctx.quiet {
        match args.key.as_deref() {
            Some(k) => println!("remembered ({k})"),
            None => println!("remembered"),
        }
    }

    // Sidecar LINK: rows. Primary write fired first per
    // DECISION:compact-safety (additions-first); a crash between the
    // primary and the sidecars leaves more memory, not less.
    if !related.is_empty() || !args.related_task.is_empty() {
        let from = args.key.as_deref().expect(
            "clap `requires = \"key\"` guarantees --key is set when --related[-task] is used",
        );
        write_link_sidecars(&bd, ctx, from, &related, &args.related_task)?;
    }
    Ok(())
}

/// Fetch existing memories, rank candidates against the new body,
/// and prompt the interactive user to multi-select. Returns the
/// chosen memory keys (never errors on rank/prompt failure — a
/// suggestion is a courtesy, not a contract).
fn interactive_suggest_picks(
    bd: &dyn BdClient,
    new_body: &str,
    top: usize,
    already_chosen: &[String],
) -> miette::Result<Vec<String>> {
    let memories = match bd.memories() {
        Ok(m) => m,
        Err(_) => return Ok(Vec::new()),
    };
    let pairs: Vec<(&str, &str)> = memories.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let mut suggestions: Vec<Suggestion> = rank_related(new_body, &pairs, top);
    // Skip suggestions the user already passed on the command line.
    suggestions.retain(|s| !already_chosen.iter().any(|c| c == &s.key));
    if suggestions.is_empty() {
        return Ok(Vec::new());
    }

    use inquire::MultiSelect;
    let labels: Vec<String> =
        suggestions.iter().map(|s| format!("{}  ({})", s.key, s.reason)).collect();
    let picked_labels: Vec<String> =
        match MultiSelect::new("These look related — link any?", labels.clone())
            .with_help_message("space to toggle, enter to confirm, esc to skip")
            .prompt()
        {
            Ok(picks) => picks,
            // User pressed Ctrl-C / Esc — treat as "no picks" rather than
            // failing the whole remember.
            Err(_) => return Ok(Vec::new()),
        };
    let picked: Vec<String> = picked_labels
        .iter()
        .filter_map(|l| labels.iter().position(|x| x == l).map(|i| suggestions[i].key.clone()))
        .collect();
    Ok(picked)
}

fn write_link_sidecars(
    bd: &dyn BdClient,
    ctx: &Ctx,
    from: &str,
    related: &[String],
    related_task: &[String],
) -> miette::Result<()> {
    let mut written = 0usize;
    for to in related {
        let body = build_link_row_body(from, LinkKind::Memory, to);
        // No explicit --key: bd auto-derives a slug from the body,
        // which collapses identical LINK: rows to a single key on
        // re-write (idempotent at the bd layer).
        tasks::remember(bd, &body, None)?;
        written += 1;
    }
    for to in related_task {
        let body = build_link_row_body(from, LinkKind::Task, to);
        tasks::remember(bd, &body, None)?;
        written += 1;
    }
    if !ctx.quiet && written > 0 {
        let m = if written == 1 { "memory" } else { "memories" };
        println!("emitted {written} LINK: sidecar {m}");
    }
    Ok(())
}

/// Reject `--related` / `--related-task` values that can't appear in
/// the right-hand side of a LINK row. clap rejects empty arg values
/// when `value_parser` requires them, but we double-check the
/// character set here so a malformed key fails *before* the primary
/// write succeeds — turning a "missing edges" failure into a clean
/// front-door error.
fn validate_link_target(flag: &str, value: &str) -> miette::Result<()> {
    if value.is_empty() {
        return Err(miette::miette!("{flag} requires a non-empty value"));
    }
    let ok = value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'));
    if !ok {
        return Err(miette::miette!(
            "{flag}={value:?} is not a valid LINK target — \
             must match `[a-z0-9._-]+` (lowercase only, no spaces)"
        ));
    }
    Ok(())
}

fn run_bulk(bd: &dyn BdClient, entries: &[BulkEntry]) -> miette::Result<()> {
    // Validate every entry first so a malformed one in the middle of
    // the file doesn't leave a half-written batch in bd.
    let payloads: Vec<String> = entries
        .iter()
        .enumerate()
        .map(|(idx, e)| build_bulk_payload(idx, e))
        .collect::<miette::Result<Vec<_>>>()?;
    for (idx, (entry, payload)) in entries.iter().zip(payloads.iter()).enumerate() {
        tasks::remember(bd, payload, entry.key.as_deref())
            .map_err(|e| miette::miette!("entry[{idx}]: bd remember failed: {e}"))?;
    }
    Ok(())
}

fn build_bulk_payload(idx: usize, entry: &BulkEntry) -> miette::Result<String> {
    if entry.body.is_empty() {
        return Err(miette::miette!("entry[{idx}]: body is empty"));
    }
    if entry.raw {
        if entry.kind.is_some() {
            return Err(miette::miette!(
                "entry[{idx}]: `raw=true` conflicts with `type` — pass one or the other"
            ));
        }
        return Ok(entry.body.clone());
    }
    let kind = entry.kind.as_deref().ok_or_else(|| {
        miette::miette!(
            "entry[{idx}]: `type` is required unless `raw=true` (allowed: {})",
            MEMORY_PREFIXES.join(", ")
        )
    })?;
    let upper = validate_memory_type(kind).map_err(|e| miette::miette!("entry[{idx}]: {e}"))?;
    if body_already_has_known_prefix(&entry.body) {
        return Err(miette::miette!(
            "entry[{idx}]: body already starts with a known prefix — drop `type` and set `raw=true`, or strip the prefix from the body"
        ));
    }
    Ok(format!("{upper}:{}", entry.body))
}

fn body_already_has_known_prefix(body: &str) -> bool {
    let trimmed = body.trim_start();
    let Some(colon) = trimmed.find(':') else {
        return false;
    };
    let prefix = &trimmed[..colon];
    validate_memory_type(prefix).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_prefix() {
        assert!(body_already_has_known_prefix("CONVENTION:foo"));
        assert!(body_already_has_known_prefix("  DECISION:bar"));
        assert!(body_already_has_known_prefix("dep:something")); // case-insensitive via validate
    }

    #[test]
    fn ignores_bodies_without_prefix() {
        assert!(!body_already_has_known_prefix("just a note"));
        assert!(!body_already_has_known_prefix("UNKNOWN:thing"));
        assert!(!body_already_has_known_prefix(""));
    }

    // ──────── bulk-entry payload building ────────

    #[test]
    fn bulk_payload_prepends_canonical_prefix() {
        let e = BulkEntry {
            kind: Some("convention".into()),
            body: "foo — bar".into(),
            key: None,
            raw: false,
        };
        let p = build_bulk_payload(0, &e).unwrap();
        assert_eq!(p, "CONVENTION:foo — bar");
    }

    #[test]
    fn bulk_payload_raw_passes_body_verbatim() {
        let e =
            BulkEntry { kind: None, body: "CUSTOM:already prefixed".into(), key: None, raw: true };
        let p = build_bulk_payload(0, &e).unwrap();
        assert_eq!(p, "CUSTOM:already prefixed");
    }

    #[test]
    fn bulk_payload_rejects_missing_type_when_not_raw() {
        let e = BulkEntry { kind: None, body: "x".into(), key: None, raw: false };
        let err = build_bulk_payload(3, &e).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("entry[3]"), "missing entry index: {msg}");
        assert!(msg.contains("type"), "should mention type: {msg}");
    }

    #[test]
    fn bulk_payload_rejects_unknown_type() {
        let e = BulkEntry { kind: Some("bogus".into()), body: "x".into(), key: None, raw: false };
        let err = build_bulk_payload(1, &e).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("entry[1]"), "missing entry index: {msg}");
    }

    #[test]
    fn bulk_payload_rejects_raw_with_type() {
        let e =
            BulkEntry { kind: Some("convention".into()), body: "x".into(), key: None, raw: true };
        let err = build_bulk_payload(0, &e).unwrap_err();
        assert!(format!("{err:?}").contains("conflicts"));
    }

    #[test]
    fn bulk_payload_rejects_empty_body() {
        let e = BulkEntry {
            kind: Some("convention".into()),
            body: String::new(),
            key: None,
            raw: false,
        };
        let err = build_bulk_payload(0, &e).unwrap_err();
        assert!(format!("{err:?}").contains("body is empty"));
    }

    #[test]
    fn bulk_payload_rejects_double_prefix() {
        let e = BulkEntry {
            kind: Some("convention".into()),
            body: "CONVENTION:already prefixed".into(),
            key: None,
            raw: false,
        };
        let err = build_bulk_payload(2, &e).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("already starts with a known prefix"));
        assert!(msg.contains("entry[2]"));
    }

    #[test]
    fn bulk_entry_round_trips_through_json() {
        let entries = vec![
            BulkEntry {
                kind: Some("convention".into()),
                body: "rule one".into(),
                key: Some("k1".into()),
                raw: false,
            },
            BulkEntry { kind: None, body: "RAW:thing".into(), key: None, raw: true },
        ];
        let json = serde_json::to_string(&entries).unwrap();
        let back: Vec<BulkEntry> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, entries);
    }

    #[test]
    fn bulk_entry_parses_minimal_form() {
        let json = r#"[{"type":"factual","body":"hello"}]"#;
        let entries: Vec<BulkEntry> = serde_json::from_str(json).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].body, "hello");
        assert_eq!(entries[0].kind.as_deref(), Some("factual"));
        assert!(!entries[0].raw);
        assert!(entries[0].key.is_none());
    }
}
