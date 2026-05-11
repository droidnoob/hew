use clap::CommandFactory;
use clap_mangen::Man;
use hew_core::Ctx;

pub fn run(_ctx: &Ctx, _: ()) -> miette::Result<()> {
    let cmd = crate::cli::Cli::command();
    let man = Man::new(cmd);
    man.render(&mut std::io::stdout()).map_err(|e| miette::miette!("write manpage: {e}"))?;
    Ok(())
}
