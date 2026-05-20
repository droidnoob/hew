//! `hew task <verb>` — thin clap layer over [`hew_core::tasks`].
//!
//! Text output is the default; `--json` (or the global `--json` flag) opts
//! in to the schemars-derived JSON contract. Every verb routes through
//! `hew_core::tasks::*` so a future bd JSON-shape change touches one file.

use clap::{Args as ClapArgs, Subcommand};
use hew_core::bd::{BdClient, RealBd};
use hew_core::tasks::{self, NewTaskArgs, TaskListFilter, TaskSummary};
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Show a single task — text by default, `--json` opts in.
    Show(ShowArgs),
    /// List tasks with optional filters.
    List(ListArgs),
    /// Atomically claim a task (sets status=in_progress + assignee).
    Claim(IdArgs),
    /// Close a task with a one-line reason.
    Close(CloseArgs),
    /// Create a new task via `bd q` (returns the new id).
    New(NewArgs),
    /// Reopen a closed task.
    Reopen(ReopenArgs),
    /// List direct children of a parent (one level only).
    Children(ChildrenArgs),
    /// Append a note to a task.
    Note(NoteArgs),
    /// Search tasks by title / id prefix.
    Search(SearchArgs),
}

#[derive(Debug, ClapArgs)]
pub struct ShowArgs {
    /// Issue id (e.g. `hew-4az.1`).
    pub id: String,
    /// Emit `TaskSummary` JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct ListArgs {
    /// Comma-joined status filter (e.g. `open,in_progress`).
    #[arg(long)]
    pub status: Option<String>,
    /// Single-value type filter (e.g. `task`, `epic`, `bug`).
    #[arg(long = "type")]
    pub issue_type: Option<String>,
    /// Filter to children of this parent id.
    #[arg(long)]
    pub parent: Option<String>,
    /// Resolves to `bd list --closed-after`. Accepts a bd task id (uses
    /// its `closed_at`) or a date/timestamp passthrough.
    #[arg(long)]
    pub since: Option<String>,
    /// Max results (`0` = unlimited). Default 20.
    #[arg(long, default_value_t = 20)]
    pub n: u32,
    /// Flip to oldest-first (default is newest-first).
    #[arg(long)]
    pub head: bool,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct IdArgs {
    pub id: String,
}

#[derive(Debug, ClapArgs)]
pub struct CloseArgs {
    pub id: String,
    /// One-line close reason.
    #[arg(long)]
    pub reason: String,
    /// Optional deviation-rule tag (1-3); prepended as `[Rule N]` to the
    /// reason. See `/hew:execute` Step 10's deviation handling.
    #[arg(long = "type", value_parser = clap::value_parser!(u8).range(1..=3))]
    pub rule: Option<u8>,
    /// Bypass `bd`'s blocked-by-open-prereq check. Use when a dep edge
    /// shouldn't gate this close (e.g., over-conservative planner dep
    /// that didn't actually block the work). Still emits a regular
    /// close event; deviation type still recorded via `--type`.
    #[arg(long)]
    pub force: bool,
}

#[derive(Debug, ClapArgs)]
pub struct NewArgs {
    #[arg(long)]
    pub title: String,
    /// Defaults to `task` (bd q's default).
    #[arg(long = "type")]
    pub issue_type: Option<String>,
    /// Optional description; applied via a follow-up `bd update --description`.
    #[arg(long)]
    pub description: Option<String>,
    #[arg(long)]
    pub parent: Option<String>,
    /// Priority 0-4 (0 = highest). Defaults to bd q's default (2).
    #[arg(long, value_parser = clap::value_parser!(u8).range(0..=4))]
    pub priority: Option<u8>,
    /// Comma-separated labels.
    #[arg(long)]
    pub labels: Option<String>,
}

#[derive(Debug, ClapArgs)]
pub struct ReopenArgs {
    pub id: String,
    #[arg(long)]
    pub reason: Option<String>,
}

#[derive(Debug, ClapArgs)]
pub struct ChildrenArgs {
    pub id: String,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct NoteArgs {
    pub id: String,
    /// Note text. Multi-word values must be quoted at the shell.
    pub text: String,
}

#[derive(Debug, ClapArgs)]
pub struct SearchArgs {
    pub query: String,
    #[arg(long, default_value_t = 20)]
    pub n: u32,
    #[arg(long)]
    pub json: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    match args.op {
        Op::Show(a) => show(ctx, &bd, a),
        Op::List(a) => list(ctx, &bd, a),
        Op::Claim(a) => claim(ctx, &bd, a),
        Op::Close(a) => close(ctx, &bd, a),
        Op::New(a) => new_task(ctx, &bd, a),
        Op::Reopen(a) => reopen(ctx, &bd, a),
        Op::Children(a) => children(ctx, &bd, a),
        Op::Note(a) => note(ctx, &bd, a),
        Op::Search(a) => search(ctx, &bd, a),
    }
}

// ────────────────────────────────────────────────────────────────────────────

fn show(ctx: &Ctx, bd: &dyn BdClient, args: ShowArgs) -> miette::Result<()> {
    let t = tasks::show(bd, &args.id)?;
    if wants_json(ctx, args.json) {
        emit_json(&t)?;
    } else {
        print_task_long(&t);
    }
    Ok(())
}

fn list(ctx: &Ctx, bd: &dyn BdClient, args: ListArgs) -> miette::Result<()> {
    let since = match args.since.as_deref() {
        Some(raw) => Some(resolve_since(bd, raw)?),
        None => None,
    };
    let filter = TaskListFilter {
        status: args
            .status
            .as_deref()
            .map(|s| s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
            .unwrap_or_default(),
        issue_type: args.issue_type,
        parent: args.parent,
        since,
        n: args.n,
        newest_first: !args.head,
    };
    let items = tasks::list(bd, &filter)?;
    if wants_json(ctx, args.json) {
        emit_json(&items)?;
    } else if items.is_empty() {
        println!("(no tasks)");
    } else {
        for t in &items {
            print_task_row(t);
        }
        println!();
        println!("{} task(s)", items.len());
    }
    Ok(())
}

fn claim(ctx: &Ctx, bd: &dyn BdClient, args: IdArgs) -> miette::Result<()> {
    tasks::claim(bd, &args.id)?;
    if !ctx.quiet {
        let title = tasks::show(bd, &args.id).map(|t| t.title).unwrap_or_default();
        if title.is_empty() {
            println!("claimed {}", args.id);
        } else {
            println!("claimed {} — {}", args.id, title);
        }
    }
    Ok(())
}

fn close(ctx: &Ctx, bd: &dyn BdClient, args: CloseArgs) -> miette::Result<()> {
    let reason = match args.rule {
        Some(n) => format!("[Rule {n}] {}", args.reason),
        None => args.reason,
    };
    tasks::close_with_reason_force(bd, &args.id, &reason, args.force)?;
    if !ctx.quiet {
        let suffix = if args.force { " (forced)" } else { "" };
        println!("closed {}{suffix} — {reason}", args.id);
    }
    Ok(())
}

fn new_task(ctx: &Ctx, bd: &dyn BdClient, args: NewArgs) -> miette::Result<()> {
    let labels = args
        .labels
        .as_deref()
        .map(|s| s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    let id = tasks::new_task(
        bd,
        NewTaskArgs {
            title: args.title,
            issue_type: args.issue_type,
            priority: args.priority,
            labels,
            parent: args.parent,
        },
    )?;
    if let Some(desc) = args.description {
        // bd q can't set description; follow up with `bd update --description`.
        // Run through the raw escape hatch so we don't widen the tasks API
        // for a one-shot post-create patch.
        let id_os = std::ffi::OsString::from(&id);
        let desc_os = std::ffi::OsString::from(&desc);
        bd.run_raw(&[
            std::ffi::OsStr::new("update"),
            id_os.as_os_str(),
            std::ffi::OsStr::new("--description"),
            desc_os.as_os_str(),
        ])?;
    }
    if ctx.quiet {
        print!("{id}");
    } else {
        println!("{id}");
    }
    Ok(())
}

fn reopen(ctx: &Ctx, bd: &dyn BdClient, args: ReopenArgs) -> miette::Result<()> {
    tasks::reopen(bd, &args.id, args.reason.as_deref())?;
    if !ctx.quiet {
        println!("reopened {}", args.id);
    }
    Ok(())
}

fn children(ctx: &Ctx, bd: &dyn BdClient, args: ChildrenArgs) -> miette::Result<()> {
    let items = tasks::children(bd, &args.id)?;
    if wants_json(ctx, args.json) {
        emit_json(&items)?;
    } else if items.is_empty() {
        println!("(no children)");
    } else {
        for t in &items {
            print_task_row(t);
        }
    }
    Ok(())
}

fn note(ctx: &Ctx, bd: &dyn BdClient, args: NoteArgs) -> miette::Result<()> {
    tasks::note(bd, &args.id, &args.text)?;
    if !ctx.quiet {
        println!("note added to {}", args.id);
    }
    Ok(())
}

fn search(ctx: &Ctx, bd: &dyn BdClient, args: SearchArgs) -> miette::Result<()> {
    let mut items = tasks::search(bd, &args.query)?;
    if args.n > 0 {
        items.truncate(args.n as usize);
    }
    if wants_json(ctx, args.json) {
        emit_json(&items)?;
    } else if items.is_empty() {
        println!("(no matches)");
    } else {
        for t in &items {
            print_task_row(t);
        }
        println!();
        println!("{} match(es)", items.len());
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Rendering helpers
// ────────────────────────────────────────────────────────────────────────────

fn wants_json(ctx: &Ctx, local: bool) -> bool {
    local || matches!(ctx.output, OutputMode::Json)
}

fn emit_json<T: serde::Serialize>(value: &T) -> miette::Result<()> {
    let s = serde_json::to_string_pretty(value).map_err(|e| miette::miette!("json: {e}"))?;
    println!("{s}");
    Ok(())
}

fn print_task_long(t: &TaskSummary) {
    println!("{} — {}", t.id, t.title);
    println!("  type:     {}", display_or(&t.issue_type, "?"));
    println!("  priority: P{}", t.priority);
    println!("  status:   {}", display_or(&t.status, infer_status(t)));
    if let Some(p) = &t.parent {
        println!("  parent:   {p}");
    }
    if !t.closed_at.is_empty() {
        match &t.close_reason {
            Some(r) if !r.is_empty() => {
                println!("  closed:   {} (reason: {r})", t.closed_at)
            }
            _ => println!("  closed:   {}", t.closed_at),
        }
    }
    if !t.description.trim().is_empty() {
        println!();
        for line in t.description.lines() {
            println!("  {line}");
        }
    }
}

fn print_task_row(t: &TaskSummary) {
    let marker = status_marker(t);
    println!(
        "  {marker}  {:<14} P{}  {:<6}  {}",
        t.id,
        t.priority,
        if t.issue_type.is_empty() { "?".into() } else { t.issue_type.clone() },
        t.title,
    );
}

fn status_marker(t: &TaskSummary) -> &'static str {
    match infer_status(t) {
        "closed" => "✓",
        "in_progress" => "◐",
        "blocked" => "●",
        "deferred" => "❄",
        _ => "○",
    }
}

fn infer_status(t: &TaskSummary) -> &'static str {
    if !t.status.is_empty() {
        // Return owned variant via match — &str needs a borrowed lifetime.
        return match t.status.as_str() {
            "closed" => "closed",
            "in_progress" => "in_progress",
            "blocked" => "blocked",
            "deferred" => "deferred",
            _ => "open",
        };
    }
    if !t.closed_at.is_empty() { "closed" } else { "open" }
}

fn display_or<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

/// `--since` accepts a bd task id (resolved to its `closed_at`), or any
/// passthrough string that bd understands (`YYYY-MM-DD`, RFC3339, etc.).
/// Git-ref resolution is intentionally not implemented yet.
fn resolve_since(bd: &dyn BdClient, raw: &str) -> miette::Result<String> {
    if looks_like_bd_id(raw)
        && let Ok(t) = tasks::show(bd, raw)
        && !t.closed_at.is_empty()
    {
        return Ok(t.closed_at);
    }
    Ok(raw.to_string())
}

fn looks_like_bd_id(s: &str) -> bool {
    // `<prefix>-<token>(.<n>)*` where prefix is alphabetic and there's at
    // least one `-`. Avoids treating `2026-05-12` as an id.
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !first.is_ascii_alphabetic() {
        return false;
    }
    s.contains('-') && !s.chars().next().is_some_and(|c| c.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_like_bd_id_accepts_typical_ids() {
        assert!(looks_like_bd_id("hew-4az"));
        assert!(looks_like_bd_id("hew-4az.1"));
        assert!(looks_like_bd_id("bd-9zz"));
    }

    #[test]
    fn looks_like_bd_id_rejects_dates_and_numbers() {
        assert!(!looks_like_bd_id("2026-05-12"));
        assert!(!looks_like_bd_id("abc"));
        assert!(!looks_like_bd_id(""));
    }
}
