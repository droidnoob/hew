use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Which schema to emit.
    #[arg(value_enum)]
    pub which: Which,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum Which {
    Prime,
    Status,
    Config,
}

pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    miette::bail!("`hew schema` is not yet implemented (tracked: hew-3xq.2.14)");
}
