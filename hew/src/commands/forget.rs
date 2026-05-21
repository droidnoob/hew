//! `hew forget <KEY>` — top-level alias for `hew memories --forget <KEY>`.
//!
//! Splits out as its own subcommand so the next link in the
//! Memory Links epic (ML.6, hew-jem) has a clean surface to extend
//! with cascade-delete of outbound LINK: rows. The cascade isn't here
//! yet — this PR only adds the subcommand. Today the body is exactly
//! the same call `hew memories --forget` already makes.

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::tasks;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Memory key to forget.
    #[arg(value_name = "KEY")]
    pub key: String,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    run_with_bd(ctx, &bd, &args.key)
}

fn run_with_bd(ctx: &Ctx, bd: &dyn BdClient, key: &str) -> miette::Result<()> {
    tasks::forget(bd, key)?;
    if !ctx.quiet {
        println!("forgot {key}");
    }
    Ok(())
}
