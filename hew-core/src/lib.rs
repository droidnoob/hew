//! hew-core — pure logic for the hew CLI.
//!
//! Keep this crate free of clap/inquire/tracing-subscriber so it stays
//! easy to unit test. The `hew` binary wires presentation on top.

pub mod bd;
pub mod ctx;
pub mod error;
pub mod install;
pub mod prime;
pub mod skills;
pub mod tty;

pub use ctx::{Ctx, OutputMode};
pub use error::HewError;
