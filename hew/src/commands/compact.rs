//! `hew compact` — apply a [`CompactPlan`] or survey per-prefix
//! memory counts. The clustering itself happens in the
//! `hew-compact` skill body; this CLI consumes the resulting plan.
//!
//! Two subcommands:
//!
//! - `hew compact apply [--dry-run]` — reads a [`CompactPlan`] JSON
//!   from stdin, validates it, and either prints what *would* happen
//!   (`--dry-run`) or executes it via [`hew_core::compact::apply`].
//! - `hew compact list-prefixes` — surveys the current memory store
//!   and prints per-prefix counts so the user knows where compaction
//!   is worth running.

use std::collections::BTreeMap;
use std::io::Read;

use clap::{Args as ClapArgs, Subcommand};
use hew_core::bd::{BdClient, RealBd};
use hew_core::compact::{self, CompactPlan};
use hew_core::config;
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Apply a CompactPlan JSON read from stdin.
    Apply(ApplyArgs),
    /// Print per-prefix memory counts so the user knows where
    /// compaction is worth running.
    ListPrefixes,
}

#[derive(Debug, ClapArgs)]
pub struct ApplyArgs {
    /// Print the ApplyReport that WOULD result, without touching bd.
    /// Honors `compact.dry_run_default = true` (the global default)
    /// when neither --dry-run nor --apply is passed.
    #[arg(long, conflicts_with = "apply")]
    pub dry_run: bool,

    /// Force apply even when `compact.dry_run_default` is true.
    #[arg(long)]
    pub apply: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    match args.op {
        Op::Apply(a) => run_apply(ctx, a),
        Op::ListPrefixes => run_list_prefixes(ctx),
    }
}

fn run_apply(ctx: &Ctx, args: ApplyArgs) -> miette::Result<()> {
    let plan = read_plan_from_stdin()?;

    // Validate before any bd contact — per craft.fail-fast.
    let errs = compact::validate(&plan);
    if !errs.is_empty() {
        for e in &errs {
            eprintln!("invalid CompactPlan: {e}");
        }
        return Err(miette::miette!("CompactPlan failed validation ({} errors)", errs.len()));
    }

    let cfg = config::load()?;
    let want_dry_run = if args.dry_run {
        true
    } else if args.apply {
        false
    } else {
        cfg.compact.dry_run_default
    };

    if want_dry_run {
        emit_dry_run(ctx, &plan, &cfg)?;
        return Ok(());
    }

    let bd = RealBd::discover()?;
    let iso_ts = iso_now_utc();
    let report = compact::apply(&bd, &plan, &cfg, &iso_ts)?;

    if matches!(ctx.output, OutputMode::Json) {
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        print_report(&report);
    }
    Ok(())
}

fn emit_dry_run(ctx: &Ctx, plan: &CompactPlan, cfg: &config::Config) -> miette::Result<()> {
    // Build a synthetic ApplyReport showing what would happen, without
    // touching bd. We can't know drift-guard skips without reading
    // memories, so we hit bd read-only here.
    let bd = RealBd::discover().ok();
    let memories = match bd.as_ref() {
        Some(b) => b.memories().unwrap_or_default(),
        None => BTreeMap::new(),
    };

    let mut would_forget = Vec::new();
    let mut exempt_skipped = Vec::new();
    let mut drift_guard_skipped = Vec::new();
    let exempt_set: std::collections::BTreeSet<&str> =
        cfg.compact.exempt.iter().map(|s| s.as_str()).collect();
    let hardcoded = ["STATUS:scan", "STATUS:convention", "STATUS:plan", "STATUS:decompose"];

    for cluster in &plan.clusters {
        for key in &cluster.source_keys {
            let key_str = key.as_str();
            let is_exempt = exempt_set.contains(key_str)
                || hardcoded.iter().any(|p| key == p || key.starts_with(&format!("{p}:")));
            if is_exempt {
                exempt_skipped.push(key.clone());
                continue;
            }
            let already_compacted =
                memories.get(key).is_some_and(|v| v.contains("[compacted-from:"));
            if !plan.allow_recompact && already_compacted {
                drift_guard_skipped.push(key.clone());
                continue;
            }
            would_forget.push(key.clone());
        }
    }
    let would_add: Vec<String> =
        plan.clusters.iter().flat_map(|c| c.replacement_bodies.iter().cloned()).collect();

    if matches!(ctx.output, OutputMode::Json) {
        let preview = serde_json::json!({
            "dry_run": true,
            "prefix": plan.prefix,
            "would_add_count": would_add.len(),
            "would_forget": would_forget,
            "exempt_skipped": exempt_skipped,
            "drift_guard_skipped": drift_guard_skipped,
        });
        println!("{}", serde_json::to_string_pretty(&preview).unwrap());
    } else {
        println!("DRY RUN — no memories written or forgotten.");
        println!();
        println!("Prefix: {}", plan.prefix);
        println!(
            "Would add {} new memory entries across {} clusters.",
            would_add.len(),
            plan.clusters.len()
        );
        println!("Would forget {} source keys:", would_forget.len());
        for k in &would_forget {
            println!("  - {k}");
        }
        if !exempt_skipped.is_empty() {
            println!("Skipped (exempt) {}:", exempt_skipped.len());
            for k in &exempt_skipped {
                println!("  - {k}");
            }
        }
        if !drift_guard_skipped.is_empty() {
            println!("Skipped (drift-guard — already compacted) {}:", drift_guard_skipped.len());
            for k in &drift_guard_skipped {
                println!("  - {k}");
            }
            println!(
                "  (pass `allow_recompact: true` in the plan to override; see DECISION:compact-drift-guard)"
            );
        }
        if !ctx.quiet {
            println!();
            println!("Re-run with --apply to execute.");
        }
    }
    Ok(())
}

fn print_report(report: &hew_core::compact::ApplyReport) {
    println!("COMPACT applied:");
    println!("  added:               {}", report.added.len());
    println!("  forgotten:           {}", report.forgotten.len());
    println!("  exempt skipped:      {}", report.exempt_skipped.len());
    println!("  drift-guard skipped: {}", report.drift_guard_skipped.len());
    if let Some(m) = &report.marker_written {
        println!("  marker:              {m}");
    }
}

fn read_plan_from_stdin() -> miette::Result<CompactPlan> {
    let mut buf = String::new();
    std::io::stdin().read_to_string(&mut buf).map_err(|e| miette::miette!("read stdin: {e}"))?;
    if buf.trim().is_empty() {
        return Err(miette::miette!(
            "stdin is empty — pipe a CompactPlan JSON to `hew compact apply`"
        ));
    }
    serde_json::from_str::<CompactPlan>(&buf)
        .map_err(|e| miette::miette!("parse CompactPlan from stdin: {e}"))
}

fn run_list_prefixes(ctx: &Ctx) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    let memories = bd.memories()?;
    let counts = count_by_prefix(&memories);

    if matches!(ctx.output, OutputMode::Json) {
        println!("{}", serde_json::to_string_pretty(&counts).unwrap());
    } else if counts.is_empty() {
        println!("(no memories)");
    } else {
        let max_label = counts.keys().map(|k| k.len()).max().unwrap_or(8);
        for (prefix, count) in &counts {
            println!("  {prefix:<width$}  {count:>4}", width = max_label);
        }
        println!();
        let total: usize = counts.values().map(|v| *v as usize).sum();
        println!("{} memories across {} prefixes", total, counts.len());
    }
    Ok(())
}

/// Format `SystemTime::now()` as `YYYY-MM-DDTHH:MM:SSZ`. Avoids
/// pulling chrono into the dep tree for a single timestamp emit.
fn iso_now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso_from_unix(secs)
}

/// Convert a Unix timestamp (seconds since epoch) into an ISO-8601
/// UTC string. Algorithm: civil_from_days from Howard Hinnant's
/// public-domain date-calendar code, restated for the Gregorian
/// proleptic calendar — handles every year hew will plausibly see.
fn iso_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;

    // Shift so day 0 is 0000-03-01 (Hinnant's algorithm).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146_096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y_shift = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { y_shift + 1 } else { y_shift };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Group memories by their top-level prefix (everything before the
/// first colon). Memories without a colon land under `(no-prefix)`.
/// Returns a BTreeMap so iteration is deterministic + sorted.
pub fn count_by_prefix(memories: &BTreeMap<String, String>) -> BTreeMap<String, u32> {
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for body in memories.values() {
        let prefix = extract_prefix(body).unwrap_or_else(|| "(no-prefix)".to_string());
        *counts.entry(prefix).or_insert(0) += 1;
    }
    counts
}

/// Hew memory prefix shape: `^[A-Z][A-Z0-9_-]*:` at the start (after
/// any leading whitespace). Returns the slug (without the trailing
/// colon), or `None` if the body doesn't match the convention.
///
/// This is deliberately stricter than `body.split_once(':')` so that
/// natural-language factual memories like "Build: cargo workspace…"
/// don't get treated as carrying a "Build" prefix.
fn extract_prefix(body: &str) -> Option<String> {
    let trimmed = body.trim_start();
    let (head, _) = trimmed.split_once(':')?;
    if head.is_empty() {
        return None;
    }
    let valid =
        head.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_' || c == '-')
            && head.chars().next()?.is_ascii_uppercase();
    if valid { Some(head.to_string()) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memories(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect()
    }

    #[test]
    fn count_by_prefix_groups_correctly() {
        let m = memories(&[
            ("k1", "CONVENTION:foo — bar"),
            ("k2", "CONVENTION:baz — qux"),
            ("k3", "BOUNDARY:api — POST /x"),
            ("k4", "factual snippet with no prefix"),
            ("k5", "RESEARCH:topic [VERIFIED] something"),
        ]);
        let counts = count_by_prefix(&m);
        assert_eq!(counts.get("CONVENTION").copied(), Some(2));
        assert_eq!(counts.get("BOUNDARY").copied(), Some(1));
        assert_eq!(counts.get("RESEARCH").copied(), Some(1));
        assert_eq!(counts.get("(no-prefix)").copied(), Some(1));
    }

    #[test]
    fn count_by_prefix_handles_leading_whitespace() {
        let m = memories(&[("k", "  STATUS:scan — done")]);
        let counts = count_by_prefix(&m);
        assert_eq!(counts.get("STATUS").copied(), Some(1));
    }

    #[test]
    fn count_by_prefix_treats_multiword_prefix_as_no_prefix() {
        // "hello world: foo" — the prefix would be "hello world" which
        // contains whitespace, so it's not a real prefix.
        let m = memories(&[("k", "hello world: foo")]);
        let counts = count_by_prefix(&m);
        assert_eq!(counts.get("(no-prefix)").copied(), Some(1));
    }

    #[test]
    fn count_by_prefix_rejects_natural_language_colons() {
        // Hew convention is UPPER-SNAKE slug + colon. Natural-language
        // factual memories like "Build: cargo workspace…" must NOT be
        // bucketed as carrying a "Build" prefix.
        let m = memories(&[
            ("k1", "Build: cargo workspace with resolver=2"),
            ("k2", "Layout: src/ at the top"),
            ("k3", "CONVENTION:foo — real"),
        ]);
        let counts = count_by_prefix(&m);
        assert_eq!(counts.get("Build"), None);
        assert_eq!(counts.get("Layout"), None);
        assert_eq!(counts.get("(no-prefix)").copied(), Some(2));
        assert_eq!(counts.get("CONVENTION").copied(), Some(1));
    }

    #[test]
    fn count_by_prefix_accepts_hyphen_and_underscore_prefixes() {
        let m = memories(&[
            ("k1", "CRAFT-CATALOG:v1 — 28 principles"),
            ("k2", "SCAN_RESULT:dirs — top-level src/"),
        ]);
        let counts = count_by_prefix(&m);
        assert_eq!(counts.get("CRAFT-CATALOG").copied(), Some(1));
        assert_eq!(counts.get("SCAN_RESULT").copied(), Some(1));
    }

    #[test]
    fn iso_from_unix_formats_known_epochs() {
        assert_eq!(iso_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_from_unix(86_400), "1970-01-02T00:00:00Z");
        // 2024-01-01T00:00:00Z = 1704067200 (well-known constant).
        assert_eq!(iso_from_unix(1_704_067_200), "2024-01-01T00:00:00Z");
        // March 1, 2000 — leap-year boundary case (2000 is leap).
        assert_eq!(iso_from_unix(951_868_800), "2000-03-01T00:00:00Z");
        // Feb 29, 2024 — another leap day.
        assert_eq!(iso_from_unix(1_709_164_800), "2024-02-29T00:00:00Z");
        // Time-of-day combinations.
        assert_eq!(iso_from_unix(1_704_067_200 + 3661), "2024-01-01T01:01:01Z");
    }

    #[test]
    fn iso_now_utc_has_expected_shape() {
        let s = iso_now_utc();
        // Format: YYYY-MM-DDTHH:MM:SSZ — exactly 20 chars.
        assert_eq!(s.len(), 20, "got: {s:?}");
        assert!(s.ends_with('Z'));
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], "T");
        assert_eq!(&s[13..14], ":");
    }

    #[test]
    fn read_plan_rejects_malformed_json() {
        // The function reads stdin so we can't unit-test the read; but
        // the serde error path is exercised by parsing a bad string
        // through the same serde_json::from_str call.
        let bad = "{ \"prefix\": \"X\", \"clusters\": [";
        let result = serde_json::from_str::<CompactPlan>(bad);
        assert!(result.is_err());
    }
}
