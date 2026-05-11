use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Skill name to prime context for (e.g. execute, plan, scan).
    pub skill: String,
}

pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    miette::bail!("`hew prime` is not yet implemented (tracked: hew-3xq.2.4)");
}
