use clap::{Args as ClapArgs, Subcommand};
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    Get { key: String },
    Set { key: String, value: String },
    List,
    Reset,
}

pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    miette::bail!("`hew config` is not yet implemented (tracked: hew-3xq.2.13)");
}
