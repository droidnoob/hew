//! `hew epic <verb>` — epic-shaped queries over [`hew_core::tasks`].
//!
//! - `list`     lists epics (open by default; `--all` includes closed).
//! - `show`     fetches the epic + first-level children.
//! - `tree`     recursively walks parent-child (transitive children).
//! - `close`    refuses if any child is still open (unless `--force`).
//! - `audit`    flags children with thin close reasons.
//! - `summary`  one-line-per-child readout.

use clap::{Args as ClapArgs, Subcommand};
use hew_core::bd::{BdClient, RealBd};
use hew_core::tasks::{self, EpicSummary, TaskListFilter, TaskSummary};
use hew_core::{Ctx, OutputMode};
use serde::Serialize;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// List epics. Open-only by default; pass `--all` to include closed.
    List(ListArgs),
    /// Show epic body + first-level children.
    Show(ShowArgs),
    /// Walk parent-child transitively.
    Tree(TreeArgs),
    /// Close an epic. Refuses if any child is still open unless `--force`.
    Close(CloseArgs),
    /// Audit children for thin close reasons / missing deviation tags.
    Audit(AuditArgs),
    /// One-line-per-child readout for stand-ups.
    Summary(IdArgs),
}

#[derive(Debug, ClapArgs)]
pub struct ListArgs {
    /// Include closed epics in the listing.
    #[arg(long)]
    pub all: bool,
    /// Only list closed epics (overrides `--all`).
    #[arg(long, conflicts_with = "all")]
    pub closed: bool,
    /// Cap on rows (`0` = unlimited).
    #[arg(long, default_value_t = 0)]
    pub n: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct ShowArgs {
    pub id: String,
    /// Cap on children listed in text mode (`0` = unlimited).
    #[arg(long, default_value_t = 50)]
    pub n: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct TreeArgs {
    pub id: String,
    /// `0` = unlimited.
    #[arg(long, default_value_t = 3)]
    pub depth: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct CloseArgs {
    pub id: String,
    #[arg(long)]
    pub reason: Option<String>,
    /// Close even if children are still open.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, ClapArgs)]
pub struct AuditArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct IdArgs {
    pub id: String,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    match args.op {
        Op::List(a) => list(ctx, &bd, a),
        Op::Show(a) => show(ctx, &bd, a),
        Op::Tree(a) => tree(ctx, &bd, a),
        Op::Close(a) => close(ctx, &bd, a),
        Op::Audit(a) => audit(ctx, &bd, a),
        Op::Summary(a) => summary(ctx, &bd, a),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// list
// ────────────────────────────────────────────────────────────────────────────

fn list(ctx: &Ctx, bd: &dyn BdClient, args: ListArgs) -> miette::Result<()> {
    let status = if args.closed {
        vec!["closed".into()]
    } else if args.all {
        Vec::new()
    } else {
        vec!["open".into(), "in_progress".into(), "blocked".into(), "deferred".into()]
    };
    let filter =
        TaskListFilter { status, issue_type: Some("epic".into()), n: args.n, ..Default::default() };
    let epics = tasks::list(bd, &filter)?;

    if wants_json(ctx, args.json) {
        emit_json(&epics)?;
        return Ok(());
    }

    if epics.is_empty() {
        if !ctx.quiet {
            println!("(no epics)");
        }
        return Ok(());
    }

    for e in &epics {
        let status = display_status(e);
        println!(
            "  {}  {:<14} P{}  {:<8}  {}",
            status_marker(status),
            e.id,
            e.priority,
            status,
            e.title,
        );
    }
    Ok(())
}

fn display_status(t: &TaskSummary) -> &str {
    if t.status.is_empty() && !t.closed_at.is_empty() {
        "closed"
    } else if t.status.is_empty() {
        "open"
    } else {
        t.status.as_str()
    }
}

// ────────────────────────────────────────────────────────────────────────────
// show / tree
// ────────────────────────────────────────────────────────────────────────────

fn show(ctx: &Ctx, bd: &dyn BdClient, args: ShowArgs) -> miette::Result<()> {
    let e = tasks::show_epic(bd, &args.id)?;
    if wants_json(ctx, args.json) {
        emit_json(&e)?;
        return Ok(());
    }

    println!("{} — {}", e.id, e.title);
    if !e.status.is_empty() {
        println!("  status:       {}", e.status);
    }
    println!("  child_count:  {}", e.child_count);
    if !e.body.trim().is_empty() {
        println!();
        for line in e.body.lines() {
            println!("  {line}");
        }
    }
    println!();
    println!("children:");
    let cap = if args.n == 0 { e.children.len() } else { (args.n as usize).min(e.children.len()) };
    for c in e.children.iter().take(cap) {
        print_child(c);
    }
    if cap < e.children.len() {
        println!("  … {} more (pass --n 0 to list all)", e.children.len() - cap);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct TreeNode {
    id: String,
    title: String,
    status: String,
    issue_type: String,
    children: Vec<TreeNode>,
}

fn tree(ctx: &Ctx, bd: &dyn BdClient, args: TreeArgs) -> miette::Result<()> {
    // Root node needs an explicit show — the parent's children() call
    // doesn't exist for the very top of the tree. Every level below
    // reuses TaskSummary objects returned by children() so each node
    // costs exactly one bd subprocess invocation (the children query),
    // not two (show + children).
    //
    // Before: O(2N) bd subprocess calls for N nodes. ~5s on a 55-task
    // graph.  After: O(N) calls — see hew-ara / GH issue #21.
    let summary = tasks::show(bd, &args.id)?;
    let root = build_tree_from_summary(bd, &summary, 0, args.depth)?;
    if wants_json(ctx, args.json) {
        emit_json(&root)?;
        return Ok(());
    }
    render_tree_text(&root, "");
    Ok(())
}

fn build_tree_from_summary(
    bd: &dyn BdClient,
    summary: &TaskSummary,
    depth: u32,
    max_depth: u32,
) -> miette::Result<TreeNode> {
    let at_max = max_depth != 0 && depth >= max_depth.saturating_sub(1);
    // Leaf-skip heuristic: non-epic, non-milestone tasks rarely parent
    // anything. Skipping `children()` on them avoids one bd subprocess
    // per leaf — the bulk of the remaining latency on wide epics.
    // If a leaf does have unexpected children, they'll show up the next
    // time the user runs `hew task children <id>` directly.
    let can_have_children = matches!(summary.issue_type.as_str(), "epic" | "milestone" | "");
    let children = if at_max || !can_have_children {
        Vec::new()
    } else {
        let kids = tasks::children(bd, &summary.id)?;
        let mut out = Vec::with_capacity(kids.len());
        for k in kids {
            out.push(build_tree_from_summary(bd, &k, depth + 1, max_depth)?);
        }
        out
    };
    Ok(TreeNode {
        id: summary.id.clone(),
        title: summary.title.clone(),
        status: summary.status.clone(),
        issue_type: summary.issue_type.clone(),
        children,
    })
}

fn render_tree_text(node: &TreeNode, prefix: &str) {
    let marker = status_marker(&node.status);
    println!("{prefix}{marker} {}  {}", node.id, node.title);
    let next = format!("{prefix}  ");
    for c in &node.children {
        render_tree_text(c, &next);
    }
}

// ────────────────────────────────────────────────────────────────────────────
// close / audit / summary
// ────────────────────────────────────────────────────────────────────────────

fn close(ctx: &Ctx, bd: &dyn BdClient, args: CloseArgs) -> miette::Result<()> {
    let kids = tasks::children(bd, &args.id)?;
    let still_open: Vec<&TaskSummary> = kids.iter().filter(|c| !is_closed(c)).collect();
    if !still_open.is_empty() && !args.force {
        let mut msg =
            format!("cannot close {}: {} child task(s) still open", args.id, still_open.len());
        for c in &still_open {
            msg.push_str(&format!("\n  - {} ({}) {}", c.id, c.status, c.title));
        }
        msg.push_str("\nre-run with --force to close anyway.");
        return Err(miette::miette!("{msg}"));
    }
    let reason = args.reason.clone().unwrap_or_else(|| "epic complete".to_string());
    tasks::close_with_reason(bd, &args.id, &reason)?;
    if !ctx.quiet {
        println!("closed {} — {reason}", args.id);
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct AuditFinding {
    id: String,
    title: String,
    status: String,
    issues: Vec<String>,
}

fn audit(ctx: &Ctx, bd: &dyn BdClient, args: AuditArgs) -> miette::Result<()> {
    let kids = tasks::children(bd, &args.id)?;
    let findings: Vec<AuditFinding> = kids
        .iter()
        .map(|c| AuditFinding {
            id: c.id.clone(),
            title: c.title.clone(),
            status: if c.status.is_empty() && !c.closed_at.is_empty() {
                "closed".into()
            } else {
                c.status.clone()
            },
            issues: collect_audit_issues(c),
        })
        .collect();

    if wants_json(ctx, args.json) {
        emit_json(&findings)?;
        return Ok(());
    }

    let bad: Vec<&AuditFinding> = findings.iter().filter(|f| !f.issues.is_empty()).collect();
    if bad.is_empty() {
        println!("audit: {} child(ren) clean", findings.len());
        return Ok(());
    }
    println!("audit: {} of {} child(ren) flagged", bad.len(), findings.len());
    for f in bad {
        println!("  {} {} — {}", status_marker(&f.status), f.id, f.title);
        for i in &f.issues {
            println!("      • {i}");
        }
    }
    Ok(())
}

fn collect_audit_issues(c: &TaskSummary) -> Vec<String> {
    let mut out = Vec::new();
    if !is_closed(c) {
        return out;
    }
    let reason = c.close_reason.as_deref().unwrap_or("").trim();
    if reason.is_empty() {
        out.push("missing close_reason".into());
    } else if is_thin_reason(reason) {
        out.push(format!("thin close_reason: {reason:?}"));
    }
    out
}

fn is_thin_reason(reason: &str) -> bool {
    const THIN: &[&str] = &["done", "shipped", "complete", "ok", "fixed", "yes"];
    let lower = reason.to_ascii_lowercase();
    let trimmed = lower.trim().trim_end_matches('.');
    THIN.iter().any(|t| &trimmed == t)
}

fn summary(_ctx: &Ctx, bd: &dyn BdClient, args: IdArgs) -> miette::Result<()> {
    let kids = tasks::children(bd, &args.id)?;
    if kids.is_empty() {
        println!("(no children under {})", args.id);
        return Ok(());
    }
    for c in &kids {
        let marker = if is_closed(c) { "✓" } else { "○" };
        println!("{marker} {} {}", c.id, c.title);
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

fn wants_json(ctx: &Ctx, local: bool) -> bool {
    local || matches!(ctx.output, OutputMode::Json)
}

fn emit_json<T: Serialize>(value: &T) -> miette::Result<()> {
    let s = serde_json::to_string_pretty(value).map_err(|e| miette::miette!("json: {e}"))?;
    println!("{s}");
    Ok(())
}

fn is_closed(t: &TaskSummary) -> bool {
    t.status == "closed" || !t.closed_at.is_empty()
}

fn status_marker(status: &str) -> &'static str {
    match status {
        "closed" => "✓",
        "in_progress" => "◐",
        "blocked" => "●",
        "deferred" => "❄",
        "" => "·",
        _ => "○",
    }
}

fn print_child(c: &TaskSummary) {
    let status = if c.status.is_empty() && !c.closed_at.is_empty() {
        "closed"
    } else if c.status.is_empty() {
        "open"
    } else {
        c.status.as_str()
    };
    println!(
        "  {}  {:<14} P{}  {:<6}  {}",
        status_marker(status),
        c.id,
        c.priority,
        if c.issue_type.is_empty() { "?".into() } else { c.issue_type.clone() },
        c.title,
    );
}

// Silence unused-import diagnostics on minimal feature configurations.
#[allow(dead_code)]
fn _unused_marker(_: &EpicSummary) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_reason_detection() {
        assert!(is_thin_reason("done"));
        assert!(is_thin_reason("Done."));
        assert!(is_thin_reason("shipped"));
        assert!(!is_thin_reason("shipped via abc123"));
        assert!(!is_thin_reason("Closed because the integration changed."));
    }

    fn list_status_filter(args: &ListArgs) -> Vec<String> {
        if args.closed {
            vec!["closed".into()]
        } else if args.all {
            Vec::new()
        } else {
            vec!["open".into(), "in_progress".into(), "blocked".into(), "deferred".into()]
        }
    }

    #[test]
    fn list_defaults_to_open_statuses() {
        let f = list_status_filter(&ListArgs { all: false, closed: false, n: 0, json: false });
        assert_eq!(f, vec!["open", "in_progress", "blocked", "deferred"]);
    }

    #[test]
    fn list_all_drops_status_filter() {
        let f = list_status_filter(&ListArgs { all: true, closed: false, n: 0, json: false });
        assert!(f.is_empty(), "--all should produce empty (no status filter)");
    }

    #[test]
    fn list_closed_only() {
        let f = list_status_filter(&ListArgs { all: false, closed: true, n: 0, json: false });
        assert_eq!(f, vec!["closed"]);
    }

    #[test]
    fn display_status_falls_back_when_blank() {
        let t = TaskSummary {
            id: "x".into(),
            title: "t".into(),
            issue_type: "epic".into(),
            priority: 1,
            status: String::new(),
            description: String::new(),
            closed_at: String::new(),
            close_reason: None,
            parent: None,
        };
        assert_eq!(display_status(&t), "open");

        let closed = TaskSummary { closed_at: "2026-05-13".into(), ..t.clone() };
        assert_eq!(display_status(&closed), "closed");

        let explicit = TaskSummary { status: "in_progress".into(), ..t };
        assert_eq!(display_status(&explicit), "in_progress");
    }
}
