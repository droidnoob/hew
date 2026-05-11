pub mod cli;
pub mod commands;

use tracing_subscriber::EnvFilter;

pub fn tracing_init() {
    let filter = EnvFilter::try_from_env("HEW_LOG").unwrap_or_else(|_| EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

pub fn run(cli: cli::Cli) -> miette::Result<()> {
    commands::dispatch(cli)
}
