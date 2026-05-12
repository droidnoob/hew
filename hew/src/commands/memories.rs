use clap::Args as ClapArgs;
use hew_core::bd::{BdClient, RealBd};
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Filter to memories whose value starts with this prefix (e.g.
    /// CONVENTION, BOUNDARY, AUDIT, SECURITY, MIGRATION, DEP,
    /// STATUS, CHECKPOINT).
    #[arg(long)]
    pub prefix: Option<String>,

    /// Filter to memories whose value contains this substring (case-insensitive).
    #[arg(long)]
    pub grep: Option<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let client = RealBd::discover()?;
    let memories = client.memories()?;

    let needle = args.grep.as_ref().map(|s| s.to_lowercase());
    let pfx = args.prefix.as_ref().map(|p| format!("{}:", p.trim_end_matches(':')));

    let mut hits: Vec<(&String, &String)> = memories
        .iter()
        .filter(|(_, v)| pfx.as_ref().is_none_or(|p| v.trim_start().starts_with(p)))
        .filter(|(_, v)| needle.as_ref().is_none_or(|n| v.to_lowercase().contains(n)))
        .collect();
    hits.sort_by(|a, b| a.0.cmp(b.0));

    if matches!(ctx.output, OutputMode::Json) {
        let obj: serde_json::Map<String, serde_json::Value> = hits
            .iter()
            .map(|(k, v)| ((*k).clone(), serde_json::Value::String((*v).clone())))
            .collect();
        println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap());
    } else if hits.is_empty() {
        println!("(no memories match)");
    } else {
        for (k, v) in &hits {
            println!("- {k}");
            for line in v.lines() {
                println!("    {line}");
            }
        }
        println!();
        println!("{} memories", hits.len());
    }
    Ok(())
}
