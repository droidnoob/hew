use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::RealBd;
use hew_core::prime;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Skill name to prime context for (e.g. execute, plan, scan).
    pub skill: String,

    /// Pretty-print the JSON output. Default is compact.
    #[arg(long)]
    pub pretty: bool,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    let client = RealBd::discover()?;
    let output = prime::build(&client, &args.skill)?;
    let s = if args.pretty {
        serde_json::to_string_pretty(&output)
    } else {
        serde_json::to_string(&output)
    }
    .map_err(|e| miette::miette!("serialize prime output: {e}"))?;
    println!("{s}");
    Ok(())
}
