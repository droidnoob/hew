use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Re-write project-local skill files (default: keep them as-is).
    #[arg(long)]
    pub local: bool,

    /// Print version diff JSON without actually updating the binary.
    #[arg(long)]
    pub check_only: bool,

    /// Accept the update prompt non-interactively.
    #[arg(short, long)]
    pub yes: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    use axoupdater::{AxoUpdater, ReleaseSourceType};

    let mut updater = AxoUpdater::new_for("hew");
    updater
        .set_release_source(axoupdater::ReleaseSource {
            release_type: ReleaseSourceType::GitHub,
            owner: "droidnoob".to_string(),
            name: "hew".to_string(),
            app_name: "hew".to_string(),
        })
        .disable_installer_output()
        .always_update(false);

    if args.check_only {
        let current = env!("CARGO_PKG_VERSION");
        let upgrade_available = match updater.is_update_needed_sync() {
            Ok(v) => v,
            Err(e) => {
                let payload = serde_json::json!({
                    "current": current,
                    "update_available": false,
                    "error": e.to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&payload).unwrap());
                return Ok(());
            }
        };
        let payload = serde_json::json!({
            "current": current,
            "update_available": upgrade_available,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
        return Ok(());
    }

    if !args.yes && ctx.interactive {
        // Cheap human confirmation. Non-interactive callers must pass --yes.
        eprintln!("hew update: checking for a newer release on GitHub...");
    }

    match updater.run_sync() {
        Ok(Some(result)) => {
            if !ctx.quiet {
                let old =
                    result.old_version.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
                println!("hew updated: {old} -> {}", result.new_version);
            }
        }
        Ok(None) => {
            if !ctx.quiet {
                println!("hew is already at the latest version ({}).", env!("CARGO_PKG_VERSION"));
            }
        }
        Err(e) => {
            return Err(miette::miette!(
                "hew update failed: {e}. Install a newer release manually from https://github.com/droidnoob/hew/releases"
            ));
        }
    }

    if args.local && !ctx.quiet {
        eprintln!(
            "note: --local re-writes project skills via `hew init`. Run that next from the project root."
        );
    }

    Ok(())
}
