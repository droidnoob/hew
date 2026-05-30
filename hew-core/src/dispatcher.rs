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
use std::path::Path;

use crate::batch_plan::{BatchPlan, BatchSource};
use crate::bd::{BdClient, ReadyTask};
use crate::error::Result;
use crate::git::GitClient;
use crate::merge_back::{self, MergeReport};
use crate::scope::{self, Scope};
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
    /// Counts the **post-filter** set when a [`BatchPlan`] is active.
    pub ready_seen: usize,
    /// Tasks the dispatcher tried to claim but `bd` rejected — typically
    /// a race with another agent claiming the same id. The slot stays
    /// idle and will be retried next tick.
    pub claim_failures: Vec<ClaimFailure>,
    /// Provenance of the active batch plan, if any narrowed this tick.
    /// `None` when no plan is set or when the plan's source is
    /// [`BatchSource::Skipped`] (fall-through to trust-the-graph).
    pub batch_source: Option<BatchSource>,
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
    scope: Scope,
    batch_plan: Option<BatchPlan>,
}

impl Dispatcher {
    /// `jobs` is clamped to a minimum of 1 (zero workers is meaningless).
    ///
    /// `scope` decides which bd-ready tasks count as the queue. For the
    /// pre-scope behavior (every bd-ready task) pass [`Scope::Ready`].
    /// For epic-scoped runs the descendant set is recomputed inside
    /// [`Self::dispatch_tick`] on every tick, so children added to a
    /// selected epic mid-run get picked up automatically.
    pub fn new(
        jobs: u32,
        run_id: impl Into<String>,
        base_sha: impl Into<String>,
        scope: Scope,
        batch_plan: Option<BatchPlan>,
    ) -> Self {
        let n = (jobs.max(1)) as usize;
        Self {
            slots: vec![SlotState::Idle; n],
            run_id: run_id.into(),
            base_sha: base_sha.into(),
            scope,
            batch_plan,
        }
    }

    /// Provenance of the active batch plan, if one narrowed dispatch.
    /// Returns `None` when no plan is set or the plan's source is
    /// [`BatchSource::Skipped`] (fall-through to trust-the-graph).
    pub fn current_batch_source(&self) -> Option<BatchSource> {
        match &self.batch_plan {
            Some(p) if p.source != BatchSource::Skipped && !p.task_ids.is_empty() => Some(p.source),
            _ => None,
        }
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
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

        // Apply the run's scope filter to the raw ready list. For
        // `Scope::Epics` we re-resolve the descendant set each tick so
        // children added to a selected epic mid-run get picked up; for
        // `Scope::Ready` the set is irrelevant and we skip the walk.
        let descendant_set: HashSet<String> = match &self.scope {
            Scope::Ready => HashSet::new(),
            Scope::Epics { epic_ids } => scope::resolve_descendants(bd, epic_ids)?,
        };
        let ready: Vec<ReadyTask> =
            ready.into_iter().filter(|t| self.scope.includes(&t.id, &descendant_set)).collect();

        // Batch-plan narrowing — non-expansive (batch ∩ bd-ready). The
        // bd dep graph remains the safety floor per
        // `DECISION:loop-parallel-overlap-policy`; a batch can only
        // shrink the candidate set, never reintroduce a blocked task.
        // Skipped plans and empty task_ids fall through unchanged.
        let batch_source = self.current_batch_source();
        let ready: Vec<ReadyTask> = match (&self.batch_plan, batch_source) {
            (Some(plan), Some(_)) => {
                // Typical batch is <10 ids — linear contains is fine
                // and avoids the per-tick HashSet allocation.
                ready.into_iter().filter(|t| plan.task_ids.iter().any(|id| id == &t.id)).collect()
            }
            _ => ready,
        };

        let mut tick = DispatchTick { ready_seen: ready.len(), batch_source, ..Default::default() };
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

    /// Consolidate every worker branch back onto `base_branch` and file
    /// `[merge-conflict]` bug tasks for any that didn't merge cleanly.
    /// Worktrees are intentionally NOT removed — the human resolving the
    /// conflict needs to `cd ~/.hew/wt/<run-id>/<n>/` per
    /// `DECISION:loop-parallel-overlap-policy`.
    ///
    /// Returns the [`MergeReport`] plus the IDs of any bug tasks filed
    /// (one per conflict).
    pub fn shutdown_merge_back(
        &self,
        git: &dyn GitClient,
        bd: &dyn BdClient,
        project_root: &Path,
        base_branch: &str,
        worker_branches: &[String],
    ) -> Result<(MergeReport, Vec<String>)> {
        let report = merge_back::merge_back(git, project_root, base_branch, worker_branches)?;
        let bug_ids = merge_back::file_conflict_bug_tasks(bd, self.run_id(), &report.conflicts)?;
        Ok((report, bug_ids))
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
        /// `parent_id → [child_id, …]` for `bd children <id> --json`
        /// responses. Used by the `Scope::Epics` filter tests.
        children: RefCell<BTreeMap<String, Vec<String>>>,
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

        fn with_children(self, parent: &str, ids: &[&str]) -> Self {
            self.children
                .borrow_mut()
                .insert(parent.to_string(), ids.iter().map(|s| s.to_string()).collect());
            self
        }

        fn add_child(&self, parent: &str, id: &str) {
            self.children.borrow_mut().entry(parent.to_string()).or_default().push(id.to_string());
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
            // tasks::children → ["children", <id>, "--json"]
            if args.len() == 3
                && args[0] == OsStr::new("children")
                && args[2] == OsStr::new("--json")
            {
                let parent = args[1].to_string_lossy().to_string();
                let kids = self.children.borrow().get(&parent).cloned().unwrap_or_default();
                let body = kids
                    .iter()
                    .map(|id| {
                        format!(
                            r#"{{"id":"{id}","title":"t-{id}","description":"","status":"open","priority":2,"issue_type":"task","closed_at":"","close_reason":null,"parent":"{parent}"}}"#
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(",");
                return Ok(BdOutput { stdout: format!("[{body}]"), stderr: String::new() });
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
        let d = Dispatcher::new(0, "run-x", "deadbeef", Scope::Ready, None);
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
        let mut d = Dispatcher::new(1, "run-1", "sha", Scope::Ready, None);
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
        let mut d = Dispatcher::new(3, "run-2", "sha", Scope::Ready, None);
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
        let mut d = Dispatcher::new(2, "run-3", "sha", Scope::Ready, None);
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
        let mut d = Dispatcher::new(1, "run-4", "sha", Scope::Ready, None);
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

        let mut d = Dispatcher::new(1, "run-5", "sha", Scope::Ready, None);
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
        let mut d = Dispatcher::new(2, "run-6", "sha", Scope::Ready, None);
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
        let mut d = Dispatcher::new(1, "run-7", "sha", Scope::Ready, None);
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
        let mut d = Dispatcher::new(2, "run-8", "sha", Scope::Ready, None);

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

    // ── Scope filter coverage ───────────────────────────────────────

    #[test]
    fn dispatch_tick_ready_scope_unfiltered() {
        // Scope::Ready must surface every bd-ready task — no descendant
        // walk, no filtering.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c")]);
        let mut d = Dispatcher::new(3, "run-scope-ready", "sha", Scope::Ready, None);
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 3);
        let ids: Vec<&str> = tick.assignments.iter().map(|a| a.task.id.as_str()).collect();
        assert_eq!(ids, vec!["hew-a", "hew-b", "hew-c"]);
    }

    #[test]
    fn dispatch_tick_epics_scope_filters_to_descendants() {
        // Two epics in the graph, only one is selected. Unrelated tasks
        // are filtered out before slot assignment, and ready_seen
        // counts the filtered set.
        let bd =
            MockBd::new(vec![ready("hew-child-1"), ready("hew-stranger"), ready("hew-child-2")])
                .with_children("hew-epic-a", &["hew-child-1", "hew-child-2"])
                .with_children("hew-epic-b", &["hew-stranger"])
                .with_children("hew-child-1", &[])
                .with_children("hew-child-2", &[])
                .with_children("hew-stranger", &[]);
        let scope = Scope::Epics { epic_ids: vec!["hew-epic-a".into()] };
        let mut d = Dispatcher::new(3, "run-scope-epic", "sha", scope, None);
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 2, "stranger filtered out");
        let ids: Vec<&str> = tick.assignments.iter().map(|a| a.task.id.as_str()).collect();
        assert_eq!(ids, vec!["hew-child-1", "hew-child-2"]);
        assert_eq!(bd.claimed(), vec!["hew-child-1", "hew-child-2"]);
    }

    #[test]
    fn dispatch_tick_epics_scope_empty_when_no_match() {
        // Selected epic has no descendants in the ready set. Nothing
        // assigned, no claim attempted, ready_seen reports the filtered
        // zero so callers see "queue drained" for this scope.
        let bd = MockBd::new(vec![ready("hew-stranger"), ready("hew-other")])
            .with_children("hew-epic-empty", &[]);
        let scope = Scope::Epics { epic_ids: vec!["hew-epic-empty".into()] };
        let mut d = Dispatcher::new(2, "run-scope-empty", "sha", scope, None);
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 0);
        assert!(tick.assignments.is_empty());
        assert!(bd.claimed().is_empty());
        assert!(d.all_idle());
    }

    #[test]
    fn dispatch_tick_epics_recomputes_descendants_each_tick() {
        // Mid-run a new child is added to the selected epic. The next
        // dispatch_tick must re-walk descendants and pick it up
        // without a Dispatcher rebuild — the cache is per-tick by
        // design.
        let bd = MockBd::new(vec![ready("hew-child-1")])
            .with_children("hew-epic-live", &["hew-child-1"])
            .with_children("hew-child-1", &[]);
        let scope = Scope::Epics { epic_ids: vec!["hew-epic-live".into()] };
        let mut d = Dispatcher::new(2, "run-scope-live", "sha", scope, None);

        let t1 = d.dispatch_tick(&bd).expect("first tick");
        assert_eq!(t1.assignments.len(), 1);
        assert_eq!(t1.assignments[0].task.id, "hew-child-1");

        // New child added to the live epic + becomes ready.
        bd.add_child("hew-epic-live", "hew-child-2");
        bd.ready.borrow_mut().push(ready("hew-child-2"));

        let t2 = d.dispatch_tick(&bd).expect("second tick");
        assert_eq!(t2.ready_seen, 1, "newly-added child seen after recompute");
        assert_eq!(t2.assignments.len(), 1);
        assert_eq!(t2.assignments[0].task.id, "hew-child-2");
    }

    // ── BatchPlan filter coverage ───────────────────────────────────

    fn plan(iter: u32, source: BatchSource, ids: &[&str]) -> BatchPlan {
        BatchPlan {
            schema_version: crate::batch_plan::SCHEMA_VERSION,
            iter_number: iter,
            task_ids: ids.iter().map(|s| s.to_string()).collect(),
            source,
            reason: None,
            created_at: "2026-05-30T00:00:00Z".into(),
            planner_tokens: None,
        }
    }

    #[test]
    fn dispatch_tick_no_plan_behaves_as_today() {
        // Sanity: a `batch_plan: None` Dispatcher behaves identically
        // to the pre-batch-plan world. Mirrors `n1_dispatcher_assigns_…`.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b")]);
        let mut d = Dispatcher::new(2, "run-bp-none", "sha", Scope::Ready, None);
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 2);
        assert_eq!(tick.assignments.len(), 2);
        assert!(tick.batch_source.is_none(), "no plan → no batch_source");
        assert!(d.current_batch_source().is_none());
    }

    #[test]
    fn dispatch_tick_agent_plan_filters_candidates() {
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c")]);
        let plan = plan(1, BatchSource::Agent, &["hew-b"]);
        let mut d = Dispatcher::new(2, "run-bp-agent", "sha", Scope::Ready, Some(plan));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 1, "post-filter count");
        assert_eq!(tick.assignments.len(), 1);
        assert_eq!(tick.assignments[0].task.id, "hew-b");
        assert_eq!(tick.batch_source, Some(BatchSource::Agent));
        assert_eq!(bd.claimed(), vec!["hew-b"]);
    }

    #[test]
    fn dispatch_tick_planner_plan_filters_candidates() {
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c")]);
        let plan = plan(1, BatchSource::Planner, &["hew-a", "hew-c"]);
        let mut d = Dispatcher::new(3, "run-bp-planner", "sha", Scope::Ready, Some(plan));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 2);
        let ids: Vec<&str> = tick.assignments.iter().map(|a| a.task.id.as_str()).collect();
        assert_eq!(ids, vec!["hew-a", "hew-c"]);
        assert_eq!(tick.batch_source, Some(BatchSource::Planner));
    }

    #[test]
    fn dispatch_tick_skipped_plan_falls_through_to_full_bd_ready() {
        // Source::Skipped means trust-the-graph — no filtering, no
        // batch_source on the tick.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b")]);
        let plan = plan(1, BatchSource::Skipped, &[]);
        let mut d = Dispatcher::new(2, "run-bp-skip", "sha", Scope::Ready, Some(plan));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 2);
        assert_eq!(tick.assignments.len(), 2);
        assert!(tick.batch_source.is_none());
        assert!(d.current_batch_source().is_none());
    }

    #[test]
    fn dispatch_tick_empty_task_ids_falls_through_to_full_bd_ready() {
        // Defensive: an Agent/Planner plan with an empty task_ids array
        // is treated as no-narrowing rather than "block everything".
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b")]);
        let plan = plan(1, BatchSource::Agent, &[]);
        let mut d = Dispatcher::new(2, "run-bp-empty", "sha", Scope::Ready, Some(plan));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 2);
        assert_eq!(tick.assignments.len(), 2);
        assert!(tick.batch_source.is_none(), "empty task_ids → no narrowing signaled");
    }

    #[test]
    fn dispatch_tick_batch_task_id_not_in_ready_is_dropped() {
        // Hard floor: a batch naming a blocked or unknown task does not
        // resurrect it. batch ∩ bd-ready, never batch ∪ anything.
        let bd = MockBd::new(vec![ready("hew-a")]);
        let plan = plan(1, BatchSource::Agent, &["hew-a", "hew-blocked", "hew-ghost"]);
        let mut d = Dispatcher::new(3, "run-bp-floor", "sha", Scope::Ready, Some(plan));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 1, "blocked/ghost ids dropped by intersect");
        assert_eq!(tick.assignments.len(), 1);
        assert_eq!(tick.assignments[0].task.id, "hew-a");
        assert_eq!(bd.claimed(), vec!["hew-a"]);
    }

    #[test]
    fn dispatch_tick_ready_seen_reflects_post_filter_count() {
        // Explicit pin: ready_seen is the post-filter, post-scope count
        // — what downstream summary aggregation consumes.
        let bd = MockBd::new(vec![ready("hew-a"), ready("hew-b"), ready("hew-c"), ready("hew-d")]);
        let plan = plan(2, BatchSource::Planner, &["hew-b", "hew-c"]);
        let mut d = Dispatcher::new(4, "run-bp-count", "sha", Scope::Ready, Some(plan));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.ready_seen, 2, "two of four candidates survived the filter");
    }

    #[test]
    fn dispatch_tick_batch_source_captured_for_summary() {
        // The summary path reads `Dispatcher::current_batch_source()`
        // out-of-band of any tick; verify the accessor returns the
        // active provenance and matches the per-tick field.
        let bd = MockBd::new(vec![ready("hew-a")]);
        let plan = plan(1, BatchSource::Agent, &["hew-a"]);
        let mut d = Dispatcher::new(1, "run-bp-summary", "sha", Scope::Ready, Some(plan));
        assert_eq!(d.current_batch_source(), Some(BatchSource::Agent));
        let tick = d.dispatch_tick(&bd).expect("tick");
        assert_eq!(tick.batch_source, Some(BatchSource::Agent));
        assert_eq!(d.current_batch_source(), Some(BatchSource::Agent));
    }
}
