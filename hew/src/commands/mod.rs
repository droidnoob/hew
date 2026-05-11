use hew_core::Ctx;

use crate::cli::{Cli, Command};

pub mod config;
pub mod doctor;
pub mod init;
pub mod prime;
pub mod schema;
pub mod status;
pub mod update;

pub fn dispatch(cli: Cli) -> miette::Result<()> {
    let output = if cli.json { hew_core::OutputMode::Json } else { cli.output.into() };
    let ctx = Ctx::new(cli.non_interactive, output, cli.quiet, cli.verbose);

    match cli.command {
        Command::Init(a) => init::run(&ctx, a),
        Command::Prime(a) => prime::run(&ctx, a),
        Command::Status => status::run(&ctx),
        Command::Doctor(a) => doctor::run(&ctx, a),
        Command::Config(a) => config::run(&ctx, a),
        Command::Schema(a) => schema::run(&ctx, a),
        Command::Update(a) => update::run(&ctx, a),
    }
}
