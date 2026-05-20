//! `hew update` — upgrade the installed binary, then refresh skill files.
//!
//! Routes by install source ([`InstallSource`] from `current_exe`) so
//! every distribution channel uses its native upgrade tool:
//!
//! - **Brew** → `brew upgrade hew` (the only path that touches a brew install)
//! - **Cargo** → `cargo install --git https://github.com/droidnoob/hew hew --force`
//! - **Dev build** → refuse; print "cargo build" hint
//! - **Unknown** → fall through to axoupdater (curl-installer users), with
//!   the manual-install hint on any failure
//!
//! After a successful upgrade in any path, the *new* `hew` on PATH is
//! invoked with `update --local` so freshly-installed skill bodies land
//! in cwd's `.claude/skills/hew/` (etc.). Without that step a brew
//! upgrade would swap the binary but leave projects running stale skill
//! bodies indefinitely.
//!
//! Background: cargo-dist's `install-updater` knob is off (see
//! `Cargo.toml`), so axoupdater never finds an install receipt for any
//! channel we ship. PR #24 / hew-rr8 patched `hew update --local`; this
//! is the wider fix for the bare `hew update` path (hew-lv2).

use std::process::Command;

use clap::Args as ClapArgs;
use hew_core::install::{self, InstallSource, Runtime};
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

    /// After upgrading the binary, skip the auto re-exec that refreshes
    /// project-local skill files. Use when running `hew update` outside
    /// any project root and you don't want a "no runtimes found" warning.
    #[arg(long)]
    pub no_refresh: bool,
}

const MANUAL_INSTALL_HINT: &str = "\
Install or upgrade manually using one of:
  • brew install droidnoob/hew/hew         (homebrew tap)
  • cargo install --git https://github.com/droidnoob/hew hew --force
  • Download a binary from https://github.com/droidnoob/hew/releases

Then re-run `hew update --local` to refresh project-local skill files.";

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    if args.local {
        return refresh_local_skills(ctx);
    }

    if args.check_only {
        return run_check_only(ctx);
    }

    let source = install::detect_install_source();
    if !ctx.quiet {
        eprintln!("hew update: detected install source = {}", source.as_str());
    }

    match source {
        InstallSource::Brew => upgrade_via_brew(ctx)?,
        InstallSource::Cargo => upgrade_via_cargo(ctx)?,
        InstallSource::Dev => {
            return Err(miette::miette!(
                "this binary lives under `target/{{debug,release}}` — `hew update` won't \
                 upgrade a dev build. Run `cargo build --release` or `cargo install --path hew` \
                 instead."
            ));
        }
        InstallSource::Unknown => upgrade_via_axoupdater(ctx, args.yes)?,
    }

    if !args.no_refresh {
        reexec_local_refresh(ctx);
    } else if !ctx.quiet {
        println!();
        println!("Run `hew update --local` from each project root to refresh skill files.");
    }

    Ok(())
}

// ───────────────────────────────────────────────────────────────────────
// Routed upgrades
// ───────────────────────────────────────────────────────────────────────

fn upgrade_via_brew(ctx: &Ctx) -> miette::Result<()> {
    if !ctx.quiet {
        println!("running `brew upgrade hew`...");
    }
    let status = Command::new("brew").args(["upgrade", "hew"]).status().map_err(|e| {
        miette::miette!("failed to invoke `brew upgrade hew`: {e}.\n\n{MANUAL_INSTALL_HINT}")
    })?;
    if !status.success() {
        return Err(miette::miette!(
            "`brew upgrade hew` exited with status {status}.\n\n{MANUAL_INSTALL_HINT}"
        ));
    }
    Ok(())
}

fn upgrade_via_cargo(ctx: &Ctx) -> miette::Result<()> {
    if !ctx.quiet {
        println!("running `cargo install --git https://github.com/droidnoob/hew hew --force`...");
    }
    let status = Command::new("cargo")
        .args(["install", "--git", "https://github.com/droidnoob/hew", "hew", "--force"])
        .status()
        .map_err(|e| {
            miette::miette!("failed to invoke `cargo install`: {e}.\n\n{MANUAL_INSTALL_HINT}")
        })?;
    if !status.success() {
        return Err(miette::miette!(
            "`cargo install` exited with status {status}.\n\n{MANUAL_INSTALL_HINT}"
        ));
    }
    Ok(())
}

fn upgrade_via_axoupdater(ctx: &Ctx, yes: bool) -> miette::Result<()> {
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

    if !yes && ctx.interactive && !ctx.quiet {
        eprintln!("hew update: checking for a newer release on GitHub...");
    }

    match updater.run_sync() {
        Ok(Some(result)) => {
            if !ctx.quiet {
                let old =
                    result.old_version.map(|v| v.to_string()).unwrap_or_else(|| "?".to_string());
                println!("hew updated: {old} -> {}", result.new_version);
            }
            Ok(())
        }
        Ok(None) => {
            if !ctx.quiet {
                println!("hew is already at the latest version ({}).", env!("CARGO_PKG_VERSION"));
            }
            Ok(())
        }
        Err(e) => Err(miette::miette!("hew update failed: {e}.\n\n{MANUAL_INSTALL_HINT}")),
    }
}

// ───────────────────────────────────────────────────────────────────────
// --check-only (read-only via GitHub releases API)
// ───────────────────────────────────────────────────────────────────────

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

// ───────────────────────────────────────────────────────────────────────
// Skill refresh (--local and the post-upgrade re-exec)
// ───────────────────────────────────────────────────────────────────────

/// After a successful binary upgrade, re-exec the newly-installed `hew`
/// to refresh skill files in cwd. We can't rewrite skills from the
/// current process — its bundled `install::install` payload was frozen
/// at the *old* version's compile time. The fresh binary on PATH has
/// the new skill bodies baked in.
///
/// Best-effort: failure here doesn't fail the upgrade. We print a hint
/// so the user can run `hew update --local` themselves.
fn reexec_local_refresh(ctx: &Ctx) {
    let project_root = match std::env::current_dir() {
        Ok(p) => p,
        Err(_) => return,
    };
    if install::detect_runtimes(&project_root).is_empty() {
        if !ctx.quiet {
            println!();
            println!(
                "Binary upgraded. No runtime markers in cwd; skipping skill refresh. Run \
                 `hew update --local` from each project root to refresh skill files."
            );
        }
        return;
    }

    if !ctx.quiet {
        println!();
        println!("refreshing project skill files via newly-installed `hew update --local`...");
    }
    let result = Command::new("hew").args(["update", "--local"]).status();
    match result {
        Ok(s) if s.success() => {}
        Ok(s) => {
            if !ctx.quiet {
                eprintln!(
                    "warning: `hew update --local` exited {s}. Run it manually to refresh skill \
                     files."
                );
            }
        }
        Err(e) => {
            if !ctx.quiet {
                eprintln!(
                    "warning: couldn't auto-invoke `hew update --local` ({e}). Run it manually to \
                     refresh skill files."
                );
            }
        }
    }
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
