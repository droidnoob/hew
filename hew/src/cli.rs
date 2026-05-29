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

    /// Print shell completions to stdout.
    Completions(crate::commands::completions::Args),

    /// Print a man page (roff) to stdout.
    Manpage,

    /// Check whether a skill's prerequisites are met. Exit 0 if met, 1 if not.
    Check(crate::commands::check::Args),

    /// List installed skills (with --category to filter).
    Skills(crate::commands::skills::Args),

    /// List installed slash commands.
    Commands,

    /// List bd memories, optionally filtered by prefix or grep substring.
    Memories(crate::commands::memories::Args),

    /// Reverse `hew init` — remove skills + slash commands for this project.
    /// `--purge` also deletes `.beads/` (drops the task graph + memories).
    Uninstall(crate::commands::uninstall::Args),

    /// Branch operations (`hew branch new --prefix=feat --slug='Add Auth'`).
    Branch(crate::commands::branch::Args),

    /// Review utilities: bundle JSON for the review skills + Step 10 trigger check.
    Review(crate::commands::review::Args),

    /// Task operations: show, list, claim, close, new, reopen, children, note, search.
    Task(crate::commands::task::Args),

    /// Dependency operations: add, remove, tree, blocked.
    Dep(crate::commands::dep::Args),

    /// Write a memory with an enforced type allowlist (or --raw to bypass).
    Remember(crate::commands::remember::Args),

    /// Write a canonical `CHECKPOINT:<ISO> — <body>` memory in one shot.
    /// Auto-generates the timestamp + key so `hew prime resume` always
    /// finds the newest. Replaces the foot-gunny `hew remember --raw
    /// "CHECKPOINT:…" --key …` flow the skill used to recommend.
    Checkpoint(crate::commands::checkpoint::Args),

    /// Forget a memory by key. Alias for `hew memories --forget <KEY>`;
    /// later epic work (ML.6 cascade) will extend this surface with
    /// automatic purge of outbound LINK: rows.
    Forget(crate::commands::forget::Args),

    /// Epic operations: show, tree, close, audit, summary.
    Epic(crate::commands::epic::Args),

    /// Memory compaction: apply a CompactPlan from stdin, or survey
    /// per-prefix memory counts. See skills/optional/hew-compact.md.
    Compact(crate::commands::compact::Args),

    /// List unblocked tasks (mirrors `bd ready`).
    Ready(crate::commands::next::ReadyArgs),

    /// Pick the top ready task. Claims by default; `--no-claim` to peek.
    Next(crate::commands::next::NextArgs),

    /// Emit a one-line agent statusline summarizing project state.
    /// Consumed by Claude Code's `statusLine` hook. Stdout is the line
    /// itself; errors and noise go to stderr. Exits 0 with empty stdout
    /// when bd isn't initialized so the host falls back gracefully.
    Statusline(crate::commands::statusline::Args),

    /// Symbol-level changelog of the current branch vs the base ref.
    /// Walks `git diff --unified=0 <base>...HEAD`, intersects each
    /// touched line range with tree-sitter-extracted symbols, and
    /// prints the symbols whose definitions overlap a diff hunk.
    ///
    /// Requires the `treesitter` build feature.
    Blast(crate::commands::blast::Args),

    /// Autonomous outer loop: `hew loop run` drives the queue; `hew
    /// loop cancel` / `logs` / `list` inspect or stop running and
    /// completed runs.
    Loop(crate::commands::loop_cmd::LoopCmd),

    /// External-state gates: create a bd task that resolves when an
    /// external condition fires (currently: a GitHub PR being merged).
    /// `hew gate new --gh-pr=N --title="..."`, `hew gate poll`,
    /// `hew gate list`. Pair with `hew dep add <next-epic> <gate-id>`
    /// to block downstream work on the gate.
    Gate(crate::commands::gate::Args),
}
