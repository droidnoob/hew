//! Per-run log directory + per-iter JSON serializer for `hew loop`.
//!
//! Layout (rooted at `<cwd>/.hew/loop/<run-id>/`):
//!
//! ```text
//! run.json          — config + summary (rewritten after every iter; N=1 fast path)
//! iter-001.json     — per-iter log (atomic write: temp + rename; N=1 fast path)
//! iter-002.json
//! ...
//! manifest.json     — top-level worker manifest (parallel runs)
//! worker-0/         — per-worker subdir (N>=2; absent in N=1 fast path)
//!   run.json
//!   iter-001.json
//! worker-1/
//!   ...
//! ask-1.md          — interactive-mode ask-file (optional)
//! .stop             — sentinel for `hew loop cancel` (optional)
//! ```
//!
//! In the `--jobs=1` fast path (`worker_n = None`) iter + run logs land
//! directly under the run dir, byte-identical to the pre-parallel
//! layout. In parallel runs (`worker_n = Some(n)`) each worker's logs
//! live under its own `worker-<n>/` subdir, and `manifest.json` at the
//! run-dir root aggregates worker-final outcomes.
//!
//! Atomic writes use the temp-file + rename pattern so a kill -9 mid-
//! write leaves either the old contents or the complete new contents
//! — never a partial file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::runner::{Iter, IterOutcome, Run, StopReason, TokenSpend};
use crate::time::iso_now_utc;

/// Default root for loop artifacts, relative to the project root.
pub const LOOP_ROOT: &str = ".hew/loop";

/// Build a fresh run-id of the form `loop-YYYYMMDDTHHMMSSZ-<hex>`.
/// The hex suffix is the low 32 bits of nanos-since-epoch, formatted
/// as 8 lowercase hex digits — enough collision resistance for the
/// "two runs started in the same second" case without pulling in rand.
pub fn new_run_id() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let nanos = now.subsec_nanos();
    let pid = std::process::id();
    let suffix = (nanos ^ pid).to_be_bytes();
    let hex = suffix.iter().map(|b| format!("{b:02x}")).collect::<String>();
    // Compact ISO: drop punctuation so it lives cleanly in a filename.
    let iso = iso_now_utc();
    let compact: String = iso.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    format!("loop-{compact}-{hex}")
}

/// Resolve `<root>/.hew/loop/<run-id>/`, creating it if needed.
pub fn run_dir(project_root: &Path, run_id: &str) -> Result<PathBuf> {
    let dir = project_root.join(LOOP_ROOT).join(run_id);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Wire-format for the per-iter log file. Mirrors [`Iter`] but with
/// serde-friendly types and a few extras the runtime collects
/// (prefix_hash, tool_calls summary).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IterLog {
    pub number: u32,
    pub task_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub outcome: Option<String>,
    pub prompt_prefix_hash: Option<String>,
    pub cost: TokenSpend,
    pub decisions: Vec<String>,
    pub deferred: Vec<String>,
    pub tool_calls: Vec<String>,
    pub stderr_tail: Option<String>,
    /// Symbols the iter actually touched, as `<file>:<symbol-name>`
    /// strings. Populated from `hew_core::blast::compute_blast_with`
    /// against the pre-iter sha when the `treesitter` feature is
    /// enabled and the iter produced commits; empty otherwise.
    #[serde(default)]
    pub symbols_touched: Vec<String>,
    /// Which runtime drove this iter, as the [`crate::runtime::RuntimeKind`]
    /// string (`"claude"` / `"codex"`). `None` for dry-run iters where
    /// no spawner ran. Populated by `hew loop` so multi-runtime runs
    /// stay debuggable in the per-iter log.
    #[serde(default)]
    pub runtime_used: Option<String>,
    /// True iff the cooldown state machine had primary on hold when
    /// this iter completed (i.e. the loop was routing iters to the
    /// fallback runtime, or about to retry the primary after a window).
    /// Always `false` when no fallback is configured.
    #[serde(default)]
    pub cooldown_engaged: bool,
    /// Model the spawner was invoked with for this iter, as resolved by
    /// `hew_core::loop_model::resolve_model` (description tag > label >
    /// config). `None` ⇒ runtime default was used (display as `(default)`
    /// in `hew loop summary`). Absent in pre-epic-D iter logs.
    #[serde(default)]
    pub model: Option<String>,
}

impl IterLog {
    /// Project an in-memory [`Iter`] into the wire-format used on disk.
    /// `prompt_prefix_hash` and `tool_calls` come from the spawner layer.
    pub fn from_iter(
        it: &Iter,
        prompt_prefix_hash: Option<String>,
        tool_calls: Vec<String>,
        symbols_touched: Vec<String>,
    ) -> Self {
        Self {
            number: it.number,
            task_id: it.task_id.clone(),
            started_at: it.started_at.clone(),
            ended_at: it.ended_at.clone(),
            outcome: it.outcome.map(outcome_label).map(str::to_string),
            prompt_prefix_hash,
            cost: it.cost,
            decisions: it.decisions.clone(),
            deferred: it.deferred.clone(),
            tool_calls,
            stderr_tail: it.stderr_tail.clone(),
            symbols_touched,
            runtime_used: None,
            cooldown_engaged: false,
            model: None,
        }
    }
}

fn outcome_label(o: IterOutcome) -> &'static str {
    match o {
        IterOutcome::Closed => "closed",
        IterOutcome::NoClose => "no_close",
        IterOutcome::BackpressureFail => "backpressure_fail",
        IterOutcome::RuntimeError => "runtime_error",
    }
}

/// Wire-format for the per-run summary file (`run.json`), rewritten
/// after each iter completes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunLog {
    pub id: String,
    pub started_at: String,
    pub last_updated_at: String,
    pub iter_count: u32,
    pub cumulative_tokens: u64,
    pub stop_reason: Option<String>,
    /// `--max-iter` cap as configured. `None` = unlimited.
    pub max_iter: Option<u32>,
    pub strict: bool,
    pub interactive: bool,
}

impl RunLog {
    pub fn from_run(run: &Run) -> Self {
        Self {
            id: run.id.clone(),
            started_at: run.started_at.clone(),
            last_updated_at: iso_now_utc(),
            iter_count: run.iters.len() as u32,
            cumulative_tokens: run.cumulative_tokens(),
            stop_reason: run.stop_reason.map(stop_reason_label).map(str::to_string),
            max_iter: run.config.max_iter,
            strict: run.config.strict,
            interactive: run.config.interactive,
        }
    }
}

fn stop_reason_label(r: StopReason) -> &'static str {
    match r {
        StopReason::Cancelled => "cancelled",
        StopReason::StopFile => "stop_file",
        StopReason::BudgetTokens => "budget_tokens",
        StopReason::BudgetWall => "budget_wall",
        StopReason::MaxIter => "max_iter",
        StopReason::ReadyEmpty => "ready_empty",
        StopReason::GuardTrip => "guard_trip",
        StopReason::RuntimeError => "runtime_error",
    }
}

/// Atomically write JSON to `path` (temp-file + rename).
pub fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| std::io::Error::other(format!("path has no parent: {}", path.display())))?;
    let tmp = parent
        .join(format!(".{}.tmp", path.file_name().and_then(|s| s.to_str()).unwrap_or("write")));
    let body = serde_json::to_vec_pretty(value)?;
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(&body)?;
        f.sync_all().ok();
    }
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Pure path composer for a worker's log subdir under the run dir.
/// `<run-dir>/worker-<n>/`. Does NOT touch the filesystem.
pub fn worker_dir(run_dir: &Path, worker_n: u32) -> PathBuf {
    run_dir.join(format!("worker-{worker_n}"))
}

/// Create (`mkdir -p`) the worker log subdir and return its path. Use
/// this at dispatcher setup time when constructing per-worker
/// [`Worker`](crate::dispatcher) state.
pub fn ensure_worker_dir(run_dir: &Path, worker_n: u32) -> Result<PathBuf> {
    let dir = worker_dir(run_dir, worker_n);
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Resolve `<run-dir>[/worker-<n>]/iter-NNN.json` with zero-padded
/// 3-digit iter number.
///
/// `worker_n = None` keeps the pre-parallel layout (`--jobs=1` fast
/// path); `worker_n = Some(n)` slots logs under the worker subdir.
pub fn iter_log_path(run_dir: &Path, worker_n: Option<u32>, iter_number: u32) -> PathBuf {
    let dir = match worker_n {
        Some(n) => worker_dir(run_dir, n),
        None => run_dir.to_path_buf(),
    };
    dir.join(format!("iter-{iter_number:03}.json"))
}

/// Resolve `<run-dir>[/worker-<n>]/run.json`. Mirror of [`iter_log_path`].
pub fn run_log_path(run_dir: &Path, worker_n: Option<u32>) -> PathBuf {
    let dir = match worker_n {
        Some(n) => worker_dir(run_dir, n),
        None => run_dir.to_path_buf(),
    };
    dir.join("run.json")
}

pub fn stop_file_path(run_dir: &Path) -> PathBuf {
    run_dir.join(".stop")
}

/// Top-level cross-worker manifest written at dispatcher shutdown.
///
/// One entry per worker that participated in the run; carries enough
/// info for `hew loop summary` / `hew loop logs` to fold N
/// `worker-<n>/` slices into a single report without re-walking the
/// per-worker `run.json` files.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub run_id: String,
    pub jobs: u32,
    pub started_at: String,
    pub completed_at: String,
    pub workers: Vec<ManifestWorker>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestWorker {
    pub id: u32,
    /// Branch the worker committed to. Empty for the single-slot fast
    /// path where the loop runs on the project's checked-out branch.
    #[serde(default)]
    pub branch: String,
    /// Worker subdir name (e.g. `"worker-0"`), or `None` when the
    /// worker wrote logs directly under the run-dir root (N=1 fast
    /// path).
    #[serde(default)]
    pub log_subdir: Option<String>,
    pub iter_count: u32,
    pub cumulative_tokens: u64,
    #[serde(default)]
    pub stop_reason: Option<String>,
}

pub fn manifest_path(run_dir: &Path) -> PathBuf {
    run_dir.join("manifest.json")
}

/// Atomically write the worker manifest at `<run-dir>/manifest.json`.
pub fn write_manifest(run_dir: &Path, manifest: &Manifest) -> Result<()> {
    write_json_atomic(&manifest_path(run_dir), manifest)
}

/// Run-ids under `loop_root` (`<project>/.hew/loop/`) whose `run.json`
/// reports no `stop_reason` yet — i.e. the run never reached a clean
/// shutdown.
///
/// Used by `hew loop prune-worktrees` to decide which worktrees under
/// `~/.hew/wt/<run-id>/` may still be owned by a live process. A run
/// without `run.json` is treated as active too (just starting up or
/// crashed before the first iter wrote run.json — we err on the side of
/// not deleting); a run with `stop_reason` set is considered finished
/// and its worktrees are eligible for pruning.
///
/// Returns an empty set — not an error — when `loop_root` is absent.
pub fn active_run_ids(loop_root: &Path) -> Result<std::collections::HashSet<String>> {
    let mut out = std::collections::HashSet::new();
    let entries = match fs::read_dir(loop_root) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e.into()),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else { continue };
        if !name.starts_with("loop-") {
            continue;
        }
        let rl_path = path.join("run.json");
        match fs::read_to_string(&rl_path) {
            Ok(body) => match serde_json::from_str::<RunLog>(&body) {
                Ok(rl) if rl.stop_reason.is_none() => {
                    out.insert(name.to_string());
                }
                Ok(_) => {}
                // Unparseable run.json: treat conservatively as active
                // so we don't delete worktrees the operator may still
                // need to triage by hand.
                Err(_) => {
                    out.insert(name.to_string());
                }
            },
            // No run.json yet: a run that crashed before the first iter,
            // or one mid-init. Treat as active — operator decides via
            // `--force` (future flag) whether to clean up.
            Err(_) => {
                out.insert(name.to_string());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Iter, RunConfig, TokenSpend};

    #[test]
    fn stop_reason_label_round_trips_through_from_label() {
        for r in [
            StopReason::Cancelled,
            StopReason::StopFile,
            StopReason::BudgetTokens,
            StopReason::BudgetWall,
            StopReason::MaxIter,
            StopReason::ReadyEmpty,
            StopReason::GuardTrip,
            StopReason::RuntimeError,
        ] {
            assert_eq!(StopReason::from_label(stop_reason_label(r)), Some(r), "drift on {r:?}");
        }
        assert_eq!(StopReason::from_label("bogus"), None);
    }

    fn tmpdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "hew-loop-log-{}-{}",
            std::process::id(),
            new_run_id()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn new_run_id_has_expected_shape() {
        let id = new_run_id();
        assert!(id.starts_with("loop-"));
        // loop-<compact-iso>-<8-hex>
        let parts: Vec<&str> = id.rsplitn(2, '-').collect();
        assert_eq!(parts[0].len(), 8, "hex suffix should be 8 chars: {id}");
        assert!(parts[0].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn new_run_id_avoids_immediate_collision() {
        let a = new_run_id();
        let b = new_run_id();
        // Same second is fine; the hex suffix should differ from xor with pid+nanos.
        // We can't strictly guarantee but check there's at least one different char.
        assert_ne!(a, b, "successive run-ids collided: {a} vs {b}");
    }

    #[test]
    fn run_dir_is_created() {
        let root = tmpdir();
        let dir = run_dir(&root, "loop-test").unwrap();
        assert!(dir.exists());
        assert!(dir.ends_with(".hew/loop/loop-test"));
    }

    #[test]
    fn iter_log_path_is_zero_padded() {
        let p = iter_log_path(Path::new("/tmp/r"), None, 7);
        assert!(p.ends_with("iter-007.json"));
        let p = iter_log_path(Path::new("/tmp/r"), None, 142);
        assert!(p.ends_with("iter-142.json"));
    }

    #[test]
    fn iter_log_path_omits_worker_dir_when_none() {
        // Backward-compat with the --jobs=1 fast path: iter logs land
        // directly under the run dir, byte-identical to the layout the
        // pre-parallel loop wrote.
        let p = iter_log_path(Path::new("/tmp/r"), None, 3);
        assert_eq!(p, Path::new("/tmp/r/iter-003.json"));
        let p = run_log_path(Path::new("/tmp/r"), None);
        assert_eq!(p, Path::new("/tmp/r/run.json"));
    }

    #[test]
    fn iter_log_path_includes_worker_dir_when_set() {
        // Parallel layout: iter logs live under worker-<n>/.
        let p = iter_log_path(Path::new("/tmp/r"), Some(0), 3);
        assert_eq!(p, Path::new("/tmp/r/worker-0/iter-003.json"));
        let p = iter_log_path(Path::new("/tmp/r"), Some(7), 42);
        assert_eq!(p, Path::new("/tmp/r/worker-7/iter-042.json"));
        let p = run_log_path(Path::new("/tmp/r"), Some(2));
        assert_eq!(p, Path::new("/tmp/r/worker-2/run.json"));
    }

    #[test]
    fn worker_dir_composes_path() {
        assert_eq!(worker_dir(Path::new("/tmp/r"), 0), Path::new("/tmp/r/worker-0"));
        assert_eq!(worker_dir(Path::new("/tmp/r"), 12), Path::new("/tmp/r/worker-12"));
    }

    #[test]
    fn ensure_worker_dir_creates_subdir() {
        let root = tmpdir();
        let run = run_dir(&root, "loop-test-wd").unwrap();
        let w = ensure_worker_dir(&run, 3).unwrap();
        assert!(w.exists());
        assert!(w.ends_with("worker-3"));
        // Idempotent — second call doesn't error.
        let w2 = ensure_worker_dir(&run, 3).unwrap();
        assert_eq!(w, w2);
    }

    #[test]
    fn manifest_lists_all_workers_after_shutdown() {
        let root = tmpdir();
        let run = run_dir(&root, "loop-test-mf").unwrap();
        let manifest = Manifest {
            run_id: "loop-test-mf".into(),
            jobs: 2,
            started_at: "2026-05-29T00:00:00Z".into(),
            completed_at: "2026-05-29T00:01:00Z".into(),
            workers: vec![
                ManifestWorker {
                    id: 0,
                    branch: "loop/run-mf/w0".into(),
                    log_subdir: Some("worker-0".into()),
                    iter_count: 3,
                    cumulative_tokens: 1500,
                    stop_reason: Some("ready_empty".into()),
                },
                ManifestWorker {
                    id: 1,
                    branch: "loop/run-mf/w1".into(),
                    log_subdir: Some("worker-1".into()),
                    iter_count: 2,
                    cumulative_tokens: 900,
                    stop_reason: Some("ready_empty".into()),
                },
            ],
        };
        write_manifest(&run, &manifest).unwrap();
        let path = manifest_path(&run);
        assert!(path.exists());
        assert!(path.ends_with("manifest.json"));
        let body = std::fs::read(&path).unwrap();
        let parsed: Manifest = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.run_id, "loop-test-mf");
        assert_eq!(parsed.jobs, 2);
        assert_eq!(parsed.workers.len(), 2);
        assert_eq!(parsed.workers[0].id, 0);
        assert_eq!(parsed.workers[0].log_subdir.as_deref(), Some("worker-0"));
        assert_eq!(parsed.workers[0].iter_count, 3);
        assert_eq!(parsed.workers[1].id, 1);
        assert_eq!(parsed.workers[1].cumulative_tokens, 900);
    }

    #[test]
    fn write_json_atomic_roundtrips() {
        let root = tmpdir();
        let path = root.join("payload.json");
        let value = TokenSpend { input: 1, output: 2, cache_read: 3, cache_create: 4 };
        write_json_atomic(&path, &value).unwrap();
        let parsed: TokenSpend = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed, value);
    }

    #[test]
    fn write_json_atomic_overwrites_existing() {
        let root = tmpdir();
        let path = root.join("payload.json");
        let a = TokenSpend { input: 10, ..Default::default() };
        let b = TokenSpend { input: 99, ..Default::default() };
        write_json_atomic(&path, &a).unwrap();
        write_json_atomic(&path, &b).unwrap();
        let parsed: TokenSpend = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(parsed, b);
    }

    #[test]
    fn iter_log_from_iter_carries_fields() {
        let mut it = Iter::new(3, "2026-05-26T00:00:00Z");
        it.task_id = Some("hew-abc".into());
        it.outcome = Some(IterOutcome::Closed);
        it.cost = TokenSpend { input: 5, output: 6, cache_read: 7, cache_create: 8 };
        it.decisions.push("mem-d1".into());
        it.deferred.push("mem-q1".into());
        let log = IterLog::from_iter(&it, Some("abc123".into()), vec!["Read".into()], Vec::new());
        assert_eq!(log.number, 3);
        assert_eq!(log.task_id.as_deref(), Some("hew-abc"));
        assert_eq!(log.outcome.as_deref(), Some("closed"));
        assert_eq!(log.prompt_prefix_hash.as_deref(), Some("abc123"));
        assert_eq!(log.tool_calls, vec!["Read".to_string()]);
        assert_eq!(log.decisions, vec!["mem-d1".to_string()]);
        assert_eq!(log.deferred, vec!["mem-q1".to_string()]);
    }

    #[test]
    fn run_log_from_run_summarizes_state() {
        let cfg = RunConfig { max_iter: Some(5), strict: false, ..RunConfig::default() };
        let mut run = Run::new("loop-x", "2026-05-26T00:00:00Z", cfg);
        let mut i = Iter::new(1, "t");
        i.cost = TokenSpend { input: 100, output: 50, cache_read: 0, cache_create: 0 };
        run.iters.push(i);
        run.stop_reason = Some(StopReason::MaxIter);
        let log = RunLog::from_run(&run);
        assert_eq!(log.id, "loop-x");
        assert_eq!(log.iter_count, 1);
        assert_eq!(log.cumulative_tokens, 150);
        assert_eq!(log.stop_reason.as_deref(), Some("max_iter"));
        assert_eq!(log.max_iter, Some(5));
        assert!(!log.strict);
    }

    #[test]
    fn outcome_label_covers_all_variants() {
        for o in [
            IterOutcome::Closed,
            IterOutcome::NoClose,
            IterOutcome::BackpressureFail,
            IterOutcome::RuntimeError,
        ] {
            let l = outcome_label(o);
            assert!(!l.is_empty());
        }
    }

    #[test]
    fn active_run_ids_includes_running_runs() {
        let root = tmpdir();
        let loop_root = root.join(LOOP_ROOT);
        let running_dir = run_dir(&root, "loop-running").unwrap();
        // run.json with no stop_reason → run is live.
        let cfg = RunConfig::default();
        let run = Run::new("loop-running", "2026-05-30T00:00:00Z", cfg);
        write_json_atomic(&running_dir.join("run.json"), &RunLog::from_run(&run)).unwrap();

        let active = active_run_ids(&loop_root).unwrap();
        assert!(active.contains("loop-running"), "running run must be flagged active");
    }

    #[test]
    fn active_run_ids_excludes_completed_runs() {
        let root = tmpdir();
        let loop_root = root.join(LOOP_ROOT);
        let completed_dir = run_dir(&root, "loop-completed").unwrap();
        let mut run = Run::new("loop-completed", "2026-05-30T00:00:00Z", RunConfig::default());
        run.stop_reason = Some(StopReason::ReadyEmpty);
        write_json_atomic(&completed_dir.join("run.json"), &RunLog::from_run(&run)).unwrap();

        let active = active_run_ids(&loop_root).unwrap();
        assert!(!active.contains("loop-completed"), "completed run must not be active");
    }

    #[test]
    fn active_run_ids_treats_missing_run_json_as_active() {
        // A run that crashed before the first iter wrote run.json: leave
        // it in the active set so its worktree isn't auto-pruned.
        let root = tmpdir();
        let loop_root = root.join(LOOP_ROOT);
        let _ = run_dir(&root, "loop-crashed").unwrap();

        let active = active_run_ids(&loop_root).unwrap();
        assert!(active.contains("loop-crashed"));
    }

    #[test]
    fn active_run_ids_empty_when_loop_root_absent() {
        let root = tmpdir();
        let missing = root.join(LOOP_ROOT); // never created
        assert!(active_run_ids(&missing).unwrap().is_empty());
    }

    #[test]
    fn iter_log_round_trips_model_field() {
        let mut it = Iter::new(1, "2026-05-26T00:00:00Z");
        it.outcome = Some(IterOutcome::Closed);
        let mut log = IterLog::from_iter(&it, None, Vec::new(), Vec::new());
        log.model = Some("opus-4.7".into());
        let json = serde_json::to_string(&log).unwrap();
        let parsed: IterLog = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.model.as_deref(), Some("opus-4.7"));
    }

    #[test]
    fn iter_log_from_iter_defaults_model_to_none() {
        let it = Iter::new(1, "2026-05-26T00:00:00Z");
        let log = IterLog::from_iter(&it, None, Vec::new(), Vec::new());
        assert!(log.model.is_none());
    }

    #[test]
    fn iter_log_parses_pre_model_fixture_with_model_none() {
        // Backward-compat: legacy iter logs written before the `model`
        // field existed must still parse cleanly (model defaults to None).
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("iter-log-pre-model.json");
        let body = std::fs::read_to_string(&path).expect("read pre-model fixture");
        let parsed: IterLog = serde_json::from_str(&body).expect("parse pre-model fixture");
        assert_eq!(parsed.number, 1);
        assert_eq!(parsed.task_id.as_deref(), Some("hew-abc"));
        assert_eq!(parsed.outcome.as_deref(), Some("closed"));
        assert_eq!(parsed.runtime_used.as_deref(), Some("claude"));
        assert!(parsed.model.is_none(), "missing model must deserialize to None");
    }

    #[test]
    fn stop_reason_label_covers_all_variants() {
        for r in [
            StopReason::Cancelled,
            StopReason::StopFile,
            StopReason::BudgetTokens,
            StopReason::BudgetWall,
            StopReason::MaxIter,
            StopReason::ReadyEmpty,
            StopReason::GuardTrip,
            StopReason::RuntimeError,
        ] {
            assert!(!stop_reason_label(r).is_empty());
        }
    }
}
