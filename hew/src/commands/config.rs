use clap::{Args as ClapArgs, Subcommand};
use hew_core::config;
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Print the value of a single config key.
    Get { key: String },
    /// Set a config key. Pass an empty value to clear an optional key.
    Set { key: String, value: String },
    /// Show all config keys with their current values.
    List,
    /// Reset config to defaults.
    Reset,
    /// Print the resolved config file path.
    Path,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    match args.op {
        Op::Path => {
            let p = config::config_path()?;
            println!("{}", p.display());
            Ok(())
        }
        Op::Get { key } => {
            let cfg = config::load()?;
            match config::get(&cfg, &key) {
                Some(v) => {
                    println!("{v}");
                    Ok(())
                }
                None => Err(miette::miette!(
                    "unknown or unset key `{key}`. Run `hew config list` for the full set."
                )),
            }
        }
        Op::Set { key, value } => {
            let mut cfg = config::load()?;
            config::set(&mut cfg, &key, &value)?;
            let path = config::save(&cfg)?;
            if !ctx.quiet {
                println!("set {key} = {value} ({})", path.display());
            }
            Ok(())
        }
        Op::List => {
            let cfg = config::load()?;
            if matches!(ctx.output, OutputMode::Json) {
                let mut obj = serde_json::Map::new();
                for k in config::keys() {
                    obj.insert(
                        (*k).into(),
                        serde_json::Value::String(config::get(&cfg, k).unwrap_or_default()),
                    );
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap()
                );
            } else {
                for k in config::keys() {
                    let v = config::get(&cfg, k).unwrap_or_else(|| "(unset)".to_string());
                    println!("  {k:<28} {v}");
                }
            }
            Ok(())
        }
        Op::Reset => {
            let cfg = config::Config::default();
            let path = config::save(&cfg)?;
            if !ctx.quiet {
                println!("reset config to defaults ({})", path.display());
            }
            Ok(())
        }
    }
}
