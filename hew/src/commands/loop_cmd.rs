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
//! the ready queue and writes its iter log. Ctrl+C installs a `ctrlc`
//! handler that flips the shared `CancelFlag` → next snapshot →
//! `StopReason::Cancelled` (the currently-running iter completes
//! cleanly because we use `Command::output()` and don't kill the
//! child; a follow-up will wire `Child::kill()` for fast abort). The
//! `--interactive` ask-file flow is still stubbed pending hew-cyk.

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
use hew_core::runner::{Iter, IterOutcome, Run, RunConfig};
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

/// Production gate runner. Reads `(test, lint)` from project-authored
/// signals (`Makefile`, `justfile`, `package.json` scripts) — see
/// [`hew_core::gate`]. No signals → no gate; the agent runs whatever
/// checks it wants directly inside the iter via Bash, which is the
/// correct default given we'd otherwise be guessing.
///
/// Spawn errors are split: `ErrorKind::NotFound` (tool not installed)
/// degrades to skip-pass with a breadcrumb; any other error or a
/// non-zero exit fails the gate normally.
#[derive(Debug, Default)]
pub struct AutoGateRunner;

/// Kept as a thin alias so any external callers wiring the production
/// runner by name still compile. The behavior no longer hardcodes
/// cargo — see [`AutoGateRunner`] / [`hew_core::gate`].
pub type CargoGateRunner = AutoGateRunner;

impl GateRunner for AutoGateRunner {
    fn run_gate(&self, project_root: &Path) -> GateCheck {
        let spec = hew_core::gate::detect(project_root);
        if !spec.has_any() {
            eprintln!(
                "hew loop: no gate signals (Makefile/justfile/package.json) at {} — gate skipped",
                project_root.display()
            );
            return GateCheck { tests_passed: true, lint_passed: true, ..Default::default() };
        }
        let tests_passed = run_gate_step("test", &spec.test_cmd, project_root);
        let lint_passed = run_gate_step("lint", &spec.lint_cmd, project_root);
        GateCheck { tests_passed, lint_passed, ..Default::default() }
    }
}

/// Run one step of the auto-detected gate. Empty `cmd` (e.g. Node repo
/// with no `lint` script) is a deliberate skip and passes. ENOENT on
/// the binary is treated as skip-pass with a breadcrumb so a missing
/// optional toolchain (`ruff`, `pytest`) doesn't trap the loop. Any
/// other spawn error or non-zero exit fails the step.
fn run_gate_step(label: &str, cmd: &[String], project_root: &Path) -> bool {
    if cmd.is_empty() {
        return true;
    }
    let mut command = std::process::Command::new(&cmd[0]);
    command.args(&cmd[1..]).current_dir(project_root);
    match command.status() {
        Ok(s) => s.success(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("hew loop: `{}` not found in PATH — {label} gate skipped", cmd[0]);
            true
        }
        Err(e) => {
            eprintln!("hew loop: {label} gate spawn failed ({e}); treating as fail");
            false
        }
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

/// Symbol-level changelog of one iter, rendered as `<path>:<sym>`
/// strings for the iter log. Returns an empty vec under any of:
/// - `treesitter` feature disabled (compile-time fallback);
/// - rollback iter (outcome was a backpressure_fail / runtime_error);
/// - missing pre-iter sha (e.g. no git repo);
/// - blast computation returned an error (best-effort observability).
#[cfg(feature = "treesitter")]
fn compute_iter_symbols(
    _project_root: &Path,
    pre_iter_sha: Option<&str>,
    outcome: &IterOutcome,
) -> Vec<String> {
    if matches!(outcome, IterOutcome::BackpressureFail | IterOutcome::RuntimeError) {
        return Vec::new();
    }
    let Some(sha) = pre_iter_sha else {
        return Vec::new();
    };
    let Ok(git) = hew_core::git::RealGit::discover() else {
        return Vec::new();
    };
    let entries = match hew_core::blast::compute_blast_with(&git, Some(sha)) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries {
        for sym in entry.symbols {
            out.push(format!("{}:{}", entry.path, sym.name));
        }
    }
    out
}

/// Stub: feature-disabled builds never compute symbols.
#[cfg(not(feature = "treesitter"))]
fn compute_iter_symbols(
    _project_root: &Path,
    _pre_iter_sha: Option<&str>,
    _outcome: &IterOutcome,
) -> Vec<String> {
    Vec::new()
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
    /// Re-render the end-of-run summary for a completed (or running)
    /// loop from its persisted logs. Defaults to the most recent run.
    Summary(SummaryArgs),
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

#[derive(Debug, ClapArgs)]
pub struct SummaryArgs {
    /// Run-id to summarize. Defaults to the most recent run.
    #[arg(long)]
    pub run_id: Option<String>,
}

pub fn run(ctx: &Ctx, cmd: LoopCmd) -> miette::Result<()> {
    match cmd.sub {
        LoopSub::Run(a) => run_loop(ctx, a),
        LoopSub::Cancel(a) => run_cancel(ctx, a),
        LoopSub::Logs(a) => run_logs(ctx, a),
        LoopSub::List(a) => run_list(ctx, a),
        LoopSub::Summary(a) => run_summary(ctx, a),
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

    /// Promote craft warnings to failures. Default on; `--no-strict` opts out.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub strict: bool,

    /// Pause on ask-files for operator input. Default off; the v1 wiring
    /// is stubbed (hew-cyk) — passing `--interactive` is honored in the
    /// run config but doesn't yet drive any prompts. Mutually exclusive
    /// with `--unattended`.
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set, conflicts_with = "unattended")]
    pub interactive: bool,

    /// Resolve any new `DEFERRED:<topic>` memory the agent files during
    /// an iter by running `decide::resolve` after the iter completes.
    /// Matches → `DECISION:` memory; misses → leave the DEFERRED for
    /// operator review. Mutually exclusive with `--interactive`.
    #[arg(long, default_value_t = false, action = clap::ArgAction::Set)]
    pub unattended: bool,

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
    let gate = AutoGateRunner;
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
        strict: args.strict,
        interactive: args.interactive,
        unattended: args.unattended,
    };

    let run_id = new_run_id();
    let dir = run_dir(project_root, &run_id).map_err(|e| miette::miette!("create run dir: {e}"))?;
    let stop_path = args.stop_file.unwrap_or_else(|| stop_file_path(&dir));
    let collector = Collector::new(stop_path);
    let mut run_state = Run::new(run_id.clone(), iso_now_utc(), cfg.clone());

    // Install (or refresh) the SIGINT handler so Ctrl+C flips the
    // shared CancelFlag instead of killing the loop process mid-iter.
    // `ctrlc::set_handler` is process-global and refuses a second
    // install; we silently ignore that case so back-to-back loop runs
    // (e.g. integration tests) still work.
    {
        let flag = collector.cancel.clone();
        let _ = ctrlc::set_handler(move || flag.cancel());
    }

    if !ctx.quiet {
        eprintln!("hew loop {} — run-dir={}", &run_id, dir.display());
        if args.dry_run {
            eprintln!("(--dry-run: no subprocess, no git ops)");
        }
    }

    let allowed = allowed_tools::for_skill(&args.skill);
    let mut last_outcome: Option<IterOutcome> = None;
    let mut iter_logs: Vec<IterLog> = Vec::new();

    // Freeze the bd prime payload once at run start: this is the
    // cacheable per-run primer the agent sees on every iter. New
    // memories filed mid-run (STATUS:loop-iter-failed, DECISIONs from
    // unattended resolution, etc.) deliberately don't appear in the
    // prompt until the next `hew loop run` invocation — the trade-off
    // is agent-stale-by-up-to-one-run for a byte-stable prompt prefix
    // that the Anthropic prompt cache can hit across iters. Agents can
    // always shell `hew memories --prefix=…` inside a spawn if they
    // need fresh state.
    let primer_text = bd.prime_raw().unwrap_or_default();

    loop {
        let ready = bd.ready().map_err(|e| miette::miette!("bd ready: {e}"))?;
        let signals = collector.snapshot(&run_state, ready.len() as u32, last_outcome);
        if let Some(reason) = signals.evaluate(&cfg) {
            run_state.stop_reason = Some(reason);
            break;
        }

        // Snapshot the pre-iter ready set so we can detect closure
        // out-of-band: if the iter's task is no longer in `bd.ready()`
        // after the spawn, it was closed — even when the agent ran
        // `hew task close` via Bash and the literal `closed <id>`
        // marker never made it into the model's final reply. The
        // marker-text path (`detect_closed_task`) is kept as a
        // secondary signal for cases where the agent closes a
        // *different* task than the one we primed.
        let pre_ready_ids: std::collections::BTreeSet<String> =
            ready.iter().map(|t| t.id.clone()).collect();

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

        // Snapshot the memory set so the post-iter DEFERRED-resolution
        // pass can identify which entries the agent filed *during this
        // iter*. Only collected when `--unattended` is active.
        let pre_iter_memory_ids: std::collections::BTreeSet<String> = if args.unattended {
            bd.memories().map(|m| m.keys().cloned().collect()).unwrap_or_default()
        } else {
            std::collections::BTreeSet::new()
        };

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

        // Per-iter task fields live in the *tail* — they change every
        // iter and must not invalidate the cacheable prefix.
        let task_brief = format!(
            "Current task: {} ({}, P{}, status={}).\n\nDrive `{}` to close per the {} skill body.",
            task.id, task.title, task.priority, task.status, task.id, args.skill,
        );
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

        // Out-of-band closure detection. detect_closed_task only
        // fires when the model echoes `closed <id>` in its final
        // reply; agents that close via `hew task close` (Bash tool)
        // don't surface the marker. Promote NoClose → Closed if the
        // iter's task is no longer in the ready set. Skipped under
        // --dry-run (no real bd state changes).
        if !args.dry_run
            && matches!(outcome, IterOutcome::NoClose)
            && let Ok(post_ready) = bd.ready()
        {
            let still_ready = post_ready.iter().any(|t| t.id == task.id);
            let was_ready = pre_ready_ids.contains(&task.id);
            if was_ready && !still_ready {
                outcome = IterOutcome::Closed;
            }
        }

        // Backpressure gate: run tests + lint after a non-error spawn,
        // skip under `--dry-run`. Pure verdict logic lives in
        // `hew_core::backpressure::evaluate`; this layer reverts the
        // worktree to `pre_iter_sha` and files a STATUS memory on Fail.
        // Skip the gate entirely if the user pressed Ctrl+C while the
        // spawn was running — no point burning a `cargo test` cycle on
        // a run we're about to abort.
        let cancelled_mid_iter = collector.cancel.is_cancelled();
        if !args.dry_run && !matches!(outcome, IterOutcome::RuntimeError) && !cancelled_mid_iter {
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

        // Unattended decision-resolution: convert any new DEFERRED
        // memories the agent filed during this iter into DECISIONs
        // when `decide::resolve` finds prior art (memory / code).
        // Pure research-driven resolution is not yet wired (see
        // `BdDecisionContext::run_research`), so topics that only
        // depend on web research stay deferred and the operator sees
        // the existing DEFERRED on next review.
        if args.unattended
            && !args.dry_run
            && !matches!(outcome, IterOutcome::RuntimeError)
            && let Ok(after) = bd.memories()
        {
            for (id, body) in after.iter() {
                if pre_iter_memory_ids.contains(id) {
                    continue;
                }
                if !body.trim_start().starts_with("DEFERRED:") {
                    continue;
                }
                let Some(topic) = hew_core::decide::extract_deferred_topic(body) else {
                    continue;
                };
                let mut dctx =
                    hew_core::decide::BdDecisionContext::new(bd, project_root.to_path_buf());
                let resolution = hew_core::decide::resolve(&topic, &mut dctx);
                match resolution {
                    hew_core::decide::Resolution::Memory(hit) => {
                        let decision = format!(
                            "DECISION:{topic} — resolved from prior memory {} ({})",
                            hit.id,
                            hit.body.trim().chars().take(120).collect::<String>(),
                        );
                        let _ = bd.remember(&decision);
                        iter.decisions.push(topic.clone());
                    }
                    hew_core::decide::Resolution::Code(citations) => {
                        let cite = citations
                            .first()
                            .map(|c| format!("{}:{}", c.file, c.line))
                            .unwrap_or_else(|| "(no citation)".to_string());
                        let decision =
                            format!("DECISION:{topic} — resolved from prior art at {cite}",);
                        let _ = bd.remember(&decision);
                        iter.decisions.push(topic.clone());
                    }
                    hew_core::decide::Resolution::Decided { decision_body, .. } => {
                        let _ = bd.remember(&decision_body);
                        iter.decisions.push(topic.clone());
                    }
                    hew_core::decide::Resolution::Deferred { .. } => {
                        iter.deferred.push(id.clone());
                    }
                }
            }
        }

        iter.outcome = Some(outcome);
        iter.cost = tokens;
        iter.ended_at = Some(iso_now_utc());
        iter.stderr_tail = stderr_tail;

        let prefix_hash_hex = Some(format!("{:016x}", assembled.prefix_hash));
        // Symbol-level changelog of the iter: blast against the
        // pre-iter sha when treesitter is compiled in and the iter
        // actually produced commits. Best-effort: any error in the
        // blast path collapses to an empty list — we never let an
        // observability signal block iter logging.
        let symbols_touched = compute_iter_symbols(project_root, pre_iter_sha.as_deref(), &outcome);
        let log = IterLog::from_iter(&iter, prefix_hash_hex, Vec::new(), symbols_touched);
        write_json_atomic(&iter_log_path(&dir, iter_number), &log)
            .map_err(|e| miette::miette!("write iter log: {e}"))?;
        iter_logs.push(log);

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

    print_summary(ctx, &run_state, &iter_logs, &dir);
    Ok(())
}

fn print_summary(ctx: &Ctx, run: &Run, iter_logs: &[IterLog], dir: &std::path::Path) {
    if ctx.quiet {
        return;
    }
    let summary = hew_core::loop_summary::summarize(run, iter_logs);
    let colorize = std::env::var_os("NO_COLOR").is_none();
    print!("{}", hew_core::loop_summary::render(&summary, &dir.display().to_string(), colorize),);
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

pub fn run_summary(ctx: &Ctx, args: SummaryArgs) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;
    let run_id = match args.run_id {
        Some(id) => id,
        None => latest_run_id(&project_root)?,
    };
    let dir = loop_root(&project_root).join(&run_id);
    if !dir.exists() {
        return Err(miette::miette!("run-dir not found: {}", dir.display()));
    }

    // Load the persisted run header for id + stop reason.
    let rl_body = std::fs::read_to_string(run_log_path(&dir))
        .map_err(|e| miette::miette!("read run.json: {e}"))?;
    let rl: RunLog =
        serde_json::from_str(&rl_body).map_err(|e| miette::miette!("parse run.json: {e}"))?;

    let iter_logs = collect_iter_logs(&dir)?;

    // Reconstruct the minimal `Run` that `loop_summary::summarize`
    // reads: id, stop_reason, and per-iter timestamps (for duration).
    // Everything else in the summary is derived from `iter_logs`.
    let run = Run {
        id: rl.id.clone(),
        started_at: rl.started_at.clone(),
        config: RunConfig::default(),
        iters: iter_logs
            .iter()
            .map(|l| Iter {
                number: l.number,
                task_id: l.task_id.clone(),
                started_at: l.started_at.clone(),
                ended_at: l.ended_at.clone(),
                outcome: None,
                cost: l.cost,
                decisions: l.decisions.clone(),
                deferred: l.deferred.clone(),
                stderr_tail: l.stderr_tail.clone(),
            })
            .collect(),
        stop_reason: rl.stop_reason.as_deref().and_then(hew_core::runner::StopReason::from_label),
    };

    print_summary(ctx, &run, &iter_logs, &dir);
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
}
