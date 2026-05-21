use hew_core::Ctx;

use crate::cli::{Cli, Command};

pub mod branch;
pub mod check;
pub mod compact;
pub mod completions;
pub mod config;
pub mod dep;
pub mod doctor;
pub mod epic;
pub mod forget;
pub mod init;
pub mod manpage;
pub mod memories;
pub mod next;
pub mod prime;
pub mod remember;
pub mod review;
pub mod schema;
pub mod skills;
pub mod slashes;
pub mod status;
pub mod task;
pub mod uninstall;
pub mod update;

pub fn dispatch(cli: Cli) -> miette::Result<()> {
    let want_json = cli.json || matches!(cli.output, crate::cli::OutputArg::Json);
    let output = if want_json { hew_core::OutputMode::Json } else { hew_core::OutputMode::Text };
    let ctx = Ctx::new(cli.non_interactive, output, cli.quiet, cli.verbose);

    match cli.command {
        Command::Init(a) => init::run(&ctx, a),
        Command::Prime(a) => prime::run(&ctx, a),
        Command::Status => status::run(&ctx, ()),
        Command::Doctor(a) => doctor::run(&ctx, a),
        Command::Config(a) => config::run(&ctx, a),
        Command::Schema(a) => schema::run(&ctx, a),
        Command::Update(a) => update::run(&ctx, a),
        Command::Completions(a) => completions::run(&ctx, a),
        Command::Manpage => manpage::run(&ctx, ()),
        Command::Check(a) => check::run(&ctx, a),
        Command::Skills(a) => skills::run(&ctx, a),
        Command::Commands => slashes::run(&ctx, ()),
        Command::Memories(a) => memories::run(&ctx, a),
        Command::Uninstall(a) => uninstall::run(&ctx, a),
        Command::Branch(a) => branch::run(&ctx, a),
        Command::Review(a) => review::run(&ctx, a),
        Command::Task(a) => task::run(&ctx, a),
        Command::Dep(a) => dep::run(&ctx, a),
        Command::Remember(a) => remember::run(&ctx, a),
        Command::Forget(a) => forget::run(&ctx, a),
        Command::Epic(a) => epic::run(&ctx, a),
        Command::Compact(a) => compact::run(&ctx, a),
        Command::Ready(a) => next::run_ready(&ctx, a),
        Command::Next(a) => next::run_next(&ctx, a),
    }
}
