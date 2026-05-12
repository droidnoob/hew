//! `hew review-bundle` — emit a [`ReviewBundle`] as JSON for the review skills.
//!
//! Default scope: last N closed tasks where N comes from
//! `review.batch_size` (default 8). `--since=<ref>` and `--n=<count>`
//! override. `--since` auto-detects ref kind by trying bd issue lookup
//! first, then git `rev-parse`.

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::config;
use hew_core::git::{GitClient, RealGit};
use hew_core::review::{self, ReviewScope};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Anchor ref. Auto-detects bd epic-id / bd task-id / git ref by trying
    /// each in order. Mutually exclusive with --n.
    #[arg(long, conflicts_with = "n")]
    pub since: Option<String>,

    /// Number of most-recent closed tasks to include. Overrides
    /// `review.batch_size` for one invocation.
    #[arg(long)]
    pub n: Option<u32>,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
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
    args: &Args,
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

/// Decide whether `since` names a bd issue (epic or task) or a git ref.
/// Tries bd first; falls back to git. Errors if neither resolves.
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

/// `bd show <id> --json` returns the issue if it exists. Use `issue_type`
/// to discriminate epic vs task vs other.
fn try_bd_id(bd: &dyn BdClient, id: &str) -> miette::Result<Option<ReviewScope>> {
    use std::ffi::{OsStr, OsString};
    let id_os = OsString::from(id);
    let out = bd.run_raw(&[OsStr::new("show"), id_os.as_os_str(), OsStr::new("--json")]);
    let Ok(out) = out else { return Ok(None) };
    // bd show on a missing id may still exit 0 with empty / non-array body.
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
