use std::path::Path;

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::install::{self, Runtime};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Agent runtime to uninstall from. Defaults to auto-detect all
    /// installed runtimes in the project.
    #[arg(long, value_enum)]
    pub runtime: Option<crate::commands::init::RuntimeArg>,

    /// Also delete `.beads/` (destructive — drops the entire Beads
    /// task graph + memories for this project).
    #[arg(long)]
    pub purge: bool,

    /// Accept the purge prompt non-interactively.
    #[arg(short, long)]
    pub yes: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;

    let runtimes: Vec<Runtime> = match args.runtime {
        Some(r) => vec![r.into()],
        None => {
            let detected = install::detect_runtimes(&project_root);
            if detected.is_empty() {
                if !ctx.quiet {
                    eprintln!("hew uninstall: no agent runtime markers found in this project.");
                }
                Vec::new()
            } else {
                detected
            }
        }
    };

    let mut total_removed = 0usize;
    for runtime in &runtimes {
        let plan = install::uninstall(*runtime, &project_root)?;
        total_removed += plan.removed.len();
        if !ctx.quiet {
            if plan.removed.is_empty() {
                println!("hew uninstall: nothing to remove for {}", plan.runtime.as_str());
            } else {
                println!(
                    "hew uninstall: removed {} item{} for {}",
                    plan.removed.len(),
                    if plan.removed.len() == 1 { "" } else { "s" },
                    plan.runtime.as_str()
                );
                for p in &plan.removed {
                    println!("  - {}", p.display());
                }
            }
        }
    }

    if args.purge {
        purge_beads(ctx, &project_root, args.yes)?;
    }

    if !ctx.quiet && total_removed == 0 && !args.purge {
        eprintln!(
            "Nothing was removed. Pass --runtime=<name> to force a specific runtime, or --purge to also drop .beads/."
        );
    }

    Ok(())
}

fn purge_beads(ctx: &Ctx, project_root: &Path, force_yes: bool) -> miette::Result<()> {
    let beads_dir = project_root.join(".beads");
    if !beads_dir.exists() {
        if !ctx.quiet {
            println!("hew uninstall: .beads/ not present, nothing to purge.");
        }
        return Ok(());
    }

    if !force_yes && ctx.interactive {
        use inquire::Confirm;
        let go = Confirm::new(
            "Delete .beads/ ? This drops the entire task graph + memories for this project.",
        )
        .with_default(false)
        .prompt()
        .map_err(|e| miette::miette!("confirm: {e}"))?;
        if !go {
            if !ctx.quiet {
                println!("hew uninstall: purge skipped.");
            }
            return Ok(());
        }
    } else if !force_yes {
        return Err(miette::miette!(
            "--purge in non-interactive mode requires -y / --yes to confirm. \
             Re-run with --purge --yes."
        ));
    }

    std::fs::remove_dir_all(&beads_dir).map_err(|e| miette::miette!("remove .beads/: {e}"))?;
    if !ctx.quiet {
        println!("hew uninstall: purged {}", beads_dir.display());
    }
    Ok(())
}
