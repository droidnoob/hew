//! hew-core — pure logic for the hew CLI.
//!
//! Keep this crate free of clap/inquire/tracing-subscriber so it stays
//! easy to unit test. The `hew` binary wires presentation on top.

pub mod bd;
pub mod branch;
pub mod config;
pub mod craft;
pub mod ctx;
pub mod doctor;
pub mod error;
pub mod git;
pub mod install;
pub mod notify;
pub mod os;
pub mod prime;
pub mod review;
pub mod skills;
pub mod slash;
pub mod stacks;
pub mod status;
pub mod tasks;
pub mod tty;

pub use ctx::{Ctx, OutputMode};
pub use error::HewError;
