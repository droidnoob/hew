use clap::Args as ClapArgs;
use hew_core::install::{self, Runtime};
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Re-write project-local skill files only — skips the binary
    /// self-updater entirely. Useful when the binary is fine but the
    /// `.claude/skills/hew/` (or `.cursorrules` / `AGENTS.md` etc.) bundle
    /// has drifted from the binary's version.
    #[arg(long)]
    pub local: bool,

    /// Check for a newer release without updating the binary. Prints
    /// text by default; use the global `--json` for the structured
    /// `{current, update_available, error?}` payload.
    #[arg(long)]
    pub check_only: bool,

    /// Accept the update prompt non-interactively.
    #[arg(short, long)]
    pub yes: bool,
}

/// Manual-install hint shown whenever the bundled self-updater can't run.
/// Spells out the canonical install methods so the user is never stuck
/// at an empty "install manually" instruction.
const MANUAL_INSTALL_HINT: &str = "\
Install or upgrade manually using one of:
  • brew install droidnoob/hew/hew         (homebrew tap)
  • cargo install --git https://github.com/droidnoob/hew hew
  • Download a binary from https://github.com/droidnoob/hew/releases

Then re-run `hew update --local` to refresh project-local skill files.";

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    // `--local` short-circuits before any axoupdater call: refresh skill
    // files for the detected runtime(s) without touching the binary.
    // See GH issue #19 — the previous behavior coupled --local to a
    // working self-updater, leaving users stuck when the updater
    // wasn't configured for their install method.
    if args.local {
        return refresh_local_skills(ctx);
    }

    if args.check_only {
        return run_check_only(ctx);
    }

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
                println!();
                println!("Run `hew update --local` from each project root to refresh skill files.");
            }
        }
        Ok(None) => {
            if !ctx.quiet {
                println!("hew is already at the latest version ({}).", env!("CARGO_PKG_VERSION"));
            }
        }
        Err(e) => {
            return Err(miette::miette!("hew update failed: {e}.\n\n{MANUAL_INSTALL_HINT}"));
        }
    }

    Ok(())
}

fn run_check_only(ctx: &Ctx) -> miette::Result<()> {
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

    let current = env!("CARGO_PKG_VERSION");
    let want_json = matches!(ctx.output, OutputMode::Json);
    let upgrade_available = match updater.is_update_needed_sync() {
        Ok(v) => v,
        Err(e) => {
            if want_json {
                let payload = serde_json::json!({
                    "current": current,
                    "update_available": false,
                    "error": e.to_string(),
                });
                println!("{}", serde_json::to_string_pretty(&payload).unwrap());
            } else {
                println!("hew {current} — update check failed: {e}");
                println!();
                println!("{MANUAL_INSTALL_HINT}");
            }
            return Ok(());
        }
    };
    if want_json {
        let payload = serde_json::json!({
            "current": current,
            "update_available": upgrade_available,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else if upgrade_available {
        println!("hew {current} — update available. Run `hew update` to upgrade.");
    } else {
        println!("hew {current} — up to date.");
    }
    Ok(())
}

/// Refresh skill files for every runtime already installed under
/// `cwd/`. Detects via `install::detect_runtimes` so users don't have
/// to remember which runtime they picked at init time.
fn refresh_local_skills(ctx: &Ctx) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;
    let detected: Vec<Runtime> = install::detect_runtimes(&project_root);

    if detected.is_empty() {
        return Err(miette::miette!(
            "no runtime markers found under {}.\n\nRun `hew init` first to install for a specific \
             runtime (claude, cursor, codex, windsurf). `hew update --local` only refreshes existing \
             installations.",
            project_root.display()
        ));
    }

    if !ctx.quiet {
        let names: Vec<&'static str> = detected.iter().map(|r| r.as_str()).collect();
        println!("hew update --local: refreshing skills for {}", names.join(", "));
    }

    let mut total_written = 0usize;
    for runtime in detected {
        let plan = install::install(runtime, &project_root)?;
        total_written += plan.written.len();
        if !ctx.quiet {
            println!(
                "  ✓ {}: {} file(s) under {}",
                plan.runtime.as_str(),
                plan.written.len(),
                plan.root.display()
            );
        }
    }
    if !ctx.quiet {
        println!();
        println!(
            "Refreshed {total_written} file(s) total. Binary stayed at {}.",
            env!("CARGO_PKG_VERSION")
        );
    }
    Ok(())
}
