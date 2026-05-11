use clap::{Args as ClapArgs, CommandFactory, ValueEnum};
use clap_complete::Shell;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Target shell.
    #[arg(value_enum)]
    pub shell: ShellArg,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum ShellArg {
    Bash,
    Zsh,
    Fish,
    PowerShell,
    Elvish,
}

impl From<ShellArg> for Shell {
    fn from(s: ShellArg) -> Self {
        match s {
            ShellArg::Bash => Shell::Bash,
            ShellArg::Zsh => Shell::Zsh,
            ShellArg::Fish => Shell::Fish,
            ShellArg::PowerShell => Shell::PowerShell,
            ShellArg::Elvish => Shell::Elvish,
        }
    }
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    let mut cmd = crate::cli::Cli::command();
    let name = cmd.get_name().to_string();
    clap_complete::generate(Shell::from(args.shell), &mut cmd, name, &mut std::io::stdout());
    Ok(())
}
