use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Attempt to auto-repair detected issues.
    #[arg(long)]
    pub fix: bool,
}

pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    miette::bail!("`hew doctor` is not yet implemented (tracked: hew-3xq.2.12)");
}
