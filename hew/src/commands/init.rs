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

    /// How to install `bd` if missing. `brew` runs `brew install beads`,
    /// `curl` pipes the beads.sh installer through sh, `skip` errors out
    /// asking the user to install Beads themselves.
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
    if let Ok(bd) = RealBd::discover() {
        return Ok(bd);
    }

    // bd is missing. Honor --install-bd by actually installing.
    if !ctx.quiet {
        eprintln!("hew init: `bd` not on PATH.");
    }
    match mode {
        InstallBd::Brew => install_via_brew(ctx)?,
        InstallBd::Curl => install_via_curl(ctx)?,
        InstallBd::Skip => {
            if !ctx.quiet {
                eprintln!(
                    "  -> skipping auto-install. Install Beads and re-run, or pass --install-bd=brew|curl."
                );
                eprintln!("     docs: https://gastownhall.github.io/beads/");
            }
            return Err(HewError::BdNotFound.into());
        }
    }

    RealBd::discover().map_err(|_| {
        miette::miette!(
            "auto-install ran but `bd` still isn't on PATH. Try opening a new shell so PATH refreshes, then re-run `hew init`."
        )
    })
}

fn install_via_brew(ctx: &Ctx) -> miette::Result<()> {
    if which::which("brew").is_err() {
        return Err(miette::miette!(
            "--install-bd=brew but `brew` isn't on PATH. Install Homebrew first (https://brew.sh) or pass --install-bd=curl."
        ));
    }
    if !ctx.quiet {
        eprintln!("  -> brew install beads");
    }
    run_streaming(std::process::Command::new("brew").args(["install", "beads"]))
        .map_err(|e| miette::miette!("brew install beads failed: {e}"))
}

fn install_via_curl(ctx: &Ctx) -> miette::Result<()> {
    if which::which("curl").is_err() {
        return Err(miette::miette!("--install-bd=curl but `curl` isn't on PATH."));
    }
    if which::which("sh").is_err() {
        return Err(miette::miette!("--install-bd=curl but `sh` isn't on PATH."));
    }
    if !ctx.quiet {
        eprintln!("  -> curl -sSL https://beads.sh/install | sh");
    }
    // Pipe curl into sh. Manual two-stage so we can stream both stdouts.
    let mut curl = std::process::Command::new("curl")
        .args(["-sSL", "https://beads.sh/install"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| miette::miette!("spawn curl: {e}"))?;
    let curl_stdout = curl.stdout.take().expect("piped");
    let sh_status = std::process::Command::new("sh")
        .stdin(curl_stdout)
        .status()
        .map_err(|e| miette::miette!("spawn sh: {e}"))?;
    let curl_status = curl.wait().map_err(|e| miette::miette!("wait curl: {e}"))?;
    if !curl_status.success() {
        return Err(miette::miette!("curl exited {:?}", curl_status.code()));
    }
    if !sh_status.success() {
        return Err(miette::miette!("beads install script exited {:?}", sh_status.code()));
    }
    Ok(())
}

fn run_streaming(cmd: &mut std::process::Command) -> std::io::Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!("exit {:?}", status.code())));
    }
    Ok(())
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
