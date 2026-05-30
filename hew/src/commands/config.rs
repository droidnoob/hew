use std::path::PathBuf;

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
    Set {
        key: String,
        value: String,
        /// Write to the user-global config (`~/.config/hew/config.toml`).
        #[arg(long, conflicts_with = "project")]
        global: bool,
        /// Write to the project-local config (`./.hew.toml`). Creates the
        /// file with the starter header if absent.
        #[arg(long)]
        project: bool,
    },
    /// Show all config keys with their current values.
    List,
    /// Show effective config with per-key source attribution.
    Show,
    /// Reset config to defaults.
    Reset,
    /// Print the resolved config file path.
    Path,
}

/// Where a `hew config set` should land.
enum WriteTarget {
    UserGlobal(PathBuf),
    Project(PathBuf),
}

/// Resolve the on-disk target for a `hew config set` call. Implements
/// the 5-branch table from `hew-k2gm`:
///   1. `--global` + `--project`  → clap rejects upstream (`conflicts_with`).
///   2. `--global`                 → user-global.
///   3. `--project`                → project file (existing or `<root>/.hew.toml`).
///   4. neither + project exists   → refuse with the dual-flag message.
///   5. neither + no project file  → user-global (back-compat).
fn resolve_write_target(
    global: bool,
    project: bool,
    key: &str,
    value: &str,
) -> miette::Result<WriteTarget> {
    if global {
        return Ok(WriteTarget::UserGlobal(config::config_path()?));
    }

    let cwd = std::env::current_dir().map_err(|e| miette::miette!("cwd unavailable: {e}"))?;
    let project_root = config::discover_project_root(&cwd);
    let project_path = project_root.as_ref().and_then(|r| config::discover_project_config(r));

    if project {
        let path = match project_path {
            Some(p) => p,
            None => {
                let root = project_root.ok_or_else(|| {
                    miette::miette!(
                        "--project: no project root found (no `.beads/` or `.git` ancestor of {})",
                        cwd.display()
                    )
                })?;
                root.join(".hew.toml")
            }
        };
        return Ok(WriteTarget::Project(path));
    }

    match project_path {
        Some(p) => {
            let display_root = p
                .parent()
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| p.display().to_string());
            let file_name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| ".hew.toml".to_string());
            Err(miette::miette!(
                "refusing to write to user-global config when `{file_name}` exists at {display_root}\n\
                 \x20      team-shared config lives in `{file_name}`. Use one of:\n\
                 \x20        hew config set --project {key} {value}   # commit-shared\n\
                 \x20        hew config set --global  {key} {value}   # personal override"
            ))
        }
        None => Ok(WriteTarget::UserGlobal(config::config_path()?)),
    }
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
        Op::Set { key, value, global, project } => {
            let target = resolve_write_target(global, project, &key, &value)?;
            let (path, is_project) = match target {
                WriteTarget::UserGlobal(p) => (p, false),
                WriteTarget::Project(p) => (p, true),
            };
            let mut cfg = config::load_from(&path)?;
            config::set(&mut cfg, &key, &value)?;
            if is_project {
                config::save_project_to(&path, &cfg)?;
            } else {
                config::save_to(&path, &cfg)?;
            }
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
        Op::Show => {
            let loaded = config::load_with_provenance()?;
            if matches!(ctx.output, OutputMode::Json) {
                let sources: Vec<serde_json::Value> = loaded
                    .source_paths()
                    .into_iter()
                    .map(|(label, path)| {
                        serde_json::json!({ "label": label, "path": path.display().to_string() })
                    })
                    .collect();
                let mut keys_obj = serde_json::Map::new();
                for k in config::keys() {
                    let value = config::get(&loaded.config, k).unwrap_or_default();
                    let source =
                        loaded.sources.get(*k).copied().unwrap_or(config::ConfigSource::Default);
                    keys_obj.insert(
                        (*k).to_string(),
                        serde_json::json!({
                            "value": value,
                            "source": source.to_string(),
                        }),
                    );
                }
                let out = serde_json::json!({
                    "sources": sources,
                    "keys": serde_json::Value::Object(keys_obj),
                });
                println!("{}", serde_json::to_string_pretty(&out).unwrap());
            } else {
                let sps = loaded.source_paths();
                if sps.is_empty() {
                    println!("sources: (none — all defaults)");
                } else {
                    println!("sources (in precedence order):");
                    for (label, path) in &sps {
                        println!("  [{label}] {}", path.display());
                    }
                }
                println!();
                println!("effective config:");
                let width = config::keys().iter().map(|k| k.len()).max().unwrap_or(0);
                for k in config::keys() {
                    let v = config::get(&loaded.config, k).unwrap_or_default();
                    let src =
                        loaded.sources.get(*k).copied().unwrap_or(config::ConfigSource::Default);
                    let display_value = if v.is_empty() { "(unset)".to_string() } else { v };
                    println!("  {k:width$} = {display_value:<28} ({src})");
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
