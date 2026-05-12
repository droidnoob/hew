//! `hew dep <verb>` — curated dependency operations over
//! [`hew_core::tasks`]. Thin clap layer; text by default; `--json` opts
//! in (where the underlying bd shape is JSON).

use clap::{Args as ClapArgs, Subcommand};
use hew_core::bd::{BdClient, RealBd};
use hew_core::tasks::{self, TaskSummary};
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Add a dependency: `<dependent>` becomes blocked by `<prerequisite>`.
    Add(AddArgs),
    /// Remove a dependency edge.
    Remove(RemoveArgs),
    /// Show the dependency tree for an issue.
    Tree(TreeArgs),
    /// List every task currently blocked by an open prerequisite.
    Blocked(BlockedArgs),
}

#[derive(Debug, ClapArgs)]
pub struct AddArgs {
    /// The task that becomes blocked.
    pub dependent: String,
    /// The prerequisite that must close first.
    #[arg(long = "on")]
    pub prerequisite: String,
}

#[derive(Debug, ClapArgs)]
pub struct RemoveArgs {
    pub dependent: String,
    pub prerequisite: String,
}

#[derive(Debug, ClapArgs)]
pub struct TreeArgs {
    pub id: String,
    /// `0` = unlimited (bd default).
    #[arg(long, default_value_t = 3)]
    pub depth: u32,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct BlockedArgs {
    #[arg(long)]
    pub json: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    match args.op {
        Op::Add(a) => add(ctx, &bd, a),
        Op::Remove(a) => remove(ctx, &bd, a),
        Op::Tree(a) => tree(ctx, &bd, a),
        Op::Blocked(a) => blocked(ctx, &bd, a),
    }
}

fn add(ctx: &Ctx, bd: &dyn BdClient, args: AddArgs) -> miette::Result<()> {
    tasks::dep_add(bd, &args.dependent, &args.prerequisite)?;
    if !ctx.quiet {
        println!("{} now depends on {}", args.dependent, args.prerequisite);
    }
    Ok(())
}

fn remove(ctx: &Ctx, bd: &dyn BdClient, args: RemoveArgs) -> miette::Result<()> {
    tasks::dep_remove(bd, &args.dependent, &args.prerequisite)?;
    if !ctx.quiet {
        println!("removed dep {} -> {}", args.dependent, args.prerequisite);
    }
    Ok(())
}

fn tree(ctx: &Ctx, bd: &dyn BdClient, args: TreeArgs) -> miette::Result<()> {
    // bd dep tree --json is the source of truth — depth-truncate locally so
    // the wrapper can render text without a second round-trip.
    let raw = tasks::dep_tree(bd, &args.id)?;
    if wants_json(ctx, args.json) {
        let truncated = truncate_tree(&raw, args.depth);
        let s =
            serde_json::to_string_pretty(&truncated).map_err(|e| miette::miette!("json: {e}"))?;
        println!("{s}");
    } else {
        render_tree_text(&raw, args.depth);
    }
    Ok(())
}

fn blocked(ctx: &Ctx, bd: &dyn BdClient, args: BlockedArgs) -> miette::Result<()> {
    let items = tasks::blocked(bd)?;
    if wants_json(ctx, args.json) {
        let s = serde_json::to_string_pretty(&items).map_err(|e| miette::miette!("json: {e}"))?;
        println!("{s}");
    } else if items.is_empty() {
        println!("(nothing blocked)");
    } else {
        for t in &items {
            print_row(t);
        }
        println!();
        println!("{} blocked", items.len());
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tree rendering
//
// `bd dep tree --json` returns a flat DFS-ordered array of nodes, each
// carrying its own `depth` integer (and `parent_id`). We render with
// `depth * "  "` indentation; `--depth N` keeps nodes where `depth < N`
// (`N == 0` means unlimited). Nested-tree variants are not used.
// ────────────────────────────────────────────────────────────────────────────

fn truncate_tree(value: &serde_json::Value, max_depth: u32) -> serde_json::Value {
    let Some(arr) = value.as_array() else {
        return value.clone();
    };
    let kept: Vec<serde_json::Value> = arr
        .iter()
        .filter(|node| {
            let depth = node.get("depth").and_then(|d| d.as_u64()).unwrap_or(0) as u32;
            max_depth == 0 || depth < max_depth
        })
        .cloned()
        .collect();
    serde_json::Value::Array(kept)
}

fn render_tree_text(value: &serde_json::Value, max_depth: u32) {
    let Some(arr) = value.as_array() else {
        println!("(no tree)");
        return;
    };
    if arr.is_empty() {
        println!("(no tree)");
        return;
    }
    for node in arr {
        let depth = node.get("depth").and_then(|d| d.as_u64()).unwrap_or(0) as u32;
        if max_depth != 0 && depth >= max_depth {
            continue;
        }
        let id = node.get("id").and_then(|x| x.as_str()).unwrap_or("?");
        let title = node.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let status = node.get("status").and_then(|x| x.as_str()).unwrap_or("");
        let marker = match status {
            "closed" => "✓",
            "in_progress" => "◐",
            "blocked" => "●",
            "deferred" => "❄",
            "" => "·",
            _ => "○",
        };
        let indent = "  ".repeat(depth as usize);
        println!("{indent}{marker} {id}  {title}");
    }
}

// ────────────────────────────────────────────────────────────────────────────

fn wants_json(ctx: &Ctx, local: bool) -> bool {
    local || matches!(ctx.output, OutputMode::Json)
}

fn print_row(t: &TaskSummary) {
    let marker = match t.status.as_str() {
        "blocked" => "●",
        "in_progress" => "◐",
        "deferred" => "❄",
        "closed" => "✓",
        _ => "○",
    };
    println!(
        "  {marker}  {:<14} P{}  {:<6}  {}",
        t.id,
        t.priority,
        if t.issue_type.is_empty() { "?".into() } else { t.issue_type.clone() },
        t.title,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn flat_tree() -> serde_json::Value {
        // Matches bd dep tree --json: DFS flat array, each node carries depth.
        json!([
            {"id": "a", "title": "root", "depth": 0, "status": "open"},
            {"id": "b", "title": "kid",  "depth": 1, "status": "open"},
            {"id": "c", "title": "grand","depth": 2, "status": "open"},
        ])
    }

    #[test]
    fn truncate_tree_filters_by_depth() {
        let cut = truncate_tree(&flat_tree(), 2);
        let arr = cut.as_array().unwrap();
        // depth=0 (a) + depth=1 (b) kept; depth=2 (c) dropped.
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["id"], "a");
        assert_eq!(arr[1]["id"], "b");
    }

    #[test]
    fn truncate_tree_depth_zero_is_unlimited() {
        let cut = truncate_tree(&flat_tree(), 0);
        let arr = cut.as_array().unwrap();
        assert_eq!(arr.len(), 3);
        assert_eq!(arr[2]["id"], "c");
    }
}
