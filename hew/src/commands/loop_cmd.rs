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

use clap::{Args as ClapArgs, Subcommand, ValueEnum};
use hew_core::backpressure::{self, GateCheck, Verdict};
use hew_core::batch_plan;
use hew_core::batch_plan::{BatchPlan, BatchSource, SCHEMA_VERSION as BATCH_PLAN_SCHEMA_VERSION};
use hew_core::batch_plan_parse::extract_next_iteration;
use hew_core::bd::{BdClient, ReadyTask, RealBd};
use hew_core::config::{LoopModelConfig, LoopPlannerConfig};
use hew_core::error::HewError;
use hew_core::loop_log::{
    IterLog, LOOP_ROOT, Manifest, ManifestWorker, RunLog, iter_log_path, new_run_id, run_dir,
    run_log_path, stop_file_path, write_json_atomic, write_manifest,
};
use hew_core::loop_model::{TaskRecord, resolve_model};
use hew_core::prompt;
use hew_core::runner::{CooldownState, Iter, IterOutcome, Run, RunConfig};
use hew_core::runtime::{
    ClaudeSpawner, CodexSpawner, FallbackConfig, RuntimeKind, RuntimeSpawner, SpawnFailureClass,
    SpawnOpts,
};
use hew_core::scope::Scope;
use hew_core::stop_signals::Collector;
use hew_core::tasks;
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
#[allow(clippy::large_enum_variant)]
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

    /// Restrict the run's queue. `ready` = today's behavior (every
    /// bd-ready task counts); `epics` = only tasks transitively under
    /// the epics passed via `--epics`. Omitting the flag opens a
    /// picker on a TTY and errors in non-interactive mode.
    #[arg(long, value_enum)]
    pub scope: Option<ScopeArg>,

    /// Comma-separated epic ids to scope this run to. Required (or
    /// picked interactively) when `--scope=epics`. May be repeated.
    /// Example: `--epics=hew-6az,hew-1tq`.
    #[arg(long, value_delimiter = ',')]
    pub epics: Vec<String>,

    /// Ergonomic singular alias for `--epics`; merges into the same
    /// list. Example: `--epic hew-6az --epic hew-1tq`.
    #[arg(long = "epic", value_name = "EPIC_ID")]
    pub epic: Vec<String>,

    /// Disable the inter-iter planner for this run. When set, every
    /// iter-end that doesn't surface an agent-named `next_iteration:`
    /// block writes a `Skipped { reason: "planner_disabled" }` batch
    /// plan instead of spawning a planner subprocess. Overrides
    /// `loop.planner.enabled` config.
    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    pub no_planner: bool,

    /// Per-spawn token-estimate budget for the planner. Overrides
    /// `loop.planner.budget_tokens` config. `0` disables planner
    /// spawns without flipping `--no-planner`. Default `10000`.
    #[arg(long)]
    pub planner_budget: Option<u32>,

    /// Runtime to drive the planner. Overrides
    /// `loop.planner.runtime` config; falling back to the loop's
    /// primary runtime when unset.
    #[arg(
        long,
        value_parser = clap::builder::PossibleValuesParser::new(RuntimeKind::VARIANTS),
    )]
    pub planner_runtime: Option<String>,

    /// Run a mandatory end-of-run test command after the last iter
    /// (and after merge-back on `--jobs >= 2`) to prove the final
    /// stacked state is green. Overrides
    /// `loop.end_of_run.verify_tests` config. Default `false`.
    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    pub verify_tests: bool,

    /// Explicit-off for the end-of-run verify step, takes precedence
    /// over `--verify-tests` and `loop.end_of_run.verify_tests`
    /// config. Useful when a config opts in globally but a particular
    /// run shouldn't pay the verify cost (e.g. dry-run experiments).
    #[arg(long, default_value_t = false, action = clap::ArgAction::SetTrue)]
    pub no_verify_tests: bool,

    /// Override the resolved verify command for this run. Empty =
    /// fall back to `loop.end_of_run.verify_command` config, then to
    /// project-authored signals (justfile/Makefile/package.json
    /// `test`) via `hew_core::gate::detect`.
    #[arg(long)]
    pub verify_command: Option<String>,
}

/// Resolve the effective [`LoopPlannerConfig`] for this run. Precedence:
///
/// 1. `--no-planner` CLI flag (sticky `enabled = false`)
/// 2. Per-flag overrides (`--planner-budget`, `--planner-runtime`)
/// 3. `loop.planner.*` config values
/// 4. Compiled-in defaults (enabled, 10_000 tokens, runtime = primary)
///
/// Validates the planner runtime is a known [`RuntimeKind`] so a bad
/// CLI/config value fails before the run starts rather than at first
/// iter-end spawn attempt.
pub fn resolve_planner_config(
    args: &Args,
    base: &LoopPlannerConfig,
) -> miette::Result<LoopPlannerConfig> {
    let mut out = base.clone();
    if args.no_planner {
        out.enabled = false;
    }
    if let Some(b) = args.planner_budget {
        out.budget_tokens = b;
    }
    if let Some(r) = args.planner_runtime.as_deref() {
        // Validate against the RuntimeKind allowlist. Empty string
        // clears the override.
        if r.is_empty() {
            out.runtime = None;
        } else {
            let _: RuntimeKind = r.parse().map_err(|e: String| miette::miette!("{e}"))?;
            out.runtime = Some(r.to_string());
        }
    } else if let Some(r) = out.runtime.as_deref() {
        // Validate config-sourced runtime too — bad on-disk config
        // shouldn't silently turn into a missing planner.
        let _: RuntimeKind = r.parse().map_err(|e: String| miette::miette!("{e}"))?;
    }
    Ok(out)
}

/// CLI surface of [`Scope`]. The runtime type lives in
/// `hew_core::scope` so dispatcher / runner / loop_log share a single
/// definition; this enum exists only to give clap a ValueEnum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum ScopeArg {
    /// Every bd-ready task counts (pre-scope default).
    Ready,
    /// Restrict to children of one or more epics.
    Epics,
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

    // Pre-flight: argv-level scope errors (MissingFlag in non-interactive
    // mode, --scope=ready + --epics contradiction) must fire BEFORE
    // `bd discover`. Otherwise a CI that lacks `bd` on PATH masks every
    // MissingFlag assertion with a generic `bd binary not found` error
    // and contract tests can't tell the two failure paths apart.
    precheck_scope_argv(&args, ctx)?;

    let bd = RealBd::discover().map_err(|e| miette::miette!("bd discover: {e}"))?;

    // Resolve --scope/--epics once, before any spawner is built. argv
    // > picker > non-interactive error. Cancel exits 0 with a note.
    let scope = match resolve_scope(&args, ctx, &bd)? {
        ResolvedScope::Scope(s) => s,
        ResolvedScope::Cancelled => {
            if !ctx.quiet {
                eprintln!("hew loop: no epics selected — run cancelled");
            }
            return Ok(());
        }
    };

    let spawner: Option<Box<dyn RuntimeSpawner>> =
        if args.dry_run { None } else { Some(build_spawner_for(kind)) };
    let fallback_spawner: Option<Box<dyn RuntimeSpawner>> =
        if args.dry_run { None } else { fallback.runtime.map(build_spawner_for) };
    let gate = AutoGateRunner;
    let loop_model = cfg.loop_cfg.model.clone();
    let planner_cfg = resolve_planner_config(&args, &cfg.loop_cfg.planner)?;
    run_loop_with_scope(
        ctx,
        args,
        &bd,
        spawner.as_deref(),
        fallback_spawner.as_deref(),
        fallback,
        loop_model,
        planner_cfg,
        &gate,
        &project_root,
        scope,
    )
}

/// Resolution outcome of [`resolve_scope`]. `Cancelled` means the user
/// backed out of the epic picker — the caller should exit 0 with a
/// note rather than starting a run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedScope {
    Scope(Scope),
    Cancelled,
}

/// Resolve the run's [`Scope`] from CLI args, the interactive picker,
/// or refuse with [`HewError::MissingFlag`] in non-interactive mode.
///
/// Precedence (per hew-xhhw acceptance):
/// 1. `--scope=ready` → [`Scope::Ready`]; reject if `--epics` was set.
/// 2. `--scope=epics --epics=<csv>` → validate each id (exists, is
///    epic, open) and return [`Scope::Epics`].
/// 3. `--scope=epics` with no `--epics`: interactive → multi-select
///    picker over open epics; non-interactive → MissingFlag("epics").
/// 4. `--scope` omitted: interactive → single-select (ready/epics),
///    chained into the epic picker on `epics`; non-interactive →
///    MissingFlag("scope").
///
/// Picker UX is implemented inline against `inquire` to match the
/// existing patterns in `commands/init.rs` and `commands/remember.rs`.
/// Pre-flight argv validation for `--scope` / `--epics`. Fires the
/// errors that don't need `bd` to be on PATH:
///
/// 1. `--scope=ready` combined with `--epics` / `--epic` — contradiction.
/// 2. `--scope=epics` with no epics list in non-interactive mode —
///    `MissingFlag("epics")`.
/// 3. No `--scope` argv in non-interactive mode — `MissingFlag("scope")`.
///
/// Called before `RealBd::discover()` so CI that lacks `bd` still
/// surfaces these as the correct error class (the contract tests in
/// `tests/loop_scope_e2e.rs` assert on the exact MissingFlag string).
/// The remaining branches (id validation, interactive pickers) live
/// in [`resolve_scope`] and run after `bd` is on hand.
pub fn precheck_scope_argv(args: &Args, ctx: &Ctx) -> miette::Result<()> {
    let mut epics: Vec<String> = args.epics.clone();
    epics.extend(args.epic.iter().cloned());

    match args.scope {
        Some(ScopeArg::Ready) if !epics.is_empty() => {
            Err(miette::miette!("--scope=ready does not accept --epics/--epic (got {:?})", epics,))
        }
        Some(ScopeArg::Epics) if epics.is_empty() && !ctx.interactive => {
            Err(HewError::MissingFlag { flag: "epics".into() }.into())
        }
        None if !ctx.interactive => Err(HewError::MissingFlag { flag: "scope".into() }.into()),
        _ => Ok(()),
    }
}

pub fn resolve_scope(args: &Args, ctx: &Ctx, bd: &dyn BdClient) -> miette::Result<ResolvedScope> {
    // Merge --epic (singular) into --epics (plural): both feed the
    // same list. Argv order is preserved so the picker echoes the
    // user's intent.
    let mut epics: Vec<String> = args.epics.clone();
    epics.extend(args.epic.iter().cloned());

    match args.scope {
        Some(ScopeArg::Ready) => {
            if !epics.is_empty() {
                return Err(miette::miette!(
                    "--scope=ready does not accept --epics/--epic (got {:?})",
                    epics,
                ));
            }
            Ok(ResolvedScope::Scope(Scope::Ready))
        }
        Some(ScopeArg::Epics) => {
            if !epics.is_empty() {
                validate_epic_ids(bd, &epics)?;
                Ok(ResolvedScope::Scope(Scope::Epics { epic_ids: epics }))
            } else if ctx.interactive {
                pick_epics(bd)
            } else {
                Err(HewError::MissingFlag { flag: "epics".into() }.into())
            }
        }
        None => {
            if ctx.interactive {
                pick_scope_then_epics(bd)
            } else {
                Err(HewError::MissingFlag { flag: "scope".into() }.into())
            }
        }
    }
}

/// Confirm each id resolves to an open epic. Closed / non-epic / unknown
/// ids fail fast at resolve time so an iter never spawns against a stale
/// queue.
fn validate_epic_ids(bd: &dyn BdClient, ids: &[String]) -> miette::Result<()> {
    for id in ids {
        let summary = tasks::show(bd, id)
            .map_err(|e| miette::miette!("--epics: id `{id}` not found in bd ({e})"))?;
        if summary.issue_type != "epic" {
            return Err(miette::miette!(
                "--epics: id `{id}` is type `{}`, not `epic`",
                summary.issue_type,
            ));
        }
        if summary.status == "closed" {
            return Err(miette::miette!("--epics: epic `{id}` is closed"));
        }
    }
    Ok(())
}

/// Interactive single-select for scope kind, chained into the
/// multi-select epic picker when the user picks "epics".
fn pick_scope_then_epics(bd: &dyn BdClient) -> miette::Result<ResolvedScope> {
    use inquire::Select;
    let labels = vec![
        "ready  — every bd-ready task (default behavior)",
        "epics  — restrict to children of selected epics",
    ];
    let pick = Select::new("Scope this run to:", labels)
        .prompt()
        .map_err(|e| miette::miette!("scope pick: {e}"))?;
    if pick.starts_with("ready") { Ok(ResolvedScope::Scope(Scope::Ready)) } else { pick_epics(bd) }
}

/// Multi-select picker over open epics. Empty selection cancels the
/// run (caller exits 0 with a note).
fn pick_epics(bd: &dyn BdClient) -> miette::Result<ResolvedScope> {
    use inquire::MultiSelect;
    let mut open_epics = tasks::list(
        bd,
        &tasks::TaskListFilter {
            status: vec!["open".into(), "in_progress".into(), "blocked".into()],
            issue_type: Some("epic".into()),
            ..Default::default()
        },
    )
    .map_err(|e| miette::miette!("bd list open epics: {e}"))?;
    open_epics.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.id.cmp(&b.id)));

    if open_epics.is_empty() {
        return Err(miette::miette!("no open epics to scope this run to"));
    }

    let labels: Vec<String> = open_epics.iter().map(|e| format!("{}  {}", e.id, e.title)).collect();
    let picked = MultiSelect::new("Select one or more epics", labels.clone())
        .with_help_message("space to toggle, enter to confirm — empty cancels the run")
        .prompt()
        .map_err(|e| miette::miette!("epic pick: {e}"))?;
    if picked.is_empty() {
        return Ok(ResolvedScope::Cancelled);
    }
    let epic_ids: Vec<String> = picked
        .iter()
        .filter_map(|l| labels.iter().position(|x| x == l).map(|i| open_epics[i].id.clone()))
        .collect();
    Ok(ResolvedScope::Scope(Scope::Epics { epic_ids }))
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
/// Back-compat wrapper for the original `run_loop_with` signature
/// (pre-hew-xhhw). Callers that don't care about scope get
/// [`Scope::Ready`], which is byte-identical to the legacy behavior.
/// Production wiring goes through [`run_loop_with_scope`] via
/// [`run_loop`]; existing integration tests stay on this signature.
#[allow(clippy::too_many_arguments)]
pub fn run_loop_with(
    ctx: &Ctx,
    args: Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    loop_model: LoopModelConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
) -> miette::Result<()> {
    run_loop_with_scope(
        ctx,
        args,
        bd,
        spawner,
        fallback_spawner,
        fallback,
        loop_model,
        LoopPlannerConfig::default(),
        gate,
        project_root,
        Scope::Ready,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_loop_with_scope(
    ctx: &Ctx,
    args: Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    loop_model: LoopModelConfig,
    planner_cfg: LoopPlannerConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
    scope: Scope,
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
            loop_model,
            planner_cfg,
            gate,
            project_root,
            scope,
        );
    }
    run_loop_parallel(
        ctx,
        args,
        bd,
        spawner,
        fallback_spawner,
        fallback,
        loop_model,
        planner_cfg,
        gate,
        project_root,
        scope,
    )
}

/// End-of-run verify step (`hew-bon7`). Opt-in via `--verify-tests`
/// or `loop.end_of_run.verify_tests = true`. Resolves the command
/// from CLI > config > [`hew_core::gate::detect`], spawns it under
/// the configured wall budget, records the outcome onto `run.verify_outcome`,
/// re-writes `run.json` so the persisted summary matches, and writes
/// a `STATUS:loop-verify-failed:<run-id>` memory on failure so the
/// next session sees the regression. No-op when verify is disabled
/// (no record written, summary line absent).
fn maybe_run_verify_step(
    ctx: &Ctx,
    args: &Args,
    bd: &dyn BdClient,
    run: &mut Run,
    working_dir: &Path,
    run_dir: &Path,
    worker_n: Option<u32>,
) {
    // CLI > config > defaults. `--no-verify-tests` always wins so a
    // global config opt-in can be vetoed per-run.
    let cfg = match hew_core::config::load() {
        Ok(c) => c.loop_cfg.end_of_run,
        Err(_) => hew_core::config::LoopEndOfRunConfig::default(),
    };
    if args.no_verify_tests {
        return;
    }
    let enabled = args.verify_tests || cfg.verify_tests;
    if !enabled {
        return;
    }

    let gate = hew_core::gate::detect(working_dir);
    let command = hew_core::verify::resolve_command(
        args.verify_command.as_deref(),
        Some(&cfg.verify_command),
        &gate,
    );
    let outcome = match command {
        None => hew_core::verify::VerifyOutcome::Skipped { reason: "no command resolved".into() },
        Some(cmd) => {
            let budget = hew_core::config::parse_budget_wall(&cfg.verify_budget_wall)
                .unwrap_or_else(|_| Duration::from_secs(600));
            let log_path = run_dir.join("verify.log");
            if !ctx.quiet {
                eprintln!("hew loop verify: {} (budget {}s)", cmd.join(" "), budget.as_secs());
            }
            hew_core::verify::run_verify(&cmd, working_dir, &log_path, budget)
        }
    };

    // Persist on the in-memory Run before re-writing run.json so the
    // summary line + manifest see a consistent state.
    run.verify_outcome = Some(outcome.clone());
    let _ = write_json_atomic(&run_log_path(run_dir, worker_n), &RunLog::from_run(run));

    // STATUS memory on failure — survives across sessions so the next
    // resume sees the regression. We deliberately do NOT file a bd
    // task because closed work is not rolled back; the memory is the
    // breadcrumb, the user decides on follow-up.
    if outcome.is_failure() {
        let summary = outcome.summary_line();
        let body = format!(
            "STATUS:loop-verify-failed:{} — {} (run-dir={})",
            run.id,
            summary,
            run_dir.display(),
        );
        if let Err(e) = bd.remember(&body) {
            tracing::warn!("failed to file STATUS:loop-verify-failed memory: {e}");
        }
    }
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
    loop_model: LoopModelConfig,
    planner_cfg: LoopPlannerConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
    scope: Scope,
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
    let mut outcome = run_worker_loop_with_scope(
        ctx,
        &args,
        bd,
        spawner,
        fallback_spawner,
        fallback,
        loop_model.clone(),
        planner_cfg.clone(),
        gate,
        &worker,
        &skill,
        &primer_text,
        &run_id,
        &allowed,
        &stop_path,
        scope.clone(),
    )?;

    // End-of-run verify (hew-bon7): opt-in. Runs in the project root
    // for the serial path; on `--jobs N>=2` the parallel path runs it
    // after merge-back below. Records the outcome onto `Run.verify_outcome`
    // so the summary line + STATUS memory + exit code branch on a
    // single value.
    maybe_run_verify_step(ctx, &args, bd, &mut outcome.run, project_root, &dir, worker.worker_n);

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

    let scope = Some(outcome.run.config.scope.clone());
    print_summary(ctx, &outcome.run, &outcome.iter_logs, &dir, scope);

    // Verify failure ⇒ non-zero exit (acceptance: "CI / wrapper scripts
    // can branch on this"). Closed tasks are NOT rolled back; the
    // STATUS:loop-verify-failed memory + summary line + non-zero exit
    // are the durable signals.
    if outcome.run.verify_outcome.as_ref().is_some_and(|v| v.is_failure()) {
        return Err(miette::miette!("verify-tests failed"));
    }
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
    loop_model: LoopModelConfig,
    planner_cfg: LoopPlannerConfig,
    gate: &dyn GateRunner,
    project_root: &Path,
    scope: Scope,
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
    // Dispatcher" acceptance holds across both paths. The scope was
    // resolved once at the top of `run_loop` and threaded here.
    let mut dispatcher =
        hew_core::dispatcher::Dispatcher::new(args.jobs, &run_id, &base_sha, scope.clone(), None);

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
        let outcome = run_worker_loop_with_scope(
            ctx,
            &args,
            bd,
            spawner,
            fallback_spawner,
            fallback,
            loop_model.clone(),
            planner_cfg.clone(),
            gate,
            worker,
            &skill,
            &primer_text,
            &run_id,
            &allowed,
            &stop_path,
            scope.clone(),
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

    // End-of-run verify (hew-bon7). On the parallel path the verify
    // command runs in `project_root` (post-merge-back HEAD) and the
    // outcome is recorded on the first worker's Run so the existing
    // `print_summary(first, ...)` contract picks it up. Per-worker
    // outcomes are not duplicated — the verify proves the stacked
    // post-merge state, not any one worker's branch.
    if let Some(first) = worker_outcomes.first_mut() {
        let worker_n = workers.first().and_then(|w| w.worker_n);
        maybe_run_verify_step(ctx, &args, bd, &mut first.run, project_root, &dir, worker_n);
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
    let verify_failed = worker_outcomes
        .first()
        .and_then(|f| f.run.verify_outcome.as_ref())
        .is_some_and(|v| v.is_failure());
    if let Some(first) = worker_outcomes.first() {
        let scope = Some(first.run.config.scope.clone());
        print_summary(ctx, &first.run, &first.iter_logs, &dir, scope);
    }
    if verify_failed {
        return Err(miette::miette!("verify-tests failed"));
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
///
/// Back-compat wrapper for tests that pre-date hew-xhhw: defaults
/// the scope to [`Scope::Ready`] (legacy behavior).
#[allow(clippy::too_many_arguments)]
pub fn run_worker_loop(
    ctx: &Ctx,
    args: &Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    loop_model: LoopModelConfig,
    gate: &dyn GateRunner,
    worker: &Worker,
    skill: &skills::Skill,
    primer_text: &str,
    run_id: &str,
    allowed: &[String],
    stop_path: &Path,
) -> miette::Result<WorkerOutcome> {
    run_worker_loop_with_scope(
        ctx,
        args,
        bd,
        spawner,
        fallback_spawner,
        fallback,
        loop_model,
        LoopPlannerConfig::default(),
        gate,
        worker,
        skill,
        primer_text,
        run_id,
        allowed,
        stop_path,
        Scope::Ready,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_worker_loop_with_scope(
    ctx: &Ctx,
    args: &Args,
    bd: &dyn BdClient,
    spawner: Option<&dyn RuntimeSpawner>,
    fallback_spawner: Option<&dyn RuntimeSpawner>,
    fallback: FallbackConfig,
    loop_model: LoopModelConfig,
    planner_cfg: LoopPlannerConfig,
    gate: &dyn GateRunner,
    worker: &Worker,
    skill: &skills::Skill,
    primer_text: &str,
    run_id: &str,
    allowed: &[String],
    stop_path: &Path,
    scope: Scope,
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
        loop_model,
        scope,
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

        // Per-task model resolution (Epic D / hew-1tq). Honors
        // description tag > label > config precedence — see
        // `hew_core::loop_model::resolve_model`. Empty `LoopModelConfig`
        // + un-annotated task ⇒ `None`, behavior identical to the
        // pre-epic spawner default.
        let model_override = resolve_model(
            &TaskRecord {
                description: &task.description,
                labels: &[],
                priority: task.priority,
                issue_type: &task.issue_type,
            },
            &cfg.loop_model,
        );
        let spawn_opts = SpawnOpts { model_override, working_dir: None };

        let (mut outcome, tokens, mut stderr_tail, failure_class, raw_text) = if let Some(s) =
            active_spawner
        {
            match s.spawn(&assembled, allowed, &spawn_opts) {
                Ok(out) => {
                    let oc = if out.success && out.closed_task.is_some() {
                        IterOutcome::Closed
                    } else if out.success {
                        IterOutcome::NoClose
                    } else {
                        IterOutcome::RuntimeError
                    };
                    (oc, out.tokens, Some(out.stderr_tail), out.failure_class, out.raw_text)
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
                        String::new(),
                    )
                }
            }
        } else {
            (
                IterOutcome::NoClose,
                Default::default(),
                None,
                SpawnFailureClass::Success,
                String::new(),
            )
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
            log.model = spawn_opts.model_override.clone();
        }
        log.cooldown_engaged = cooldown.as_ref().map(|c| c.in_cooldown).unwrap_or(false);
        write_json_atomic(&iter_log_path(&worker.log_dir, worker.worker_n, iter_number), &log)
            .map_err(|e| miette::miette!("write iter log: {e}"))?;
        iter_logs.push(log);

        // Iter-end batch-plan hook (hew-7k1m). Only fires for the
        // parallel path (`--jobs >= 2`); the N=1 fast path never writes
        // a batch file. The plan describes the NEXT iter's batch and
        // resolves via four branches:
        //   1. Agent: previous iter's raw_text named a `next_iteration:`
        //      block.
        //   2. Planner: planner runtime returns a fresh batch (Planner
        //      source) or declines (Skipped with parse/runtime/budget
        //      reason).
        //   3. Skipped: planner disabled → `reason = "planner_disabled"`.
        //   4. (jobs == 1): layer bypassed entirely.
        if args.jobs > 1 {
            let next_iter = iter_number + 1;
            let planner_kind = planner_cfg
                .runtime
                .as_deref()
                .map(|r| r.parse::<RuntimeKind>())
                .transpose()
                .map_err(|e: String| miette::miette!("{e}"))?
                .unwrap_or(primary_kind);
            let plan = resolve_iter_completion_plan(&raw_text, &planner_cfg, next_iter, |ni| {
                // Reuse the active loop spawner when it matches the
                // planner's runtime — avoids constructing a second
                // subprocess channel for the common config-less path.
                let inherited = if planner_kind == active_kind { active_spawner } else { None };
                let built;
                let planner_spawner: &dyn RuntimeSpawner = if let Some(s) = inherited {
                    s
                } else {
                    built = build_spawner_for(planner_kind);
                    &*built
                };
                spawn_planner_with(
                    planner_spawner,
                    &[],
                    &[],
                    planner_cfg.budget_tokens,
                    &worker.worktree_dir,
                    ni,
                )
            });
            if let Err(e) = batch_plan::write(&worker.log_dir, &plan)
                && !ctx.quiet
            {
                eprintln!("iter {iter_number} batch-plan write failed: {e}");
            }
        }

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

fn print_summary(
    ctx: &Ctx,
    run: &Run,
    iter_logs: &[IterLog],
    dir: &std::path::Path,
    scope: Option<hew_core::scope::Scope>,
) {
    if ctx.quiet {
        return;
    }
    let mut summary = hew_core::loop_summary::summarize(run, iter_logs);
    summary.scope = scope;
    summary.planner_counts = hew_core::loop_summary::scan_planner_counts(dir);
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

    // Parallel runs ship a top-level manifest.json. When present, render
    // the per-worker breakdown first, then fall through to the regular
    // aggregate block built from the union of all workers' iter logs.
    let manifest_path = hew_core::loop_log::manifest_path(&dir);
    if manifest_path.exists() {
        return run_summary_parallel(ctx, &dir, &manifest_path);
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
        verify_outcome: rl.verify_outcome.clone(),
    };

    print_summary(ctx, &run, &iter_logs, &dir, rl.scope.clone());
    Ok(())
}

fn collect_worker_iter_logs(run_dir: &Path, worker_n: u32) -> Vec<IterLog> {
    let dir = hew_core::loop_log::worker_dir(run_dir, worker_n);
    if !dir.exists() {
        return Vec::new();
    }
    collect_iter_logs(&dir).unwrap_or_default()
}

fn run_summary_parallel(ctx: &Ctx, dir: &Path, manifest_path: &Path) -> miette::Result<()> {
    let body = std::fs::read_to_string(manifest_path)
        .map_err(|e| miette::miette!("read manifest.json: {e}"))?;
    let manifest: hew_core::loop_log::Manifest =
        serde_json::from_str(&body).map_err(|e| miette::miette!("parse manifest.json: {e}"))?;

    // Build per-worker slices for the breakdown table.
    let mut slices = Vec::with_capacity(manifest.workers.len());
    let mut aggregated_iters: Vec<IterLog> = Vec::new();
    for row in &manifest.workers {
        let iter_logs = collect_worker_iter_logs(dir, row.id);
        slices.push(hew_core::loop_summary::worker_slice(row, &iter_logs));
        aggregated_iters.extend(iter_logs);
    }

    if !ctx.quiet {
        let colorize = std::env::var_os("NO_COLOR").is_none();
        print!("{}", hew_core::loop_summary::render_parallel_breakdown(&slices, colorize));
    }

    // Aggregate block: synthesize a Run covering the whole parallel
    // window so the existing summary renderer reports total tokens,
    // outcomes, and cache stats across all workers.
    aggregated_iters.sort_by(|a, b| a.started_at.cmp(&b.started_at).then(a.number.cmp(&b.number)));
    let stop_reason = manifest
        .workers
        .iter()
        .find_map(|w| w.stop_reason.as_deref())
        .and_then(hew_core::runner::StopReason::from_label);
    let mut iters: Vec<Iter> = aggregated_iters
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
        .collect();
    // Honest wall-clock window: manifest spans the whole parallel run
    // even when individual workers started later or finished earlier.
    if let Some(first) = iters.first_mut() {
        first.started_at = manifest.started_at.clone();
    }
    if let Some(last) = iters.last_mut() {
        last.ended_at = Some(manifest.completed_at.clone());
    }
    let run = Run {
        id: manifest.run_id.clone(),
        started_at: manifest.started_at.clone(),
        config: RunConfig::default(),
        iters,
        stop_reason,
        verify_outcome: None,
    };

    // Scope is dispatcher-level and identical across workers; read it
    // from the first worker's run.json so legacy parallel runs (no
    // scope field) still render as "ready (legacy)".
    let scope = manifest
        .workers
        .first()
        .map(|w| run_log_path(dir, Some(w.id)))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|b| serde_json::from_str::<RunLog>(&b).ok())
        .and_then(|rl| rl.scope);
    print_summary(ctx, &run, &aggregated_iters, dir, scope);
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

/// Embedded planner system prompt. Lives at `skills/data/planner-prompt.md`
/// so it can be tuned without recompile is not the goal — embedding via
/// `include_str!` keeps the binary self-contained while the file on disk
/// remains the canonical edit surface for prompt iteration during dev.
const PLANNER_PROMPT_BODY: &str = include_str!("../../../skills/data/planner-prompt.md");

/// Compact view of a ready task that's safe to serialize into the
/// planner prompt. We deliberately drop `description` and `parent` so
/// the prompt stays small — the planner picks parallel-safe ids by
/// title + priority, not by re-reading the whole task graph.
#[derive(serde::Serialize)]
struct PlannerReadyView<'a> {
    id: &'a str,
    title: &'a str,
    priority: u8,
    #[serde(rename = "type")]
    issue_type: &'a str,
}

/// Build the per-iter planner prompt. The system body
/// (`PLANNER_PROMPT_BODY`) goes in the cache-prefix slot; bd-ready +
/// recent-touches JSON payloads go in the tail.
fn assemble_planner_prompt(
    bd_ready: &[ReadyTask],
    recent_touches: &[String],
) -> prompt::AssembledPrompt {
    let view: Vec<PlannerReadyView<'_>> = bd_ready
        .iter()
        .map(|t| PlannerReadyView {
            id: &t.id,
            title: &t.title,
            priority: t.priority,
            issue_type: &t.issue_type,
        })
        .collect();
    let bd_ready_json = serde_json::to_string(&view).unwrap_or_else(|_| "[]".to_string());
    let touches_json = serde_json::to_string(recent_touches).unwrap_or_else(|_| "[]".to_string());
    let tail = format!("## bd_ready\n\n{bd_ready_json}\n\n## recent_touches\n\n{touches_json}\n");
    prompt::assemble(PLANNER_PROMPT_BODY, "", &tail)
}

/// Pure resolution of the iter-end batch plan when the parallel
/// dispatcher (`--jobs >= 2`) needs to seed the next iter. Splits the
/// four branches per `hew-7k1m`:
///
/// - `Some(Ok(ids))` from `extract_next_iteration(raw_text)` → Agent
/// - planner disabled OR budget = 0 → Skipped { planner_disabled }
/// - planner enabled → returned by the injected closure (which the
///   caller wires to [`spawn_planner_with`])
///
/// Splitting this away from the worker loop's side-effects keeps the
/// branch arithmetic test-friendly: no git, no bd, no spawner ctor.
pub fn resolve_iter_completion_plan<F>(
    raw_text: &str,
    planner_cfg: &LoopPlannerConfig,
    next_iter: u32,
    planner_fn: F,
) -> BatchPlan
where
    F: FnOnce(u32) -> BatchPlan,
{
    if !raw_text.is_empty()
        && let Some(ids) = hew_core::batch_plan_parse::extract_next_iteration(raw_text)
    {
        return BatchPlan {
            schema_version: BATCH_PLAN_SCHEMA_VERSION,
            iter_number: next_iter,
            task_ids: ids,
            source: BatchSource::Agent,
            reason: None,
            created_at: iso_now_utc(),
            planner_tokens: None,
        };
    }
    if !planner_cfg.enabled || planner_cfg.budget_tokens == 0 {
        return skipped_plan(next_iter, "planner_disabled");
    }
    planner_fn(next_iter)
}

/// Build a `Skipped` batch plan with the given reason. Used by every
/// non-success path in [`spawn_planner`] so the caller sees one
/// shape regardless of why the planner declined.
fn skipped_plan(iter_number: u32, reason: impl Into<String>) -> BatchPlan {
    BatchPlan {
        schema_version: BATCH_PLAN_SCHEMA_VERSION,
        iter_number,
        task_ids: Vec::new(),
        source: BatchSource::Skipped,
        reason: Some(reason.into()),
        created_at: iso_now_utc(),
        planner_tokens: None,
    }
}

/// Spawn the planner runtime to suggest a batch for `iter_number`.
///
/// Per `hew-pxw9` acceptance: this function NEVER propagates an error —
/// every failure path returns `BatchPlan { source: Skipped, ... }`. The
/// planner is an advisory signal layered on top of trust-the-graph, and
/// a broken planner must not kill the loop.
///
/// Pre-spawn budget check skips the subprocess entirely when the
/// assembled prompt's `token_estimate` exceeds `budget_tokens` — we
/// never truncate context to fit a budget per the plan's "refusing to
/// plan is strictly better than guessing badly" rule.
pub fn spawn_planner(
    bd_ready: &[ReadyTask],
    recent_touches: &[String],
    budget_tokens: u32,
    runtime: RuntimeKind,
    project_root: &Path,
    iter_number: u32,
) -> miette::Result<BatchPlan> {
    let spawner = build_spawner_for(runtime);
    Ok(spawn_planner_with(
        spawner.as_ref(),
        bd_ready,
        recent_touches,
        budget_tokens,
        project_root,
        iter_number,
    ))
}

/// Spawner-injected variant of [`spawn_planner`]. Production wires the
/// real runtime; unit tests pass a `MockSpawner` (or a custom error-
/// returning one) so the budget / parse / runtime-error branches are
/// each exercisable without touching a real subprocess.
fn spawn_planner_with(
    spawner: &dyn RuntimeSpawner,
    bd_ready: &[ReadyTask],
    recent_touches: &[String],
    budget_tokens: u32,
    project_root: &Path,
    iter_number: u32,
) -> BatchPlan {
    let prompt = assemble_planner_prompt(bd_ready, recent_touches);
    let estimate = prompt.token_estimate;
    if estimate > budget_tokens as u64 {
        return skipped_plan(
            iter_number,
            format!("budget_exceeded: estimated {estimate} tokens > budget {budget_tokens}"),
        );
    }
    let opts = SpawnOpts { model_override: None, working_dir: Some(project_root.to_path_buf()) };
    let outcome = match spawner.spawn(&prompt, &[], &opts) {
        Ok(o) => o,
        Err(e) => return skipped_plan(iter_number, format!("runtime_error: {e}")),
    };
    match extract_next_iteration(&outcome.raw_text) {
        Some(ids) => BatchPlan {
            schema_version: BATCH_PLAN_SCHEMA_VERSION,
            iter_number,
            task_ids: ids,
            source: BatchSource::Planner,
            reason: None,
            created_at: iso_now_utc(),
            planner_tokens: Some(outcome.tokens),
        },
        None => BatchPlan {
            schema_version: BATCH_PLAN_SCHEMA_VERSION,
            iter_number,
            task_ids: Vec::new(),
            source: BatchSource::Skipped,
            reason: Some("parse_error: planner response missing next_iteration block".into()),
            created_at: iso_now_utc(),
            planner_tokens: Some(outcome.tokens),
        },
    }
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

    fn default_args() -> Args {
        Args {
            max_iter: None,
            until_empty: true,
            budget_tokens: None,
            budget_wall: None,
            strict: true,
            interactive: false,
            unattended: false,
            runtime: "claude".into(),
            stop_file: None,
            dry_run: true,
            skill: "hew-execute".into(),
            fallback_runtime: None,
            fallback_cooldown_iters: None,
            jobs: 1,
            scope: None,
            epics: Vec::new(),
            epic: Vec::new(),
            no_planner: false,
            planner_budget: None,
            planner_runtime: None,
            verify_tests: false,
            no_verify_tests: false,
            verify_command: None,
        }
    }

    fn non_interactive_ctx() -> Ctx {
        Ctx::new(true, hew_core::ctx::OutputMode::Text, true, 0)
    }

    /// Stub bd that returns a single open epic for any `bd show <id>` /
    /// `bd list` call, and otherwise errors. Enough to validate
    /// resolve_scope's argv branches without touching disk.
    #[derive(Debug, Default)]
    struct FakeBd {
        epic_id: String,
        epic_status: String,
    }

    impl FakeBd {
        fn open_epic(id: &str) -> Self {
            Self { epic_id: id.into(), epic_status: "open".into() }
        }
    }

    impl hew_core::bd::BdClient for FakeBd {
        fn version(&self) -> hew_core::error::Result<hew_core::bd::BdVersion> {
            Ok(hew_core::bd::BdVersion { raw: "x".into(), semver: "0.0.0".into() })
        }
        fn ready(&self) -> hew_core::error::Result<Vec<hew_core::bd::ReadyTask>> {
            Ok(Vec::new())
        }
        fn stats(&self) -> hew_core::error::Result<hew_core::bd::StatsSummary> {
            Ok(Default::default())
        }
        fn prime_raw(&self) -> hew_core::error::Result<String> {
            Ok(String::new())
        }
        fn memories(&self) -> hew_core::error::Result<std::collections::BTreeMap<String, String>> {
            Ok(std::collections::BTreeMap::new())
        }
        fn remember(&self, _: &str) -> hew_core::error::Result<()> {
            Ok(())
        }
        fn run_raw(
            &self,
            args: &[&std::ffi::OsStr],
        ) -> hew_core::error::Result<hew_core::bd::BdOutput> {
            let argv: Vec<String> = args.iter().map(|a| a.to_string_lossy().to_string()).collect();
            let first = argv.first().map(String::as_str).unwrap_or("");
            if first == "show" {
                let id = argv.get(1).cloned().unwrap_or_default();
                if id == self.epic_id {
                    let body = format!(
                        r#"[{{"id":"{}","title":"t","description":"","status":"{}","priority":2,"issue_type":"epic","closed_at":"","close_reason":null,"parent":null}}]"#,
                        self.epic_id, self.epic_status,
                    );
                    return Ok(hew_core::bd::BdOutput { stdout: body, stderr: String::new() });
                }
                return Err(hew_core::error::HewError::BdNonZero {
                    code: 1,
                    stderr: format!("not found: {id}"),
                });
            }
            Err(hew_core::error::HewError::BdNonZero {
                code: 1,
                stderr: format!("unexpected: {argv:?}"),
            })
        }
    }

    #[test]
    fn resolve_scope_ready_argv_is_ready() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Ready);
        let bd = FakeBd::default();
        assert_eq!(
            resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap(),
            ResolvedScope::Scope(Scope::Ready),
        );
    }

    #[test]
    fn resolve_scope_ready_rejects_epics_argv() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Ready);
        args.epics = vec!["hew-6az".into()];
        let bd = FakeBd::default();
        let err = resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap_err();
        assert!(format!("{err:?}").contains("--scope=ready does not accept --epics"));
    }

    #[test]
    fn resolve_scope_epics_argv_returns_epic_list() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Epics);
        args.epics = vec!["hew-6az".into()];
        let bd = FakeBd::open_epic("hew-6az");
        assert_eq!(
            resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap(),
            ResolvedScope::Scope(Scope::Epics { epic_ids: vec!["hew-6az".into()] }),
        );
    }

    #[test]
    fn resolve_scope_epics_singular_alias_merges_into_list() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Epics);
        args.epic = vec!["hew-6az".into()];
        let bd = FakeBd::open_epic("hew-6az");
        assert_eq!(
            resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap(),
            ResolvedScope::Scope(Scope::Epics { epic_ids: vec!["hew-6az".into()] }),
        );
    }

    #[test]
    fn resolve_scope_missing_in_non_interactive_errors() {
        let args = default_args();
        let bd = FakeBd::default();
        let err = resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("scope"), "expected MissingFlag scope, got: {msg}");
    }

    #[test]
    fn resolve_scope_epics_kind_without_epics_in_non_interactive_errors() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Epics);
        let bd = FakeBd::default();
        let err = resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap_err();
        let msg = format!("{err:?}");
        assert!(msg.contains("epics"), "expected MissingFlag epics, got: {msg}");
    }

    #[test]
    fn resolve_scope_epics_argv_rejects_closed_epic() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Epics);
        args.epics = vec!["hew-6az".into()];
        let mut bd = FakeBd::open_epic("hew-6az");
        bd.epic_status = "closed".into();
        let err = resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap_err();
        assert!(format!("{err:?}").contains("closed"));
    }

    #[test]
    fn resolve_scope_epics_argv_rejects_unknown_id() {
        let mut args = default_args();
        args.scope = Some(ScopeArg::Epics);
        args.epics = vec!["hew-bogus".into()];
        let bd = FakeBd::open_epic("hew-6az");
        let err = resolve_scope(&args, &non_interactive_ctx(), &bd).unwrap_err();
        assert!(format!("{err:?}").contains("not found"));
    }

    // ---- spawn_planner (hew-pxw9) -----------------------------------

    use hew_core::runner::TokenSpend;
    use hew_core::runtime::{MockSpawner, SpawnFailureClass, SpawnOutcome};

    fn ready(id: &str, prio: u8) -> ReadyTask {
        ReadyTask {
            id: id.into(),
            title: format!("title for {id}"),
            description: String::new(),
            priority: prio,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }
    }

    fn planner_outcome_with(raw_text: impl Into<String>) -> SpawnOutcome {
        SpawnOutcome {
            success: true,
            closed_task: None,
            tokens: TokenSpend { input: 1234, output: 56, cache_read: 0, cache_create: 0 },
            stderr_tail: String::new(),
            raw_text: raw_text.into(),
            failure_class: SpawnFailureClass::Success,
        }
    }

    #[test]
    fn planner_skips_when_estimated_tokens_exceed_budget() {
        let bd_ready = vec![ready("hew-aaa", 1), ready("hew-bbb", 2)];
        let touches = vec!["src/foo.rs:bar".to_string()];
        let mock = MockSpawner::new(planner_outcome_with(""));
        let plan =
            spawn_planner_with(&mock, &bd_ready, &touches, /*budget*/ 1, Path::new("/"), 7);
        assert_eq!(plan.source, BatchSource::Skipped);
        assert!(plan.task_ids.is_empty());
        let reason = plan.reason.as_deref().unwrap_or("");
        assert!(
            reason.starts_with("budget_exceeded:") && reason.contains("budget 1"),
            "reason should name the cause + budget: {reason}",
        );
        // No subprocess spawned.
        assert!(
            mock.last_args.borrow().is_none(),
            "spawner must not be invoked when budget already exceeded",
        );
        assert!(plan.planner_tokens.is_none(), "no spawn → no tokens accounted");
        assert_eq!(plan.iter_number, 7);
        assert_eq!(plan.schema_version, BATCH_PLAN_SCHEMA_VERSION);
    }

    #[test]
    fn planner_returns_plan_on_clean_response() {
        let bd_ready = vec![ready("hew-aaa", 1), ready("hew-bbb", 2)];
        let mock = MockSpawner::new(planner_outcome_with(
            "thinking...\n\n```next_iteration\n[\"hew-aaa\", \"hew-bbb\"]\n```\n",
        ));
        let plan =
            spawn_planner_with(&mock, &bd_ready, &[], /*budget*/ 100_000, Path::new("/"), 3);
        assert_eq!(plan.source, BatchSource::Planner);
        assert_eq!(plan.task_ids, vec!["hew-aaa".to_string(), "hew-bbb".to_string()]);
        assert_eq!(plan.iter_number, 3);
        assert!(plan.reason.is_none());
    }

    #[test]
    fn planner_skips_on_parse_error() {
        let mock = MockSpawner::new(planner_outcome_with("no fenced block here, just prose"));
        let plan =
            spawn_planner_with(&mock, &[ready("hew-aaa", 1)], &[], 100_000, Path::new("/"), 1);
        assert_eq!(plan.source, BatchSource::Skipped);
        assert!(plan.task_ids.is_empty());
        let reason = plan.reason.as_deref().unwrap_or("");
        assert!(reason.starts_with("parse_error:"), "got {reason:?}");
    }

    #[test]
    fn planner_skips_on_runtime_error() {
        #[derive(Debug)]
        struct ErrSpawner;
        impl RuntimeSpawner for ErrSpawner {
            fn spawn(
                &self,
                _: &prompt::AssembledPrompt,
                _: &[String],
                _: &SpawnOpts,
            ) -> hew_core::error::Result<SpawnOutcome> {
                Err(std::io::Error::other("simulated spawn failure").into())
            }
        }
        let plan = spawn_planner_with(
            &ErrSpawner,
            &[ready("hew-aaa", 1)],
            &[],
            100_000,
            Path::new("/"),
            4,
        );
        assert_eq!(plan.source, BatchSource::Skipped);
        let reason = plan.reason.as_deref().unwrap_or("");
        assert!(reason.starts_with("runtime_error:"), "got {reason:?}");
        assert!(reason.contains("simulated spawn failure"), "must surface the cause: {reason}");
    }

    #[test]
    fn planner_prompt_includes_bd_ready_and_recent_touches() {
        let bd_ready = vec![ready("hew-aaa", 1), ready("hew-bbb", 3)];
        let touches = vec!["src/dispatcher.rs:run".into(), "src/loop_log.rs:write".into()];
        let prompt = assemble_planner_prompt(&bd_ready, &touches);
        // System body lands in the cache prefix.
        assert!(prompt.prefix.contains("Hew loop"), "prefix must carry the system body");
        // Task tail carries both inputs.
        assert!(prompt.tail.contains("hew-aaa"), "bd_ready ids missing from tail");
        assert!(prompt.tail.contains("hew-bbb"));
        assert!(prompt.tail.contains("\"priority\":1"), "priority emitted");
        assert!(prompt.tail.contains("src/dispatcher.rs:run"), "touches missing from tail");
        assert!(prompt.tail.contains("src/loop_log.rs:write"));
        // The full prompt is what the spawner actually receives — it
        // must contain both halves.
        assert!(prompt.full_text.contains("Hew loop"));
        assert!(prompt.full_text.contains("hew-aaa"));
    }

    // ---- iter-end batch hook (hew-7k1m) -----------------------------

    fn agent_plan_via_fenced_block() -> &'static str {
        "thinking...\n\n```next_iteration\n[\"hew-foo\", \"hew-bar\"]\n```\nDone.\n"
    }

    fn never_planner(_ni: u32) -> BatchPlan {
        panic!("planner closure must not run for this branch");
    }

    #[test]
    fn iter_completion_writes_agent_sourced_batch_when_block_present() {
        let cfg = LoopPlannerConfig::default();
        let plan =
            resolve_iter_completion_plan(agent_plan_via_fenced_block(), &cfg, 7, never_planner);
        assert_eq!(plan.source, BatchSource::Agent);
        assert_eq!(plan.iter_number, 7);
        assert_eq!(plan.task_ids, vec!["hew-foo".to_string(), "hew-bar".to_string()]);
        assert!(plan.reason.is_none());
        assert!(plan.planner_tokens.is_none());
    }

    #[test]
    fn iter_completion_writes_planner_sourced_batch_when_agent_silent() {
        let cfg = LoopPlannerConfig::default();
        let plan = resolve_iter_completion_plan("no fenced block, just prose", &cfg, 4, |ni| {
            // Stand-in for spawn_planner_with: returns a Planner-sourced plan.
            BatchPlan {
                schema_version: BATCH_PLAN_SCHEMA_VERSION,
                iter_number: ni,
                task_ids: vec!["hew-planned".into()],
                source: BatchSource::Planner,
                reason: None,
                created_at: iso_now_utc(),
                planner_tokens: Some(TokenSpend {
                    input: 1,
                    output: 1,
                    cache_read: 0,
                    cache_create: 0,
                }),
            }
        });
        assert_eq!(plan.source, BatchSource::Planner);
        assert_eq!(plan.iter_number, 4);
        assert_eq!(plan.task_ids, vec!["hew-planned".to_string()]);
        assert!(plan.planner_tokens.is_some());
    }

    #[test]
    fn iter_completion_writes_skipped_batch_when_planner_disabled() {
        let cfg = LoopPlannerConfig { enabled: false, ..Default::default() };
        let plan = resolve_iter_completion_plan("", &cfg, 3, never_planner);
        assert_eq!(plan.source, BatchSource::Skipped);
        assert_eq!(plan.iter_number, 3);
        assert!(plan.task_ids.is_empty());
        assert_eq!(plan.reason.as_deref(), Some("planner_disabled"));
    }

    #[test]
    fn iter_completion_skipped_when_budget_zero() {
        // `--planner-budget 0` is the documented "off without flipping
        // --no-planner" knob; it must short-circuit before the closure
        // runs.
        let cfg = LoopPlannerConfig { budget_tokens: 0, ..Default::default() };
        let plan = resolve_iter_completion_plan("", &cfg, 2, never_planner);
        assert_eq!(plan.source, BatchSource::Skipped);
        assert_eq!(plan.reason.as_deref(), Some("planner_disabled"));
    }

    #[test]
    fn iter_completion_agent_wins_over_planner() {
        // Agent block in raw_text → planner closure must not fire even
        // when planner is enabled.
        let cfg = LoopPlannerConfig::default();
        let plan =
            resolve_iter_completion_plan(agent_plan_via_fenced_block(), &cfg, 9, never_planner);
        assert_eq!(plan.source, BatchSource::Agent);
    }

    #[test]
    fn cli_no_planner_skips_planner_call_even_when_agent_silent() {
        let mut args = default_args();
        args.no_planner = true;
        let resolved = resolve_planner_config(&args, &LoopPlannerConfig::default()).unwrap();
        assert!(!resolved.enabled);
        let plan = resolve_iter_completion_plan("", &resolved, 1, never_planner);
        assert_eq!(plan.source, BatchSource::Skipped);
        assert_eq!(plan.reason.as_deref(), Some("planner_disabled"));
    }

    #[test]
    fn cli_planner_budget_overrides_config() {
        let mut args = default_args();
        args.planner_budget = Some(42);
        let base = LoopPlannerConfig { budget_tokens: 999, ..Default::default() };
        let resolved = resolve_planner_config(&args, &base).unwrap();
        assert_eq!(resolved.budget_tokens, 42);
    }

    #[test]
    fn cli_planner_runtime_overrides_config() {
        let mut args = default_args();
        args.planner_runtime = Some("codex".into());
        let resolved = resolve_planner_config(&args, &LoopPlannerConfig::default()).unwrap();
        assert_eq!(resolved.runtime.as_deref(), Some("codex"));
    }

    #[test]
    fn cli_planner_runtime_rejects_unknown_kind() {
        let mut args = default_args();
        args.planner_runtime = Some("cursor".into());
        assert!(resolve_planner_config(&args, &LoopPlannerConfig::default()).is_err());
    }

    #[test]
    fn planner_tokens_field_populated_on_success() {
        let mock = MockSpawner::new(planner_outcome_with("```next_iteration\n[\"hew-aaa\"]\n```"));
        let plan =
            spawn_planner_with(&mock, &[ready("hew-aaa", 1)], &[], 100_000, Path::new("/"), 2);
        let tokens = plan.planner_tokens.expect("planner_tokens populated on success");
        assert_eq!(tokens.input, 1234);
        assert_eq!(tokens.output, 56);
    }
}
