//! Per-run dispatcher for the parallel `hew loop`.
//!
//! Tracks N worker slots over the run's lifetime: queries `bd ready`,
//! claims free tasks atomically, and reports which slot each task got
//! assigned to. The caller drives the actual workers (threads, real
//! `git` ops, runtime spawns) — the dispatcher itself is a pure state
//! machine so unit tests don't touch threads, git, bd, or claude.
//!
//! v1 default is `jobs=1`, in which case the dispatcher is a thin
//! wrapper over the existing sequential loop: one slot, fill it from
//! `bd ready`, complete, repeat.
//!
//! Per `DECISION:loop-parallel-overlap-policy` ("trust-the-graph"),
//! the dispatcher trusts that any `bd ready` task is parallelizable —
//! it does NOT do overlap detection. Conflicts on merge-back surface
//! later as `[merge-conflict]` bug tasks.

use std::collections::HashSet;

use crate::bd::{BdClient, ReadyTask};
use crate::error::Result;
use crate::tasks;

/// State of a single worker slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotState {
    /// Slot is free — next [`Dispatcher::dispatch_tick`] may fill it.
    Idle,
    /// A task has been claimed and assigned to this slot. The caller's
    /// worker is responsible for closing the task and then calling
    /// [`Dispatcher::complete`] to release the slot.
    Running { task_id: String },
}

/// Result of one [`Dispatcher::dispatch_tick`] call.
///
/// `ready_seen` reports how many `bd ready` tasks the dispatcher
/// looked at (before slot capacity / claim failures), so callers can
/// detect "queue drained" by checking `ready_seen == 0`.
#[derive(Debug, Default)]
pub struct DispatchTick {
    /// New slot assignments made this tick.
    pub assignments: Vec<Assignment>,
    /// Number of `bd ready` tasks visible (whether or not assigned).
    pub ready_seen: usize,
    /// Tasks the dispatcher tried to claim but `bd` rejected — typically
    /// a race with another agent claiming the same id. The slot stays
    /// idle and will be retried next tick.
    pub claim_failures: Vec<ClaimFailure>,
}

/// A task that was claimed and pinned to a specific slot this tick.
#[derive(Debug, Clone)]
pub struct Assignment {
    pub slot_id: u32,
    pub task: ReadyTask,
}

/// A claim attempt that bd rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimFailure {
    pub task_id: String,
    pub message: String,
}

/// Per-run dispatcher. Owns the slot vector + run metadata; consults
/// the injected [`BdClient`] for `ready` + `claim` only.
#[derive(Debug)]
pub struct Dispatcher {
    slots: Vec<SlotState>,
    run_id: String,
    base_sha: String,
}

impl Dispatcher {
    /// `jobs` is clamped to a minimum of 1 (zero workers is meaningless).
    pub fn new(jobs: u32, run_id: impl Into<String>, base_sha: impl Into<String>) -> Self {
        let n = (jobs.max(1)) as usize;
        Self { slots: vec![SlotState::Idle; n], run_id: run_id.into(), base_sha: base_sha.into() }
    }

    pub fn jobs(&self) -> u32 {
        self.slots.len() as u32
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn base_sha(&self) -> &str {
        &self.base_sha
    }

    pub fn slot(&self, slot_id: u32) -> Option<&SlotState> {
        self.slots.get(slot_id as usize)
    }
    pub fn slots(&self) -> &[SlotState] {
        &self.slots
    }

    pub fn free_slot_ids(&self) -> Vec<u32> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| matches!(s, SlotState::Idle).then_some(i as u32))
            .collect()
    }

    pub fn running(&self) -> Vec<(u32, &str)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(i, s)| match s {
                SlotState::Running { task_id } => Some((i as u32, task_id.as_str())),
                SlotState::Idle => None,
            })
            .collect()
    }

    pub fn all_idle(&self) -> bool {
        self.slots.iter().all(|s| matches!(s, SlotState::Idle))
    }

    /// Fill every free slot by pulling from `bd ready`. Each picked
    /// task is claimed atomically via `hew_core::tasks::claim`; on a
    /// claim race the slot stays idle and the failure is reported.
    ///
    /// The dispatcher skips any `bd ready` task whose id matches a
    /// currently-running slot — defense against stale snapshots where
    /// a just-claimed task still appears in the ready list.
    pub fn dispatch_tick(&mut self, bd: &dyn BdClient) -> Result<DispatchTick> {
        let free = self.free_slot_ids();
        if free.is_empty() {
            return Ok(DispatchTick::default());
        }
        let ready = bd.ready()?;
        let mut tick = DispatchTick { ready_seen: ready.len(), ..Default::default() };
        if ready.is_empty() {
            return Ok(tick);
        }

        let in_flight: HashSet<String> = self
            .slots
            .iter()
            .filter_map(|s| match s {
                SlotState::Running { task_id } => Some(task_id.clone()),
                SlotState::Idle => None,
            })
            .collect();

        let mut free_iter = free.into_iter();
        for task in ready {
            if in_flight.contains(&task.id) {
                continue;
            }
            let Some(slot) = free_iter.next() else { break };
            match tasks::claim(bd, &task.id) {
                Ok(()) => {
                    self.slots[slot as usize] = SlotState::Running { task_id: task.id.clone() };
                    tick.assignments.push(Assignment { slot_id: slot, task });
                }
                Err(e) => {
                    tick.claim_failures
                        .push(ClaimFailure { task_id: task.id, message: e.to_string() });
                    // Slot stays idle. Put it back at the front so the
                    // next ready task can land in it.
                    free_iter =
                        std::iter::once(slot).chain(free_iter).collect::<Vec<_>>().into_iter();
                }
            }
        }

        Ok(tick)
    }

    /// Release `slot_id`, returning the task id that was running there
    /// (or `None` if the slot was already idle / out of range).
    pub fn complete(&mut self, slot_id: u32) -> Option<String> {
        let idx = slot_id as usize;
        if idx >= self.slots.len() {
            return None;
        }
        match std::mem::replace(&mut self.slots[idx], SlotState::Idle) {
            SlotState::Running { task_id } => Some(task_id),
            SlotState::Idle => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bd::{BdOutput, BdVersion, StatsSummary};
    use crate::error::HewError;
    use std::cell::RefCell;
    use std::collections::{BTreeMap, HashSet as StdHashSet};
    use std::ffi::OsStr;

    #[derive(Debug, Default)]
    struct MockBd {
        ready: RefCell<Vec<ReadyTask>>,
        claimed: RefCell<Vec<String>>,
        /// Task ids whose claim attempt should fail (simulates a race
        /// against another agent). Failure is one-shot per id —
        /// removed after the first attempt.
        claim_fails: RefCell<StdHashSet<String>>,
    }

    impl MockBd {
        fn new(ready: Vec<ReadyTask>) -> Self {
            Self { ready: RefCell::new(ready), ..Default::default() }
        }

        fn fail_claim(&self, id: &str) {
            self.claim_fails.borrow_mut().insert(id.to_string());
        }

        fn claimed(&self) -> Vec<String> {
            self.claimed.borrow().clone()
        }
    }

    impl BdClient for MockBd {
        fn version(&self) -> Result<BdVersion> {
            Ok(BdVersion { raw: "test".into(), semver: "0.0.0".into() })
        }
        fn ready(&self) -> Result<Vec<ReadyTask>> {
            Ok(self.ready.borrow().clone())
        }
        fn stats(&self) -> Result<StatsSummary> {
            Ok(StatsSummary::default())
        }
        fn prime_raw(&self) -> Result<String> {
            Ok(String::new())
        }
        fn memories(&self) -> Result<BTreeMap<String, String>> {
            Ok(BTreeMap::new())
        }
        fn remember(&self, _text: &str) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
            // tasks::claim → ["update", <id>, "--claim"]
            if args.len() == 3
                && args[0] == OsStr::new("update")
                && args[2] == OsStr::new("--claim")
            {
                let id = args[1].to_string_lossy().to_string();
                if self.claim_fails.borrow_mut().remove(&id) {
                    return Err(HewError::BdNonZero {
                        code: 1,
                        stderr: format!("issue {id} already claimed"),
                    });
                }
                // Mimic real bd: a claimed task disappears from the
                // ready queue. Tests that rely on stale snapshots
                // disable this by pre-populating `ready` after the
                // claim, but the default behavior matches production.
                self.ready.borrow_mut().retain(|t| t.id != id);
                self.claimed.borrow_mut().push(id);
                return Ok(BdOutput { stdout: String::new(), stderr: String::new() });
            }
            Ok(BdOutput { stdout: String::new(), stderr: String::new() })
        }
    }

    fn ready(id: &str) -> ReadyTask {
        ReadyTask {
            id: id.into(),
            title: format!("task {id}"),
            description: String::new(),
            priority: 1,
            status: "open".into(),
            issue_type: "task".into(),
            parent: None,
        }
    }

    #[test]
    fn new_clamps_jobs_to_at_least_one() {
        let d = Dispatcher::new(0, "run-x", "deadbeef");
        assert_eq!(d.jobs(), 1);
        assert_eq!(d.slots().len(), 1);
        assert!(d.all_idle());
        assert_eq!(d.run_id(), "run-x");
        assert_eq!(d.base_sha(), "deadbeef");
    }

    #[test]
    fn n1_dispatcher_assigns_one_task_and_leaves_others() {
        // Regression for acceptance: N=1 picks the first ready task and
        // stops — identical to today's serial loop.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c")]);
        let mut d = Dispatcher::new(1, "run-1", "sha");
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.assignments.len(), 1, "exactly one slot filled");
        assert_eq!(tick.assignments[0].slot_id, 0);
        assert_eq!(tick.assignments[0].task.id, "hew-a");
        assert_eq!(tick.ready_seen, 3);
        assert_eq!(bd.claimed(), vec!["hew-a"]);
        assert!(!d.all_idle());
        assert_eq!(d.running(), vec![(0, "hew-a")]);
    }

    #[test]
    fn dispatcher_fills_all_slots_when_ready_has_enough() {
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c"), ready("hew-d")]);
        let mut d = Dispatcher::new(3, "run-2", "sha");
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.assignments.len(), 3, "all 3 slots filled");
        let ids: Vec<&str> = tick.assignments.iter().map(|a| a.task.id.as_str()).collect();
        assert_eq!(ids, vec!["hew-a", "hew-b", "hew-c"]);
        // slot ids are 0,1,2 in order.
        let slot_ids: Vec<u32> = tick.assignments.iter().map(|a| a.slot_id).collect();
        assert_eq!(slot_ids, vec![0, 1, 2]);
        assert_eq!(bd.claimed(), vec!["hew-a", "hew-b", "hew-c"]);
        assert!(d.free_slot_ids().is_empty());
    }

    #[test]
    fn dispatcher_skips_assignment_when_ready_empty() {
        let bd = MockBd::new(vec![]);
        let mut d = Dispatcher::new(2, "run-3", "sha");
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert!(tick.assignments.is_empty());
        assert_eq!(tick.ready_seen, 0);
        assert!(d.all_idle());
        assert!(bd.claimed().is_empty());
    }

    #[test]
    fn dispatcher_does_nothing_when_all_slots_busy() {
        // No `bd ready` should even be called when capacity = 0.
        let bd = MockBd::new(vec![ready("hew-z")]);
        let mut d = Dispatcher::new(1, "run-4", "sha");
        d.dispatch_tick(&bd).expect("first tick");
        // Second tick: slot is full.
        let tick = d.dispatch_tick(&bd).expect("second tick");
        assert!(tick.assignments.is_empty());
        assert_eq!(tick.ready_seen, 0, "ready not queried when slots full");
        assert_eq!(bd.claimed(), vec!["hew-z"], "no double-claim");
    }

    #[test]
    fn dispatcher_double_claim_handled_via_bd_atomic() {
        // bd rejects the claim (race with another agent). Slot stays
        // idle; next ready task fills it.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b")]);
        bd.fail_claim("hew-a");

        let mut d = Dispatcher::new(1, "run-5", "sha");
        let tick = d.dispatch_tick(&bd).expect("tick");

        assert_eq!(tick.claim_failures.len(), 1);
        assert_eq!(tick.claim_failures[0].task_id, "hew-a");
        assert_eq!(tick.assignments.len(), 1, "fell through to hew-b");
        assert_eq!(tick.assignments[0].task.id, "hew-b");
        assert_eq!(tick.assignments[0].slot_id, 0);
        assert_eq!(bd.claimed(), vec!["hew-b"]);
    }

    #[test]
    fn dispatcher_skips_tasks_already_in_flight() {
        // Simulates a stale `bd ready` snapshot that still lists a
        // task already running in another slot. Real-world cause: bd
        // returned the task to two `ready` queries before either
        // claim landed. Dispatcher must not double-assign.
        let bd = MockBd::new(vec![ready("hew-a")]);
        let mut d = Dispatcher::new(2, "run-6", "sha");
        let tick1 = d.dispatch_tick(&bd).expect("first tick");
        assert_eq!(tick1.assignments.len(), 1);
        assert_eq!(tick1.assignments[0].task.id, "hew-a");

        // Push the same task back into ready (mimics a stale snapshot).
        bd.ready.borrow_mut().push(ready("hew-a"));
        let tick2 = d.dispatch_tick(&bd).expect("second tick");
        assert!(tick2.assignments.is_empty(), "no double-assignment of in-flight task");
        // Only one claim was made — the dispatcher refused the second.
        assert_eq!(bd.claimed(), vec!["hew-a"]);
    }

    #[test]
    fn complete_returns_running_task_id_and_idles_slot() {
        let bd = MockBd::new(vec![ready("hew-a")]);
        let mut d = Dispatcher::new(1, "run-7", "sha");
        d.dispatch_tick(&bd).expect("tick");
        assert_eq!(d.complete(0), Some("hew-a".into()));
        assert!(d.all_idle());
        // Second complete on the same slot returns None.
        assert_eq!(d.complete(0), None);
        // Out-of-range slot is None, not panic.
        assert_eq!(d.complete(99), None);
    }

    #[test]
    fn polls_completed_workers_and_records_outcomes() {
        // Stand-in for the future "polls_completed" surface: the
        // dispatcher's `complete()` is the seam each worker thread
        // calls when its iter body returns. Two workers run, both
        // complete in turn, capacity restores.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c")]);
        let mut d = Dispatcher::new(2, "run-8", "sha");

        let t1 = d.dispatch_tick(&bd).expect("tick 1");
        assert_eq!(t1.assignments.len(), 2);

        // Worker for slot 1 finishes first.
        assert_eq!(d.complete(1), Some("hew-b".into()));
        assert_eq!(d.free_slot_ids(), vec![1]);

        // Dispatcher refills slot 1.
        let t2 = d.dispatch_tick(&bd).expect("tick 2");
        assert_eq!(t2.assignments.len(), 1);
        assert_eq!(t2.assignments[0].slot_id, 1);
        assert_eq!(t2.assignments[0].task.id, "hew-c");

        // Both workers finish.
        assert_eq!(d.complete(0), Some("hew-a".into()));
        assert_eq!(d.complete(1), Some("hew-c".into()));
        assert!(d.all_idle());
    }
}
