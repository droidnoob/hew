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

use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args as ClapArgs, Subcommand};
use hew_core::backpressure::{self, GateCheck, Verdict};
use hew_core::bd::{BdClient, RealBd};
use hew_core::loop_log::{
    IterLog, LOOP_ROOT, RunLog, iter_log_path, new_run_id, run_dir, run_log_path, stop_file_path,
    write_json_atomic,
};
use hew_core::prompt;
use hew_core::runner::{Iter, IterOutcome, ResearchBudget, Run, RunConfig};
use hew_core::runtime::{ClaudeSpawner, RuntimeSpawner};
use hew_core::stop_signals::Collector;
use hew_core::time::iso_now_utc;
use hew_core::{Ctx, allowed_tools, skills};

/// Runs the per-iter test+lint commands. Production wires the cargo
/// invocations; tests inject a [`StaticGateRunner`] with a canned
/// `GateCheck`.
pub trait GateRunner {
    fn run_gate(&self, project_root: &Path) -> GateCheck;
}

/// Production gate runner: `cargo test --quiet` + `cargo clippy
/// --all-targets -- -D warnings`. v1 leaves the craft signals unset
/// (false) — a follow-up task will wire them from `hew_core::config`.
#[derive(Debug, Default)]
pub struct CargoGateRunner;

impl GateRunner for CargoGateRunner {
    fn run_gate(&self, project_root: &Path) -> GateCheck {
        let tests_passed = std::process::Command::new("cargo")
            .args(["test", "--quiet"])
            .current_dir(project_root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let lint_passed = std::process::Command::new("cargo")
            .args(["clippy", "--all-targets", "--", "-D", "warnings"])
            .current_dir(project_root)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        GateCheck { tests_passed, lint_passed, ..Default::default() }
    }
}

/// Test-only gate runner that always returns a canned [`GateCheck`].
#[derive(Debug, Clone)]
pub struct StaticGateRunner(pub GateCheck);

impl GateRunner for StaticGateRunner {
    fn run_gate(&self, _project_root: &Path) -> GateCheck {
        self.0.clone()
    }
}

/// Capture `git rev-parse HEAD` for the worktree at `project_root`.
fn git_head_sha(project_root: &Path) -> miette::Result<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(project_root)
        .output()
        .map_err(|e| miette::miette!("git rev-parse: {e}"))?;
    if !out.status.success() {
        return Err(miette::miette!(
            "git rev-parse HEAD failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// `git reset --hard <sha>` in `project_root`. Used to revert an iter's
/// commits when the backpressure gate fails.
fn git_reset_hard(project_root: &Path, sha: &str) -> miette::Result<()> {
    let out = std::process::Command::new("git")
        .args(["reset", "--hard", sha])
        .current_dir(project_root)
        .output()
        .map_err(|e| miette::miette!("git reset: {e}"))?;
    if !out.status.success() {
        return Err(miette::miette!(
            "git reset --hard {sha} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

#[derive(Debug, ClapArgs)]
pub struct LoopCmd {
    #[command(subcommand)]
    pub sub: LoopSub,
}

#[derive(Debug, Subcommand)]
pub enum LoopSub {
    /// Drive the autonomous outer loop until a stop signal fires.
    Run(Args),
    /// Touch the stop-file of a running loop. Defaults to the most
    /// recent run.
    Cancel(CancelArgs),
    /// Pretty-print iter logs from a completed or running loop.
    Logs(LogsArgs),
    /// List recent loop runs and their state.
    List(ListArgs),
}

#[derive(Debug, ClapArgs)]
pub struct CancelArgs {
    /// Specific run-id to cancel. Defaults to the most recent run.
    #[arg(long)]
    pub run_id: Option<String>,
}

#[derive(Debug, ClapArgs)]
pub struct LogsArgs {
    /// Run-id to inspect. Defaults to the most recent run.
    #[arg(long)]
    pub run_id: Option<String>,
    /// Only show the last N iters (default 5; `0` = all).
    #[arg(long, default_value_t = 5)]
    pub tail: u32,
    /// Read a single iter by number.
    #[arg(long)]
    pub iter: Option<u32>,
    /// Emit JSON rather than the pretty table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, ClapArgs)]
pub struct ListArgs {
    /// Max rows to show. Default 20.
    #[arg(long, default_value_t = 20)]
    pub n: u32,
}

pub fn run(ctx: &Ctx, cmd: LoopCmd) -> miette::Result<()> {
    match cmd.sub {
        LoopSub::Run(a) => run_loop(ctx, a),
        LoopSub::Cancel(a) => run_cancel(ctx, a),
        LoopSub::Logs(a) => run_logs(ctx, a),
        LoopSub::List(a) => run_list(ctx, a),
    }
}

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

pub fn run_loop(ctx: &Ctx, args: Args) -> miette::Result<()> {
    if args.runtime != "claude" {
        return Err(miette::miette!(
            "unsupported runtime `{}`; only `claude` is wired in v1",
            args.runtime
        ));
    }

    let project_root = std::env::current_dir().map_err(|e| miette::miette!("resolve cwd: {e}"))?;
    let bd = RealBd::discover().map_err(|e| miette::miette!("bd discover: {e}"))?;
    let spawner: Option<Box<dyn RuntimeSpawner>> =
        if args.dry_run { None } else { Some(Box::new(ClaudeSpawner::from_env())) };
    let gate = CargoGateRunner;
    run_loop_with(ctx, args, &bd, spawner.as_deref(), &gate, &project_root)
}

/// Testable inner. Production [`run_loop`] resolves `bd`, the spawner,
/// the gate runner and the project root; tests construct mocks and call
/// this directly. Returns the same `miette::Result` as `run_loop`.
pub fn run_loop_with(
    ctx: &Ctx,
    args: Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    gate: &dyn GateRunner,
    project_root: &Path,
) -> miette::Result<()> {
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
    let dir = run_dir(project_root, &run_id).map_err(|e| miette::miette!("create run dir: {e}"))?;
    let stop_path = args.stop_file.unwrap_or_else(|| stop_file_path(&dir));
    let collector = Collector::new(stop_path);
    let mut run_state = Run::new(run_id.clone(), iso_now_utc(), cfg.clone());

    if !ctx.quiet {
        eprintln!("hew loop {} — run-dir={}", &run_id, dir.display());
        if args.dry_run {
            eprintln!("(--dry-run: no subprocess, no git ops)");
        }
    }

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

        // Capture pre-iter HEAD before the agent runs so the
        // backpressure gate can revert iter commits on Fail. Skipped
        // under `--dry-run` (no commits) and tolerated as None when
        // there's no git repo at `project_root`.
        let pre_iter_sha: Option<String> = if args.dry_run {
            None
        } else {
            match git_head_sha(project_root) {
                Ok(sha) => Some(sha),
                Err(e) => {
                    if !ctx.quiet {
                        eprintln!(
                            "iter {iter_number} skipping rollback capture (no git HEAD): {e}"
                        );
                    }
                    None
                }
            }
        };

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

        let (mut outcome, tokens, mut stderr_tail) = if let Some(s) = spawner {
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

        // Backpressure gate: run tests + lint after a non-error spawn,
        // skip under `--dry-run`. Pure verdict logic lives in
        // `hew_core::backpressure::evaluate`; this layer reverts the
        // worktree to `pre_iter_sha` and files a STATUS memory on Fail.
        if !args.dry_run && !matches!(outcome, IterOutcome::RuntimeError) {
            let check = gate.run_gate(project_root);
            let verdict = backpressure::evaluate(&check, args.strict);
            match verdict {
                Verdict::Pass => {}
                Verdict::WarnOnly(reasons) => {
                    if !ctx.quiet {
                        eprintln!("iter {iter_number} gate warnings: {}", reasons.join("; "));
                    }
                }
                Verdict::Fail(reasons) => {
                    if !ctx.quiet {
                        eprintln!("iter {iter_number} gate FAIL: {}", reasons.join("; "));
                    }
                    if let Some(sha) = pre_iter_sha.as_deref() {
                        if let Err(e) = git_reset_hard(project_root, sha) {
                            eprintln!("iter {iter_number} rollback to {sha} failed: {e}");
                        } else if !ctx.quiet {
                            eprintln!("iter {iter_number} rolled back to {sha}");
                        }
                    }
                    let reasons_joined = reasons.join("; ");
                    let memory_body = format!(
                        "STATUS:loop-iter-failed:{}:{}:{}\nreasons: {}",
                        run_id,
                        iter_number,
                        iso_now_utc(),
                        reasons_joined,
                    );
                    if let Err(e) = bd.remember(&memory_body)
                        && !ctx.quiet
                    {
                        eprintln!(
                            "iter {iter_number} failed to record STATUS:loop-iter-failed memory: {e}"
                        );
                    }
                    outcome = IterOutcome::BackpressureFail;
                    stderr_tail = Some(match stderr_tail {
                        Some(t) if !t.is_empty() => format!("{t}\ngate-fail: {reasons_joined}"),
                        _ => format!("gate-fail: {reasons_joined}"),
                    });
                }
            }
        }

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

fn loop_root(project_root: &std::path::Path) -> PathBuf {
    project_root.join(LOOP_ROOT)
}

/// Return the most recently modified subdirectory of `.hew/loop/`. Used
/// when the user omits `--run-id` on cancel/logs.
fn latest_run_id(project_root: &std::path::Path) -> miette::Result<String> {
    let root = loop_root(project_root);
    let mut best: Option<(std::time::SystemTime, String)> = None;
    let entries = std::fs::read_dir(&root)
        .map_err(|e| miette::miette!("no loop runs in {}: {e}", root.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.starts_with("loop-") {
            continue;
        }
        let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        match &best {
            None => best = Some((mtime, name.to_string())),
            Some((bm, _)) if mtime > *bm => best = Some((mtime, name.to_string())),
            _ => {}
        }
    }
    best.map(|(_, n)| n).ok_or_else(|| miette::miette!("no loop runs found in {}", root.display()))
}

pub fn run_cancel(ctx: &Ctx, args: CancelArgs) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;
    let run_id = match args.run_id {
        Some(id) => id,
        None => latest_run_id(&project_root)?,
    };
    let dir = loop_root(&project_root).join(&run_id);
    if !dir.exists() {
        return Err(miette::miette!("run-dir not found: {}", dir.display()));
    }
    let stop = stop_file_path(&dir);
    std::fs::write(&stop, b"cancel\n")
        .map_err(|e| miette::miette!("write {}: {e}", stop.display()))?;
    if !ctx.quiet {
        println!("cancelled {} (stop-file: {})", run_id, stop.display());
    }
    Ok(())
}

pub fn run_logs(ctx: &Ctx, args: LogsArgs) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;
    let run_id = match args.run_id {
        Some(id) => id,
        None => latest_run_id(&project_root)?,
    };
    let dir = loop_root(&project_root).join(&run_id);
    if !dir.exists() {
        return Err(miette::miette!("run-dir not found: {}", dir.display()));
    }

    if let Some(n) = args.iter {
        let path = iter_log_path(&dir, n);
        let body = std::fs::read_to_string(&path)
            .map_err(|e| miette::miette!("read {}: {e}", path.display()))?;
        if args.json {
            print!("{body}");
        } else {
            let log: IterLog = serde_json::from_str(&body)
                .map_err(|e| miette::miette!("parse {}: {e}", path.display()))?;
            print_iter(&log);
        }
        return Ok(());
    }

    let logs = collect_iter_logs(&dir)?;
    let logs: Vec<IterLog> = if args.tail == 0 {
        logs
    } else {
        let n = args.tail as usize;
        let start = logs.len().saturating_sub(n);
        logs[start..].to_vec()
    };

    if args.json {
        let body = serde_json::to_string_pretty(&logs)
            .map_err(|e| miette::miette!("serialize logs: {e}"))?;
        println!("{body}");
        return Ok(());
    }

    let run_log_path = run_log_path(&dir);
    if !ctx.quiet
        && run_log_path.exists()
        && let Ok(body) = std::fs::read_to_string(&run_log_path)
        && let Ok(rl) = serde_json::from_str::<RunLog>(&body)
    {
        println!(
            "run {} — iters={} tokens={} stop={}",
            rl.id,
            rl.iter_count,
            rl.cumulative_tokens,
            rl.stop_reason.unwrap_or_else(|| "(running)".into()),
        );
    }
    for log in &logs {
        print_iter(log);
    }
    Ok(())
}

pub fn run_list(_ctx: &Ctx, args: ListArgs) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;
    let root = loop_root(&project_root);
    if !root.exists() {
        println!("(no loop runs)");
        return Ok(());
    }
    let mut runs: Vec<(std::time::SystemTime, String, RunListRow)> = Vec::new();
    for entry in std::fs::read_dir(&root)
        .map_err(|e| miette::miette!("read {}: {e}", root.display()))?
        .flatten()
    {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.starts_with("loop-") {
            continue;
        }
        let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
        let row = load_run_list_row(&path);
        runs.push((mtime, name.to_string(), row));
    }
    runs.sort_by_key(|r| std::cmp::Reverse(r.0));
    if runs.is_empty() {
        println!("(no loop runs)");
        return Ok(());
    }
    let max = if args.n == 0 { runs.len() } else { args.n as usize };
    for (_, name, row) in runs.into_iter().take(max) {
        println!("{:<46} {:<10} iters={:<3} stop={}", name, row.state, row.iters, row.stop);
    }
    Ok(())
}

struct RunListRow {
    state: &'static str,
    iters: u32,
    stop: String,
}

fn load_run_list_row(dir: &std::path::Path) -> RunListRow {
    let stop_present = stop_file_path(dir).exists();
    let rl_path = run_log_path(dir);
    let parsed = std::fs::read_to_string(&rl_path)
        .ok()
        .and_then(|s| serde_json::from_str::<RunLog>(&s).ok());
    let (iters, stop) = parsed
        .as_ref()
        .map(|r| (r.iter_count, r.stop_reason.clone().unwrap_or_default()))
        .unwrap_or((0, String::new()));
    let state = if !stop.is_empty() {
        "completed"
    } else if stop_present {
        "cancelled"
    } else {
        "running"
    };
    RunListRow { state, iters, stop }
}

fn collect_iter_logs(dir: &std::path::Path) -> miette::Result<Vec<IterLog>> {
    let mut logs: Vec<IterLog> = Vec::new();
    for entry in std::fs::read_dir(dir)
        .map_err(|e| miette::miette!("read {}: {e}", dir.display()))?
        .flatten()
    {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.starts_with("iter-") || !name.ends_with(".json") {
            continue;
        }
        if let Ok(body) = std::fs::read_to_string(&path)
            && let Ok(log) = serde_json::from_str::<IterLog>(&body)
        {
            logs.push(log);
        }
    }
    logs.sort_by_key(|l| l.number);
    Ok(logs)
}

fn print_iter(log: &IterLog) {
    let outcome = log.outcome.clone().unwrap_or_else(|| "running".into());
    let task = log.task_id.clone().unwrap_or_else(|| "-".into());
    let tokens = log.cost.total();
    let hash = log.prompt_prefix_hash.clone().unwrap_or_default();
    println!(
        "iter {:>3} task={} outcome={} tokens={} prefix={}",
        log.number, task, outcome, tokens, hash
    );
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
