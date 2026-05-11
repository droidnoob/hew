use clap::Args as ClapArgs;
use hew_core::Ctx;

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
    pub runtime: Option<Runtime>,

    /// Installation scope.
    #[arg(long, value_enum, default_value_t = Scope::Local)]
    pub scope: Scope,

    /// Project type. Defaults to detection (empty dir = new, otherwise existing).
    #[arg(long, value_enum)]
    pub project_type: Option<ProjectType>,

    /// How to install `bd` if missing.
    #[arg(long, value_enum, default_value_t = InstallBd::Skip)]
    pub install_bd: InstallBd,

    /// Accept all interactive defaults.
    #[arg(short, long)]
    pub yes: bool,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum Runtime {
    Claude,
    Cursor,
    Codex,
    Windsurf,
    Generic,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum Scope {
    Local,
    Global,
    Both,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum ProjectType {
    New,
    Existing,
}

#[derive(Debug, Copy, Clone, clap::ValueEnum)]
pub enum InstallBd {
    Brew,
    Curl,
    Skip,
}

pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    miette::bail!("`hew init` is not yet implemented (tracked: hew-3xq.2.3)");
}
