//! End-of-loop summary aggregator + renderer.
//!
//! Builds a [`Summary`] from the run state + per-iter logs, then
//! renders it as a coloured text block for stdout. Pure (no I/O); the
//! CLI layer prints whatever [`render`] returns.

use std::collections::BTreeMap;

use crate::loop_log::{IterLog, ManifestWorker};
use crate::runner::{Run, StopReason, TokenSpend};
use crate::scope::{self, Scope};
use crate::time::parse_iso_utc;

/// Aggregate view of a finished (or in-flight) loop run. All fields
/// derive from `run` + the per-iter log Vec — no extra state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub run_id: String,
    pub iter_count: u32,
    /// Wall-clock seconds if every iter has a valid `started_at`/
    /// `ended_at`. `None` when any iter is missing one (e.g. an
    /// in-flight crash before `ended_at` was written).
    pub duration_secs: Option<i64>,
    /// Outcome label → count. Labels mirror `loop_log::outcome_label`.
    pub outcomes: BTreeMap<String, u32>,
    pub cost: TokenSpend,
    /// First iter (1-indexed) where `prompt_prefix_hash` matched the
    /// preceding iter's, marking the start of the cache-hit run.
    /// `None` if no two consecutive iters share a hash.
    pub cache_stable_from: Option<u32>,
    pub decisions: u32,
    pub deferred: u32,
    /// Per-iter token totals (in iter-number order). Drives the
    /// sparkline.
    pub per_iter_tokens: Vec<u64>,
    pub stop_reason: Option<StopReason>,
    /// All symbols the run touched across every iter, deduplicated.
    /// Empty when treesitter is off or no commits were made.
    pub symbols_touched: Vec<String>,
    /// Per-model token breakdown, in first-seen order. Empty when no
    /// iter populated the `model` field (legacy / pre-Epic-D runs).
    pub per_model: Vec<ModelBreakdown>,
    /// Which slice of bd-ready tasks the run was scoped to. `None`
    /// renders as `"ready (legacy)"` for `run.json` files that predate
    /// the field; `Some(Scope::Ready)` renders as `"ready"`. Defaults
    /// to `None` from [`summarize`]; live callers populate from
    /// `run.config.scope`, re-render callers from `RunLog.scope`.
    pub scope: Option<Scope>,
}

/// One row of the per-model breakdown table. `model` is the resolved
/// label as written to `IterLog.model`; iters with `model=None` are
/// grouped under `"(default)"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelBreakdown {
    pub model: String,
    pub iter_count: u32,
    pub tasks_closed: u32,
    pub input: u64,
    pub cached: u64,
    pub output: u64,
    pub total: u64,
}

/// Build a [`Summary`] from the run + its iter logs. `iter_logs` is
/// expected to be in iter-number order; callers in the live loop
/// already maintain that invariant.
pub fn summarize(run: &Run, iter_logs: &[IterLog]) -> Summary {
    let iter_count = iter_logs.len() as u32;

    let duration_secs = run.iters.first().and_then(|first| {
        let start = parse_iso_utc(&first.started_at)?;
        let last = run.iters.last()?;
        let end = parse_iso_utc(last.ended_at.as_deref()?)?;
        Some(end - start)
    });

    let mut outcomes: BTreeMap<String, u32> = BTreeMap::new();
    for log in iter_logs {
        if let Some(label) = log.outcome.as_deref() {
            *outcomes.entry(label.to_string()).or_insert(0) += 1;
        }
    }

    let mut cost = TokenSpend::default();
    let mut per_iter_tokens = Vec::with_capacity(iter_logs.len());
    let mut decisions = 0u32;
    let mut deferred = 0u32;
    for log in iter_logs {
        cost.input += log.cost.input;
        cost.output += log.cost.output;
        cost.cache_read += log.cost.cache_read;
        cost.cache_create += log.cost.cache_create;
        per_iter_tokens.push(log.cost.total());
        decisions += log.decisions.len() as u32;
        deferred += log.deferred.len() as u32;
    }

    let mut symbols_touched: Vec<String> = Vec::new();
    for log in iter_logs {
        for s in &log.symbols_touched {
            if !symbols_touched.contains(s) {
                symbols_touched.push(s.clone());
            }
        }
    }

    let per_model = aggregate_per_model(iter_logs);

    let cache_stable_from = iter_logs.windows(2).enumerate().find_map(|(i, pair)| {
        match (&pair[0].prompt_prefix_hash, &pair[1].prompt_prefix_hash) {
            (Some(a), Some(b)) if a == b => Some((i + 2) as u32),
            _ => None,
        }
    });

    Summary {
        run_id: run.id.clone(),
        iter_count,
        duration_secs,
        outcomes,
        cost,
        cache_stable_from,
        decisions,
        deferred,
        per_iter_tokens,
        stop_reason: run.stop_reason,
        symbols_touched,
        per_model,
        scope: None,
    }
}

/// Group iters by their `model` label and sum tokens + iter/task counts.
/// Returns rows in first-seen order. When no iter has `model=Some(_)`
/// we return an empty Vec — the render side hides the section entirely.
fn aggregate_per_model(iter_logs: &[IterLog]) -> Vec<ModelBreakdown> {
    let any_populated = iter_logs.iter().any(|l| l.model.is_some());
    if !any_populated {
        return Vec::new();
    }
    let mut order: Vec<String> = Vec::new();
    let mut rows: BTreeMap<String, ModelBreakdown> = BTreeMap::new();
    for log in iter_logs {
        let key = log.model.clone().unwrap_or_else(|| "(default)".to_string());
        if !rows.contains_key(&key) {
            order.push(key.clone());
            rows.insert(
                key.clone(),
                ModelBreakdown {
                    model: key.clone(),
                    iter_count: 0,
                    tasks_closed: 0,
                    input: 0,
                    cached: 0,
                    output: 0,
                    total: 0,
                },
            );
        }
        let row = rows.get_mut(&key).expect("inserted above");
        row.iter_count += 1;
        if log.outcome.as_deref() == Some("closed") {
            row.tasks_closed += 1;
        }
        row.input += log.cost.input;
        row.cached += log.cost.cache_read + log.cost.cache_create;
        row.output += log.cost.output;
        row.total += log.cost.total();
    }
    order.into_iter().map(|k| rows.remove(&k).expect("present")).collect()
}

/// Render the summary as a coloured terminal block. Pass `colorize=false`
/// to strip ANSI escapes (caller honours `NO_COLOR`).
pub fn render(summary: &Summary, logs_path: &str, colorize: bool) -> String {
    use std::fmt::Write;
    let mut s = String::new();
    let mag = if colorize { "\x1b[1;35m" } else { "" };
    let dim = if colorize { "\x1b[2m" } else { "" };
    let bold = if colorize { "\x1b[1m" } else { "" };
    let green = if colorize { "\x1b[32m" } else { "" };
    let yellow = if colorize { "\x1b[33m" } else { "" };
    let red = if colorize { "\x1b[31m" } else { "" };
    let reset = if colorize { "\x1b[0m" } else { "" };

    // Small "hew" banner — 3 lines, magenta.
    let _ = writeln!(s, "{mag} | |_   ___  __ __ __{reset}");
    let _ = writeln!(s, "{mag} | ' \\ / -_) \\ V  V / {reset}{dim}loop summary{reset}");
    let _ = writeln!(s, "{mag} |_||_|\\___|  \\_/\\_/{reset}");
    let _ = writeln!(s);

    let dur = match summary.duration_secs {
        Some(d) if d > 0 => format_duration(d),
        _ => "—".to_string(),
    };
    let _ = writeln!(
        s,
        "  {bold}run-id{reset}:    {} {dim}({dur}, {} iter{}){reset}",
        summary.run_id,
        summary.iter_count,
        if summary.iter_count == 1 { "" } else { "s" },
    );

    // Outcomes — colour-code by label.
    if !summary.outcomes.is_empty() {
        let mut parts: Vec<String> = Vec::new();
        for (label, count) in &summary.outcomes {
            let painted = match label.as_str() {
                "closed" => format!("{green}{count} {label}{reset}"),
                "no_close" => format!("{yellow}{count} {label}{reset}"),
                "backpressure_fail" | "runtime_error" => {
                    format!("{red}{count} {label}{reset}")
                }
                _ => format!("{count} {label}"),
            };
            parts.push(painted);
        }
        let _ = writeln!(s, "  {bold}outcomes{reset}:  {}", parts.join("  "));
    }

    // Scope line — between outcomes and tokens so operators can see at
    // a glance which queue the run consumed.
    let scope_label = scope::label_optional(summary.scope.as_ref());
    let _ = writeln!(s, "  {bold}scope{reset}:     {scope_label}");

    // Token breakdown.
    let total = summary.cost.total();
    let _ = writeln!(s, "  {bold}tokens{reset}:    {} total", fmt_int(total));
    if total > 0 {
        let line = |label: &str, n: u64, last: bool| -> String {
            let pct = (n as f64 / total as f64) * 100.0;
            let connector = if last { "└─" } else { "├─" };
            format!(
                "             {dim}{connector}{reset} {label:<14} {:>11}  {dim}({:>4.1}%){reset}",
                fmt_int(n),
                pct,
            )
        };
        let _ = writeln!(s, "{}", line("input:", summary.cost.input, false));
        let _ = writeln!(s, "{}", line("output:", summary.cost.output, false));
        let _ = writeln!(s, "{}", line("cache_read:", summary.cost.cache_read, false));
        let _ = writeln!(s, "{}", line("cache_create:", summary.cost.cache_create, true));
    }

    // Cache stability.
    let cache_line = match summary.cache_stable_from {
        Some(start) if summary.iter_count >= start => {
            format!(
                "iter {}-{} hit {dim}(prefix_hash stable from iter {}){reset}",
                start,
                summary.iter_count,
                start - 1,
            )
        }
        _ if summary.iter_count <= 1 => "—".to_string(),
        _ => format!("{yellow}no cross-iter cache hits{reset}"),
    };
    let _ = writeln!(s, "  {bold}cache{reset}:     {cache_line}");

    // Decisions / deferred.
    if summary.decisions > 0 || summary.deferred > 0 {
        let _ = writeln!(
            s,
            "  {bold}decisions{reset}: {} resolved, {} deferred",
            summary.decisions, summary.deferred,
        );
    }

    // Symbols touched across the run (top 8 + footer).
    if !summary.symbols_touched.is_empty() {
        let total = summary.symbols_touched.len();
        let shown: Vec<&str> = summary.symbols_touched.iter().take(8).map(String::as_str).collect();
        let footer = if total > 8 {
            format!(" {dim}…(+{} more){reset}", total - 8)
        } else {
            String::new()
        };
        let _ = writeln!(s, "  {bold}symbols{reset}:   {}{footer}", shown.join(", "));
    }

    // Per-model breakdown (hidden when no iter recorded a model).
    if !summary.per_model.is_empty() {
        let _ = writeln!(s, "  {bold}by model{reset}:");
        let headers = ["model", "iters", "tasks", "input", "cached", "output", "total"];
        let mut widths: [usize; 7] = std::array::from_fn(|i| headers[i].chars().count());
        let rendered_rows: Vec<[String; 7]> = summary
            .per_model
            .iter()
            .map(|m| {
                [
                    m.model.clone(),
                    m.iter_count.to_string(),
                    m.tasks_closed.to_string(),
                    fmt_int(m.input),
                    fmt_int(m.cached),
                    fmt_int(m.output),
                    fmt_int(m.total),
                ]
            })
            .collect();
        for row in &rendered_rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let fmt_row = |cells: &[&str; 7]| -> String {
            // First column left-aligned, numeric columns right-aligned.
            let mut out = format!("    {:<w$}", cells[0], w = widths[0]);
            for i in 1..7 {
                out.push_str(&format!("  {:>w$}", cells[i], w = widths[i]));
            }
            out
        };
        let header_refs: [&str; 7] = std::array::from_fn(|i| headers[i]);
        let _ = writeln!(s, "{dim}{}{reset}", fmt_row(&header_refs));
        for row in &rendered_rows {
            let refs: [&str; 7] = std::array::from_fn(|i| row[i].as_str());
            let _ = writeln!(s, "{}", fmt_row(&refs));
        }
    }

    // Sparkline (skip when only one iter).
    if summary.per_iter_tokens.len() >= 2 {
        let spark = sparkline(&summary.per_iter_tokens);
        let _ = writeln!(s, "  {bold}per-iter{reset}:  {spark}  {dim}(token spend){reset}");
    }

    let stop =
        summary.stop_reason.map(|r| format!("{r:?}")).unwrap_or_else(|| "(none)".to_string());
    let _ = writeln!(s, "  {bold}stop{reset}:      {stop}");
    let _ = writeln!(s, "  {bold}logs{reset}:      {dim}{logs_path}{reset}");
    s
}

/// Per-worker slice of a parallel loop run: enough to render one row
/// of the breakdown table without re-walking the full per-iter logs at
/// render time. Built from a [`ManifestWorker`] + that worker's iter
/// logs by [`worker_slice`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerSlice {
    pub worker_n: u32,
    pub iter_count: u32,
    /// Iters whose outcome was `closed` — a proxy for "tasks this
    /// worker pushed across the line."
    pub tasks_closed: u32,
    /// Last non-`None` `runtime_used` seen in the worker's iters.
    /// Reflects the runtime the worker ended on (post-cooldown for
    /// fallback runs); `None` when no iter recorded one (dry-run).
    pub runtime_used: Option<String>,
    pub total_tokens: u64,
    pub stop_reason: Option<String>,
}

/// Build a [`WorkerSlice`] from the manifest row + the worker's
/// per-iter logs. `iter_count` and `total_tokens` mirror the manifest
/// row (authoritative — written at shutdown); `tasks_closed` /
/// `runtime_used` come from walking the logs.
pub fn worker_slice(row: &ManifestWorker, iter_logs: &[IterLog]) -> WorkerSlice {
    let tasks_closed =
        iter_logs.iter().filter(|l| l.outcome.as_deref() == Some("closed")).count() as u32;
    let runtime_used =
        iter_logs.iter().rev().find_map(|l| l.runtime_used.clone()).filter(|s| !s.is_empty());
    WorkerSlice {
        worker_n: row.id,
        iter_count: row.iter_count,
        tasks_closed,
        runtime_used,
        total_tokens: row.cumulative_tokens,
        stop_reason: row.stop_reason.clone(),
    }
}

/// Render a per-worker breakdown table with totals row. Empty input
/// returns an empty string (caller decides whether to print the
/// section header).
pub fn render_parallel_breakdown(slices: &[WorkerSlice], colorize: bool) -> String {
    use std::fmt::Write;
    if slices.is_empty() {
        return String::new();
    }
    let bold = if colorize { "\x1b[1m" } else { "" };
    let dim = if colorize { "\x1b[2m" } else { "" };
    let reset = if colorize { "\x1b[0m" } else { "" };

    let mut s = String::new();
    let _ = writeln!(s, "  {bold}per-worker{reset}:");
    // Column widths sized for the common case (10s of iters, <10M
    // tokens, runtime ∈ {claude, codex}). Stop column truncates at 16.
    let _ = writeln!(
        s,
        "             {dim}{:<4} {:>6} {:>7} {:<8} {:>11} {:<16}{reset}",
        "wkr", "iters", "closed", "runtime", "tokens", "stop",
    );

    let mut total_iters = 0u32;
    let mut total_closed = 0u32;
    let mut total_tokens = 0u64;
    for sl in slices {
        let runtime = sl.runtime_used.as_deref().unwrap_or("-");
        let stop = sl.stop_reason.as_deref().unwrap_or("-");
        let stop_trunc: String = stop.chars().take(16).collect();
        let _ = writeln!(
            s,
            "             {:<4} {:>6} {:>7} {:<8} {:>11} {:<16}",
            sl.worker_n,
            sl.iter_count,
            sl.tasks_closed,
            runtime,
            fmt_int(sl.total_tokens),
            stop_trunc,
        );
        total_iters += sl.iter_count;
        total_closed += sl.tasks_closed;
        total_tokens += sl.total_tokens;
    }
    let _ = writeln!(
        s,
        "             {dim}{:<4} {:>6} {:>7} {:<8} {:>11} {:<16}{reset}",
        "all",
        total_iters,
        total_closed,
        "",
        fmt_int(total_tokens),
        "",
    );
    s
}

/// 8-block Unicode sparkline scaled to the max value in the slice.
/// Empty slice → empty string. All-zero slice → all-`▁`.
fn sparkline(values: &[u64]) -> String {
    const BARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = values.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return BARS[0].to_string().repeat(values.len());
    }
    values
        .iter()
        .map(|&v| {
            let idx = ((v as f64 / max as f64) * (BARS.len() - 1) as f64).round() as usize;
            BARS[idx.min(BARS.len() - 1)]
        })
        .collect()
}

/// Format seconds as `Hh Mm Ss`, trimming zero-prefix segments.
fn format_duration(secs: i64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match (h, m, s) {
        (0, 0, s) => format!("{s}s"),
        (0, m, s) => format!("{m}m {s}s"),
        (h, m, s) => format!("{h}h {m}m {s}s"),
    }
}

/// Thousands-separated integer ("1,696,142").
fn fmt_int(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, b) in bytes.iter().enumerate() {
        let from_end = bytes.len() - i;
        if i > 0 && from_end.is_multiple_of(3) {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Iter, IterOutcome, Run, RunConfig};

    fn run_with(iters: Vec<Iter>) -> Run {
        let mut r = Run::new("loop-test", "2026-05-26T00:00:00Z", RunConfig::default());
        r.iters = iters;
        r
    }

    fn iter(n: u32, started: &str, ended: &str, outcome: IterOutcome, tokens: TokenSpend) -> Iter {
        let mut it = Iter::new(n, started);
        it.ended_at = Some(ended.into());
        it.outcome = Some(outcome);
        it.cost = tokens;
        it
    }

    fn iter_log(n: u32, label: &str, prefix: Option<&str>, tokens: TokenSpend) -> IterLog {
        iter_log_with_symbols(n, label, prefix, tokens, Vec::new())
    }

    fn iter_log_with_symbols(
        n: u32,
        label: &str,
        prefix: Option<&str>,
        tokens: TokenSpend,
        symbols_touched: Vec<String>,
    ) -> IterLog {
        IterLog {
            number: n,
            task_id: None,
            started_at: format!("2026-05-26T00:00:{:02}Z", n - 1),
            ended_at: Some(format!("2026-05-26T00:00:{:02}Z", n)),
            outcome: Some(label.into()),
            prompt_prefix_hash: prefix.map(str::to_string),
            cost: tokens,
            decisions: Vec::new(),
            deferred: Vec::new(),
            tool_calls: Vec::new(),
            stderr_tail: None,
            symbols_touched,
            runtime_used: None,
            cooldown_engaged: false,
            model: None,
        }
    }

    #[test]
    fn fmt_int_inserts_thousand_separators() {
        assert_eq!(fmt_int(0), "0");
        assert_eq!(fmt_int(42), "42");
        assert_eq!(fmt_int(1_000), "1,000");
        assert_eq!(fmt_int(1_696_142), "1,696,142");
    }

    #[test]
    fn format_duration_trims_zero_prefixes() {
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(125), "2m 5s");
        assert_eq!(format_duration(3725), "1h 2m 5s");
    }

    #[test]
    fn sparkline_scales_to_max_and_handles_zeros() {
        assert_eq!(sparkline(&[]), "");
        // All-zero → first bar repeated.
        assert_eq!(sparkline(&[0, 0, 0]), "▁▁▁");
        // Linear ramp → ascending bars.
        let s = sparkline(&[1, 2, 4, 8]);
        let chars: Vec<char> = s.chars().collect();
        assert_eq!(chars.len(), 4);
        // Last char is the peak.
        assert_eq!(chars[3], '█');
    }

    #[test]
    fn summarize_aggregates_outcomes_and_cost() {
        let t = |i, o, cr, cc| TokenSpend { input: i, output: o, cache_read: cr, cache_create: cc };
        let logs = vec![
            iter_log(1, "closed", Some("hashA"), t(100, 50, 0, 1000)),
            iter_log(2, "closed", Some("hashA"), t(200, 100, 5000, 0)),
            iter_log(3, "backpressure_fail", Some("hashA"), t(50, 25, 0, 0)),
        ];
        let iters = vec![
            iter(
                1,
                "2026-05-26T00:00:00Z",
                "2026-05-26T00:00:10Z",
                IterOutcome::Closed,
                t(100, 50, 0, 1000),
            ),
            iter(
                2,
                "2026-05-26T00:00:10Z",
                "2026-05-26T00:00:30Z",
                IterOutcome::Closed,
                t(200, 100, 5000, 0),
            ),
            iter(
                3,
                "2026-05-26T00:00:30Z",
                "2026-05-26T00:00:50Z",
                IterOutcome::BackpressureFail,
                t(50, 25, 0, 0),
            ),
        ];
        let run = run_with(iters);
        let sum = summarize(&run, &logs);

        assert_eq!(sum.iter_count, 3);
        assert_eq!(sum.outcomes.get("closed").copied(), Some(2));
        assert_eq!(sum.outcomes.get("backpressure_fail").copied(), Some(1));
        assert_eq!(sum.cost.input, 350);
        assert_eq!(sum.cost.output, 175);
        assert_eq!(sum.cost.cache_read, 5000);
        assert_eq!(sum.cost.cache_create, 1000);
        assert_eq!(sum.duration_secs, Some(50));
        // Cache stable from iter 2 (hashA on iter 1, matches on iter 2).
        assert_eq!(sum.cache_stable_from, Some(2));
        assert_eq!(sum.per_iter_tokens, vec![1150, 5300, 75]);
    }

    #[test]
    fn summarize_handles_single_iter() {
        let logs = vec![iter_log(1, "closed", Some("h1"), TokenSpend::default())];
        let it = iter(
            1,
            "2026-05-26T00:00:00Z",
            "2026-05-26T00:00:05Z",
            IterOutcome::Closed,
            TokenSpend::default(),
        );
        let run = run_with(vec![it]);
        let sum = summarize(&run, &logs);
        assert_eq!(sum.iter_count, 1);
        assert_eq!(sum.cache_stable_from, None);
        assert_eq!(sum.duration_secs, Some(5));
    }

    #[test]
    fn summarize_handles_no_cache_hits() {
        let logs = vec![
            iter_log(1, "closed", Some("h1"), TokenSpend::default()),
            iter_log(2, "closed", Some("h2"), TokenSpend::default()),
        ];
        let iters = vec![
            iter(
                1,
                "2026-05-26T00:00:00Z",
                "2026-05-26T00:00:05Z",
                IterOutcome::Closed,
                TokenSpend::default(),
            ),
            iter(
                2,
                "2026-05-26T00:00:05Z",
                "2026-05-26T00:00:10Z",
                IterOutcome::Closed,
                TokenSpend::default(),
            ),
        ];
        let run = run_with(iters);
        let sum = summarize(&run, &logs);
        assert_eq!(sum.cache_stable_from, None);
    }

    #[test]
    fn render_contains_expected_sections() {
        let logs = vec![
            iter_log(
                1,
                "closed",
                Some("hashA"),
                TokenSpend { input: 100, output: 50, cache_read: 0, cache_create: 1000 },
            ),
            iter_log(
                2,
                "closed",
                Some("hashA"),
                TokenSpend { input: 200, output: 100, cache_read: 5000, cache_create: 0 },
            ),
        ];
        let iters = vec![
            iter(
                1,
                "2026-05-26T00:00:00Z",
                "2026-05-26T00:00:10Z",
                IterOutcome::Closed,
                logs[0].cost,
            ),
            iter(
                2,
                "2026-05-26T00:00:10Z",
                "2026-05-26T00:00:30Z",
                IterOutcome::Closed,
                logs[1].cost,
            ),
        ];
        let run = run_with(iters);
        let sum = summarize(&run, &logs);
        let txt = render(&sum, "/tmp/loop-test", false);

        // Banner.
        assert!(txt.contains("\\___|"), "missing banner shape: {txt}");
        // Run id row.
        assert!(txt.contains("loop-test"));
        // Outcomes line.
        assert!(txt.contains("2 closed"));
        // Token breakdown rows.
        assert!(txt.contains("input:"));
        assert!(txt.contains("cache_read:"));
        // Cache hit line.
        assert!(txt.contains("iter 2-2 hit") || txt.contains("iter 2-"));
        // Sparkline (at least 2 bar chars).
        assert!(txt.contains("per-iter:"));
        // Logs path.
        assert!(txt.contains("/tmp/loop-test"));
    }

    #[test]
    fn summarize_dedupes_symbols_across_iters() {
        let logs = vec![
            iter_log_with_symbols(
                1,
                "closed",
                Some("h1"),
                TokenSpend::default(),
                vec!["src/a.rs:foo".into(), "src/a.rs:bar".into()],
            ),
            iter_log_with_symbols(
                2,
                "closed",
                Some("h2"),
                TokenSpend::default(),
                vec!["src/a.rs:bar".into(), "src/b.rs:baz".into()],
            ),
        ];
        let iters = vec![
            iter(
                1,
                "2026-05-26T00:00:00Z",
                "2026-05-26T00:00:05Z",
                IterOutcome::Closed,
                TokenSpend::default(),
            ),
            iter(
                2,
                "2026-05-26T00:00:05Z",
                "2026-05-26T00:00:10Z",
                IterOutcome::Closed,
                TokenSpend::default(),
            ),
        ];
        let run = run_with(iters);
        let sum = summarize(&run, &logs);
        assert_eq!(sum.symbols_touched, vec!["src/a.rs:foo", "src/a.rs:bar", "src/b.rs:baz"],);
    }

    #[test]
    fn render_symbols_row_appears_with_touched_list() {
        let logs = vec![iter_log_with_symbols(
            1,
            "closed",
            Some("h1"),
            TokenSpend::default(),
            vec!["src/x.rs:fn_a".into(), "src/y.rs:fn_b".into()],
        )];
        let run = run_with(vec![iter(
            1,
            "2026-05-26T00:00:00Z",
            "2026-05-26T00:00:05Z",
            IterOutcome::Closed,
            TokenSpend::default(),
        )]);
        let sum = summarize(&run, &logs);
        let txt = render(&sum, "/x", false);
        assert!(txt.contains("symbols:"), "missing symbols row:\n{txt}");
        assert!(txt.contains("src/x.rs:fn_a"));
        assert!(txt.contains("src/y.rs:fn_b"));
    }

    fn iter_log_with_model(
        n: u32,
        label: &str,
        tokens: TokenSpend,
        model: Option<&str>,
    ) -> IterLog {
        let mut log = iter_log(n, label, Some("h"), tokens);
        log.model = model.map(str::to_string);
        log
    }

    #[test]
    fn summarize_per_model_groups_and_sums_mixed_run() {
        let t = |i, o, cr, cc| TokenSpend { input: i, output: o, cache_read: cr, cache_create: cc };
        let logs = vec![
            iter_log_with_model(1, "closed", t(100, 50, 0, 1000), Some("opus")),
            iter_log_with_model(2, "no_close", t(80, 40, 200, 0), Some("opus")),
            iter_log_with_model(3, "closed", t(200, 100, 5000, 0), Some("sonnet")),
            iter_log_with_model(4, "closed", t(150, 75, 0, 500), Some("sonnet")),
            iter_log_with_model(5, "closed", t(50, 25, 0, 0), Some("sonnet")),
        ];
        let run = run_with(Vec::new());
        let sum = summarize(&run, &logs);
        assert_eq!(sum.per_model.len(), 2);
        // First-seen order: opus then sonnet.
        let opus = &sum.per_model[0];
        assert_eq!(opus.model, "opus");
        assert_eq!(opus.iter_count, 2);
        assert_eq!(opus.tasks_closed, 1);
        assert_eq!(opus.input, 180);
        assert_eq!(opus.cached, 1200);
        assert_eq!(opus.output, 90);
        assert_eq!(opus.total, 180 + 90 + 1200);

        let sonnet = &sum.per_model[1];
        assert_eq!(sonnet.model, "sonnet");
        assert_eq!(sonnet.iter_count, 3);
        assert_eq!(sonnet.tasks_closed, 3);
        assert_eq!(sonnet.input, 400);
        assert_eq!(sonnet.cached, 5500);
        assert_eq!(sonnet.output, 200);
        assert_eq!(sonnet.total, 400 + 200 + 5500);
    }

    #[test]
    fn summarize_per_model_hides_section_when_no_model_recorded() {
        let logs = vec![
            iter_log(1, "closed", Some("h1"), TokenSpend::default()),
            iter_log(2, "closed", Some("h1"), TokenSpend::default()),
        ];
        let run = run_with(Vec::new());
        let sum = summarize(&run, &logs);
        assert!(sum.per_model.is_empty(), "per_model must be empty when no iter set model");
        let txt = render(&sum, "/x", false);
        assert!(!txt.contains("by model"), "section must be hidden:\n{txt}");
    }

    #[test]
    fn summarize_per_model_default_label_for_unlabeled_iters() {
        // Mixed: one iter has model=None, one has Some("opus"). The None
        // group should appear as "(default)".
        let logs = vec![
            iter_log_with_model(1, "closed", TokenSpend::default(), None),
            iter_log_with_model(2, "closed", TokenSpend::default(), Some("opus")),
        ];
        let run = run_with(Vec::new());
        let sum = summarize(&run, &logs);
        assert_eq!(sum.per_model.len(), 2);
        assert_eq!(sum.per_model[0].model, "(default)");
        assert_eq!(sum.per_model[1].model, "opus");
    }

    #[test]
    fn render_per_model_table_appears_with_columns() {
        let t = |i, o, cr, cc| TokenSpend { input: i, output: o, cache_read: cr, cache_create: cc };
        let logs = vec![
            iter_log_with_model(1, "closed", t(100, 50, 0, 0), Some("opus")),
            iter_log_with_model(2, "closed", t(200, 100, 1000, 0), Some("sonnet")),
        ];
        let run = run_with(Vec::new());
        let sum = summarize(&run, &logs);
        let txt = render(&sum, "/x", false);
        assert!(txt.contains("by model"), "missing per-model header:\n{txt}");
        assert!(txt.contains("opus"));
        assert!(txt.contains("sonnet"));
        // Header row.
        assert!(txt.contains("model"));
        assert!(txt.contains("iters"));
        assert!(txt.contains("tasks"));
        assert!(txt.contains("input"));
        assert!(txt.contains("cached"));
        assert!(txt.contains("output"));
        assert!(txt.contains("total"));
        // Cached column for sonnet row = 1000 (cache_read+cache_create).
        assert!(txt.contains("1,000"), "expected cached sum to render with separator:\n{txt}");
    }

    fn one_iter_summary() -> Summary {
        let logs = vec![iter_log(1, "closed", Some("h1"), TokenSpend::default())];
        let run = run_with(vec![iter(
            1,
            "2026-05-26T00:00:00Z",
            "2026-05-26T00:00:05Z",
            IterOutcome::Closed,
            TokenSpend::default(),
        )]);
        summarize(&run, &logs)
    }

    #[test]
    fn summary_renders_scope_ready() {
        let mut sum = one_iter_summary();
        sum.scope = Some(Scope::Ready);
        let txt = render(&sum, "/x", false);
        assert!(txt.contains("scope:     ready"), "missing ready scope line:\n{txt}");
        assert!(!txt.contains("ready (legacy)"));
        assert!(!txt.contains("epics ["));
    }

    #[test]
    fn summary_renders_scope_epics_with_ids() {
        let mut sum = one_iter_summary();
        sum.scope = Some(Scope::Epics { epic_ids: vec!["hew-6az".into(), "hew-1tq".into()] });
        let txt = render(&sum, "/x", false);
        assert!(
            txt.contains("scope:     epics [hew-6az, hew-1tq]"),
            "missing epics scope line:\n{txt}"
        );
    }

    #[test]
    fn summary_renders_legacy_no_scope_field_as_ready_legacy() {
        let mut sum = one_iter_summary();
        sum.scope = None;
        let txt = render(&sum, "/x", false);
        assert!(txt.contains("scope:     ready (legacy)"), "missing legacy scope line:\n{txt}");
    }

    #[test]
    fn summary_scope_line_appears_between_outcomes_and_tokens() {
        let mut sum = one_iter_summary();
        sum.scope = Some(Scope::Ready);
        let txt = render(&sum, "/x", false);
        let outcomes_pos = txt.find("outcomes:").expect("outcomes row present");
        let scope_pos = txt.find("scope:").expect("scope row present");
        let tokens_pos = txt.find("tokens:").expect("tokens row present");
        assert!(
            outcomes_pos < scope_pos && scope_pos < tokens_pos,
            "scope must sit between outcomes and tokens:\n{txt}"
        );
    }

    #[test]
    fn render_strips_ansi_when_colorize_false() {
        let logs = vec![iter_log(1, "closed", Some("h1"), TokenSpend::default())];
        let it = iter(
            1,
            "2026-05-26T00:00:00Z",
            "2026-05-26T00:00:05Z",
            IterOutcome::Closed,
            TokenSpend::default(),
        );
        let run = run_with(vec![it]);
        let sum = summarize(&run, &logs);
        let txt = render(&sum, "/x", false);
        assert!(!txt.contains('\x1b'), "expected no ANSI escapes, got: {txt:?}");
    }
}
