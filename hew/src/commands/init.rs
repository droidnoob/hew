use std::ffi::OsStr;
use std::path::PathBuf;

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::error::HewError;
use hew_core::install::{self, Runtime};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Issue prefix passed through to `bd init` (default: directory name).
    #[arg(long)]
    pub prefix: Option<String>,

    /// Track .beads/ in git (default: false — added to .gitignore).
    #[arg(long)]
    pub git_track: bool,

    /// Agent runtime to install for. Defaults to auto-detect.
    #[arg(long, value_enum)]
    pub runtime: Option<RuntimeArg>,

    /// Installation scope.
    #[arg(long, value_enum, default_value_t = Scope::Local)]
    pub scope: Scope,

    /// How to install `bd` if missing. Only `skip` is honored automatically;
    /// `brew`/`curl` print instructions and exit non-zero.
    #[arg(long, value_enum, default_value_t = InstallBd::Skip)]
    pub install_bd: InstallBd,

    /// Accept all defaults non-interactively.
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum RuntimeArg {
    Claude,
    Cursor,
    Codex,
    Windsurf,
    Generic,
}

impl From<RuntimeArg> for Runtime {
    fn from(a: RuntimeArg) -> Self {
        match a {
            RuntimeArg::Claude => Self::Claude,
            RuntimeArg::Cursor => Self::Cursor,
            RuntimeArg::Codex => Self::Codex,
            RuntimeArg::Windsurf => Self::Windsurf,
            RuntimeArg::Generic => Self::Generic,
        }
    }
}

#[derive(Debug, Copy, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum Scope {
    Local,
    Global,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum, PartialEq, Eq)]
pub enum InstallBd {
    Brew,
    Curl,
    Skip,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;

    let runtime = resolve_runtime(ctx, &args, &project_root)?;
    let install_root = resolve_install_root(args.scope, &project_root)?;

    let bd = ensure_bd(ctx, args.install_bd)?;
    run_bd_init(&bd, &project_root, args.prefix.as_deref())?;

    if !args.git_track {
        let _ = install::ensure_beads_gitignored(&project_root)
            .map_err(|e| miette::miette!("update .gitignore: {e}"))?;
    }

    let plan = install::install(runtime, &install_root)?;

    if !ctx.quiet {
        println!(
            "hew installed for {} ({:?} scope) → {} files under {}",
            plan.runtime.as_str(),
            args.scope,
            plan.written.len(),
            plan.root.display()
        );
    }
    Ok(())
}

fn resolve_runtime(
    ctx: &Ctx,
    args: &Args,
    project_root: &std::path::Path,
) -> miette::Result<Runtime> {
    if let Some(r) = args.runtime {
        return Ok(r.into());
    }
    let detected = install::detect_runtimes(project_root);

    match detected.as_slice() {
        [single] => Ok(*single),
        [] => {
            if !ctx.interactive {
                return Err(HewError::MissingFlag { flag: "runtime".into() }.into());
            }
            // Interactive path: ask. inquire blocks; non_interactive guard above
            // means we only get here with a real human.
            interactive_runtime_pick()
        }
        many => {
            if !ctx.interactive {
                return Err(HewError::MissingFlag {
                    flag: format!(
                        "runtime (multiple detected: {})",
                        many.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
                    ),
                }
                .into());
            }
            interactive_runtime_pick()
        }
    }
}

fn interactive_runtime_pick() -> miette::Result<Runtime> {
    use inquire::Select;
    let opts = vec!["claude", "cursor", "codex", "windsurf", "generic"];
    let pick = Select::new("Which agent runtime?", opts)
        .prompt()
        .map_err(|e| miette::miette!("runtime pick: {e}"))?;
    Ok(match pick {
        "claude" => Runtime::Claude,
        "cursor" => Runtime::Cursor,
        "codex" => Runtime::Codex,
        "windsurf" => Runtime::Windsurf,
        _ => Runtime::Generic,
    })
}

fn resolve_install_root(scope: Scope, project_root: &std::path::Path) -> miette::Result<PathBuf> {
    match scope {
        Scope::Local => Ok(project_root.to_path_buf()),
        Scope::Global => {
            use etcetera::BaseStrategy;
            // ~/ — adapters lay down their own subpaths beneath this root.
            let strategy = etcetera::choose_base_strategy()
                .map_err(|e| miette::miette!("home strategy: {e}"))?;
            Ok(strategy.home_dir().to_path_buf())
        }
    }
}

fn ensure_bd(ctx: &Ctx, mode: InstallBd) -> miette::Result<RealBd> {
    match RealBd::discover() {
        Ok(bd) => Ok(bd),
        Err(HewError::BdNotFound) => {
            let hint = match mode {
                InstallBd::Brew => "run `brew install beads` and re-run hew init",
                InstallBd::Curl => "run `curl -sSL https://beads.sh/install | sh` and re-run",
                InstallBd::Skip => "install `bd` (https://gastownhall.github.io/beads/) and re-run",
            };
            if !ctx.quiet {
                eprintln!("hew init needs `bd` on PATH — {hint}");
            }
            Err(HewError::BdNotFound.into())
        }
        Err(e) => Err(e.into()),
    }
}

fn run_bd_init(
    bd: &RealBd,
    project_root: &std::path::Path,
    prefix: Option<&str>,
) -> miette::Result<()> {
    if project_root.join(".beads").exists() {
        return Ok(()); // already initialized
    }
    let mut args: Vec<&OsStr> = vec![OsStr::new("init"), OsStr::new("--non-interactive")];
    let pfx_os: std::ffi::OsString;
    if let Some(p) = prefix {
        args.push(OsStr::new("--prefix"));
        pfx_os = std::ffi::OsString::from(p);
        args.push(pfx_os.as_os_str());
    }
    bd.run_raw(&args)?;
    Ok(())
}
