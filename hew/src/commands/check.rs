use clap::Args as ClapArgs;
use hew_core::bd::RealBd;
use hew_core::prime;
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Skill name to check prerequisites for (e.g. execute, plan, scan).
    pub skill: String,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let client = RealBd::discover()?;
    let out = prime::build(&client, &args.skill)?;
    let met = out.prerequisites.met;

    if matches!(ctx.output, OutputMode::Json) {
        let payload = serde_json::json!({
            "skill": out.skill,
            "met": met,
            "missing": out.prerequisites.missing,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else if met {
        println!("✓ {}: prerequisites met", out.skill);
    } else {
        println!(
            "✗ {}: missing prerequisites: {}",
            out.skill,
            out.prerequisites.missing.join(", ")
        );
    }

    if met { Ok(()) } else { Err(miette::miette!("prerequisites not met")) }
}
