//! Research-budget tracker and cross-iter cache for `hew loop`.
//!
//! Each iter starts with a fresh [`Budget`] (default 5 web + 3 fetch
//! per the epic). The decision-resolution flow consumes from the budget
//! via [`Budget::try_spend`]; exhaustion forces the resolve step to
//! route straight to `DEFERRED:` instead of doing more research.
//!
//! [`ResearchCache`] dedupes research across iters within one `Run`:
//! a topic that already has a `RESEARCH:<topic>` memory from a prior
//! iter returns a cache hit, so the loop's research compounds run-over-
//! run (the "loop teaches itself" property called out in the epic).

use std::collections::HashMap;

use crate::runner::ResearchSpend;

/// Per-iter research budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub web_remaining: u32,
    pub fetch_remaining: u32,
}

impl Budget {
    /// Build a fresh budget from a [`crate::runner::ResearchBudget`]
    /// config (defaults: 5 web + 3 fetch).
    pub fn new(cfg: crate::runner::ResearchBudget) -> Self {
        Self { web_remaining: cfg.web, fetch_remaining: cfg.fetch }
    }

    /// True if at least one unit of the requested kind remains.
    pub fn can_spend(&self, kind: ResearchKind) -> bool {
        match kind {
            ResearchKind::Web => self.web_remaining > 0,
            ResearchKind::Fetch => self.fetch_remaining > 0,
        }
    }

    /// Attempt to consume one unit. Returns true on success; false if
    /// the budget for that kind is exhausted.
    pub fn try_spend(&mut self, kind: ResearchKind) -> bool {
        match kind {
            ResearchKind::Web => {
                if self.web_remaining == 0 {
                    return false;
                }
                self.web_remaining -= 1;
                true
            }
            ResearchKind::Fetch => {
                if self.fetch_remaining == 0 {
                    return false;
                }
                self.fetch_remaining -= 1;
                true
            }
        }
    }

    /// Convert spend so far into a [`ResearchSpend`] for iter logging.
    /// Pass the original config so we can compute "spent = budget - remaining".
    pub fn spent_against(&self, cfg: crate::runner::ResearchBudget) -> ResearchSpend {
        ResearchSpend {
            web: cfg.web.saturating_sub(self.web_remaining),
            fetch: cfg.fetch.saturating_sub(self.fetch_remaining),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResearchKind {
    Web,
    Fetch,
}

/// Provenance of a cache hit — which iter originally researched the
/// topic, used for traceability in the per-iter log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheHit {
    pub topic: String,
    pub originated_in_iter: u32,
    /// Reference into the memory store: typically the `RESEARCH:<topic>`
    /// memory id created when the original research ran.
    pub memory_id: Option<String>,
}

/// Cross-iter dedupe layer. Keyed by canonical topic string (caller
/// should normalize — lower-case, trim, etc. — before insertion).
#[derive(Clone, Debug, Default)]
pub struct ResearchCache {
    by_topic: HashMap<String, CacheHit>,
}

impl ResearchCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a fresh research result so subsequent iters skip re-research.
    pub fn record(&mut self, topic: impl Into<String>, iter: u32, memory_id: Option<String>) {
        let topic = topic.into();
        self.by_topic
            .insert(topic.clone(), CacheHit { topic, originated_in_iter: iter, memory_id });
    }

    /// Look up a topic. Returns `None` on miss.
    pub fn lookup(&self, topic: &str) -> Option<&CacheHit> {
        self.by_topic.get(topic)
    }

    /// Number of cached topics — useful for log summaries.
    pub fn len(&self) -> usize {
        self.by_topic.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_topic.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::ResearchBudget;

    fn fresh() -> Budget {
        Budget::new(ResearchBudget::default())
    }

    #[test]
    fn default_budget_is_five_plus_three() {
        let b = fresh();
        assert_eq!(b.web_remaining, 5);
        assert_eq!(b.fetch_remaining, 3);
    }

    #[test]
    fn can_spend_reflects_remaining() {
        let mut b = fresh();
        assert!(b.can_spend(ResearchKind::Web));
        for _ in 0..5 {
            assert!(b.try_spend(ResearchKind::Web));
        }
        assert!(!b.can_spend(ResearchKind::Web));
        assert!(b.can_spend(ResearchKind::Fetch));
    }

    #[test]
    fn try_spend_returns_false_when_exhausted() {
        let mut b = Budget { web_remaining: 1, fetch_remaining: 0 };
        assert!(b.try_spend(ResearchKind::Web));
        assert!(!b.try_spend(ResearchKind::Web));
        assert!(!b.try_spend(ResearchKind::Fetch));
    }

    #[test]
    fn spent_against_computes_diff() {
        let cfg = ResearchBudget::default();
        let mut b = Budget::new(cfg);
        b.try_spend(ResearchKind::Web);
        b.try_spend(ResearchKind::Web);
        b.try_spend(ResearchKind::Fetch);
        let spent = b.spent_against(cfg);
        assert_eq!(spent.web, 2);
        assert_eq!(spent.fetch, 1);
    }

    #[test]
    fn cache_miss_then_record_then_hit() {
        let mut c = ResearchCache::new();
        assert!(c.lookup("rust-async").is_none());
        c.record("rust-async", 3, Some("mem-42".into()));
        let hit = c.lookup("rust-async").expect("should hit");
        assert_eq!(hit.originated_in_iter, 3);
        assert_eq!(hit.memory_id.as_deref(), Some("mem-42"));
    }

    #[test]
    fn cache_record_overwrites_prior_entry() {
        let mut c = ResearchCache::new();
        c.record("topic", 1, None);
        c.record("topic", 5, Some("mem-x".into()));
        let hit = c.lookup("topic").unwrap();
        assert_eq!(hit.originated_in_iter, 5);
    }

    #[test]
    fn cache_len_and_is_empty() {
        let mut c = ResearchCache::new();
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);
        c.record("a", 1, None);
        c.record("b", 1, None);
        assert_eq!(c.len(), 2);
        assert!(!c.is_empty());
    }

    #[test]
    fn budget_with_zero_config_is_immediately_exhausted() {
        let mut b = Budget::new(ResearchBudget { web: 0, fetch: 0 });
        assert!(!b.try_spend(ResearchKind::Web));
        assert!(!b.try_spend(ResearchKind::Fetch));
    }
}
