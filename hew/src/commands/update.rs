use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Update local project skills (in addition to / instead of global).
    #[arg(long)]
    pub local: bool,

    /// Print version diff JSON without actually updating.
    #[arg(long)]
    pub check_only: bool,

    /// Accept update prompt non-interactively.
    #[arg(short, long)]
    pub yes: bool,
}

pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    miette::bail!("`hew update` is not yet implemented (tracked: hew-3xq.2.8)");
}
