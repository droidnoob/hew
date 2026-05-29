//! hew-core — pure logic for the hew CLI.
//!
//! Keep this crate free of clap/inquire/tracing-subscriber so it stays
//! easy to unit test. The `hew` binary wires presentation on top.

pub mod allowed_tools;
pub mod backpressure;
pub mod bd;
#[cfg(feature = "treesitter")]
pub mod blast;
pub mod branch;
pub mod checkpoint;
pub mod compact;
pub mod config;
pub mod craft;
pub mod ctx;
pub mod decide;
pub mod diff_hunks;
pub mod dispatcher;
pub mod doctor;
pub mod error;
pub mod external_gate;
pub mod gate;
pub mod git;
pub mod guard;
pub mod install;
pub mod loop_log;
pub mod loop_summary;
pub mod memories;
pub mod notify;
pub mod os;
pub mod prime;
pub(crate) mod process;
pub mod prompt;
pub mod review;
pub mod runner;
pub mod runtime;
pub mod skills;
pub mod slash;
pub mod stacks;
pub mod status;
pub mod statusline;
pub mod stop_signals;
pub mod tasks;
#[cfg(unix)]
pub mod testing;
pub mod time;
pub mod treesitter;
pub mod tty;
pub mod worktree;

pub use ctx::{Ctx, OutputMode};
pub use error::HewError;
