use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::RealBd;
use hew_core::prime;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Skill name to prime context for (e.g. execute, plan, scan), or
    /// the reserved value `resume` to emit skill-agnostic project state
    /// for SessionStart hooks.
    pub skill: String,

    /// Pretty-print the JSON output. Default is compact. Implies `--json`.
    #[arg(long)]
    pub pretty: bool,

    /// Emit JSON instead of plaintext. Plaintext is the default for
    /// `resume`; other skills always emit JSON (this flag is a no-op
    /// for them, kept for forward-compat).
    #[arg(long)]
    pub json: bool,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    let client = RealBd::discover()?;
    let want_json = args.json || args.pretty;

    if args.skill == "resume" {
        let output = prime::resume(&client)?;
        if want_json {
            let s = if args.pretty {
                serde_json::to_string_pretty(&output)
            } else {
                serde_json::to_string(&output)
            }
            .map_err(|e| miette::miette!("serialize prime output: {e}"))?;
            println!("{s}");
        } else {
            print!("{}", prime::render_resume_text(&output));
        }
    } else {
        let output = prime::build(&client, &args.skill)?;
        let s = if args.pretty {
            serde_json::to_string_pretty(&output)
        } else {
            serde_json::to_string(&output)
        }
        .map_err(|e| miette::miette!("serialize prime output: {e}"))?;
        println!("{s}");
    }
    Ok(())
}
