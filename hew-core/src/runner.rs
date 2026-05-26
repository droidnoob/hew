//! Loop runner orchestrator — pure types and precedence logic for
//! `hew loop`. No I/O: callers gather signals (stop-file presence,
//! wall-clock, ready-queue length, runtime exit codes) and feed them in
//! as values, so this module stays trivially testable.
//!
//! See epic hew-gr1 for the full design. The CLI verb is `hew loop`;
//! the internal module is `runner` because `loop` is a Rust keyword.

use std::time::Duration;

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
    /// Research budget per iter (web searches + fetches). Default 5+3.
    pub research_budget: ResearchBudget,
    /// Promote craft.testing + craft.lint warnings to failures.
    pub strict: bool,
    /// Allow ask-files to interrupt the loop for operator input.
    pub interactive: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            max_iter: None,
            stop_on_ready_empty: true,
            budget_tokens: None,
            budget_wall: None,
            research_budget: ResearchBudget::default(),
            strict: true,
            interactive: false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ResearchBudget {
    pub web: u32,
    pub fetch: u32,
}

impl Default for ResearchBudget {
    fn default() -> Self {
        Self { web: 5, fetch: 3 }
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
    pub research_spent: ResearchSpend,
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
            research_spent: ResearchSpend::default(),
            decisions: Vec::new(),
            deferred: Vec::new(),
            stderr_tail: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResearchSpend {
    pub web: u32,
    pub fetch: u32,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> RunConfig {
        RunConfig::default()
    }

    #[test]
    fn default_config_is_strict_and_until_empty() {
        let c = cfg();
        assert!(c.strict);
        assert!(c.stop_on_ready_empty);
        assert!(!c.interactive);
        assert_eq!(c.research_budget.web, 5);
        assert_eq!(c.research_budget.fetch, 3);
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
}
