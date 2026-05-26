//! Stop-signal gathering for `hew loop`.
//!
//! Decoupled from the pure precedence logic in [`crate::runner`]: this
//! module deals with the I/O side (stop-file polling, wall-clock,
//! SIGINT flag) and produces a [`crate::runner::StopSignals`] snapshot
//! the caller feeds into `evaluate()`.
//!
//! SIGINT integration is intentionally kept as a [`CancelFlag`]
//! wrapper around `Arc<AtomicBool>` rather than installing a handler
//! here — the CLI layer (in the `hew` binary) is responsible for
//! actually wiring `ctrlc` or `signal_hook` to set the flag. That
//! keeps hew-core dependency-light and tractable to test.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::runner::{IterOutcome, Run, StopSignals};

/// Shared cancel flag. Cloneable, thread-safe. The CLI's SIGINT
/// handler calls [`CancelFlag::cancel`]; the loop main thread polls
/// via [`CancelFlag::is_cancelled`].
#[derive(Clone, Debug, Default)]
pub struct CancelFlag {
    flag: Arc<AtomicBool>,
}

impl CancelFlag {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}

/// Watches a sentinel file. Presence = stop.
#[derive(Clone, Debug)]
pub struct StopFileWatcher {
    path: PathBuf,
}

impl StopFileWatcher {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Polls for presence. Any I/O error is treated as "not present"
    /// (e.g. missing parent dir before the run starts).
    pub fn is_set(&self) -> bool {
        self.path.try_exists().unwrap_or(false)
    }
}

/// Wall-clock tracker started at run begin.
#[derive(Clone, Copy, Debug)]
pub struct WallClock {
    started_at: Instant,
}

impl WallClock {
    pub fn start() -> Self {
        Self { started_at: Instant::now() }
    }

    pub fn elapsed(&self) -> Duration {
        self.started_at.elapsed()
    }
}

/// One-shot bundle of all collectors. Constructed at run start, used
/// to project a [`StopSignals`] snapshot before each iter and after
/// each spawner return.
pub struct Collector {
    pub cancel: CancelFlag,
    pub stop_file: StopFileWatcher,
    pub clock: WallClock,
}

impl Collector {
    pub fn new(stop_file_path: impl Into<PathBuf>) -> Self {
        Self {
            cancel: CancelFlag::new(),
            stop_file: StopFileWatcher::new(stop_file_path),
            clock: WallClock::start(),
        }
    }

    /// Build a `StopSignals` snapshot from the collectors plus the
    /// caller-supplied "soft" inputs (ready-queue length and the last
    /// iter's outcome). Keeping those as arguments — rather than
    /// queried in-module — preserves hew-core's freedom from `bd`
    /// dependencies inside this layer.
    pub fn snapshot(
        &self,
        run: &Run,
        ready_queue_len: u32,
        last_iter_outcome: Option<IterOutcome>,
    ) -> StopSignals {
        let (guard_trip, runtime_error) = match last_iter_outcome {
            Some(IterOutcome::BackpressureFail) => (true, false),
            Some(IterOutcome::RuntimeError) => (false, true),
            _ => (false, false),
        };
        StopSignals {
            cancelled: self.cancel.is_cancelled(),
            stop_file_present: self.stop_file.is_set(),
            tokens_spent: run.cumulative_tokens(),
            wall_elapsed: self.clock.elapsed(),
            iters_completed: run.iters.len() as u32,
            ready_queue_len,
            last_iter_guard_trip: guard_trip,
            last_iter_runtime_error: runtime_error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::{Iter, IterOutcome, Run, RunConfig, TokenSpend};
    use std::thread;

    #[test]
    fn cancel_flag_starts_unset_and_sets() {
        let f = CancelFlag::new();
        assert!(!f.is_cancelled());
        f.cancel();
        assert!(f.is_cancelled());
    }

    #[test]
    fn cancel_flag_is_shared_across_clones() {
        let a = CancelFlag::new();
        let b = a.clone();
        thread::spawn(move || b.cancel()).join().unwrap();
        assert!(a.is_cancelled());
    }

    #[test]
    fn stop_file_watcher_reports_absence() {
        let w = StopFileWatcher::new("/tmp/hew-test-stop-file-does-not-exist-xyz123");
        assert!(!w.is_set());
    }

    #[test]
    fn stop_file_watcher_detects_presence() {
        let tmp = std::env::temp_dir().join(format!("hew-stop-{}", std::process::id()));
        std::fs::write(&tmp, "").unwrap();
        let w = StopFileWatcher::new(&tmp);
        assert!(w.is_set());
        std::fs::remove_file(&tmp).unwrap();
        assert!(!w.is_set());
    }

    #[test]
    fn wall_clock_elapsed_increases() {
        let c = WallClock::start();
        let first = c.elapsed();
        std::thread::sleep(Duration::from_millis(2));
        assert!(c.elapsed() > first);
    }

    fn empty_run() -> Run {
        Run::new("loop-test", "2026-05-26T00:00:00Z", RunConfig::default())
    }

    #[test]
    fn snapshot_carries_cumulative_tokens_and_iter_count() {
        let mut run = empty_run();
        let mut i = Iter::new(1, "t");
        i.cost = TokenSpend { input: 50, output: 25, cache_read: 0, cache_create: 0 };
        run.iters.push(i);
        let c = Collector::new("/nonexistent");
        let snap = c.snapshot(&run, 4, None);
        assert_eq!(snap.tokens_spent, 75);
        assert_eq!(snap.iters_completed, 1);
        assert_eq!(snap.ready_queue_len, 4);
        assert!(!snap.last_iter_guard_trip);
        assert!(!snap.last_iter_runtime_error);
    }

    #[test]
    fn snapshot_maps_backpressure_to_guard_trip() {
        let run = empty_run();
        let c = Collector::new("/nonexistent");
        let snap = c.snapshot(&run, 1, Some(IterOutcome::BackpressureFail));
        assert!(snap.last_iter_guard_trip);
        assert!(!snap.last_iter_runtime_error);
    }

    #[test]
    fn snapshot_maps_runtime_error() {
        let run = empty_run();
        let c = Collector::new("/nonexistent");
        let snap = c.snapshot(&run, 1, Some(IterOutcome::RuntimeError));
        assert!(snap.last_iter_runtime_error);
        assert!(!snap.last_iter_guard_trip);
    }

    #[test]
    fn snapshot_clean_outcomes_dont_trigger_flags() {
        let run = empty_run();
        let c = Collector::new("/nonexistent");
        for outcome in [IterOutcome::Closed, IterOutcome::NoClose] {
            let snap = c.snapshot(&run, 1, Some(outcome));
            assert!(!snap.last_iter_guard_trip);
            assert!(!snap.last_iter_runtime_error);
        }
    }

    #[test]
    fn snapshot_propagates_cancel_flag() {
        let run = empty_run();
        let c = Collector::new("/nonexistent");
        c.cancel.cancel();
        let snap = c.snapshot(&run, 1, None);
        assert!(snap.cancelled);
    }
}
