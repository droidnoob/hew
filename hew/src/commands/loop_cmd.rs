//! `hew loop` — process-level outer harness that drains the ready
//! queue by repeatedly invoking the agent runtime (Claude Code).
//!
//! The runner orchestrator, prompt assembler, runtime spawner,
//! backpressure gate, stop-signal collector and per-iter logger live in
//! `hew_core`. This module is the thin CLI layer: parse args, wire the
//! pieces, drive the loop, emit a final summary.
//!
//! v1 ships full coverage of `--dry-run` (no subprocess, no git ops;
//! exercises prompt assembly + iter logging). Real spawn requires the
//! Claude Code CLI on PATH (or `HEW_LOOP_CLAUDE_BIN`); when present, a
//! plain `hew loop --max-iter 1` runs one real iter against the top of
//! the ready queue and writes its iter log. SIGINT handling and the
//! `--interactive` ask-file flow are stubbed pending hew-bif.

use std::path::PathBuf;
use std::time::Duration;

use clap::Args as ClapArgs;
use hew_core::bd::{BdClient, RealBd};
use hew_core::loop_log::{
    IterLog, RunLog, iter_log_path, new_run_id, run_dir, run_log_path, stop_file_path,
    write_json_atomic,
};
use hew_core::prompt;
use hew_core::runner::{Iter, IterOutcome, ResearchBudget, Run, RunConfig};
use hew_core::runtime::{ClaudeSpawner, RuntimeSpawner};
use hew_core::stop_signals::Collector;
use hew_core::time::iso_now_utc;
use hew_core::{Ctx, allowed_tools, skills};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Hard cap on iterations. Omit for unlimited (stop via other signals).
    #[arg(long)]
    pub max_iter: Option<u32>,

    /// Stop when the ready queue drains. Default true; pass
    /// `--no-until-empty` to drive the loop purely off other stops.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub until_empty: bool,

    /// Cumulative token budget across all iters. Omit for unlimited.
    #[arg(long)]
    pub budget_tokens: Option<u64>,

    /// Wall-clock budget, e.g. `30m`, `2h`. Omit for unlimited.
    #[arg(long, value_parser = parse_duration)]
    pub budget_wall: Option<Duration>,

    /// Research budget per iter, formatted `<web>+<fetch>` (default `5+3`).
    #[arg(long, value_parser = parse_research_budget, default_value = "5+3")]
    pub research_budget: ResearchBudget,

    /// Promote craft warnings to failures. Default on; `--no-strict` opts out.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub strict: bool,

    /// Pause on ask-files for operator input. Default off; the v1 wiring
    /// is stubbed (hew-bif) — passing `--interactive` is honored in the
    /// run config but doesn't yet drive any prompts.
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    pub interactive: bool,

    /// Runtime to drive. Only `claude` is wired in v1.
    #[arg(long, default_value = "claude")]
    pub runtime: String,

    /// Override the stop-file path. Defaults to `<run-dir>/.stop`.
    #[arg(long)]
    pub stop_file: Option<PathBuf>,

    /// Assemble prompts + log iters without spawning the runtime.
    #[arg(long)]
    pub dry_run: bool,

    /// Skill name used to assemble per-iter prompts. Defaults to
    /// `hew-execute` — the methodology body the loop drives.
    #[arg(long, default_value = "hew-execute")]
    pub skill: String,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    if args.runtime != "claude" {
        return Err(miette::miette!(
            "unsupported runtime `{}`; only `claude` is wired in v1",
            args.runtime
        ));
    }

    let project_root = std::env::current_dir().map_err(|e| miette::miette!("resolve cwd: {e}"))?;
    let bd = RealBd::discover().map_err(|e| miette::miette!("bd discover: {e}"))?;
    let skill = skills::find(&args.skill)
        .ok_or_else(|| miette::miette!("unknown skill `{}`", args.skill))?;

    let cfg = RunConfig {
        max_iter: args.max_iter,
        stop_on_ready_empty: args.until_empty,
        budget_tokens: args.budget_tokens,
        budget_wall: args.budget_wall,
        research_budget: args.research_budget,
        strict: args.strict,
        interactive: args.interactive,
    };

    let run_id = new_run_id();
    let dir =
        run_dir(&project_root, &run_id).map_err(|e| miette::miette!("create run dir: {e}"))?;
    let stop_path = args.stop_file.unwrap_or_else(|| stop_file_path(&dir));
    let collector = Collector::new(stop_path);
    let mut run_state = Run::new(run_id.clone(), iso_now_utc(), cfg.clone());

    if !ctx.quiet {
        eprintln!("hew loop {} — run-dir={}", &run_id, dir.display());
        if args.dry_run {
            eprintln!("(--dry-run: no subprocess, no git ops)");
        }
    }

    let spawner: Option<ClaudeSpawner> =
        if args.dry_run { None } else { Some(ClaudeSpawner::from_env()) };

    let allowed = allowed_tools::for_skill(&args.skill);
    let mut last_outcome: Option<IterOutcome> = None;

    loop {
        let ready = bd.ready().map_err(|e| miette::miette!("bd ready: {e}"))?;
        let signals = collector.snapshot(&run_state, ready.len() as u32, last_outcome);
        if let Some(reason) = signals.evaluate(&cfg) {
            run_state.stop_reason = Some(reason);
            break;
        }

        let task = match ready.into_iter().next() {
            Some(t) => t,
            None => {
                run_state.stop_reason = Some(hew_core::runner::StopReason::ReadyEmpty);
                break;
            }
        };

        let iter_number = run_state.next_iter_number();
        let started_at = iso_now_utc();
        let mut iter = Iter::new(iter_number, &started_at);
        iter.task_id = Some(task.id.clone());

        let primer_text = format!(
            "task: {}\ntitle: {}\npriority: P{}\nstatus: {}\n",
            task.id, task.title, task.priority, task.status
        );
        let task_brief =
            format!("Drive task `{}` to close per the {} skill body.", task.id, args.skill);
        let assembled = prompt::assemble(skill.body, &primer_text, &task_brief);

        if !ctx.quiet {
            eprintln!(
                "iter {} — task={} prefix_hash={:016x} est_tokens={}",
                iter_number, task.id, assembled.prefix_hash, assembled.token_estimate
            );
        }

        let (outcome, tokens, stderr_tail) = if let Some(s) = spawner.as_ref() {
            match s.spawn(&assembled, &allowed) {
                Ok(out) => {
                    let oc = if out.success && out.closed_task.is_some() {
                        IterOutcome::Closed
                    } else if out.success {
                        IterOutcome::NoClose
                    } else {
                        IterOutcome::RuntimeError
                    };
                    (oc, out.tokens, Some(out.stderr_tail))
                }
                Err(e) => {
                    if !ctx.quiet {
                        eprintln!("iter {iter_number} runtime error: {e}");
                    }
                    (IterOutcome::RuntimeError, Default::default(), Some(format!("{e}")))
                }
            }
        } else {
            (IterOutcome::NoClose, Default::default(), None)
        };

        iter.outcome = Some(outcome);
        iter.cost = tokens;
        iter.ended_at = Some(iso_now_utc());
        iter.stderr_tail = stderr_tail;

        let prefix_hash_hex = Some(format!("{:016x}", assembled.prefix_hash));
        let log = IterLog::from_iter(&iter, prefix_hash_hex, Vec::new());
        write_json_atomic(&iter_log_path(&dir, iter_number), &log)
            .map_err(|e| miette::miette!("write iter log: {e}"))?;

        last_outcome = Some(outcome);
        run_state.iters.push(iter);

        // Rewrite run.json after each iter.
        write_json_atomic(&run_log_path(&dir), &RunLog::from_run(&run_state))
            .map_err(|e| miette::miette!("write run log: {e}"))?;
    }

    // Final summary.
    let summary = RunLog::from_run(&run_state);
    write_json_atomic(&run_log_path(&dir), &summary)
        .map_err(|e| miette::miette!("write final run log: {e}"))?;

    print_summary(ctx, &run_state, &dir);
    Ok(())
}

fn print_summary(ctx: &Ctx, run: &Run, dir: &std::path::Path) {
    if ctx.quiet {
        return;
    }
    let stop = run.stop_reason.map(|r| format!("{r:?}")).unwrap_or_else(|| "(none)".to_string());
    println!("hew loop summary");
    println!("  run-id: {}", run.id);
    println!("  iters:  {}", run.iters.len());
    println!("  tokens: {}", run.cumulative_tokens());
    println!("  stop:   {stop}");
    println!("  logs:   {}", dir.display());
}

fn parse_duration(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let (num_str, unit) = s.split_at(s.len() - 1);
    let n: u64 = num_str.parse().map_err(|e| format!("invalid number `{num_str}`: {e}"))?;
    match unit {
        "s" => Ok(Duration::from_secs(n)),
        "m" => Ok(Duration::from_secs(n * 60)),
        "h" => Ok(Duration::from_secs(n * 3600)),
        other => Err(format!("unknown duration unit `{other}` (expected s/m/h)")),
    }
}

fn parse_research_budget(s: &str) -> Result<ResearchBudget, String> {
    let mut parts = s.splitn(2, '+');
    let web: u32 = parts
        .next()
        .ok_or_else(|| "expected <web>+<fetch>".to_string())?
        .trim()
        .parse()
        .map_err(|e| format!("invalid web budget: {e}"))?;
    let fetch: u32 = parts
        .next()
        .ok_or_else(|| "expected <web>+<fetch>".to_string())?
        .trim()
        .parse()
        .map_err(|e| format!("invalid fetch budget: {e}"))?;
    Ok(ResearchBudget { web, fetch })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_accepts_s_m_h() {
        assert_eq!(parse_duration("30s").unwrap(), Duration::from_secs(30));
        assert_eq!(parse_duration("5m").unwrap(), Duration::from_secs(300));
        assert_eq!(parse_duration("2h").unwrap(), Duration::from_secs(7200));
    }

    #[test]
    fn parse_duration_rejects_bad_unit() {
        assert!(parse_duration("5d").is_err());
        assert!(parse_duration("").is_err());
        assert!(parse_duration("xs").is_err());
    }

    #[test]
    fn parse_research_budget_defaults_5_plus_3() {
        let rb = parse_research_budget("5+3").unwrap();
        assert_eq!(rb.web, 5);
        assert_eq!(rb.fetch, 3);
    }

    #[test]
    fn parse_research_budget_custom() {
        let rb = parse_research_budget("10+0").unwrap();
        assert_eq!(rb.web, 10);
        assert_eq!(rb.fetch, 0);
    }

    #[test]
    fn parse_research_budget_rejects_malformed() {
        assert!(parse_research_budget("5").is_err());
        assert!(parse_research_budget("a+b").is_err());
    }
}
