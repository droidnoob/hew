//! `hew loop graph` — render a single (or multi-) run's history as a DAG.
//!
//! Two layers in one module:
//!   * [`LoopGraph`] IR + pure renderers ([`render_mermaid`],
//!     [`render_dot`], [`render_ascii`]) — no I/O.
//!   * [`build_from_run_dir`] / [`build_from_loop_root`] — read iter +
//!     batch + run + manifest JSON and lift them into the IR.
//!
//! The split keeps snapshot tests trivial: assemble an IR by hand, call
//! the renderer, compare against a fixed expected string.
//!
//! Unhappy paths the renderer must distinguish (per the task body):
//!   * incomplete iter — `started_at` but no `ended_at` (⋯ dashed)
//!   * cancelled mid-run — run stopped via `.stop`; the in-flight iter
//!     gets ⊘
//!   * runtime error with empty stderr — possibly hung; annotate
//!   * backpressure with rollback — ↺ self-edge with `rolled back` note
//!   * verify failed — verify node renders red + failed test names
//!   * pre-batchplan legacy runs — no `batch-*.json` files; sequential
//!     edges only

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::batch_plan::BatchSource;
use crate::error::Result;
use crate::loop_log::{IterLog, Manifest, RunLog};
use crate::verify::VerifyOutcome;

/// Wire-format-agnostic outcome glyph for a node.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeGlyph {
    Closed,
    NoClose,
    RuntimeError,
    BackpressureFail,
    Cancelled,
    Incomplete,
}

impl OutcomeGlyph {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Closed => "✓",
            Self::NoClose => "◐",
            Self::RuntimeError => "✗",
            Self::BackpressureFail => "↺",
            Self::Cancelled => "⊘",
            Self::Incomplete => "⋯",
        }
    }

    pub fn ascii_glyph(self) -> &'static str {
        match self {
            Self::Closed => "OK",
            Self::NoClose => "NC",
            Self::RuntimeError => "ER",
            Self::BackpressureFail => "BP",
            Self::Cancelled => "CX",
            Self::Incomplete => "..",
        }
    }

    /// Mermaid classDef name. Stable; LOOP.md docs depend on the spelling.
    pub fn mermaid_class(self) -> &'static str {
        match self {
            Self::Closed => "iter-closed",
            Self::NoClose => "iter-no-close",
            Self::RuntimeError => "iter-runtime-err",
            Self::BackpressureFail => "iter-backpressure",
            Self::Cancelled => "iter-cancelled",
            Self::Incomplete => "iter-incomplete",
        }
    }

    pub fn dot_color(self) -> &'static str {
        match self {
            Self::Closed => "green",
            Self::NoClose => "orange",
            Self::RuntimeError | Self::BackpressureFail => "red",
            Self::Cancelled | Self::Incomplete => "gray",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeKind {
    /// Default sequential next-iter edge.
    Sequential,
    /// Previous iter's agent emitted `next_iteration:` containing this iter's task.
    BatchAgent,
    /// Planner sub-process picked this iter's task.
    BatchPlanner,
    /// No batch — dispatcher used `bd ready`.
    Fallback,
    /// Backpressure rollback target. Carries the short sha in `annotation`.
    Rollback,
    /// Final verify-tests edge into the verify node.
    Verify,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: String,
    pub iter_number: u32,
    pub worker_n: Option<u32>,
    pub task_id: Option<String>,
    pub outcome: OutcomeGlyph,
    pub tokens: u64,
    pub duration_secs: Option<u64>,
    /// True when the iter is a runtime error with empty stderr — strong
    /// hint that the subprocess hung. Surface as label annotation.
    pub stderr_hung: bool,
}

#[derive(Clone, Debug)]
pub struct VerifyNode {
    pub outcome: VerifyOutcome,
    /// Top 3 failed test names, when known. Annotates the verify node
    /// in the renderer.
    pub failure_lines: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Edge {
    pub from: String,
    pub to: String,
    pub kind: EdgeKind,
    pub annotation: Option<String>,
}

/// In-memory representation of one run's DAG.
#[derive(Clone, Debug, Default)]
pub struct LoopGraph {
    pub run_id: String,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub verify: Option<VerifyNode>,
    /// ISO timestamp of the cancel signal, when the run terminated via
    /// `.stop`. Annotates the cancelled node.
    pub cancelled_at: Option<String>,
    /// Worker IDs present in the run, sorted. Empty for the `--jobs=1`
    /// fast path; >=2 entries for parallel runs (subgraphs).
    pub workers: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Mermaid,
    Dot,
    Ascii,
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// Read a single run-dir and build its [`LoopGraph`].
pub fn build_from_run_dir(run_dir: &Path) -> Result<LoopGraph> {
    let run_log = read_run_log(run_dir)?;
    let manifest = read_manifest(run_dir);

    let mut g = LoopGraph {
        run_id: run_log.as_ref().map(|r| r.id.clone()).unwrap_or_else(|| {
            run_dir.file_name().and_then(|s| s.to_str()).unwrap_or("loop-unknown").to_string()
        }),
        ..LoopGraph::default()
    };

    let cancelled = run_log.as_ref().and_then(|r| r.stop_reason.as_deref()) == Some("cancelled");

    if let Some(m) = manifest.as_ref() {
        g.workers = m.workers.iter().map(|w| w.id).collect();
        g.workers.sort_unstable();
        for w in &m.workers {
            let dir = match w.log_subdir.as_deref() {
                Some(sub) => run_dir.join(sub),
                None => run_dir.to_path_buf(),
            };
            ingest_worker(&mut g, &dir, Some(w.id), cancelled)?;
        }
    } else {
        ingest_worker(&mut g, run_dir, None, cancelled)?;
    }

    // Mark cancellation annotation timestamp (best-effort: use the
    // last_updated_at from run.json since the .stop file mtime is the
    // ground truth but reading it adds I/O for a cosmetic annotation).
    if cancelled && let Some(rl) = run_log.as_ref() {
        g.cancelled_at = Some(rl.last_updated_at.clone());
    }

    // Verify node sits at the tail.
    if let Some(rl) = run_log.as_ref()
        && let Some(out) = rl.verify_outcome.clone()
    {
        let failure_lines = read_verify_failure_lines(run_dir);
        g.verify = Some(VerifyNode { outcome: out, failure_lines });
        // Wire the last seen iter into the verify node.
        if let Some(last) = g.nodes.last() {
            g.edges.push(Edge {
                from: last.id.clone(),
                to: "verify".into(),
                kind: EdgeKind::Verify,
                annotation: None,
            });
        }
    }

    Ok(g)
}

fn ingest_worker(
    g: &mut LoopGraph,
    dir: &Path,
    worker_n: Option<u32>,
    cancelled: bool,
) -> Result<()> {
    let iter_logs = collect_iter_logs(dir)?;
    if iter_logs.is_empty() {
        return Ok(());
    }
    let last_idx = iter_logs.len() - 1;

    let prefix = match worker_n {
        Some(n) => format!("w{n}_"),
        None => String::new(),
    };

    let node_ids: Vec<String> =
        iter_logs.iter().map(|l| format!("{prefix}iter{}", l.number)).collect();

    for (idx, log) in iter_logs.iter().enumerate() {
        let is_last = idx == last_idx;
        let outcome = classify_outcome(log, cancelled && is_last);
        let stderr_hung = matches!(log.outcome.as_deref(), Some("runtime_error"))
            && log.stderr_tail.as_deref().is_none_or(str::is_empty);

        g.nodes.push(Node {
            id: node_ids[idx].clone(),
            iter_number: log.number,
            worker_n,
            task_id: log.task_id.clone(),
            outcome,
            tokens: log.cost.total(),
            duration_secs: duration_secs(log),
            stderr_hung,
        });

        if idx > 0 {
            let from = node_ids[idx - 1].clone();
            let to = node_ids[idx].clone();
            // Determine edge kind from the batch plan for this iter (if any).
            let edge_kind = batch_edge_kind(dir, log.number);
            g.edges.push(Edge { from, to, kind: edge_kind, annotation: None });
        }

        if matches!(outcome, OutcomeGlyph::BackpressureFail) && idx > 0 {
            g.edges.push(Edge {
                from: node_ids[idx].clone(),
                to: node_ids[idx - 1].clone(),
                kind: EdgeKind::Rollback,
                annotation: Some("rolled back".into()),
            });
        }
    }
    Ok(())
}

fn classify_outcome(log: &IterLog, cancelled_in_flight: bool) -> OutcomeGlyph {
    if log.ended_at.is_none() {
        return if cancelled_in_flight {
            OutcomeGlyph::Cancelled
        } else {
            OutcomeGlyph::Incomplete
        };
    }
    match log.outcome.as_deref() {
        Some("closed") => OutcomeGlyph::Closed,
        Some("no_close") => OutcomeGlyph::NoClose,
        Some("runtime_error") => OutcomeGlyph::RuntimeError,
        Some("backpressure_fail") => OutcomeGlyph::BackpressureFail,
        _ if cancelled_in_flight => OutcomeGlyph::Cancelled,
        _ => OutcomeGlyph::NoClose,
    }
}

fn duration_secs(log: &IterLog) -> Option<u64> {
    let start = parse_iso(&log.started_at)?;
    let end = parse_iso(log.ended_at.as_deref()?)?;
    Some(end.saturating_sub(start))
}

/// Truncated ISO 8601 parser sufficient for `YYYY-MM-DDTHH:MM:SSZ`.
/// Returns seconds since unix epoch.
fn parse_iso(s: &str) -> Option<u64> {
    // YYYY-MM-DDTHH:MM:SSZ — 20 chars
    if s.len() < 19 {
        return None;
    }
    let y: i64 = s.get(0..4)?.parse().ok()?;
    let mo: u32 = s.get(5..7)?.parse().ok()?;
    let d: u32 = s.get(8..10)?.parse().ok()?;
    let h: u32 = s.get(11..13)?.parse().ok()?;
    let mi: u32 = s.get(14..16)?.parse().ok()?;
    let se: u32 = s.get(17..19)?.parse().ok()?;
    // Days-from-civil: Howard Hinnant. Good for the 1970..3000 range we care about.
    let y = if mo <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as u64;
    let doy: u64 = (153 * (if mo > 2 { mo - 3 } else { mo + 9 }) as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe as i64 - 719_468;
    if days < 0 {
        return None;
    }
    Some((days as u64) * 86_400 + (h as u64) * 3_600 + (mi as u64) * 60 + se as u64)
}

fn batch_edge_kind(dir: &Path, iter_number: u32) -> EdgeKind {
    match crate::batch_plan::read(dir, iter_number) {
        Ok(Some(plan)) => match plan.source {
            BatchSource::Agent => EdgeKind::BatchAgent,
            BatchSource::Planner => EdgeKind::BatchPlanner,
            BatchSource::Skipped => EdgeKind::Fallback,
        },
        _ => EdgeKind::Sequential,
    }
}

fn collect_iter_logs(dir: &Path) -> Result<Vec<IterLog>> {
    let mut out: Vec<IterLog> = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.starts_with("iter-") || !name.ends_with(".json") {
            continue;
        }
        if let Ok(body) = fs::read_to_string(&path)
            && let Ok(log) = serde_json::from_str::<IterLog>(&body)
        {
            out.push(log);
        }
    }
    out.sort_by_key(|l| l.number);
    Ok(out)
}

fn read_run_log(dir: &Path) -> Result<Option<RunLog>> {
    let path = dir.join("run.json");
    let body = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    Ok(serde_json::from_str(&body).ok())
}

fn read_manifest(dir: &Path) -> Option<Manifest> {
    let path = dir.join("manifest.json");
    let body = fs::read_to_string(&path).ok()?;
    serde_json::from_str(&body).ok()
}

fn read_verify_failure_lines(dir: &Path) -> Vec<String> {
    let path = dir.join("verify.log");
    let body = match fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    body.lines()
        .filter(|l| l.contains("FAILED") || l.contains("failed") || l.contains("test "))
        .map(str::to_string)
        .take(3)
        .collect()
}

/// Build one graph per run under `loop_root`, sorted by run-id.
pub fn build_from_loop_root(loop_root: &Path) -> Result<Vec<LoopGraph>> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(loop_root) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("loop-"))
        })
        .collect();
    dirs.sort();
    for d in dirs {
        out.push(build_from_run_dir(&d)?);
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Entry point: render one graph in the given format.
pub fn render(g: &LoopGraph, format: Format) -> String {
    match format {
        Format::Mermaid => render_mermaid(g),
        Format::Dot => render_dot(g),
        Format::Ascii => render_ascii(g),
    }
}

/// Render N graphs as a single document. `--all` mode wraps each run as
/// its own subgraph (mermaid/dot) or a stacked section (ascii).
pub fn render_all(graphs: &[LoopGraph], format: Format) -> String {
    match format {
        Format::Mermaid => render_all_mermaid(graphs),
        Format::Dot => render_all_dot(graphs),
        Format::Ascii => render_all_ascii(graphs),
    }
}

fn render_mermaid(g: &LoopGraph) -> String {
    let mut out = String::new();
    out.push_str("flowchart TD\n");
    render_mermaid_body(&mut out, g, "");
    out
}

fn render_all_mermaid(graphs: &[LoopGraph]) -> String {
    let mut out = String::new();
    out.push_str("flowchart TD\n");
    for g in graphs {
        let safe = sanitize_id(&g.run_id);
        out.push_str(&format!("  subgraph {safe}[\"{}\"]\n", g.run_id));
        render_mermaid_body(&mut out, g, "  ");
        out.push_str("  end\n");
    }
    out
}

fn render_mermaid_body(out: &mut String, g: &LoopGraph, indent: &str) {
    // Group nodes by worker for swimlanes when parallel.
    let nodes_by_worker = group_nodes_by_worker(g);
    let parallel = g.workers.len() >= 2;

    for (worker, nodes) in &nodes_by_worker {
        if parallel && let Some(w) = worker {
            out.push_str(&format!("{indent}  subgraph worker-{w}\n"));
        }
        for n in nodes {
            let label = mermaid_label(g, n);
            let shape = if matches!(n.outcome, OutcomeGlyph::Incomplete) {
                format!("{}[/\"{}\"\\]", n.id, label)
            } else {
                format!("{}[\"{}\"]", n.id, label)
            };
            let pad = if parallel { "    " } else { "  " };
            out.push_str(&format!("{indent}{pad}{shape}\n"));
        }
        if parallel && worker.is_some() {
            out.push_str(&format!("{indent}  end\n"));
        }
    }

    // Verify node.
    if let Some(v) = &g.verify {
        let label = verify_label(v);
        out.push_str(&format!("{indent}  verify[\"{label}\"]\n"));
    }

    // Edges.
    for e in &g.edges {
        let arrow = match e.kind {
            EdgeKind::Sequential => format!("{} --> {}", e.from, e.to),
            EdgeKind::BatchAgent => format!("{} -. agent .-> {}", e.from, e.to),
            EdgeKind::BatchPlanner => format!("{} -. planner .-> {}", e.from, e.to),
            EdgeKind::Fallback => format!("{} == fallback ==> {}", e.from, e.to),
            EdgeKind::Rollback => {
                let ann = e.annotation.as_deref().unwrap_or("rollback");
                format!("{} -.{ann}.-> {}", e.from, e.to)
            }
            EdgeKind::Verify => format!("{} --> {}", e.from, e.to),
        };
        out.push_str(&format!("{indent}  {arrow}\n"));
    }

    // Class assignments.
    for n in &g.nodes {
        out.push_str(&format!("{indent}  class {} {};\n", n.id, n.outcome.mermaid_class()));
    }
    if let Some(v) = &g.verify {
        let cls = match &v.outcome {
            VerifyOutcome::Passed { .. } => "verify-passed",
            VerifyOutcome::Failed { .. } | VerifyOutcome::TimedOut { .. } => "verify-failed",
            VerifyOutcome::Skipped { .. } => "verify-skipped",
        };
        out.push_str(&format!("{indent}  class verify {cls};\n"));
    }
}

fn mermaid_label(g: &LoopGraph, n: &Node) -> String {
    let glyph = n.outcome.glyph();
    let dur = n.duration_secs.map(|s| format!("{s}s")).unwrap_or_else(|| "-".into());
    let task = n.task_id.as_deref().unwrap_or("-");
    let mut s = format!("iter-{}<br/>{}<br/>{} {} {}t", n.iter_number, task, glyph, dur, n.tokens);
    if n.stderr_hung {
        s.push_str("<br/>(no stderr — possibly hung)");
    }
    if matches!(n.outcome, OutcomeGlyph::Cancelled)
        && let Some(ts) = g.cancelled_at.as_deref()
    {
        s.push_str(&format!("<br/>cancelled @ {ts}"));
    }
    s
}

fn verify_label(v: &VerifyNode) -> String {
    let head = match &v.outcome {
        VerifyOutcome::Passed { .. } => "Verify ✓",
        VerifyOutcome::Failed { .. } => "Verify ✗",
        VerifyOutcome::Skipped { .. } => "Verify (skipped)",
        VerifyOutcome::TimedOut { .. } => "Verify ⏱",
    };
    let mut s = head.to_string();
    if !v.failure_lines.is_empty() {
        s.push_str("<br/>");
        for (i, l) in v.failure_lines.iter().enumerate() {
            if i > 0 {
                s.push_str("<br/>");
            }
            // Escape any embedded quotes for the mermaid string.
            s.push_str(&l.replace('"', "'"));
        }
    }
    s
}

fn render_dot(g: &LoopGraph) -> String {
    let mut out = String::new();
    out.push_str("digraph loop {\n");
    out.push_str("  rankdir=TB;\n");
    out.push_str("  node [shape=box, fontname=\"Helvetica\"];\n");
    render_dot_body(&mut out, g, "");
    out.push_str("}\n");
    out
}

fn render_all_dot(graphs: &[LoopGraph]) -> String {
    let mut out = String::new();
    out.push_str("digraph loop_all {\n");
    out.push_str("  rankdir=TB;\n");
    out.push_str("  node [shape=box, fontname=\"Helvetica\"];\n");
    for g in graphs {
        let safe = sanitize_id(&g.run_id);
        out.push_str(&format!("  subgraph cluster_{safe} {{\n"));
        out.push_str(&format!("    label=\"{}\";\n", g.run_id));
        render_dot_body(&mut out, g, "  ");
        out.push_str("  }\n");
    }
    out.push_str("}\n");
    out
}

fn render_dot_body(out: &mut String, g: &LoopGraph, indent: &str) {
    let nodes_by_worker = group_nodes_by_worker(g);
    let parallel = g.workers.len() >= 2;
    for (worker, nodes) in &nodes_by_worker {
        if parallel && let Some(w) = worker {
            out.push_str(&format!("{indent}  subgraph cluster_worker_{w} {{\n"));
            out.push_str(&format!("{indent}    label=\"worker-{w}\";\n"));
        }
        for n in nodes {
            let label = dot_label(g, n);
            let style =
                if matches!(n.outcome, OutcomeGlyph::Incomplete) { ", style=dashed" } else { "" };
            out.push_str(&format!(
                "{indent}    {} [label=\"{}\", color={}{}];\n",
                n.id,
                label,
                n.outcome.dot_color(),
                style
            ));
        }
        if parallel && worker.is_some() {
            out.push_str(&format!("{indent}  }}\n"));
        }
    }
    if let Some(v) = &g.verify {
        let color = match &v.outcome {
            VerifyOutcome::Passed { .. } => "green",
            VerifyOutcome::Failed { .. } | VerifyOutcome::TimedOut { .. } => "red",
            VerifyOutcome::Skipped { .. } => "gray",
        };
        let label = verify_label(v).replace("<br/>", "\\n");
        out.push_str(&format!("{indent}  verify [label=\"{label}\", color={color}];\n"));
    }
    for e in &g.edges {
        let style = match e.kind {
            EdgeKind::Sequential | EdgeKind::Verify => "",
            EdgeKind::BatchAgent => " [style=dotted, label=\"agent\"]",
            EdgeKind::BatchPlanner => " [style=dotted, label=\"planner\"]",
            EdgeKind::Fallback => " [style=bold, label=\"fallback\"]",
            EdgeKind::Rollback => " [style=dashed, label=\"rolled back\"]",
        };
        out.push_str(&format!("{indent}  {} -> {}{};\n", e.from, e.to, style));
    }
}

fn dot_label(g: &LoopGraph, n: &Node) -> String {
    let glyph = n.outcome.glyph();
    let dur = n.duration_secs.map(|s| format!("{s}s")).unwrap_or_else(|| "-".into());
    let task = n.task_id.as_deref().unwrap_or("-");
    let mut s = format!("iter-{}\\n{}\\n{} {} {}t", n.iter_number, task, glyph, dur, n.tokens);
    if n.stderr_hung {
        s.push_str("\\n(no stderr — possibly hung)");
    }
    if matches!(n.outcome, OutcomeGlyph::Cancelled)
        && let Some(ts) = g.cancelled_at.as_deref()
    {
        s.push_str(&format!("\\ncancelled @ {ts}"));
    }
    s
}

fn render_ascii(g: &LoopGraph) -> String {
    let mut out = String::new();
    out.push_str(&format!("run: {}\n", g.run_id));
    render_ascii_body(&mut out, g);
    out
}

fn render_all_ascii(graphs: &[LoopGraph]) -> String {
    let mut out = String::new();
    for (i, g) in graphs.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("=== {} ===\n", g.run_id));
        render_ascii_body(&mut out, g);
    }
    out
}

fn render_ascii_body(out: &mut String, g: &LoopGraph) {
    if g.nodes.is_empty() {
        out.push_str("  (no iters)\n");
    }
    let nodes_by_worker = group_nodes_by_worker(g);
    let parallel = g.workers.len() >= 2;
    for (worker, nodes) in &nodes_by_worker {
        if parallel && let Some(w) = worker {
            out.push_str(&format!("worker-{w}:\n"));
        }
        for (i, n) in nodes.iter().enumerate() {
            let task = n.task_id.as_deref().unwrap_or("-");
            let dur = n.duration_secs.map(|s| format!("{s}s")).unwrap_or_else(|| "-".into());
            let glyph = n.outcome.ascii_glyph();
            let mut line = format!(
                "  [{:>2}] iter-{:>3} {} task={} {} {}t",
                glyph, n.iter_number, glyph, task, dur, n.tokens
            );
            if n.stderr_hung {
                line.push_str("  (no stderr — possibly hung)");
            }
            if matches!(n.outcome, OutcomeGlyph::Cancelled)
                && let Some(ts) = g.cancelled_at.as_deref()
            {
                line.push_str(&format!("  cancelled @ {ts}"));
            }
            out.push_str(&line);
            out.push('\n');
            if i + 1 < nodes.len() {
                let edge = edge_between(g, &nodes[i].id, &nodes[i + 1].id);
                out.push_str(&format!("    {}\n", ascii_edge_label(edge)));
            }
        }
    }
    if let Some(v) = &g.verify {
        let head = match &v.outcome {
            VerifyOutcome::Passed { .. } => "verify: OK",
            VerifyOutcome::Failed { .. } => "verify: FAIL",
            VerifyOutcome::Skipped { .. } => "verify: skipped",
            VerifyOutcome::TimedOut { .. } => "verify: timed out",
        };
        out.push_str(&format!("  {head}\n"));
        for l in &v.failure_lines {
            out.push_str(&format!("    {l}\n"));
        }
    }
}

fn edge_between(g: &LoopGraph, from: &str, to: &str) -> Option<EdgeKind> {
    g.edges
        .iter()
        .find(|e| e.from == from && e.to == to && !matches!(e.kind, EdgeKind::Rollback))
        .map(|e| e.kind)
}

fn ascii_edge_label(kind: Option<EdgeKind>) -> &'static str {
    match kind {
        Some(EdgeKind::BatchAgent) => "|  (agent)",
        Some(EdgeKind::BatchPlanner) => "|  (planner)",
        Some(EdgeKind::Fallback) => "|| (fallback)",
        Some(EdgeKind::Rollback) => "↺  (rolled back)",
        Some(EdgeKind::Verify) => "|",
        _ => "|",
    }
}

fn group_nodes_by_worker(g: &LoopGraph) -> Vec<(Option<u32>, Vec<&Node>)> {
    // Preserve workers order from g.workers; ungrouped (worker_n=None)
    // nodes come first when present.
    let mut buckets: BTreeMap<Option<u32>, Vec<&Node>> = BTreeMap::new();
    for n in &g.nodes {
        buckets.entry(n.worker_n).or_default().push(n);
    }
    let mut out: Vec<(Option<u32>, Vec<&Node>)> = Vec::new();
    if let Some(nodes) = buckets.remove(&None) {
        out.push((None, nodes));
    }
    for w in &g.workers {
        if let Some(nodes) = buckets.remove(&Some(*w)) {
            out.push((Some(*w), nodes));
        }
    }
    out
}

fn sanitize_id(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch_plan::{BatchPlan, SCHEMA_VERSION};
    use crate::loop_log::{ManifestWorker, write_json_atomic};
    use crate::runner::TokenSpend;
    use std::path::PathBuf;

    fn tmpdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "hew-loop-graph-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default(),
            COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn write_iter(
        dir: &Path,
        n: u32,
        task: &str,
        outcome: &str,
        started: &str,
        ended: Option<&str>,
    ) {
        let log = IterLog {
            number: n,
            task_id: Some(task.into()),
            started_at: started.into(),
            ended_at: ended.map(str::to_string),
            outcome: Some(outcome.into()),
            prompt_prefix_hash: None,
            cost: TokenSpend { input: 100, output: 50, cache_read: 0, cache_create: 0 },
            decisions: Vec::new(),
            deferred: Vec::new(),
            tool_calls: Vec::new(),
            stderr_tail: None,
            symbols_touched: Vec::new(),
            runtime_used: None,
            cooldown_engaged: false,
            model: None,
        };
        write_json_atomic(&dir.join(format!("iter-{n:03}.json")), &log).unwrap();
    }

    fn write_run(dir: &Path, id: &str, stop_reason: Option<&str>, verify: Option<VerifyOutcome>) {
        let mut rl = RunLog {
            id: id.into(),
            started_at: "2026-05-30T00:00:00Z".into(),
            last_updated_at: "2026-05-30T00:01:00Z".into(),
            iter_count: 3,
            cumulative_tokens: 450,
            stop_reason: stop_reason.map(str::to_string),
            max_iter: None,
            strict: false,
            interactive: false,
            scope: None,
            verify_outcome: verify,
        };
        // suppress unused-field warnings; serialization carries everything.
        rl.last_updated_at = "2026-05-30T00:01:00Z".into();
        write_json_atomic(&dir.join("run.json"), &rl).unwrap();
    }

    #[test]
    fn graph_renders_simple_3_iter_run_as_mermaid() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        write_iter(
            &dir,
            2,
            "hew-b",
            "closed",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_iter(
            &dir,
            3,
            "hew-c",
            "closed",
            "2026-05-30T00:00:20Z",
            Some("2026-05-30T00:00:30Z"),
        );
        write_run(&dir, "loop-simple", Some("ready_empty"), None);

        let g = build_from_run_dir(&dir).unwrap();
        let out = render(&g, Format::Mermaid);
        assert!(out.starts_with("flowchart TD\n"));
        assert!(out.contains("iter1[\"iter-1<br/>hew-a<br/>✓ 10s 150t\"]"));
        assert!(out.contains("iter1 --> iter2"));
        assert!(out.contains("iter2 --> iter3"));
        assert!(out.contains("class iter1 iter-closed;"));
    }

    #[test]
    fn graph_renders_same_run_as_dot() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        write_iter(
            &dir,
            2,
            "hew-b",
            "closed",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_run(&dir, "loop-dot", Some("ready_empty"), None);

        let g = build_from_run_dir(&dir).unwrap();
        let out = render(&g, Format::Dot);
        assert!(out.starts_with("digraph loop {\n"));
        assert!(out.contains("iter1 [label=\"iter-1\\nhew-a\\n✓ 10s 150t\", color=green];"));
        assert!(out.contains("iter1 -> iter2;"));
        assert!(out.ends_with("}\n"));
    }

    #[test]
    fn graph_handles_incomplete_iter_with_dashed_border() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        // iter-2 started but never ended (no ended_at).
        write_iter(&dir, 2, "hew-b", "closed", "2026-05-30T00:00:10Z", None);
        write_run(&dir, "loop-incomplete", None, None);

        let g = build_from_run_dir(&dir).unwrap();
        let mermaid = render(&g, Format::Mermaid);
        assert!(mermaid.contains("iter2[/\""));
        assert!(mermaid.contains("⋯"));
        assert!(mermaid.contains("class iter2 iter-incomplete;"));

        let dot = render(&g, Format::Dot);
        assert!(dot.contains("style=dashed"));
    }

    #[test]
    fn graph_handles_cancelled_run_with_annotation() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        // iter-2 was running when .stop fired — no ended_at AND run.stop_reason=cancelled.
        write_iter(&dir, 2, "hew-b", "closed", "2026-05-30T00:00:10Z", None);
        write_run(&dir, "loop-cancel", Some("cancelled"), None);

        let g = build_from_run_dir(&dir).unwrap();
        // The in-flight iter must classify as Cancelled (not Incomplete).
        let last = g.nodes.last().unwrap();
        assert_eq!(last.outcome, OutcomeGlyph::Cancelled);
        let out = render(&g, Format::Mermaid);
        assert!(out.contains("⊘"));
        assert!(out.contains("cancelled @"));
    }

    #[test]
    fn graph_renders_batch_source_edges_distinctly() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        write_iter(
            &dir,
            2,
            "hew-b",
            "closed",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_iter(
            &dir,
            3,
            "hew-c",
            "closed",
            "2026-05-30T00:00:20Z",
            Some("2026-05-30T00:00:30Z"),
        );
        write_iter(
            &dir,
            4,
            "hew-d",
            "closed",
            "2026-05-30T00:00:30Z",
            Some("2026-05-30T00:00:40Z"),
        );
        write_run(&dir, "loop-batches", Some("ready_empty"), None);
        // iter-2 was chosen by previous iter's agent emit
        crate::batch_plan::write(
            &dir,
            &BatchPlan {
                schema_version: SCHEMA_VERSION,
                iter_number: 2,
                task_ids: vec!["hew-b".into()],
                source: BatchSource::Agent,
                reason: None,
                created_at: "2026-05-30T00:00:09Z".into(),
                planner_tokens: None,
            },
        )
        .unwrap();
        // iter-3 was chosen by the planner subprocess
        crate::batch_plan::write(
            &dir,
            &BatchPlan {
                schema_version: SCHEMA_VERSION,
                iter_number: 3,
                task_ids: vec!["hew-c".into()],
                source: BatchSource::Planner,
                reason: None,
                created_at: "2026-05-30T00:00:19Z".into(),
                planner_tokens: None,
            },
        )
        .unwrap();
        // iter-4 fell back to trust-the-graph
        crate::batch_plan::write(
            &dir,
            &BatchPlan {
                schema_version: SCHEMA_VERSION,
                iter_number: 4,
                task_ids: Vec::new(),
                source: BatchSource::Skipped,
                reason: Some("planner_disabled".into()),
                created_at: "2026-05-30T00:00:29Z".into(),
                planner_tokens: None,
            },
        )
        .unwrap();

        let g = build_from_run_dir(&dir).unwrap();
        let out = render(&g, Format::Mermaid);
        assert!(out.contains("iter1 -. agent .-> iter2"), "got: {out}");
        assert!(out.contains("iter2 -. planner .-> iter3"));
        assert!(out.contains("iter3 == fallback ==> iter4"));
    }

    #[test]
    fn graph_renders_worker_swimlanes_for_parallel_run() {
        let dir = tmpdir();
        let w0 = dir.join("worker-0");
        let w1 = dir.join("worker-1");
        std::fs::create_dir_all(&w0).unwrap();
        std::fs::create_dir_all(&w1).unwrap();
        write_iter(&w0, 1, "hew-a", "closed", "2026-05-30T00:00:00Z", Some("2026-05-30T00:00:10Z"));
        write_iter(&w0, 2, "hew-b", "closed", "2026-05-30T00:00:10Z", Some("2026-05-30T00:00:20Z"));
        write_iter(&w1, 1, "hew-c", "closed", "2026-05-30T00:00:00Z", Some("2026-05-30T00:00:15Z"));
        let manifest = Manifest {
            run_id: "loop-par".into(),
            jobs: 2,
            started_at: "2026-05-30T00:00:00Z".into(),
            completed_at: "2026-05-30T00:00:20Z".into(),
            workers: vec![
                ManifestWorker {
                    id: 0,
                    branch: "loop/par/w0".into(),
                    log_subdir: Some("worker-0".into()),
                    iter_count: 2,
                    cumulative_tokens: 300,
                    stop_reason: Some("ready_empty".into()),
                },
                ManifestWorker {
                    id: 1,
                    branch: "loop/par/w1".into(),
                    log_subdir: Some("worker-1".into()),
                    iter_count: 1,
                    cumulative_tokens: 150,
                    stop_reason: Some("ready_empty".into()),
                },
            ],
        };
        crate::loop_log::write_manifest(&dir, &manifest).unwrap();
        write_run(&dir, "loop-par", Some("ready_empty"), None);

        let g = build_from_run_dir(&dir).unwrap();
        assert_eq!(g.workers, vec![0, 1]);
        let out = render(&g, Format::Mermaid);
        assert!(out.contains("subgraph worker-0"));
        assert!(out.contains("subgraph worker-1"));
        assert!(out.contains("w0_iter1"));
        assert!(out.contains("w1_iter1"));
    }

    #[test]
    fn graph_renders_backpressure_rollback_edge_with_target() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        write_iter(
            &dir,
            2,
            "hew-b",
            "backpressure_fail",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_run(&dir, "loop-bp", Some("guard_trip"), None);

        let g = build_from_run_dir(&dir).unwrap();
        let bp = g.nodes.last().unwrap();
        assert_eq!(bp.outcome, OutcomeGlyph::BackpressureFail);
        let out = render(&g, Format::Mermaid);
        assert!(out.contains("↺") || out.contains("iter-backpressure"));
        // Rollback self-edge from bp back to previous iter.
        assert!(out.contains("iter2 -.rolled back.-> iter1"));
    }

    #[test]
    fn graph_renders_verify_node_passed_failed_skipped() {
        for (out_outcome, expected_class, expected_glyph) in [
            (
                VerifyOutcome::Passed { command: "cargo test".into(), duration_secs: 12 },
                "verify-passed",
                "Verify ✓",
            ),
            (
                VerifyOutcome::Failed {
                    command: "cargo test".into(),
                    exit_code: 1,
                    duration_secs: 22,
                    stderr_tail: "boom".into(),
                },
                "verify-failed",
                "Verify ✗",
            ),
            (
                VerifyOutcome::Skipped { reason: "no test cmd".into() },
                "verify-skipped",
                "Verify (skipped)",
            ),
        ] {
            let dir = tmpdir();
            write_iter(
                &dir,
                1,
                "hew-a",
                "closed",
                "2026-05-30T00:00:00Z",
                Some("2026-05-30T00:00:10Z"),
            );
            write_run(&dir, "loop-verify", Some("ready_empty"), Some(out_outcome));
            let g = build_from_run_dir(&dir).unwrap();
            let out = render(&g, Format::Mermaid);
            assert!(out.contains(expected_class), "missing {expected_class} in: {out}");
            assert!(out.contains(expected_glyph), "missing {expected_glyph} in: {out}");
        }
    }

    #[test]
    fn graph_renders_runtime_error_with_hung_annotation() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        // runtime_error with no stderr_tail set → "possibly hung"
        write_iter(
            &dir,
            2,
            "hew-b",
            "runtime_error",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_run(&dir, "loop-hung", Some("runtime_error"), None);

        let g = build_from_run_dir(&dir).unwrap();
        assert!(g.nodes[1].stderr_hung);
        let out = render(&g, Format::Mermaid);
        assert!(out.contains("no stderr — possibly hung"));
    }

    #[test]
    fn graph_all_mode_renders_each_run_as_subgraph() {
        let root = tmpdir();
        let loop_root = root.join(".hew/loop");
        std::fs::create_dir_all(&loop_root).unwrap();
        for id in ["loop-aaa", "loop-bbb"] {
            let d = loop_root.join(id);
            std::fs::create_dir_all(&d).unwrap();
            write_iter(
                &d,
                1,
                "hew-x",
                "closed",
                "2026-05-30T00:00:00Z",
                Some("2026-05-30T00:00:10Z"),
            );
            write_run(&d, id, Some("ready_empty"), None);
        }
        let graphs = build_from_loop_root(&loop_root).unwrap();
        assert_eq!(graphs.len(), 2);
        let out = render_all(&graphs, Format::Mermaid);
        assert!(out.contains("subgraph loop_aaa"));
        assert!(out.contains("subgraph loop_bbb"));
    }

    #[test]
    fn graph_handles_pre_batchplan_legacy_run() {
        // No batch-*.json files written; edges should be Sequential
        // (no agent/planner/fallback styling).
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        write_iter(
            &dir,
            2,
            "hew-b",
            "closed",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_run(&dir, "loop-legacy", Some("ready_empty"), None);

        let g = build_from_run_dir(&dir).unwrap();
        assert_eq!(g.edges.len(), 1);
        assert!(matches!(g.edges[0].kind, EdgeKind::Sequential));
        let out = render(&g, Format::Mermaid);
        assert!(!out.contains("agent"));
        assert!(!out.contains("planner"));
        assert!(!out.contains("fallback"));
        assert!(out.contains("iter1 --> iter2"));
    }

    #[test]
    fn ascii_renderer_produces_terminal_friendly_output() {
        let dir = tmpdir();
        write_iter(
            &dir,
            1,
            "hew-a",
            "closed",
            "2026-05-30T00:00:00Z",
            Some("2026-05-30T00:00:10Z"),
        );
        write_iter(
            &dir,
            2,
            "hew-b",
            "closed",
            "2026-05-30T00:00:10Z",
            Some("2026-05-30T00:00:20Z"),
        );
        write_run(&dir, "loop-ascii", Some("ready_empty"), None);

        let g = build_from_run_dir(&dir).unwrap();
        let out = render(&g, Format::Ascii);
        assert!(out.starts_with("run: loop-ascii\n"));
        // No unicode in the ASCII renderer.
        assert!(out.is_ascii(), "ascii output contained non-ascii: {out}");
    }

    #[test]
    fn parse_iso_handles_canonical_format() {
        assert_eq!(parse_iso("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso("1970-01-01T00:00:10Z"), Some(10));
        assert_eq!(parse_iso("2026-05-30T00:00:00Z"), parse_iso("2026-05-30T00:00:00Z"));
        // Difference of one day:
        let a = parse_iso("2026-05-30T00:00:00Z").unwrap();
        let b = parse_iso("2026-05-31T00:00:00Z").unwrap();
        assert_eq!(b - a, 86_400);
    }
}
