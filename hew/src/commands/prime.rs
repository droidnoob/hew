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

    /// Pretty-print the JSON output. Implies `--json`.
    #[arg(long)]
    pub pretty: bool,

    /// Emit JSON instead of plaintext. Plaintext is the default for
    /// every skill including `resume` — FEEDBACK:no-json-piping makes
    /// the text shape the agent-facing contract.
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
        if want_json {
            let s = if args.pretty {
                serde_json::to_string_pretty(&output)
            } else {
                serde_json::to_string(&output)
            }
            .map_err(|e| miette::miette!("serialize prime output: {e}"))?;
            println!("{s}");
        } else {
            print!("{}", prime::render_prime_text(&output));
        }
    }
    Ok(())
}
