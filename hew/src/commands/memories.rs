use clap::Args as ClapArgs;
use hew_core::bd::{BdClient, RealBd};
use hew_core::tasks;
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Filter to memories whose value starts with this prefix (e.g.
    /// CONVENTION, BOUNDARY, AUDIT, SECURITY, MIGRATION, DEP,
    /// STATUS, CHECKPOINT).
    #[arg(long, conflicts_with_all = ["recall", "forget"])]
    pub prefix: Option<String>,

    /// Filter to memories whose value contains this substring (case-insensitive).
    #[arg(long, conflicts_with_all = ["recall", "forget"])]
    pub grep: Option<String>,

    /// Sugar for `--prefix=RESEARCH --grep=<topic>`. Conflicts with --prefix.
    #[arg(long, value_name = "TOPIC", conflicts_with_all = ["prefix", "recall", "forget"])]
    pub research: Option<String>,

    /// Print a single memory by key.
    #[arg(long, value_name = "KEY", conflicts_with = "forget")]
    pub recall: Option<String>,

    /// Remove a single memory by key.
    #[arg(long, value_name = "KEY")]
    pub forget: Option<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let client = RealBd::discover()?;

    if let Some(key) = args.recall.as_deref() {
        return run_recall(ctx, &client, key);
    }
    if let Some(key) = args.forget.as_deref() {
        return run_forget(ctx, &client, key);
    }

    let memories = client.memories()?;

    // Resolve --research sugar into the underlying prefix/grep filters.
    let (prefix, grep) = if let Some(topic) = args.research.as_ref() {
        (Some("RESEARCH".to_string()), Some(topic.clone()))
    } else {
        (args.prefix.clone(), args.grep.clone())
    };

    let needle = grep.as_ref().map(|s| s.to_lowercase());
    let pfx = prefix.as_ref().map(|p| format!("{}:", p.trim_end_matches(':')));

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

fn run_recall(ctx: &Ctx, bd: &dyn BdClient, key: &str) -> miette::Result<()> {
    match tasks::recall(bd, key)? {
        Some(body) => {
            println!("{body}");
            Ok(())
        }
        None => {
            if !ctx.quiet {
                eprintln!("no memory with key `{key}`");
            }
            Err(miette::miette!("no memory with key `{key}`"))
        }
    }
}

fn run_forget(ctx: &Ctx, bd: &dyn BdClient, key: &str) -> miette::Result<()> {
    tasks::forget(bd, key)?;
    if !ctx.quiet {
        println!("forgot {key}");
    }
    Ok(())
}
