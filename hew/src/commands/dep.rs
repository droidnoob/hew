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
// ────────────────────────────────────────────────────────────────────────────

fn truncate_tree(value: &serde_json::Value, max_depth: u32) -> serde_json::Value {
    fn walk(v: &serde_json::Value, depth: u32, max: u32) -> serde_json::Value {
        match v {
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::new();
                for (k, child) in map {
                    if k == "children" || k == "dependencies" || k == "dependents" {
                        if max != 0 && depth >= max.saturating_sub(1) {
                            out.insert(k.clone(), serde_json::Value::Array(Vec::new()));
                        } else if let Some(arr) = child.as_array() {
                            let mapped: Vec<_> =
                                arr.iter().map(|c| walk(c, depth + 1, max)).collect();
                            out.insert(k.clone(), serde_json::Value::Array(mapped));
                        } else {
                            out.insert(k.clone(), child.clone());
                        }
                    } else {
                        out.insert(k.clone(), child.clone());
                    }
                }
                serde_json::Value::Object(out)
            }
            serde_json::Value::Array(arr) => {
                let mapped: Vec<_> = arr.iter().map(|c| walk(c, depth, max)).collect();
                serde_json::Value::Array(mapped)
            }
            other => other.clone(),
        }
    }
    walk(value, 0, max_depth)
}

fn render_tree_text(value: &serde_json::Value, max_depth: u32) {
    fn walk(v: &serde_json::Value, depth: u32, max: u32, prefix: &str) {
        let id = v.get("id").and_then(|x| x.as_str()).unwrap_or("?");
        let title = v.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let status = v.get("status").and_then(|x| x.as_str()).unwrap_or("");
        let marker = match status {
            "closed" => "✓",
            "in_progress" => "◐",
            "blocked" => "●",
            "deferred" => "❄",
            "" => "·",
            _ => "○",
        };
        println!("{prefix}{marker} {id}  {title}");
        if max != 0 && depth >= max.saturating_sub(1) {
            return;
        }
        for key in ["children", "dependencies", "dependents"] {
            if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
                let next_prefix = format!("{prefix}  ");
                for child in arr {
                    walk(child, depth + 1, max, &next_prefix);
                }
            }
        }
    }
    if value.is_null() {
        println!("(no tree)");
        return;
    }
    walk(value, 0, max_depth, "");
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

    #[test]
    fn truncate_tree_caps_children_at_depth() {
        let tree = json!({
            "id": "a",
            "children": [
                {"id": "b", "children": [
                    {"id": "c", "children": [{"id": "d", "children": []}]}
                ]}
            ]
        });
        let cut = truncate_tree(&tree, 2);
        // depth 0 = a, depth 1 = b (kept), but b's children must be empty.
        let b = &cut["children"][0];
        assert_eq!(b["id"], "b");
        assert!(b["children"].as_array().unwrap().is_empty());
    }

    #[test]
    fn truncate_tree_depth_zero_is_unlimited() {
        let tree = json!({
            "id": "a",
            "children": [{"id": "b", "children": [{"id": "c", "children": []}]}]
        });
        let cut = truncate_tree(&tree, 0);
        let c = &cut["children"][0]["children"][0];
        assert_eq!(c["id"], "c");
    }
}
