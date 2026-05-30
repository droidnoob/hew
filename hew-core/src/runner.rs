//! Loop runner orchestrator — pure types and precedence logic for
//! `hew loop`. No I/O: callers gather signals (stop-file presence,
//! wall-clock, ready-queue length, runtime exit codes) and feed them in
//! as values, so this module stays trivially testable.
//!
//! See epic hew-gr1 for the full design. The CLI verb is `hew loop`;
//! the internal module is `runner` because `loop` is a Rust keyword.

use std::time::Duration;

use crate::config::LoopModelConfig;
use crate::runtime::{RuntimeSpawner, SpawnFailureClass};
use crate::scope::Scope;

/// Per-run configuration. Set once at `hew loop` invocation, immutable
/// for the duration of the run.
#[derive(Clone, Debug)]
pub struct RunConfig {
    /// Hard cap on iterations. `None` = unlimited (rely on other stops).
    pub max_iter: Option<u32>,
    /// Stop when the ready queue drains. Default true for `--until-empty`.
    pub stop_on_ready_empty: bool,
    /// Cumulative token budget across all iters. `None` = unlimited.
    pub budget_tokens: Option<u64>,
    /// Wall-clock budget for the whole run. `None` = unlimited.
    pub budget_wall: Option<Duration>,
    /// Promote craft.testing + craft.lint warnings to failures.
    pub strict: bool,
    /// Allow ask-files to interrupt the loop for operator input.
    pub interactive: bool,
    /// Resolve `DEFERRED:` memories the agent files during an iter by
    /// running `decide::resolve` after the iter completes. Mutually
    /// exclusive with `interactive`.
    pub unattended: bool,
    /// Per-task model selection knobs consumed by
    /// [`crate::loop_model::resolve_model`] to pick a `--model` /
    /// `-m` override for each iter. Empty by default (no overrides;
    /// the spawner falls back to its own default).
    pub loop_model: LoopModelConfig,
    /// Which slice of bd-ready tasks this run is scoped to.
    /// [`Scope::Ready`] is the pre-scope default — every bd-ready
    /// task counts. The CLI / picker layer resolves this once at run
    /// start; the dispatcher reads it through `Dispatcher::new`.
    pub scope: Scope,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            max_iter: None,
            stop_on_ready_empty: true,
            budget_tokens: None,
            budget_wall: None,
            strict: true,
            interactive: false,
            unattended: false,
            loop_model: LoopModelConfig::default(),
            scope: Scope::default(),
        }
    }
}

/// Why the run stopped. Ordered by precedence — `Cancelled` wins over
/// `StopFile`, `StopFile` wins over budgets, etc. See
/// [`StopSignals::evaluate`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopReason {
    /// Operator pressed ctrl-C (SIGINT delivered).
    Cancelled,
    /// `.hew/loop/<run-id>/.stop` exists.
    StopFile,
    /// Cumulative token budget exhausted.
    BudgetTokens,
    /// Wall-clock budget elapsed.
    BudgetWall,
    /// Hit `--max-iter`.
    MaxIter,
    /// `hew status` ready queue is empty (and `stop_on_ready_empty` set).
    ReadyEmpty,
    /// A guard skill (hew-guard) tripped on the last iter.
    GuardTrip,
    /// Runtime spawner returned an unrecoverable error.
    RuntimeError,
}

impl StopReason {
    /// Parse the snake_case label persisted in `run.json` back into a
    /// [`StopReason`]. Inverse of `loop_log::stop_reason_label`; used by
    /// `hew loop summary` to re-render a past run from disk. Unknown
    /// labels return `None`.
    pub fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "cancelled" => Self::Cancelled,
            "stop_file" => Self::StopFile,
            "budget_tokens" => Self::BudgetTokens,
            "budget_wall" => Self::BudgetWall,
            "max_iter" => Self::MaxIter,
            "ready_empty" => Self::ReadyEmpty,
            "guard_trip" => Self::GuardTrip,
            "runtime_error" => Self::RuntimeError,
            _ => return None,
        })
    }
}

/// A snapshot of stop-relevant signals at one decision point. Caller
/// gathers; this module decides.
#[derive(Clone, Copy, Debug, Default)]
pub struct StopSignals {
    pub cancelled: bool,
    pub stop_file_present: bool,
    pub tokens_spent: u64,
    pub wall_elapsed: Duration,
    pub iters_completed: u32,
    pub ready_queue_len: u32,
    pub last_iter_guard_trip: bool,
    pub last_iter_runtime_error: bool,
}

impl StopSignals {
    /// Decide whether to stop, and why. Returns `None` when the loop
    /// should continue. Precedence matches the variant order on
    /// [`StopReason`].
    pub fn evaluate(&self, cfg: &RunConfig) -> Option<StopReason> {
        if self.cancelled {
            return Some(StopReason::Cancelled);
        }
        if self.stop_file_present {
            return Some(StopReason::StopFile);
        }
        if let Some(cap) = cfg.budget_tokens
            && self.tokens_spent >= cap
        {
            return Some(StopReason::BudgetTokens);
        }
        if let Some(deadline) = cfg.budget_wall
            && self.wall_elapsed >= deadline
        {
            return Some(StopReason::BudgetWall);
        }
        if let Some(cap) = cfg.max_iter
            && self.iters_completed >= cap
        {
            return Some(StopReason::MaxIter);
        }
        if self.last_iter_runtime_error {
            return Some(StopReason::RuntimeError);
        }
        if self.last_iter_guard_trip {
            return Some(StopReason::GuardTrip);
        }
        if cfg.stop_on_ready_empty && self.ready_queue_len == 0 {
            return Some(StopReason::ReadyEmpty);
        }
        None
    }
}

/// Outcome of a single iter.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IterOutcome {
    /// Task closed cleanly, tests/lint passed.
    Closed,
    /// Spawner exited but no task closed — operator review needed.
    NoClose,
    /// Tests or lint failed under `--strict`; iter's commits reverted.
    BackpressureFail,
    /// Spawner returned a hard error.
    RuntimeError,
}

/// Per-iter accounting. Pure data; serialized to
/// `.hew/loop/<run-id>/iter-NNN.json` by the log layer (separate task).
#[derive(Clone, Debug)]
pub struct Iter {
    pub number: u32,
    pub task_id: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub outcome: Option<IterOutcome>,
    pub cost: TokenSpend,
    /// Decisions resolved during this iter (memory ids of DECISION: entries).
    pub decisions: Vec<String>,
    /// Topics deferred for operator review (memory ids of DEFERRED: entries).
    pub deferred: Vec<String>,
    /// Brief stderr tail captured from the runtime if it errored.
    pub stderr_tail: Option<String>,
}

impl Iter {
    pub fn new(number: u32, started_at: impl Into<String>) -> Self {
        Self {
            number,
            task_id: None,
            started_at: started_at.into(),
            ended_at: None,
            outcome: None,
            cost: TokenSpend::default(),
            decisions: Vec::new(),
            deferred: Vec::new(),
            stderr_tail: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TokenSpend {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
}

impl TokenSpend {
    pub fn total(&self) -> u64 {
        self.input + self.output + self.cache_read + self.cache_create
    }
}

/// Aggregate state for one `hew loop` invocation.
#[derive(Clone, Debug)]
pub struct Run {
    pub id: String,
    pub started_at: String,
    pub config: RunConfig,
    pub iters: Vec<Iter>,
    pub stop_reason: Option<StopReason>,
}

impl Run {
    pub fn new(id: impl Into<String>, started_at: impl Into<String>, config: RunConfig) -> Self {
        Self {
            id: id.into(),
            started_at: started_at.into(),
            config,
            iters: Vec::new(),
            stop_reason: None,
        }
    }

    /// Iter number to assign to the next iter (1-indexed).
    pub fn next_iter_number(&self) -> u32 {
        self.iters.len() as u32 + 1
    }

    /// Sum of token spend across all completed iters.
    pub fn cumulative_tokens(&self) -> u64 {
        self.iters.iter().map(|i| i.cost.total()).sum()
    }
}

/// Primary-sticky cooldown state machine for the multi-runtime loop.
/// Tracks whether the loop is currently routing iters to the fallback
/// spawner after a primary `RuntimeError`, and how many fallback iters
/// remain before retrying the primary. Pure — no I/O, no spawner calls;
/// the caller threads spawner outcomes in via [`Self::record_outcome`].
///
/// Per `DECISION:loop-fallback-policy`:
/// - Primary `RuntimeError` enters cooldown (`iters_remaining = quantum`).
/// - Fallback `RuntimeError` while in cooldown extends the window.
/// - Fallback successes decrement the window; when it reaches 0 the
///   next iter retries the primary.
/// - Primary `Success` in cooldown with `iters_remaining == 0` exits
///   cooldown.
/// - `GuardTrip` / `BudgetExhausted` do not change state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CooldownState {
    pub in_cooldown: bool,
    pub iters_remaining: u32,
    pub cooldown_quantum: u32,
}

impl CooldownState {
    pub fn new(cooldown_quantum: u32) -> Self {
        Self { in_cooldown: false, iters_remaining: 0, cooldown_quantum: cooldown_quantum.max(1) }
    }

    /// True iff the next iter should route to the fallback spawner.
    pub fn should_use_fallback(&self) -> bool {
        self.in_cooldown && self.iters_remaining > 0
    }

    /// Pick the spawner for the next iter. Returns `primary` whenever
    /// fallback is `None` (the cooldown can engage but has nowhere to
    /// route, so the caller stays on primary).
    pub fn next_spawner<'a>(
        &self,
        primary: &'a dyn RuntimeSpawner,
        fallback: Option<&'a dyn RuntimeSpawner>,
    ) -> &'a dyn RuntimeSpawner {
        match (self.should_use_fallback(), fallback) {
            (true, Some(fb)) => fb,
            _ => primary,
        }
    }

    /// Update state after an iter completes. `on_fallback` is whatever
    /// [`Self::should_use_fallback`] returned for the iter that just
    /// ran — the caller must remember it because the bool can flip
    /// between successive iters.
    pub fn record_outcome(&mut self, on_fallback: bool, class: SpawnFailureClass) {
        match class {
            SpawnFailureClass::RuntimeError(_) => {
                if on_fallback {
                    // Fallback errored — extend the cooldown window.
                    self.iters_remaining = self.cooldown_quantum;
                } else {
                    // Primary errored — enter (or re-enter) cooldown.
                    self.in_cooldown = true;
                    self.iters_remaining = self.cooldown_quantum;
                }
            }
            SpawnFailureClass::Success => {
                if on_fallback && self.in_cooldown {
                    self.iters_remaining = self.iters_remaining.saturating_sub(1);
                } else if !on_fallback && self.in_cooldown && self.iters_remaining == 0 {
                    // Retry-once primary succeeded — leave cooldown.
                    self.in_cooldown = false;
                }
            }
            SpawnFailureClass::GuardTrip | SpawnFailureClass::BudgetExhausted => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{MockSpawner, RuntimeErrorKind, SpawnOutcome};

    fn cfg() -> RunConfig {
        RunConfig::default()
    }

    #[test]
    fn default_config_is_strict_and_until_empty() {
        let c = cfg();
        assert!(c.strict);
        assert!(c.stop_on_ready_empty);
        assert!(!c.interactive);
    }

    #[test]
    fn run_config_default_scope_is_ready() {
        assert_eq!(cfg().scope, Scope::Ready);
    }

    #[test]
    fn cancelled_wins_over_everything() {
        let mut s = StopSignals {
            cancelled: true,
            stop_file_present: true,
            tokens_spent: u64::MAX,
            iters_completed: u32::MAX,
            ..Default::default()
        };
        s.last_iter_runtime_error = true;
        let c = RunConfig { max_iter: Some(1), budget_tokens: Some(1), ..cfg() };
        assert_eq!(s.evaluate(&c), Some(StopReason::Cancelled));
    }

    #[test]
    fn stop_file_beats_budgets_and_max_iter() {
        let s = StopSignals {
            stop_file_present: true,
            tokens_spent: 999,
            iters_completed: 99,
            ..Default::default()
        };
        let c = RunConfig { max_iter: Some(10), budget_tokens: Some(100), ..cfg() };
        assert_eq!(s.evaluate(&c), Some(StopReason::StopFile));
    }

    #[test]
    fn token_budget_triggers_at_cap() {
        let s = StopSignals { tokens_spent: 100, ..Default::default() };
        let c = RunConfig { budget_tokens: Some(100), ..cfg() };
        assert_eq!(s.evaluate(&c), Some(StopReason::BudgetTokens));
    }

    #[test]
    fn wall_budget_triggers_at_deadline() {
        let s = StopSignals { wall_elapsed: Duration::from_secs(60), ..Default::default() };
        let c = RunConfig { budget_wall: Some(Duration::from_secs(60)), ..cfg() };
        assert_eq!(s.evaluate(&c), Some(StopReason::BudgetWall));
    }

    #[test]
    fn max_iter_triggers_when_completed() {
        let s = StopSignals { iters_completed: 5, ready_queue_len: 10, ..Default::default() };
        let c = RunConfig { max_iter: Some(5), ..cfg() };
        assert_eq!(s.evaluate(&c), Some(StopReason::MaxIter));
    }

    #[test]
    fn runtime_error_beats_guard_trip_and_ready_empty() {
        let s = StopSignals {
            last_iter_runtime_error: true,
            last_iter_guard_trip: true,
            ready_queue_len: 0,
            ..Default::default()
        };
        assert_eq!(s.evaluate(&cfg()), Some(StopReason::RuntimeError));
    }

    #[test]
    fn guard_trip_beats_ready_empty() {
        let s =
            StopSignals { last_iter_guard_trip: true, ready_queue_len: 0, ..Default::default() };
        assert_eq!(s.evaluate(&cfg()), Some(StopReason::GuardTrip));
    }

    #[test]
    fn ready_empty_only_when_configured() {
        let s = StopSignals { ready_queue_len: 0, ..Default::default() };
        let no_stop = RunConfig { stop_on_ready_empty: false, ..cfg() };
        assert_eq!(s.evaluate(&no_stop), None);
        assert_eq!(s.evaluate(&cfg()), Some(StopReason::ReadyEmpty));
    }

    #[test]
    fn keep_running_when_no_signals() {
        let s = StopSignals { ready_queue_len: 3, ..Default::default() };
        assert_eq!(s.evaluate(&cfg()), None);
    }

    #[test]
    fn budget_below_cap_does_not_trigger() {
        let s = StopSignals { tokens_spent: 99, ready_queue_len: 1, ..Default::default() };
        let c = RunConfig { budget_tokens: Some(100), ..cfg() };
        assert_eq!(s.evaluate(&c), None);
    }

    #[test]
    fn next_iter_number_starts_at_one() {
        let r = Run::new("loop-x", "2026-05-26T00:00:00Z", cfg());
        assert_eq!(r.next_iter_number(), 1);
    }

    #[test]
    fn next_iter_number_increments_with_recorded_iters() {
        let mut r = Run::new("loop-x", "2026-05-26T00:00:00Z", cfg());
        r.iters.push(Iter::new(1, "2026-05-26T00:00:01Z"));
        r.iters.push(Iter::new(2, "2026-05-26T00:00:02Z"));
        assert_eq!(r.next_iter_number(), 3);
    }

    #[test]
    fn cumulative_tokens_sums_iter_cost() {
        let mut r = Run::new("loop-x", "2026-05-26T00:00:00Z", cfg());
        let mut i1 = Iter::new(1, "t");
        i1.cost = TokenSpend { input: 100, output: 50, cache_read: 200, cache_create: 10 };
        let mut i2 = Iter::new(2, "t");
        i2.cost = TokenSpend { input: 80, output: 40, cache_read: 150, cache_create: 5 };
        r.iters.push(i1);
        r.iters.push(i2);
        assert_eq!(r.cumulative_tokens(), 100 + 50 + 200 + 10 + 80 + 40 + 150 + 5);
    }

    #[test]
    fn token_spend_total_sums_all_buckets() {
        let s = TokenSpend { input: 1, output: 2, cache_read: 4, cache_create: 8 };
        assert_eq!(s.total(), 15);
    }

    fn mock(class: SpawnFailureClass) -> MockSpawner {
        MockSpawner::new(SpawnOutcome {
            failure_class: class,
            success: matches!(class, SpawnFailureClass::Success),
            ..Default::default()
        })
    }

    #[test]
    fn cooldown_starts_disengaged() {
        let c = CooldownState::new(3);
        assert!(!c.in_cooldown);
        assert_eq!(c.iters_remaining, 0);
        assert_eq!(c.cooldown_quantum, 3);
        assert!(!c.should_use_fallback());

        let primary = mock(SpawnFailureClass::Success);
        let fb = mock(SpawnFailureClass::Success);
        // With nothing in cooldown, next_spawner returns primary even
        // when fallback is supplied.
        let s = c.next_spawner(&primary, Some(&fb));
        assert!(std::ptr::eq(s as *const _ as *const (), &primary as *const _ as *const ()));
    }

    #[test]
    fn runtime_error_engages_cooldown_for_n_iters() {
        let mut c = CooldownState::new(3);
        c.record_outcome(false, SpawnFailureClass::RuntimeError(RuntimeErrorKind::RateLimit));
        assert!(c.in_cooldown);
        assert_eq!(c.iters_remaining, 3);
        assert!(c.should_use_fallback());
    }

    #[test]
    fn success_in_cooldown_returns_to_primary_after_n() {
        let mut c = CooldownState::new(3);
        c.record_outcome(false, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Server));
        // Three fallback successes drain the window.
        for expected in [2u32, 1, 0] {
            assert!(c.should_use_fallback());
            c.record_outcome(true, SpawnFailureClass::Success);
            assert_eq!(c.iters_remaining, expected);
        }
        // Window drained — next iter routes to primary for a retry.
        assert!(!c.should_use_fallback());
        assert!(c.in_cooldown);
        // Primary retry succeeds → exit cooldown.
        c.record_outcome(false, SpawnFailureClass::Success);
        assert!(!c.in_cooldown);
    }

    #[test]
    fn runtime_error_in_cooldown_extends_window() {
        let mut c = CooldownState::new(3);
        c.record_outcome(false, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Auth));
        assert_eq!(c.iters_remaining, 3);
        // One fallback success: 3 → 2.
        c.record_outcome(true, SpawnFailureClass::Success);
        assert_eq!(c.iters_remaining, 2);
        // Fallback errors → window reset back to quantum.
        c.record_outcome(true, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Server));
        assert_eq!(c.iters_remaining, 3);
        assert!(c.in_cooldown);
    }

    #[test]
    fn guard_trip_does_not_engage_cooldown() {
        let mut c = CooldownState::new(3);
        c.record_outcome(false, SpawnFailureClass::GuardTrip);
        assert!(!c.in_cooldown);
        assert_eq!(c.iters_remaining, 0);
        c.record_outcome(false, SpawnFailureClass::BudgetExhausted);
        assert!(!c.in_cooldown);
        // And once engaged, neither outcome decrements the window.
        c.record_outcome(false, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Server));
        assert_eq!(c.iters_remaining, 3);
        c.record_outcome(true, SpawnFailureClass::GuardTrip);
        assert_eq!(c.iters_remaining, 3);
    }

    /// Scripted sequence e2e: primary errors twice, fallback succeeds,
    /// primary retry succeeds. Asserts the chosen-spawner sequence and
    /// the cooldown state at each step — the dispatching contract the
    /// loop driver depends on. Distinct from the per-transition unit
    /// tests above; this exercises the full walk.
    #[test]
    fn cooldown_sequence_primary_fails_twice_then_recovers() {
        let primary = mock(SpawnFailureClass::Success);
        let fb = mock(SpawnFailureClass::Success);

        let primary_id = &primary as *const _ as *const ();
        let fb_id = &fb as *const _ as *const ();
        let id_of = |s: &dyn RuntimeSpawner| s as *const _ as *const ();

        let mut c = CooldownState::new(2);
        let mut chosen: Vec<&'static str> = Vec::new();

        // Iter 1: primary errors → enter cooldown.
        let s = c.next_spawner(&primary, Some(&fb));
        chosen.push(if std::ptr::eq(id_of(s), primary_id) { "primary" } else { "fallback" });
        c.record_outcome(false, SpawnFailureClass::RuntimeError(RuntimeErrorKind::RateLimit));
        assert!(c.in_cooldown);
        assert_eq!(c.iters_remaining, 2);

        // Iter 2: fallback success, drain to 1.
        let s = c.next_spawner(&primary, Some(&fb));
        chosen.push(if std::ptr::eq(id_of(s), primary_id) { "primary" } else { "fallback" });
        c.record_outcome(true, SpawnFailureClass::Success);
        assert_eq!(c.iters_remaining, 1);

        // Iter 3: primary errors again on a retry attempt? No — still on
        // fallback because iters_remaining > 0. Fallback errors this
        // time → window resets to quantum (2).
        let s = c.next_spawner(&primary, Some(&fb));
        chosen.push(if std::ptr::eq(id_of(s), primary_id) { "primary" } else { "fallback" });
        c.record_outcome(true, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Server));
        assert_eq!(c.iters_remaining, 2);

        // Iter 4 + 5: two fallback successes drain to 0.
        for _ in 0..2 {
            let s = c.next_spawner(&primary, Some(&fb));
            chosen.push(if std::ptr::eq(id_of(s), primary_id) { "primary" } else { "fallback" });
            c.record_outcome(true, SpawnFailureClass::Success);
        }
        assert_eq!(c.iters_remaining, 0);
        assert!(c.in_cooldown);

        // Iter 6: window drained → route to primary for retry. Primary
        // succeeds → exit cooldown.
        let s = c.next_spawner(&primary, Some(&fb));
        chosen.push(if std::ptr::eq(id_of(s), primary_id) { "primary" } else { "fallback" });
        c.record_outcome(false, SpawnFailureClass::Success);
        assert!(!c.in_cooldown);

        assert_eq!(
            chosen,
            vec!["primary", "fallback", "fallback", "fallback", "fallback", "primary"],
            "chosen sequence diverged from cooldown contract"
        );
        // Silence the unused-binding lint in case the fb id doesn't get
        // checked above (it will, but be explicit).
        let _ = fb_id;
    }

    #[test]
    fn no_fallback_configured_never_switches() {
        let mut c = CooldownState::new(3);
        let primary = mock(SpawnFailureClass::Success);
        // Drive primary into cooldown.
        c.record_outcome(false, SpawnFailureClass::RuntimeError(RuntimeErrorKind::Server));
        assert!(c.should_use_fallback());
        // With fallback=None, next_spawner still returns primary —
        // there is nowhere else to route.
        let s = c.next_spawner(&primary, None);
        assert!(std::ptr::eq(s as *const _ as *const (), &primary as *const _ as *const ()));
    }
}
