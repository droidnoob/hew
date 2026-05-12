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
    /// Schema for `hew task show --json` output (TaskSummary).
    Task,
    /// Schema for `hew epic show --json` output (EpicSummary).
    Epic,
    /// Schema for the `hew task list` filter args (TaskListFilter).
    TaskListFilter,
    /// Schema for `hew task new` args (NewTaskArgs).
    NewTask,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    let schema = match args.which {
        Which::Prime => schema_for!(hew_core::prime::PrimeOutput),
        Which::Resume => schema_for!(hew_core::prime::ResumeOutput),
        Which::Config => schema_for!(hew_core::config::Config),
        Which::ReviewBundle => schema_for!(hew_core::review::ReviewBundle),
        Which::Task => schema_for!(hew_core::tasks::TaskSummary),
        Which::Epic => schema_for!(hew_core::tasks::EpicSummary),
        Which::TaskListFilter => schema_for!(hew_core::tasks::TaskListFilter),
        Which::NewTask => schema_for!(hew_core::tasks::NewTaskArgs),
    };
    let json = serde_json::to_string_pretty(&schema)
        .map_err(|e| miette::miette!("serialize schema: {e}"))?;
    println!("{json}");
    Ok(())
}
