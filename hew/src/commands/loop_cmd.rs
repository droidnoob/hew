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
    IterLog, LOOP_ROOT, Manifest, ManifestWorker, RunLog, iter_log_path, new_run_id, run_dir,
    run_log_path, stop_file_path, write_json_atomic, write_manifest,
};
use hew_core::prompt;
use hew_core::runner::{CooldownState, Iter, IterOutcome, Run, RunConfig};
use hew_core::runtime::{
    ClaudeSpawner, CodexSpawner, FallbackConfig, RuntimeKind, RuntimeSpawner, SpawnFailureClass,
    SpawnOpts,
};
use hew_core::stop_signals::Collector;
use hew_core::time::iso_now_utc;
use hew_core::{Ctx, allowed_tools, skills};

/// Runs the per-iter test+lint commands. Production wires the cargo
/// invocations; tests inject a [`StaticGateRunner`] with a canned
/// `GateCheck`.
///
/// `working_dir` is the directory the test/lint subprocesses run in.
/// For the single-worker loop this is the project root; for per-worker
/// parallel runs (hew-6az) the dispatcher passes the worker's worktree
/// so each worker's gate checks its own commits in isolation — see
/// hew-j4x.
pub trait GateRunner {
    fn run_gate(&self, working_dir: &Path) -> GateCheck;
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
    fn run_gate(&self, working_dir: &Path) -> GateCheck {
        let spec = hew_core::gate::detect(working_dir);
        if !spec.has_any() {
            eprintln!(
                "hew loop: no gate signals (Makefile/justfile/package.json) at {} — gate skipped",
                working_dir.display()
            );
            return GateCheck { tests_passed: true, lint_passed: true, ..Default::default() };
        }
        let tests_passed = run_gate_step("test", &spec.test_cmd, working_dir);
        let lint_passed = run_gate_step("lint", &spec.lint_cmd, working_dir);
        GateCheck { tests_passed, lint_passed, ..Default::default() }
    }
}

/// Run one step of the auto-detected gate. Empty `cmd` (e.g. Node repo
/// with no `lint` script) is a deliberate skip and passes. ENOENT on
/// the binary is treated as skip-pass with a breadcrumb so a missing
/// optional toolchain (`ruff`, `pytest`) doesn't trap the loop. Any
/// other spawn error or non-zero exit fails the step.
fn run_gate_step(label: &str, cmd: &[String], working_dir: &Path) -> bool {
    if cmd.is_empty() {
        return true;
    }
    let mut command = std::process::Command::new(&cmd[0]);
    command.args(&cmd[1..]).current_dir(working_dir);
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
    fn run_gate(&self, _working_dir: &Path) -> GateCheck {
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

/// `git -C <worktree_dir> reset --hard <sha>` — scoped rollback for one
/// worker's worktree only. Per DECISION:loop-parallel-overlap-policy the
/// parallel loop's gate-fail revert must never touch sibling worktrees;
/// delegating through [`hew_core::git::reset_hard_in`] keeps the scoping
/// in one place and unit-tested.
fn git_reset_hard(worktree_dir: &Path, sha: &str) -> miette::Result<()> {
    let git = hew_core::git::RealGit::discover().map_err(|e| miette::miette!("git: {e}"))?;
    hew_core::git::reset_hard_in(&git, worktree_dir, sha)
        .map_err(|e| miette::miette!("git reset --hard {sha} failed: {e}"))
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
    /// Remove orphaned worktrees under `~/.hew/wt/` left behind by
    /// crashed or interrupted parallel loop runs. A worktree is "orphan"
    /// when its `<run-id>` has no live run-dir under
    /// `<project>/.hew/loop/` (or that run-dir's `run.json` already
    /// records a `stop_reason`). Defaults to listing what would be
    /// removed; pass `--apply` to actually delete.
    PruneWorktrees(PruneWorktreesArgs),
}

#[derive(Debug, ClapArgs)]
pub struct PruneWorktreesArgs {
    /// Actually remove orphan worktrees. Default is dry-run.
    #[arg(long)]
    pub apply: bool,
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
        LoopSub::PruneWorktrees(a) => run_prune_worktrees(ctx, a),
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

    /// Runtime to drive. `claude` is fully wired; `codex` parses and
    /// reaches prompt assembly (dry-run only — real spawner lands with
    /// hew-9d4).
    #[arg(
        long,
        default_value = "claude",
        value_parser = clap::builder::PossibleValuesParser::new(RuntimeKind::VARIANTS),
    )]
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

    /// Fallback runtime to switch to when the primary trips a runtime
    /// error (auth / rate-limit / server). When set, the loop runs the
    /// fallback for `--fallback-cooldown-iters` iters before retrying
    /// the primary. Overrides `loop.fallback_runtime` config.
    /// Example: `hew loop run --fallback-runtime codex`.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(RuntimeKind::VARIANTS),
    )]
    pub fallback_runtime: Option<String>,

    /// Iters the loop stays on the fallback before retrying the
    /// primary. Default 3 (DECISION:loop-fallback-policy). Overrides
    /// `loop.fallback_cooldown_iters` config. Must be >= 1.
    /// Example: `hew loop run --fallback-runtime codex --fallback-cooldown-iters 5`.
    #[arg(long)]
    pub fallback_cooldown_iters: Option<u32>,

    /// Number of worker slots to drive in parallel. Default `1` keeps
    /// the existing single-threaded loop byte-for-byte. `>=2` switches
    /// to the per-worker-worktree dispatcher path (DECISION:loop-
    /// parallel-overlap-policy: trust-the-graph). Capped at 16 to
    /// prevent accidental fork-bombs.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..=16))]
    pub jobs: u32,
}

pub fn run_loop(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let kind: RuntimeKind = args.runtime.parse().map_err(|e: String| miette::miette!("{e}"))?;

    // Resolve fallback config: CLI > config > defaults. Surfaced into
    // the logs only when set; today the loop runner doesn't consume it
    // yet (hew-lc2 wires the cooldown state machine). Reading it here
    // keeps the CLI/config surfaces honest — bad inputs fail at parse,
    // not after the loop starts. The unused-binding warning is gone
    // because we use the value below.
    let cfg = hew_core::config::load().map_err(|e| miette::miette!("load hew config: {e}"))?;
    let cli_fallback = args
        .fallback_runtime
        .as_deref()
        .map(|s| s.parse::<RuntimeKind>())
        .transpose()
        .map_err(|e| miette::miette!("{e}"))?;
    let fallback = FallbackConfig::resolve(
        cli_fallback,
        args.fallback_cooldown_iters,
        cfg.loop_cfg.fallback_runtime.as_deref(),
        cfg.loop_cfg.fallback_cooldown_iters,
    )
    .map_err(|e| miette::miette!("{e}"))?;
    if fallback.runtime.is_some() {
        tracing::debug!(
            primary = kind.as_str(),
            fallback = ?fallback.runtime.map(|r| r.as_str()),
            cooldown_iters = fallback.cooldown_iters,
            "loop: fallback resolved (runner wiring in hew-lc2)"
        );
    }

    let project_root = std::env::current_dir().map_err(|e| miette::miette!("resolve cwd: {e}"))?;
    let bd = RealBd::discover().map_err(|e| miette::miette!("bd discover: {e}"))?;
    let spawner: Option<Box<dyn RuntimeSpawner>> =
        if args.dry_run { None } else { Some(build_spawner_for(kind)) };
    let fallback_spawner: Option<Box<dyn RuntimeSpawner>> =
        if args.dry_run { None } else { fallback.runtime.map(build_spawner_for) };
    let gate = AutoGateRunner;
    run_loop_with(
        ctx,
        args,
        &bd,
        spawner.as_deref(),
        fallback_spawner.as_deref(),
        fallback,
        &gate,
        &project_root,
    )
}

/// Construct the production spawner for a given runtime kind. Codex
/// is wired symmetrically to Claude: same `Default`/`from_env()` path
/// (HEW_LOOP_*_BIN override → PATH fallback). The fallback path uses
/// this directly; the primary path goes through here too so a future
/// runtime addition has one place to extend.
fn build_spawner_for(kind: RuntimeKind) -> Box<dyn RuntimeSpawner> {
    match kind {
        RuntimeKind::Claude => Box::new(ClaudeSpawner::from_env()),
        RuntimeKind::Codex => Box::new(CodexSpawner::from_env()),
    }
}

/// One worker's slot in a (potentially parallel) loop run.
///
/// In v1 the dispatcher only ever constructs a single `Worker` with
/// `worktree_dir = project_root` and `log_dir = .hew/loop/<run-id>/`
/// — that is the `--jobs=1` fast path and matches today's behavior
/// byte-for-byte. The fields exist now so the future parallel
/// dispatcher can fill multiple slots without re-plumbing
/// [`run_worker_loop`]'s signature.
#[derive(Debug, Clone)]
pub struct Worker {
    /// Stable slot id; `0` for the single-worker path.
    pub id: u32,
    /// Working tree this worker drives — every `git` / gate / agent
    /// call inside [`run_worker_loop`] targets this path.
    pub worktree_dir: PathBuf,
    /// Branch the worker is committing to. Informational for v1; the
    /// parallel dispatcher uses this to scope `git reset --hard`.
    pub branch: String,
    /// Run-dir root the worker writes logs against (always
    /// `<project>/.hew/loop/<run-id>/`). Per-iter paths are composed
    /// against this + [`Self::worker_n`] via
    /// [`hew_core::loop_log::iter_log_path`].
    pub log_dir: PathBuf,
    /// Worker slot for path composition: `None` keeps the pre-parallel
    /// layout (`<run-dir>/iter-NNN.json`); `Some(n)` slots logs under
    /// `<run-dir>/worker-<n>/`. The N=1 fast path uses `None` so the
    /// existing `hew loop summary` / `hew loop logs` surfaces continue
    /// to find logs at the run-dir root.
    pub worker_n: Option<u32>,
}

/// Final state returned by [`run_worker_loop`]. The dispatcher reads
/// this to render the end-of-run summary; in the parallel future it
/// will fold N `WorkerOutcome`s into a single report.
#[derive(Debug)]
pub struct WorkerOutcome {
    pub run: Run,
    pub iter_logs: Vec<IterLog>,
}

/// Testable inner. Production [`run_loop`] resolves `bd`, the spawner,
/// the gate runner and the project root; tests construct mocks and call
/// this directly. Returns the same `miette::Result` as `run_loop`.
///
/// Acts as the v1 dispatcher: builds the per-run scaffolding (run-id,
/// log dir, primer text, allowed-tools set), constructs a single
/// [`Worker`] over `project_root`, and delegates the iter loop to
/// [`run_worker_loop`]. The behavior is byte-identical to the
/// pre-split single-threaded loop.
#[allow(clippy::too_many_arguments)]
pub fn run_loop_with(
    ctx: &Ctx,
    args: Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
) -> miette::Result<()> {
    // jobs == 1: today's behavior, byte-identical. Skip the dispatcher
    // entirely so the N=1 fast path never pays for parallel scaffolding
    // (no worktree create, no Dispatcher::new, no merge_back).
    // jobs >= 2: dispatcher path (per task hew-wee).
    if args.jobs <= 1 {
        return run_loop_serial(
            ctx,
            args,
            bd,
            spawner,
            fallback_spawner,
            fallback,
            gate,
            project_root,
        );
    }
    run_loop_parallel(ctx, args, bd, spawner, fallback_spawner, fallback, gate, project_root)
}

/// Today's single-worker loop, factored out so [`run_loop_with`] can
/// branch on `--jobs N` without touching the existing code path. The
/// body below is the original `run_loop_with` verbatim — the rename is
/// the only structural change.
#[allow(clippy::too_many_arguments)]
fn run_loop_serial(
    ctx: &Ctx,
    args: Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
) -> miette::Result<()> {
    let skill = skills::find(&args.skill)
        .ok_or_else(|| miette::miette!("unknown skill `{}`", args.skill))?;

    let run_id = new_run_id();
    let dir = run_dir(project_root, &run_id).map_err(|e| miette::miette!("create run dir: {e}"))?;
    let stop_path = args.stop_file.clone().unwrap_or_else(|| stop_file_path(&dir));

    if !ctx.quiet {
        eprintln!("hew loop {} — run-dir={}", &run_id, dir.display());
        if args.dry_run {
            eprintln!("(--dry-run: no subprocess, no git ops)");
        }
    }

    let allowed = allowed_tools::for_skill(&args.skill);

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

    // v1: one worker, worktree_dir = project_root, log_dir = run-dir.
    // The `branch` field is informational for the single-slot path;
    // the parallel dispatcher (hew-9m5) will populate it with the
    // per-slot `loop/<run-id>/w<n>` branch.
    let worker = Worker {
        id: 0,
        worktree_dir: project_root.to_path_buf(),
        branch: String::new(),
        log_dir: dir.clone(),
        worker_n: None,
    };

    let started_at = iso_now_utc();
    let outcome = run_worker_loop(
        ctx,
        &args,
        bd,
        spawner,
        fallback_spawner,
        fallback,
        gate,
        &worker,
        &skill,
        &primer_text,
        &run_id,
        &allowed,
        &stop_path,
    )?;

    // Dispatcher-shutdown manifest: lists every worker that
    // participated in the run + their final outcome. v1 has a single
    // worker; the future parallel dispatcher folds N outcomes into the
    // same shape so `hew loop summary` / `hew loop logs` can consume
    // both layouts uniformly.
    let workers = vec![worker_manifest_row(&worker, &outcome)];
    let manifest = Manifest {
        run_id: run_id.clone(),
        jobs: workers.len() as u32,
        started_at,
        completed_at: iso_now_utc(),
        workers,
    };
    write_manifest(&dir, &manifest).map_err(|e| miette::miette!("write manifest: {e}"))?;

    print_summary(ctx, &outcome.run, &outcome.iter_logs, &dir);
    Ok(())
}

fn worker_manifest_row(worker: &Worker, outcome: &WorkerOutcome) -> ManifestWorker {
    let summary = RunLog::from_run(&outcome.run);
    ManifestWorker {
        id: worker.id,
        branch: worker.branch.clone(),
        log_subdir: worker.worker_n.map(|n| format!("worker-{n}")),
        iter_count: summary.iter_count,
        cumulative_tokens: summary.cumulative_tokens,
        stop_reason: summary.stop_reason,
    }
}

/// Parallel dispatcher path (`--jobs >= 2`). Builds a
/// [`hew_core::dispatcher::Dispatcher`], lays down one git worktree per
/// worker slot under `~/.hew/wt/<run-id>/<n>/`, and drives each slot's
/// [`run_worker_loop`] in its own scoped thread. On run end the
/// dispatcher consolidates each per-worker branch back onto the launch
/// HEAD via `merge_back`; conflicts file `[merge-conflict]` bug tasks
/// per `DECISION:loop-parallel-overlap-policy`.
///
/// Under `--dry-run` the parallel path still constructs the Dispatcher
/// and drives one tick per slot to honor the "jobs >= 2 invokes the
/// Dispatcher path" acceptance contract, but skips worktree creation
/// and runtime spawn — exactly mirroring the serial dry-run.
#[allow(clippy::too_many_arguments)]
fn run_loop_parallel(
    ctx: &Ctx,
    args: Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
) -> miette::Result<()> {
    let skill = skills::find(&args.skill)
        .ok_or_else(|| miette::miette!("unknown skill `{}`", args.skill))?;

    let run_id = new_run_id();
    let dir = run_dir(project_root, &run_id).map_err(|e| miette::miette!("create run dir: {e}"))?;
    let stop_path = args.stop_file.clone().unwrap_or_else(|| stop_file_path(&dir));

    if !ctx.quiet {
        eprintln!("hew loop {} — jobs={} run-dir={}", &run_id, args.jobs, dir.display());
        if args.dry_run {
            eprintln!("(--dry-run: no subprocess, no worktrees, no git ops)");
        }
    }

    let allowed = allowed_tools::for_skill(&args.skill);
    let primer_text = bd.prime_raw().unwrap_or_default();

    // Capture launch HEAD before any worker mutates the worktree. The
    // dispatcher's merge_back consolidates every worker branch back here.
    let base_sha = if args.dry_run { String::new() } else { git_head_sha(project_root)? };

    // Construct the Dispatcher even under --dry-run so the "invokes
    // Dispatcher" acceptance holds across both paths.
    let mut dispatcher = hew_core::dispatcher::Dispatcher::new(args.jobs, &run_id, &base_sha);

    // v1 wiring: one tick to fill all slots, then drive each worker's
    // loop in a scoped thread. The dispatcher's slot-fill state machine
    // gives us the assignment list; full multi-tick refill + concurrent
    // merge-back lands with the e2e fixtures in hew-d5gd.
    let tick = dispatcher.dispatch_tick(bd).map_err(|e| miette::miette!("dispatch_tick: {e}"))?;

    if !ctx.quiet {
        eprintln!(
            "dispatcher: jobs={} ready_seen={} assigned={} claim_failures={}",
            dispatcher.jobs(),
            tick.ready_seen,
            tick.assignments.len(),
            tick.claim_failures.len(),
        );
    }

    let started_at = iso_now_utc();

    // Build one Worker per assignment. Under --dry-run skip worktree
    // creation; each worker points at project_root and uses no branch
    // (matches the serial dry-run shape so iter logs stay parseable).
    let wt_root_opt = if args.dry_run {
        None
    } else {
        Some(hew_core::worktree::root().map_err(|e| miette::miette!("worktree root: {e}"))?)
    };

    let mut workers: Vec<Worker> = Vec::with_capacity(tick.assignments.len());
    let mut worker_handles: Vec<hew_core::worktree::WorktreeHandle> = Vec::new();
    let git_client = if args.dry_run {
        None
    } else {
        Some(hew_core::git::RealGit::discover().map_err(|e| miette::miette!("git: {e}"))?)
    };

    for a in &tick.assignments {
        let n = a.slot_id;
        let branch = hew_core::worktree::branch_name(&run_id, n);
        let (wt_dir, branch_str) = if let (Some(root), Some(git)) =
            (wt_root_opt.as_ref(), git_client.as_ref())
        {
            let handle =
                hew_core::worktree::create(git, project_root, root, &run_id, n, &base_sha, &branch)
                    .map_err(|e| miette::miette!("worktree create slot {n}: {e}"))?;
            let p = handle.path.clone();
            worker_handles.push(handle);
            (p, branch.clone())
        } else {
            (project_root.to_path_buf(), String::new())
        };
        // Materialize `<run-dir>/worker-<n>/` so the per-worker iter
        // logs land somewhere — `iter_log_path` composes the path but
        // doesn't mkdir, and the worker loop's `write_json_atomic`
        // would otherwise ENOENT on first iter.
        hew_core::loop_log::ensure_worker_dir(&dir, n)
            .map_err(|e| miette::miette!("ensure worker-{n} log dir: {e}"))?;
        workers.push(Worker {
            id: n,
            worktree_dir: wt_dir,
            branch: branch_str,
            log_dir: dir.clone(),
            worker_n: Some(n),
        });
    }

    // Drive each worker's loop in turn. v1 wires the dispatcher slot
    // machine + per-worker worktrees + merge-back without imposing
    // Send/Sync on `BdClient` / `RuntimeSpawner` / `GateRunner` — those
    // bounds + the concurrent spawn land with the e2e fixtures in
    // hew-d5gd. Each worker still owns a disjoint worktree, so this
    // path is correct (just sequential) for the parallel surface.
    let mut worker_outcomes: Vec<WorkerOutcome> = Vec::with_capacity(workers.len());
    for worker in &workers {
        let outcome = run_worker_loop(
            ctx,
            &args,
            bd,
            spawner,
            fallback_spawner,
            fallback,
            gate,
            worker,
            &skill,
            &primer_text,
            &run_id,
            &allowed,
            &stop_path,
        )?;
        worker_outcomes.push(outcome);
    }

    // Release dispatcher slots so its `all_idle()` would report true
    // post-shutdown. Cosmetic for v1 (we don't tick again) but keeps
    // the state machine honest.
    for w in &workers {
        let _ = dispatcher.complete(w.id);
    }

    // Merge each worker branch back onto launch HEAD. Cleanly-merged
    // worktrees are pruned on the way out (graceful teardown — hew-kt5q).
    // Conflicted ones survive on disk because the `[merge-conflict]` bug
    // task points at them for human resolution.
    if !args.dry_run && !workers.is_empty() {
        let branches: Vec<String> =
            workers.iter().map(|w| w.branch.clone()).filter(|b| !b.is_empty()).collect();
        if !branches.is_empty()
            && let (Some(git), Some(wt_root)) = (git_client.as_ref(), wt_root_opt.as_ref())
        {
            match dispatcher.shutdown_merge_back(git, bd, project_root, "HEAD", &branches) {
                Ok((report, bug_ids)) => {
                    if !ctx.quiet {
                        eprintln!(
                            "merge_back: merged={} conflicts={} bugs_filed={}",
                            report.merged.len(),
                            report.conflicts.len(),
                            bug_ids.len(),
                        );
                    }
                    let merged_set: std::collections::HashSet<&str> =
                        report.merged.iter().map(String::as_str).collect();
                    let mut pruned = 0u32;
                    for h in &worker_handles {
                        if !merged_set.contains(h.branch.as_str()) {
                            continue;
                        }
                        if let Err(e) = hew_core::worktree::prune(
                            git,
                            project_root,
                            wt_root,
                            &h.run_id,
                            h.worker_n,
                        ) {
                            eprintln!("worktree prune slot {} failed: {e}", h.worker_n);
                        } else {
                            pruned += 1;
                        }
                    }
                    if pruned > 0 && !ctx.quiet {
                        eprintln!("worktrees: pruned {pruned} cleanly-merged");
                    }
                }
                Err(e) => {
                    eprintln!("merge_back failed: {e}");
                }
            }
        }
    }

    // Per-worker manifest rows; jobs reflects the dispatcher's slot
    // count (matches the user's --jobs N, post-clamp).
    let manifest_rows: Vec<ManifestWorker> = workers
        .iter()
        .zip(worker_outcomes.iter())
        .map(|(w, o)| worker_manifest_row(w, o))
        .collect();
    let manifest = Manifest {
        run_id: run_id.clone(),
        jobs: dispatcher.jobs(),
        started_at,
        completed_at: iso_now_utc(),
        workers: manifest_rows,
    };
    write_manifest(&dir, &manifest).map_err(|e| miette::miette!("write manifest: {e}"))?;

    // v1: print the first worker's summary as a stand-in for the full
    // per-worker breakdown (that's hew-h0tu). Honors the existing
    // "print summary at end" contract so nothing downstream regresses.
    if let Some(first) = worker_outcomes.first() {
        print_summary(ctx, &first.run, &first.iter_logs, &dir);
    }
    Ok(())
}

/// One worker's iter loop. Pulls ready tasks from `bd`, drives the
/// runtime, runs the backpressure gate, logs iters under
/// `worker.log_dir`. All git + gate calls target `worker.worktree_dir`
/// so a future parallel dispatcher can run multiple workers against
/// disjoint worktrees without contention.
///
/// `--jobs=1` constructs a single worker with `worktree_dir =
/// project_root` and `log_dir = .hew/loop/<run-id>/`, preserving the
/// pre-split behavior byte-for-byte.
#[allow(clippy::too_many_arguments)]
pub fn run_worker_loop(
    ctx: &Ctx,
    args: &Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    gate: &dyn GateRunner,
    worker: &Worker,
    skill: &skills::Skill,
    primer_text: &str,
    run_id: &str,
    allowed: &[String],
    stop_path: &Path,
) -> miette::Result<WorkerOutcome> {
    let primary_kind: RuntimeKind =
        args.runtime.parse().map_err(|e: String| miette::miette!("{e}"))?;
    // CooldownState only matters when a fallback spawner is wired —
    // otherwise the loop has nowhere to route and the state machine
    // would just sit at `should_use_fallback() == true` forever.
    let mut cooldown: Option<CooldownState> =
        fallback_spawner.and(fallback.runtime.map(|_| CooldownState::new(fallback.cooldown_iters)));

    let cfg = RunConfig {
        max_iter: args.max_iter,
        stop_on_ready_empty: args.until_empty,
        budget_tokens: args.budget_tokens,
        budget_wall: args.budget_wall,
        strict: args.strict,
        interactive: args.interactive,
        unattended: args.unattended,
    };

    let collector = Collector::new(stop_path.to_path_buf());
    let mut run_state = Run::new(run_id.to_string(), iso_now_utc(), cfg.clone());

    // Install (or refresh) the SIGINT handler so Ctrl+C flips the
    // shared CancelFlag instead of killing the loop process mid-iter.
    // `ctrlc::set_handler` is process-global and refuses a second
    // install; we silently ignore that case so back-to-back loop runs
    // (e.g. integration tests) still work.
    {
        let flag = collector.cancel.clone();
        let _ = ctrlc::set_handler(move || flag.cancel());
    }

    let mut last_outcome: Option<IterOutcome> = None;
    let mut iter_logs: Vec<IterLog> = Vec::new();

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
            match git_head_sha(&worker.worktree_dir) {
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
        let assembled = prompt::assemble(skill.body, primer_text, &task_brief);

        if !ctx.quiet {
            eprintln!(
                "iter {} — task={} prefix_hash={:016x} est_tokens={}",
                iter_number, task.id, assembled.prefix_hash, assembled.token_estimate
            );
        }

        // Cooldown drives spawner selection when a fallback is wired:
        // primary by default; fallback while `should_use_fallback()` is
        // true. The same bool is needed AFTER the spawn to record the
        // outcome on the correct side of the state machine, so capture
        // it once.
        let on_fallback = cooldown.as_ref().map(|c| c.should_use_fallback()).unwrap_or(false)
            && fallback_spawner.is_some();
        let active_spawner: Option<&dyn RuntimeSpawner> = match (spawner, on_fallback) {
            (Some(_), true) => fallback_spawner,
            (Some(p), false) => Some(p),
            (None, _) => None,
        };
        let active_kind =
            if on_fallback { fallback.runtime.unwrap_or(primary_kind) } else { primary_kind };

        let (mut outcome, tokens, mut stderr_tail, failure_class) = if let Some(s) = active_spawner
        {
            // SpawnOpts::default() until Epic D wires per-task model
            // resolution; opts is the future channel for that override.
            match s.spawn(&assembled, allowed, &SpawnOpts::default()) {
                Ok(out) => {
                    let oc = if out.success && out.closed_task.is_some() {
                        IterOutcome::Closed
                    } else if out.success {
                        IterOutcome::NoClose
                    } else {
                        IterOutcome::RuntimeError
                    };
                    (oc, out.tokens, Some(out.stderr_tail), out.failure_class)
                }
                Err(e) => {
                    if !ctx.quiet {
                        eprintln!("iter {iter_number} runtime error: {e}");
                    }
                    (
                        IterOutcome::RuntimeError,
                        Default::default(),
                        Some(format!("{e}")),
                        SpawnFailureClass::RuntimeError(hew_core::runtime::RuntimeErrorKind::Spawn),
                    )
                }
            }
        } else {
            (IterOutcome::NoClose, Default::default(), None, SpawnFailureClass::Success)
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
            let check = gate.run_gate(&worker.worktree_dir);
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
                        if let Err(e) = git_reset_hard(&worker.worktree_dir, sha) {
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
                    hew_core::decide::BdDecisionContext::new(bd, worker.worktree_dir.clone());
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

        // Update cooldown machinery before we log the iter so
        // `cooldown_engaged` reflects post-iter state.
        if let Some(c) = cooldown.as_mut() {
            // BackpressureFail / gate failures aren't a runtime issue
            // — map only true RuntimeError outcomes through. Everything
            // else feeds the original `failure_class` so Success keeps
            // draining the cooldown window and GuardTrip/Budget pass
            // through unchanged.
            let class = if matches!(outcome, IterOutcome::RuntimeError) {
                failure_class
            } else {
                SpawnFailureClass::Success
            };
            c.record_outcome(on_fallback, class);
        }

        let prefix_hash_hex = Some(format!("{:016x}", assembled.prefix_hash));
        // Symbol-level changelog of the iter: blast against the
        // pre-iter sha when treesitter is compiled in and the iter
        // actually produced commits. Best-effort: any error in the
        // blast path collapses to an empty list — we never let an
        // observability signal block iter logging.
        let symbols_touched =
            compute_iter_symbols(&worker.worktree_dir, pre_iter_sha.as_deref(), &outcome);
        let mut log = IterLog::from_iter(&iter, prefix_hash_hex, Vec::new(), symbols_touched);
        if active_spawner.is_some() {
            log.runtime_used = Some(active_kind.as_str().to_string());
        }
        log.cooldown_engaged = cooldown.as_ref().map(|c| c.in_cooldown).unwrap_or(false);
        write_json_atomic(&iter_log_path(&worker.log_dir, worker.worker_n, iter_number), &log)
            .map_err(|e| miette::miette!("write iter log: {e}"))?;
        iter_logs.push(log);

        // When a fallback is wired and the cooldown machinery is
        // actively routing iters, swallow RuntimeError from the
        // stop-signal point of view — the loop should switch to the
        // fallback on the next iter rather than aborting. The iter
        // log still records the true outcome.
        let stop_outcome = if cooldown.as_ref().map(|c| c.in_cooldown).unwrap_or(false)
            && matches!(outcome, IterOutcome::RuntimeError)
        {
            Some(IterOutcome::NoClose)
        } else {
            Some(outcome)
        };
        last_outcome = stop_outcome;
        run_state.iters.push(iter);

        // Rewrite run.json after each iter.
        write_json_atomic(
            &run_log_path(&worker.log_dir, worker.worker_n),
            &RunLog::from_run(&run_state),
        )
        .map_err(|e| miette::miette!("write run log: {e}"))?;
    }

    // Final summary persisted to disk; the dispatcher renders the
    // pretty end-of-run report from the returned WorkerOutcome.
    let summary = RunLog::from_run(&run_state);
    write_json_atomic(&run_log_path(&worker.log_dir, worker.worker_n), &summary)
        .map_err(|e| miette::miette!("write final run log: {e}"))?;

    Ok(WorkerOutcome { run: run_state, iter_logs })
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
        // v1 read surface only consults the run-dir root (single-worker
        // layout). Per-worker reads land in hew-h0tu.
        let path = iter_log_path(&dir, None, n);
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

    let run_log_path = run_log_path(&dir, None);
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
    let rl_body = std::fs::read_to_string(run_log_path(&dir, None))
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

pub fn run_prune_worktrees(ctx: &Ctx, args: PruneWorktreesArgs) -> miette::Result<()> {
    let project_root = std::env::current_dir().map_err(|e| miette::miette!("cwd: {e}"))?;
    let wt_root = hew_core::worktree::root().map_err(|e| miette::miette!("worktree root: {e}"))?;
    let loop_root = loop_root(&project_root);

    let active = hew_core::loop_log::active_run_ids(&loop_root)
        .map_err(|e| miette::miette!("scan loop dir: {e}"))?;
    let orphans = hew_core::worktree::list_orphans(&wt_root, &active)
        .map_err(|e| miette::miette!("list orphans: {e}"))?;

    if orphans.is_empty() {
        if !ctx.quiet {
            println!("(no orphan worktrees under {})", wt_root.display());
        }
        return Ok(());
    }

    if !args.apply {
        if !ctx.quiet {
            println!(
                "{} orphan worktree{} (dry-run; pass --apply to remove):",
                orphans.len(),
                if orphans.len() == 1 { "" } else { "s" },
            );
            for h in &orphans {
                println!("  {} (branch {})", h.path.display(), h.branch);
            }
        }
        return Ok(());
    }

    let git = hew_core::git::RealGit::discover().map_err(|e| miette::miette!("git: {e}"))?;
    let mut removed = 0u32;
    let mut failed = 0u32;
    for h in &orphans {
        match hew_core::worktree::prune(&git, &project_root, &wt_root, &h.run_id, h.worker_n) {
            Ok(()) => removed += 1,
            Err(e) => {
                failed += 1;
                eprintln!("prune {} failed: {e}", h.path.display());
            }
        }
    }
    if !ctx.quiet {
        if failed == 0 {
            println!("pruned {removed} orphan worktree{}", if removed == 1 { "" } else { "s" });
        } else {
            println!("pruned {removed}; {failed} failed");
        }
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
    let rl_path = run_log_path(dir, None);
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
