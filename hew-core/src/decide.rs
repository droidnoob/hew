//! Decision-resolution flow for `hew loop` `--unattended` mode.
//!
//! When the loop hits a topic that needs a choice (library to use,
//! convention to follow, behavior to lock in) it walks four sources in
//! order:
//!
//! 1. **Memory** — `DECISION:` / `CONVENTION:` / `FEEDBACK:` with topic
//!    match. Short-circuits the rest.
//! 2. **Code** — grep the repo for prior-art signal. Short-circuits if
//!    file:line citations come back.
//! 3. **Research** — web search + fetch via the existing
//!    [`crate::research_gate`] budget, emits [`ResearchFinding`]s tagged
//!    `[VERIFIED]` / `[CITED]` / `[ASSUMED]`.
//! 4. **Decide** — `VERIFIED`/`CITED` findings → file `DECISION:<topic>`;
//!    `ASSUMED`-only or contradictions → file `DEFERRED:<topic>` with a
//!    primed brief for operator review.
//!
//! This module is pure: callers implement [`DecisionContext`] to wire in
//! real bd / grep / research, or a mock impl for tests. See epic
//! hew-gr1 for the full rationale.

/// Provenance tag on a research finding. Matches the
/// `[VERIFIED]/[CITED]/[ASSUMED]` markers `hew:research` already writes
/// into `RESEARCH:` memory bodies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// Source verified by direct primary read (official docs, code).
    Verified,
    /// Sourced via secondary citations with URL.
    Cited,
    /// Inferred from context; no external source.
    Assumed,
}

/// A single finding produced by [`DecisionContext::run_research`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResearchFinding {
    /// Stable id (typically the `RESEARCH:` memory key the research
    /// skill just wrote).
    pub id: String,
    pub provenance: Provenance,
    pub body: String,
}

/// A memory hit from [`DecisionContext::memory_lookup`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemoryHit {
    pub id: String,
    pub body: String,
}

/// One line of grep prior art.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeCitation {
    pub file: String,
    pub line: u32,
    pub snippet: String,
}

/// Outcome of [`resolve`]. The caller turns this into bd / memory side
/// effects — this module doesn't write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolution {
    /// Step 1 hit: an existing memory already answers the topic.
    Memory(MemoryHit),
    /// Step 2 hit: code grep returned prior-art citations.
    Code(Vec<CodeCitation>),
    /// Step 4 outcome: research returned `VERIFIED` or `CITED` findings.
    /// Caller should `hew remember --type=decision` the `decision_body`
    /// citing `research_ids`.
    Decided { topic: String, research_ids: Vec<String>, decision_body: String },
    /// Step 4 outcome: research only produced `ASSUMED` findings (or
    /// none). Caller should `hew remember --type=deferred` the `brief`
    /// so the operator can resolve out-of-band.
    Deferred { topic: String, research_ids: Vec<String>, brief: String },
}

/// Injected dependencies for [`resolve`]. Production wires these to bd,
/// the grep helpers, and `hew:research`; tests stub them.
pub trait DecisionContext {
    fn memory_lookup(&mut self, topic: &str) -> Option<MemoryHit>;
    fn code_search(&mut self, topic: &str) -> Vec<CodeCitation>;
    fn run_research(&mut self, topic: &str) -> Vec<ResearchFinding>;
}

/// Resolve `topic` to either a known answer (memory / code) or a
/// research-driven `DECISION` / `DEFERRED` outcome. Steps short-circuit:
/// the first source to produce signal wins.
pub fn resolve(topic: &str, ctx: &mut dyn DecisionContext) -> Resolution {
    if let Some(hit) = ctx.memory_lookup(topic) {
        return Resolution::Memory(hit);
    }
    let citations = ctx.code_search(topic);
    if !citations.is_empty() {
        return Resolution::Code(citations);
    }
    let findings = ctx.run_research(topic);
    classify_research(topic, findings)
}

/// Pure classifier: which bucket does this research bag fall into?
/// Extracted so tests can exercise the rule table without standing up
/// a full [`DecisionContext`].
pub fn classify_research(topic: &str, findings: Vec<ResearchFinding>) -> Resolution {
    let solid: Vec<&ResearchFinding> = findings
        .iter()
        .filter(|f| matches!(f.provenance, Provenance::Verified | Provenance::Cited))
        .collect();

    if !solid.is_empty() && !has_contradiction(&solid) {
        let research_ids = solid.iter().map(|f| f.id.clone()).collect();
        let decision_body = synthesize_decision_body(topic, &solid);
        return Resolution::Decided { topic: topic.to_string(), research_ids, decision_body };
    }

    let research_ids = findings.iter().map(|f| f.id.clone()).collect();
    let brief = synthesize_deferred_brief(topic, &findings);
    Resolution::Deferred { topic: topic.to_string(), research_ids, brief }
}

/// Two solid findings that flat-out disagree should defer, not decide.
/// Heuristic: distinct verified/cited bodies. Conservative — when in
/// doubt, push to the operator.
fn has_contradiction(solid: &[&ResearchFinding]) -> bool {
    if solid.len() < 2 {
        return false;
    }
    let first = solid[0].body.trim();
    solid.iter().skip(1).any(|f| f.body.trim() != first)
}

fn synthesize_decision_body(topic: &str, solid: &[&ResearchFinding]) -> String {
    let ids: Vec<&str> = solid.iter().map(|f| f.id.as_str()).collect();
    let summary = solid[0].body.trim();
    format!("DECISION:{topic} — {summary} (refs: {})", ids.join(", "))
}

fn synthesize_deferred_brief(topic: &str, findings: &[ResearchFinding]) -> String {
    if findings.is_empty() {
        return format!(
            "DEFERRED:{topic} — research returned no findings; operator review needed."
        );
    }
    let mut lines =
        vec![format!("DEFERRED:{topic} — research inconclusive; operator review needed.")];
    for f in findings {
        let tag = match f.provenance {
            Provenance::Verified => "VERIFIED",
            Provenance::Cited => "CITED",
            Provenance::Assumed => "ASSUMED",
        };
        lines.push(format!("- [{tag}] {} ({})", f.body.trim(), f.id));
    }
    lines.join("\n")
}

/// Bd-backed [`DecisionContext`] used by `hew loop --unattended`.
/// Reads memories from the supplied [`BdClient`] and shells out to
/// `git grep` for code citations. Research is not yet integrated — see
/// the `run_research` impl note.
pub struct BdDecisionContext<'a> {
    bd: &'a dyn crate::bd::BdClient,
    project_root: std::path::PathBuf,
}

impl<'a> BdDecisionContext<'a> {
    pub fn new(bd: &'a dyn crate::bd::BdClient, project_root: std::path::PathBuf) -> Self {
        Self { bd, project_root }
    }
}

impl DecisionContext for BdDecisionContext<'_> {
    /// Walk every memory body and return the first whose body carries
    /// a `DECISION:` / `CONVENTION:` / `FEEDBACK:` prefix AND mentions
    /// the topic substring (case-insensitive). Matches the
    /// "memory-first" precedence the module-level docs describe.
    fn memory_lookup(&mut self, topic: &str) -> Option<MemoryHit> {
        let mems = self.bd.memories().ok()?;
        let needle = topic.to_lowercase();
        for (id, body) in mems {
            let trimmed = body.trim_start();
            let prefixed = trimmed.starts_with("DECISION:")
                || trimmed.starts_with("CONVENTION:")
                || trimmed.starts_with("FEEDBACK:");
            if prefixed && body.to_lowercase().contains(&needle) {
                return Some(MemoryHit { id, body });
            }
        }
        None
    }

    /// `git grep -n -i -F <topic>` on the project root. Empty (or
    /// non-zero exit) on no match. Citations capped at 20 to keep the
    /// `Resolution::Code` body bounded.
    fn code_search(&mut self, topic: &str) -> Vec<CodeCitation> {
        let out = std::process::Command::new("git")
            .args(["grep", "-n", "-i", "-F", topic])
            .current_dir(&self.project_root)
            .output();
        let Ok(o) = out else { return Vec::new() };
        if !o.status.success() {
            return Vec::new();
        }
        String::from_utf8_lossy(&o.stdout)
            .lines()
            .take(20)
            .filter_map(|line| {
                let mut parts = line.splitn(3, ':');
                let file = parts.next()?.to_string();
                let line_no: u32 = parts.next()?.parse().ok()?;
                let snippet = parts.next()?.trim().to_string();
                Some(CodeCitation { file, line: line_no, snippet })
            })
            .collect()
    }

    /// Research integration is deferred — the in-process `hew:research`
    /// skill isn't yet callable from the loop crate. Returning empty
    /// means topics with no memory or code hit fall through to
    /// `Resolution::Deferred`, which is the safe outcome (the operator
    /// reviews the existing `DEFERRED:` memory the agent already
    /// filed).
    fn run_research(&mut self, _topic: &str) -> Vec<ResearchFinding> {
        Vec::new()
    }
}

/// Extract the topic from a `DEFERRED:<topic> — ...` memory body.
/// Returns `None` when the body doesn't start with the expected prefix
/// or the topic segment is empty.
pub fn extract_deferred_topic(body: &str) -> Option<String> {
    let rest = body.trim_start().strip_prefix("DEFERRED:")?;
    let head = rest.split('—').next()?.split('\n').next()?.trim();
    if head.is_empty() { None } else { Some(head.to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MockCtx {
        memory: Option<MemoryHit>,
        code: Vec<CodeCitation>,
        research: Vec<ResearchFinding>,
        memory_calls: u32,
        code_calls: u32,
        research_calls: u32,
    }

    impl DecisionContext for MockCtx {
        fn memory_lookup(&mut self, _topic: &str) -> Option<MemoryHit> {
            self.memory_calls += 1;
            self.memory.clone()
        }
        fn code_search(&mut self, _topic: &str) -> Vec<CodeCitation> {
            self.code_calls += 1;
            self.code.clone()
        }
        fn run_research(&mut self, _topic: &str) -> Vec<ResearchFinding> {
            self.research_calls += 1;
            self.research.clone()
        }
    }

    fn finding(id: &str, prov: Provenance, body: &str) -> ResearchFinding {
        ResearchFinding { id: id.into(), provenance: prov, body: body.into() }
    }

    #[test]
    fn memory_hit_short_circuits_before_code_or_research() {
        let mut ctx = MockCtx {
            memory: Some(MemoryHit { id: "m1".into(), body: "DECISION:x — use foo".into() }),
            code: vec![CodeCitation { file: "src/x.rs".into(), line: 1, snippet: "...".into() }],
            research: vec![finding("r1", Provenance::Verified, "use bar")],
            ..Default::default()
        };
        let r = resolve("x", &mut ctx);
        assert!(matches!(r, Resolution::Memory(_)));
        assert_eq!(ctx.memory_calls, 1);
        assert_eq!(ctx.code_calls, 0);
        assert_eq!(ctx.research_calls, 0);
    }

    #[test]
    fn code_hit_short_circuits_research_when_memory_misses() {
        let mut ctx = MockCtx {
            code: vec![CodeCitation {
                file: "src/x.rs".into(),
                line: 12,
                snippet: "fn x()".into(),
            }],
            research: vec![finding("r1", Provenance::Verified, "use bar")],
            ..Default::default()
        };
        let r = resolve("x", &mut ctx);
        match r {
            Resolution::Code(c) => assert_eq!(c.len(), 1),
            other => panic!("expected Code, got {other:?}"),
        }
        assert_eq!(ctx.research_calls, 0);
    }

    #[test]
    fn verified_research_produces_decision() {
        let mut ctx = MockCtx {
            research: vec![finding("r1", Provenance::Verified, "use clap-derive")],
            ..Default::default()
        };
        let r = resolve("arg-parser", &mut ctx);
        match r {
            Resolution::Decided { topic, research_ids, decision_body } => {
                assert_eq!(topic, "arg-parser");
                assert_eq!(research_ids, vec!["r1"]);
                assert!(decision_body.contains("clap-derive"));
                assert!(decision_body.contains("r1"));
            }
            other => panic!("expected Decided, got {other:?}"),
        }
    }

    #[test]
    fn cited_research_produces_decision() {
        let mut ctx = MockCtx {
            research: vec![finding("r2", Provenance::Cited, "use serde_json")],
            ..Default::default()
        };
        assert!(matches!(resolve("serial", &mut ctx), Resolution::Decided { .. }));
    }

    #[test]
    fn assumed_only_research_produces_deferred() {
        let mut ctx = MockCtx {
            research: vec![
                finding("r1", Provenance::Assumed, "maybe X"),
                finding("r2", Provenance::Assumed, "maybe Y"),
            ],
            ..Default::default()
        };
        match resolve("topic", &mut ctx) {
            Resolution::Deferred { topic, research_ids, brief } => {
                assert_eq!(topic, "topic");
                assert_eq!(research_ids, vec!["r1", "r2"]);
                assert!(brief.contains("ASSUMED"));
                assert!(brief.contains("maybe X"));
            }
            other => panic!("expected Deferred, got {other:?}"),
        }
    }

    #[test]
    fn contradicting_solid_findings_produce_deferred() {
        let mut ctx = MockCtx {
            research: vec![
                finding("r1", Provenance::Verified, "use foo"),
                finding("r2", Provenance::Cited, "use bar"),
            ],
            ..Default::default()
        };
        let r = resolve("x", &mut ctx);
        match r {
            Resolution::Deferred { research_ids, .. } => {
                // Brief covers both findings.
                assert_eq!(research_ids.len(), 2);
            }
            other => panic!("expected Deferred on contradiction, got {other:?}"),
        }
    }

    #[test]
    fn empty_research_produces_deferred_with_default_brief() {
        let mut ctx = MockCtx::default();
        match resolve("topic", &mut ctx) {
            Resolution::Deferred { research_ids, brief, .. } => {
                assert!(research_ids.is_empty());
                assert!(brief.contains("no findings"));
            }
            other => panic!("expected Deferred, got {other:?}"),
        }
    }

    #[test]
    fn single_solid_finding_is_not_a_contradiction() {
        let f =
            ResearchFinding { id: "r1".into(), provenance: Provenance::Verified, body: "x".into() };
        let solid = vec![&f];
        assert!(!has_contradiction(&solid));
    }

    #[test]
    fn matching_solid_findings_are_not_contradictions() {
        let f1 =
            ResearchFinding { id: "r1".into(), provenance: Provenance::Verified, body: "x".into() };
        let f2 =
            ResearchFinding { id: "r2".into(), provenance: Provenance::Cited, body: "x".into() };
        let solid = vec![&f1, &f2];
        assert!(!has_contradiction(&solid));
        // Verifies the "both agree on x" path produces a Decided, not Deferred.
        match classify_research("t", vec![f1, f2]) {
            Resolution::Decided { research_ids, .. } => assert_eq!(research_ids.len(), 2),
            other => panic!("expected Decided when sources agree, got {other:?}"),
        }
    }

    #[test]
    fn extract_deferred_topic_strips_prefix_and_em_dash() {
        assert_eq!(
            extract_deferred_topic("DEFERRED:auth-strategy — research inconclusive"),
            Some("auth-strategy".to_string()),
        );
        assert_eq!(
            extract_deferred_topic("  DEFERRED:logging  —  needs picking  "),
            Some("logging".to_string()),
        );
        // No prefix → None.
        assert_eq!(extract_deferred_topic("DECISION:x — already decided"), None);
        // Body without em-dash falls back to first line.
        assert_eq!(
            extract_deferred_topic("DEFERRED:single-line-topic"),
            Some("single-line-topic".to_string()),
        );
    }

    #[test]
    fn bd_decision_context_memory_lookup_matches_decision_prefix() {
        use crate::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
        use crate::error::Result;
        use std::collections::BTreeMap;
        use std::ffi::OsStr;

        #[derive(Debug)]
        struct StubBd {
            mems: BTreeMap<String, String>,
        }
        impl BdClient for StubBd {
            fn version(&self) -> Result<BdVersion> {
                Ok(BdVersion { raw: "1.0".into(), semver: "1.0".into() })
            }
            fn ready(&self) -> Result<Vec<ReadyTask>> {
                Ok(vec![])
            }
            fn stats(&self) -> Result<StatsSummary> {
                Ok(StatsSummary::default())
            }
            fn prime_raw(&self) -> Result<String> {
                Ok(String::new())
            }
            fn memories(&self) -> Result<BTreeMap<String, String>> {
                Ok(self.mems.clone())
            }
            fn remember(&self, _: &str) -> Result<()> {
                Ok(())
            }
            fn run_raw(&self, _: &[&OsStr]) -> Result<BdOutput> {
                Ok(BdOutput { stdout: String::new(), stderr: String::new() })
            }
        }

        let mut mems = BTreeMap::new();
        mems.insert("m1".to_string(), "DECISION:errors-as-types — wrap with HewError".to_string());
        mems.insert("m2".to_string(), "factual: random note about errors".to_string());
        let bd = StubBd { mems };

        let mut ctx = BdDecisionContext::new(&bd, std::path::PathBuf::from("/tmp"));
        let hit = ctx.memory_lookup("errors-as-types").expect("hit");
        assert_eq!(hit.id, "m1");
        assert!(hit.body.contains("HewError"));

        // A topic with no matching DECISION/CONVENTION/FEEDBACK prefix
        // returns None even when the substring matches a factual memory.
        assert!(ctx.memory_lookup("random note").is_none());
    }

    #[test]
    fn bd_decision_context_run_research_is_empty_stub() {
        use crate::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
        use crate::error::Result;
        use std::collections::BTreeMap;
        use std::ffi::OsStr;

        #[derive(Debug)]
        struct EmptyBd;
        impl BdClient for EmptyBd {
            fn version(&self) -> Result<BdVersion> {
                Ok(BdVersion { raw: "1.0".into(), semver: "1.0".into() })
            }
            fn ready(&self) -> Result<Vec<ReadyTask>> {
                Ok(vec![])
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
            fn remember(&self, _: &str) -> Result<()> {
                Ok(())
            }
            fn run_raw(&self, _: &[&OsStr]) -> Result<BdOutput> {
                Ok(BdOutput { stdout: String::new(), stderr: String::new() })
            }
        }

        let bd = EmptyBd;
        let mut ctx = BdDecisionContext::new(&bd, std::path::PathBuf::from("/tmp"));
        assert!(ctx.run_research("anything").is_empty());
    }
}
