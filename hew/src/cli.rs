use clap::{Parser, Subcommand, ValueEnum};

const LONG_VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "\nbuilt: ",
    env!("VERGEN_BUILD_TIMESTAMP"),
    "\nrustc: ",
    env!("VERGEN_RUSTC_SEMVER"),
);

#[derive(Debug, Parser)]
#[command(
    name = "hew",
    version,
    long_version = LONG_VERSION,
    about = "Carve code, not chaos.",
    propagate_version = true,
    arg_required_else_help = true
)]
pub struct Cli {
    /// Disable all interactive prompts. Also: HEW_NON_INTERACTIVE=1, CI=true, or non-TTY stderr.
    #[arg(long, global = true)]
    pub non_interactive: bool,

    /// Output format. `auto` = json if stdout is piped, text otherwise.
    #[arg(long, global = true, value_enum, default_value_t = OutputArg::Auto)]
    pub output: OutputArg,

    /// JSON output (shorthand for --output=json).
    #[arg(long, global = true)]
    pub json: bool,

    /// Suppress non-essential output.
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Increase log verbosity (repeat for more).
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum OutputArg {
    Auto,
    Json,
    Text,
}

impl From<OutputArg> for hew_core::OutputMode {
    fn from(a: OutputArg) -> Self {
        match a {
            OutputArg::Auto => Self::Auto,
            OutputArg::Json => Self::Json,
            OutputArg::Text => Self::Text,
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize hew in the current directory.
    Init(crate::commands::init::Args),

    /// Emit JSON context for a skill (consumed by agents).
    Prime(crate::commands::prime::Args),

    /// Human-readable project state.
    Status,

    /// Diagnose hew + beads + project health.
    Doctor(crate::commands::doctor::Args),

    /// Read/write hew configuration.
    Config(crate::commands::config::Args),

    /// Print JSON schemas for hew outputs.
    Schema(crate::commands::schema::Args),

    /// Self-update binary + skill files.
    Update(crate::commands::update::Args),
}
