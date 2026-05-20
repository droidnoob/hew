//! `hew ready` + `hew next` — list/claim from the bd ready queue.
//!
//! - `hew ready` mirrors `bd ready --json` through the curated
//!   [`hew_core::bd::ReadyTask`] type. Text-by-default; `--json` opts in.
//! - `hew next` picks the top of that queue. By default it claims; pass
//!   `--no-claim` to peek. `--branch` additionally creates a feature
//!   branch derived from the task's issue_type + title.

use clap::Args as ClapArgs;
use hew_core::bd::{BdClient, ReadyTask, RealBd};
use hew_core::git::{GitClient, RealGit};
use hew_core::{Ctx, OutputMode, branch, tasks};

#[derive(Debug, ClapArgs)]
pub struct ReadyArgs {
    /// Max results to print (`0` = unlimited). Default 20.
    #[arg(long, default_value_t = 20)]
    pub n: u32,
    /// Emit JSON `[ReadyTask, ...]` instead of text.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct NextArgs {
    /// Peek at the top ready task without claiming it.
    #[arg(long)]
    pub no_claim: bool,
    /// After claiming, create a feature branch derived from the task's
    /// issue_type (→ prefix) and title (→ slug). No-op when `--no-claim`.
    #[arg(long)]
    pub branch: bool,
    /// Override the branch prefix used by `--branch`. Otherwise inferred
    /// from issue_type via [`issue_type_to_prefix`].
    #[arg(long, requires = "branch")]
    pub prefix: Option<String>,
    /// Override the branch slug used by `--branch`. Otherwise the task
    /// title is slugified.
    #[arg(long, requires = "branch")]
    pub slug: Option<String>,
    /// Emit JSON `{task, claimed, branch}` instead of text.
    #[arg(long)]
    pub json: bool,
}

pub fn run_ready(ctx: &Ctx, args: ReadyArgs) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    let mut items = bd.ready()?;
    if args.n > 0 && items.len() > args.n as usize {
        items.truncate(args.n as usize);
    }

    if wants_json(ctx, args.json) {
        emit_json(&items)?;
    } else if items.is_empty() {
        println!("(no ready tasks)");
    } else {
        for t in &items {
            print_ready_row(t);
        }
        println!();
        println!("{} ready task(s)", items.len());
    }
    Ok(())
}

pub fn run_next(ctx: &Ctx, args: NextArgs) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    let items = bd.ready()?;
    let Some(top) = items.into_iter().next() else {
        if wants_json(ctx, args.json) {
            emit_json(&serde_json::json!({ "task": null, "claimed": false, "branch": null }))?;
        } else {
            println!("(no ready tasks)");
        }
        return Ok(());
    };

    let claimed = !args.no_claim;
    if claimed {
        tasks::claim(&bd, &top.id)?;
    }

    let branch_name = if args.branch && claimed {
        Some(create_branch_for(&top, args.prefix.as_deref(), args.slug.as_deref())?)
    } else {
        None
    };

    if wants_json(ctx, args.json) {
        emit_json(&serde_json::json!({
            "task": &top,
            "claimed": claimed,
            "branch": branch_name,
        }))?;
    } else if !ctx.quiet {
        let verb = if claimed { "claimed" } else { "next" };
        println!("{verb} {} — {}", top.id, top.title);
        if let Some(b) = &branch_name {
            println!("created branch {b}");
        }
    }
    Ok(())
}

fn create_branch_for(
    task: &ReadyTask,
    prefix_override: Option<&str>,
    slug_override: Option<&str>,
) -> miette::Result<String> {
    let prefix = prefix_override.unwrap_or_else(|| issue_type_to_prefix(&task.issue_type));
    let slug_raw = slug_override.unwrap_or(&task.title);
    let name = branch::build_branch_name(prefix, slug_raw)?;
    let git = RealGit::discover()?;
    git.checkout_new_branch(&name, None)?;
    Ok(name)
}

/// Map a bd `issue_type` to a conventional-commit branch prefix from
/// [`hew_core::branch::PREFIXES`]. Unknown types fall back to `feat`.
pub fn issue_type_to_prefix(issue_type: &str) -> &'static str {
    match issue_type {
        "bug" => "fix",
        "chore" => "chore",
        "docs" => "docs",
        // `feature`, `task`, `epic`, anything else → feat
        _ => "feat",
    }
}

fn wants_json(ctx: &Ctx, local: bool) -> bool {
    local || matches!(ctx.output, OutputMode::Json)
}

fn emit_json<T: serde::Serialize>(value: &T) -> miette::Result<()> {
    let s = serde_json::to_string_pretty(value).map_err(|e| miette::miette!("json: {e}"))?;
    println!("{s}");
    Ok(())
}

fn print_ready_row(t: &ReadyTask) {
    let kind = if t.issue_type.is_empty() { "?".to_string() } else { t.issue_type.clone() };
    println!("  ○  {:<14} P{}  {:<6}  {}", t.id, t.priority, kind, t.title);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_type_mapping() {
        assert_eq!(issue_type_to_prefix("bug"), "fix");
        assert_eq!(issue_type_to_prefix("chore"), "chore");
        assert_eq!(issue_type_to_prefix("docs"), "docs");
        assert_eq!(issue_type_to_prefix("feature"), "feat");
        assert_eq!(issue_type_to_prefix("task"), "feat");
        assert_eq!(issue_type_to_prefix("epic"), "feat");
        assert_eq!(issue_type_to_prefix(""), "feat");
        assert_eq!(issue_type_to_prefix("nonsense"), "feat");
    }
}
