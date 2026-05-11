use clap::Parser;
use hew::cli::Cli;

fn main() -> miette::Result<()> {
    miette::set_panic_hook();
    hew::tracing_init();
    let cli = Cli::parse();
    hew::run(cli)
}
