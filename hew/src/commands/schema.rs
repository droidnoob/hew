use clap::Args as ClapArgs;
use hew_core::Ctx;
use schemars::schema_for;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Which schema to emit.
    #[arg(value_enum)]
    pub which: Which,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum Which {
    /// Schema for `hew prime <skill>` output.
    Prime,
    /// Schema for `hew prime resume` output (SessionStart-hook payload).
    Resume,
    /// Schema for the persistent config TOML.
    Config,
    /// Schema for `hew review-bundle` JSON output.
    ReviewBundle,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    let schema = match args.which {
        Which::Prime => schema_for!(hew_core::prime::PrimeOutput),
        Which::Resume => schema_for!(hew_core::prime::ResumeOutput),
        Which::Config => schema_for!(hew_core::config::Config),
        Which::ReviewBundle => schema_for!(hew_core::review::ReviewBundle),
    };
    let json = serde_json::to_string_pretty(&schema)
        .map_err(|e| miette::miette!("serialize schema: {e}"))?;
    println!("{json}");
    Ok(())
}
