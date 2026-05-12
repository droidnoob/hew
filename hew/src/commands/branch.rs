//! `hew branch new --prefix=<type> --slug=<text>` — create a branch via
//! `git checkout -b <prefix>/<slug>`. Validates prefix against the
//! locked conventional set and slugifies user input.

use clap::{Args as ClapArgs, Subcommand};
use hew_core::Ctx;
use hew_core::branch;
use hew_core::git::{GitClient, RealGit};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Create a new branch named `<prefix>/<slug>`.
    New(NewArgs),
}

#[derive(Debug, ClapArgs)]
pub struct NewArgs {
    /// Conventional prefix: feat, fix, chore, docs, refactor, perf, test, style.
    #[arg(long)]
    pub prefix: String,

    /// Free-form slug; will be lowercased + non-[a-z0-9-] stripped.
    #[arg(long)]
    pub slug: String,

    /// Optional base ref. Defaults to current HEAD.
    #[arg(long)]
    pub from: Option<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    match args.op {
        Op::New(a) => new_branch(ctx, a),
    }
}

fn new_branch(ctx: &Ctx, args: NewArgs) -> miette::Result<()> {
    let name = branch::build_branch_name(&args.prefix, &args.slug)?;
    let git = RealGit::discover()?;
    git.checkout_new_branch(&name, args.from.as_deref())?;
    if !ctx.quiet {
        println!("created branch {name}");
    }
    Ok(())
}
