//! `hew review {bundle,check}` — agent-facing JSON utilities for the
//! `/hew:review` and `/hew:adversarial-review` skills.
//!
//! - `hew review bundle [--since=<ref>] [--n=<count>]` emits a
//!   [`hew_core::review::ReviewBundle`] for the skills to consume.
//! - `hew review check [--epic-closed=<bool>]` reports whether the
//!   `hew-execute` Step 10 picker should fire, per the trigger rule
//!   from DECISION:review-trigger.

use clap::{Args as ClapArgs, Subcommand};
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::config;
use hew_core::git::{GitClient, RealGit};
use hew_core::review::{self, ReviewScope};
use serde::Serialize;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Emit a JSON review bundle (closed tasks + diff + memories + epic).
    Bundle(BundleArgs),
    /// Report whether the Step 10 review picker should fire.
    Check(CheckArgs),
}

#[derive(Debug, ClapArgs)]
pub struct BundleArgs {
    /// Anchor ref. Auto-detects bd epic-id / bd task-id / git ref by trying
    /// each in order. Mutually exclusive with --n.
    #[arg(long, conflicts_with = "n")]
    pub since: Option<String>,

    /// Number of most-recent closed tasks to include. Overrides
    /// `review.batch_size` for one invocation.
    #[arg(long)]
    pub n: Option<u32>,
}

#[derive(Debug, ClapArgs)]
pub struct CheckArgs {
    /// Override auto-detection of "an epic just closed in this work cycle."
    /// Agents that just closed an epic in this turn can pass
    /// `--epic-closed=true` to be explicit. Default: auto-detect.
    #[arg(long, value_name = "BOOL")]
    pub epic_closed: Option<bool>,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    match args.op {
        Op::Bundle(a) => run_bundle(a),
        Op::Check(a) => run_check(a),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// bundle
// ────────────────────────────────────────────────────────────────────────────

fn run_bundle(args: BundleArgs) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    let git = RealGit::discover()?;
    let cfg = config::load()?;

    let scope = resolve_scope(&bd, &git, &args, &cfg)?;
    let bundle = review::bundle(&bd, &git, scope)?;

    let json = serde_json::to_string_pretty(&bundle)
        .map_err(|e| miette::miette!("serialize bundle: {e}"))?;
    println!("{json}");
    Ok(())
}

fn resolve_scope(
    bd: &dyn BdClient,
    git: &dyn GitClient,
    args: &BundleArgs,
    cfg: &config::Config,
) -> miette::Result<ReviewScope> {
    if let Some(ref since) = args.since {
        return classify_since(bd, git, since);
    }
    let n = args.n.unwrap_or(cfg.review.batch_size);
    if n == 0 {
        return Err(miette::miette!(
            "review.batch_size is 0 — set `hew config set review.batch_size <n>` (>=1) or pass --n"
        ));
    }
    Ok(ReviewScope::LastN(n))
}

fn classify_since(
    bd: &dyn BdClient,
    git: &dyn GitClient,
    since: &str,
) -> miette::Result<ReviewScope> {
    if let Some(scope) = try_bd_id(bd, since)? {
        return Ok(scope);
    }
    if git_ref_resolves(git, since) {
        return Ok(ReviewScope::GitRef(since.to_string()));
    }
    Err(miette::miette!(
        "--since={since} matches no bd issue and no git ref. Pass an epic id (e.g. hew-vhz), \
         a task id (hew-vhz.3), or a git ref (HEAD~5, origin/main, abc123)."
    ))
}

fn try_bd_id(bd: &dyn BdClient, id: &str) -> miette::Result<Option<ReviewScope>> {
    use std::ffi::{OsStr, OsString};
    let id_os = OsString::from(id);
    let out = bd.run_raw(&[OsStr::new("show"), id_os.as_os_str(), OsStr::new("--json")]);
    let Ok(out) = out else { return Ok(None) };
    let trimmed = out.stdout.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };
    let issue_type = parsed
        .get(0)
        .and_then(|i| i.get("issue_type"))
        .and_then(|t| t.as_str())
        .unwrap_or_default();

    if issue_type == "epic" {
        Ok(Some(ReviewScope::Epic(id.to_string())))
    } else if !issue_type.is_empty() {
        Ok(Some(ReviewScope::Task(id.to_string())))
    } else {
        Ok(None)
    }
}

fn git_ref_resolves(git: &dyn GitClient, rev: &str) -> bool {
    use std::ffi::{OsStr, OsString};
    let rev_os = OsString::from(rev);
    git.run_raw(&[OsStr::new("rev-parse"), OsStr::new("--verify"), rev_os.as_os_str()]).is_ok()
}

// ────────────────────────────────────────────────────────────────────────────
// check
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct CheckOutput {
    tasks_since_last_review: u32,
    last_review_at: Option<String>,
    config: ConfigSnapshot,
    epic_just_closed: bool,
    picker_should_fire: bool,
    reason: String,
}

#[derive(Debug, Serialize)]
struct ConfigSnapshot {
    after_n_tasks: u32,
    after_epic: bool,
    batch_size: u32,
}

fn run_check(args: CheckArgs) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    let cfg = config::load()?;

    let tasks_since_last_review = review::tasks_since_last_review(&bd)?;
    let last_review_at = review::last_review_marker(&bd)?;
    let epic_just_closed = match args.epic_closed {
        Some(v) => v,
        None => detect_epic_just_closed(&bd)?,
    };

    let (picker_should_fire, reason) =
        evaluate_trigger(&cfg.review, tasks_since_last_review, epic_just_closed);

    let out = CheckOutput {
        tasks_since_last_review,
        last_review_at,
        config: ConfigSnapshot {
            after_n_tasks: cfg.review.after_n_tasks,
            after_epic: cfg.review.after_epic,
            batch_size: cfg.review.batch_size,
        },
        epic_just_closed,
        picker_should_fire,
        reason,
    };

    let json =
        serde_json::to_string_pretty(&out).map_err(|e| miette::miette!("serialize check: {e}"))?;
    println!("{json}");
    Ok(())
}

/// DECISION:review-trigger — picker fires when:
/// 1. `after_n_tasks > 0 AND tasks_since_last_review >= after_n_tasks`, OR
/// 2. `after_epic AND epic_just_closed AND tasks_since_last_review > 0`.
fn evaluate_trigger(
    review_cfg: &config::ReviewConfig,
    tasks_since: u32,
    epic_just_closed: bool,
) -> (bool, String) {
    if review_cfg.after_n_tasks > 0 && tasks_since >= review_cfg.after_n_tasks {
        return (
            true,
            format!(
                "tasks_since_last_review={tasks_since} >= review.after_n_tasks={}",
                review_cfg.after_n_tasks
            ),
        );
    }
    if review_cfg.after_epic && epic_just_closed && tasks_since > 0 {
        return (
            true,
            format!(
                "epic just closed with {tasks_since} task(s) since last review (review.after_epic=true)"
            ),
        );
    }
    if review_cfg.after_n_tasks == 0 && !review_cfg.after_epic {
        return (
            false,
            "no triggers configured (review.after_n_tasks=0, review.after_epic=false)".into(),
        );
    }
    (false, "thresholds not yet met".into())
}

/// Auto-detect "an epic just closed in this work cycle" by checking whether
/// the single most recently closed issue is an epic. The agent closes child
/// tasks first; the very last close in an epic-completion cycle is the
/// epic itself.
fn detect_epic_just_closed(bd: &dyn BdClient) -> miette::Result<bool> {
    use std::ffi::OsStr;
    let out = bd.run_raw(&[
        OsStr::new("list"),
        OsStr::new("--status=closed"),
        OsStr::new("--sort=closed"),
        OsStr::new("--limit"),
        OsStr::new("1"),
        OsStr::new("--json"),
    ])?;
    let issues: Vec<serde_json::Value> = serde_json::from_str(out.stdout.trim())
        .map_err(|e| miette::miette!("decode bd list: {e}"))?;
    Ok(issues.first().and_then(|i| i.get("issue_type")).and_then(|t| t.as_str()) == Some("epic"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(after_n: u32, after_epic: bool, batch: u32) -> config::ReviewConfig {
        config::ReviewConfig { after_n_tasks: after_n, after_epic, batch_size: batch }
    }

    #[test]
    fn no_triggers_configured_never_fires() {
        let (fire, reason) = evaluate_trigger(&cfg(0, false, 8), 100, true);
        assert!(!fire);
        assert!(reason.contains("no triggers"), "{reason}");
    }

    #[test]
    fn after_n_tasks_fires_at_threshold() {
        let (fire, reason) = evaluate_trigger(&cfg(5, false, 8), 5, false);
        assert!(fire);
        assert!(reason.contains(">= review.after_n_tasks=5"), "{reason}");
    }

    #[test]
    fn after_n_tasks_does_not_fire_below_threshold() {
        let (fire, _) = evaluate_trigger(&cfg(5, false, 8), 4, false);
        assert!(!fire);
    }

    #[test]
    fn after_epic_fires_only_with_at_least_one_task() {
        let (fire, _) = evaluate_trigger(&cfg(0, true, 8), 0, true);
        assert!(!fire);
        let (fire, reason) = evaluate_trigger(&cfg(0, true, 8), 3, true);
        assert!(fire);
        assert!(reason.contains("epic just closed"), "{reason}");
    }

    #[test]
    fn after_epic_does_not_fire_without_epic_close() {
        let (fire, _) = evaluate_trigger(&cfg(0, true, 8), 5, false);
        assert!(!fire);
    }

    #[test]
    fn both_triggers_active_either_fires() {
        let (fire, _) = evaluate_trigger(&cfg(3, true, 8), 5, false);
        assert!(fire);
        let (fire, reason) = evaluate_trigger(&cfg(10, true, 8), 2, true);
        assert!(fire);
        assert!(reason.contains("epic just closed"), "{reason}");
    }
}
