use clap::Args as ClapArgs;
use hew_core::bd::RealBd;
use hew_core::doctor::{self, Severity};
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Attempt to auto-repair detected issues (only safe checks self-fix).
    #[arg(long)]
    pub fix: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let cwd = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;

    // Build a client if we can; pass a no-op fake if bd is missing so the
    // doctor still runs all the filesystem checks.
    let report = match RealBd::discover() {
        Ok(client) => doctor::run(&client, &cwd, args.fix),
        Err(_) => {
            let fake = MissingBd;
            doctor::run(&fake, &cwd, args.fix)
        }
    };

    if matches!(ctx.output, OutputMode::Json) {
        let payload = serde_json::json!({
            "overall": match report.overall() {
                Severity::Ok => "ok",
                Severity::Warn => "warn",
                Severity::Fail => "fail",
            },
            "checks": report.checks.iter().map(|c| serde_json::json!({
                "name": c.name,
                "severity": match c.severity {
                    Severity::Ok => "ok",
                    Severity::Warn => "warn",
                    Severity::Fail => "fail",
                },
                "message": c.message,
            })).collect::<Vec<_>>(),
            "fixed": report.fixed,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else {
        print!("{}", doctor::render_text(&report));
    }

    match report.overall() {
        Severity::Fail => Err(miette::miette!("doctor: one or more checks failed")),
        _ => Ok(()),
    }
}

#[derive(Debug)]
struct MissingBd;
impl hew_core::bd::BdClient for MissingBd {
    fn version(&self) -> hew_core::error::Result<hew_core::bd::BdVersion> {
        Err(hew_core::error::HewError::BdNotFound)
    }
    fn ready(&self) -> hew_core::error::Result<Vec<hew_core::bd::ReadyTask>> {
        Ok(vec![])
    }
    fn stats(&self) -> hew_core::error::Result<hew_core::bd::StatsSummary> {
        Ok(hew_core::bd::StatsSummary::default())
    }
    fn prime_raw(&self) -> hew_core::error::Result<String> {
        Ok(String::new())
    }
    fn memories(&self) -> hew_core::error::Result<std::collections::BTreeMap<String, String>> {
        Ok(Default::default())
    }
    fn remember(&self, _: &str) -> hew_core::error::Result<()> {
        Ok(())
    }
    fn run_raw(&self, _: &[&std::ffi::OsStr]) -> hew_core::error::Result<hew_core::bd::BdOutput> {
        Ok(hew_core::bd::BdOutput { stdout: String::new(), stderr: String::new() })
    }
}
