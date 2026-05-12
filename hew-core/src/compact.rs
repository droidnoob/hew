//! Memory-compaction primitives shared by `hew-compact` skill and CLI.
//!
//! The methodology accumulates memories — `CONVENTION:`, `RESEARCH:`,
//! factual snippets, `DECISION:` records — and over time some prefixes
//! grow noisy. `hew-compact` lets the user reduce a prefix from N
//! entries to 1–2 canonical entries per logical sub-cluster.
//!
//! This module is the pure-data layer (the skill body does the
//! clustering in-context with an LLM; the CLI consumes the resulting
//! [`CompactPlan`]). [`apply`] is the only mutating surface, and it
//! enforces the four safety invariants captured in the corresponding
//! `DECISION:compact-*` memories:
//!
//! - **adds-before-forgets** ordering, so a crash mid-apply leaves
//!   *more* memory, not less (`DECISION:compact-safety`).
//! - **provenance-suffix** auto-appended to every replacement body:
//!   `[compacted-from: k1, k2, ...]` (`DECISION:compact-provenance`).
//! - **drift-guard**: any source memory already carrying the
//!   `[compacted-from:` suffix is skipped unless `allow_recompact = true`
//!   (`DECISION:compact-drift-guard`).
//! - **exempt allowlist**: hardcoded `STATUS:scan/convention/plan/decompose`
//!   plus user-configured `compact.exempt` keys never touched
//!   (`DECISION:compact-safety`).
//!
//! On success [`apply`] also writes a `STATUS:compact:<prefix>:<iso-ts>`
//! marker so future runs can show "last compaction" context.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::bd::BdClient;
use crate::config::Config;
use crate::error::Result;
use crate::tasks;

const PROVENANCE_TAG: &str = "[compacted-from:";

/// Hardcoded exempt prefixes (in addition to `cfg.compact.exempt`).
/// These mark phase completion and the loss of any one would corrupt
/// the `hew prime` routing state.
const HARDCODED_EXEMPT_PREFIXES: &[&str] =
    &["STATUS:scan", "STATUS:convention", "STATUS:plan", "STATUS:decompose"];

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum Granularity {
    /// Strict prompt → fewer, broader clusters. Default; closer to the
    /// "1 or 2 per type" target.
    #[default]
    Broad,
    /// Relaxed prompt → more, finer-grained clusters. Preserves nuance.
    Fine,
}

/// One topic-cluster within a [`CompactPlan`]. The clustering itself
/// happens in the skill body; this struct just records the result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Cluster {
    /// Short human-readable cluster topic (e.g. "rust-style", "errors").
    pub topic: String,
    /// Memory keys to be forgotten on apply.
    pub source_keys: Vec<String>,
    /// Replacement memory bodies. Each becomes one new memory via
    /// `bd remember`. Typically 1–2 entries (per the
    /// `DECISION:compact-granularity` target).
    pub replacement_bodies: Vec<String>,
}

/// Full plan handed to [`apply`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct CompactPlan {
    /// The prefix being compacted (e.g. `CONVENTION`, `RESEARCH`).
    pub prefix: String,
    /// Caller-requested cluster count (informational; [`apply`] does
    /// not enforce it). 0 = "use default", but [`validate`] rejects 0
    /// when [`Cluster`]s are present.
    pub target_clusters: u32,
    pub granularity: Granularity,
    /// `true` lets the apply walk through memories already carrying
    /// the provenance suffix. Default `false` per
    /// `DECISION:compact-drift-guard`.
    #[serde(default)]
    pub allow_recompact: bool,
    pub clusters: Vec<Cluster>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ApplyReport {
    /// New memory bodies written via `bd remember` (in write order).
    pub added: Vec<String>,
    /// Source keys successfully forgotten.
    pub forgotten: Vec<String>,
    /// Source keys skipped because they matched the exempt allowlist.
    pub exempt_skipped: Vec<String>,
    /// Source keys skipped because they carried the `[compacted-from:`
    /// suffix and `allow_recompact` was `false`.
    pub drift_guard_skipped: Vec<String>,
    /// The `STATUS:compact:<prefix>:<ts>` marker written on success.
    pub marker_written: Option<String>,
}

/// Validate a plan before it touches the bd layer. Returns the list
/// of structural errors (empty `Vec` = ok).
pub fn validate(plan: &CompactPlan) -> Vec<String> {
    let mut errs = Vec::new();
    if plan.prefix.trim().is_empty() {
        errs.push("prefix is empty".to_string());
    }
    if plan.clusters.is_empty() {
        errs.push("plan has no clusters".to_string());
    }
    if plan.target_clusters == 0 && !plan.clusters.is_empty() {
        errs.push(
            "target_clusters=0 with non-empty clusters list — set target_clusters to plan.clusters.len() or higher"
                .to_string(),
        );
    }
    for (i, c) in plan.clusters.iter().enumerate() {
        if c.replacement_bodies.is_empty() {
            errs.push(format!("cluster[{i}] `{}` has no replacement_bodies", c.topic));
        }
        if c.source_keys.is_empty() {
            errs.push(format!("cluster[{i}] `{}` has no source_keys", c.topic));
        }
    }
    errs
}

/// Recommended cluster count for `n` source memories.
///
/// Formula: `ceil(sqrt(n))` capped at 6 (per `DECISION:compact-k-default`).
/// `n == 0` returns 0; `n == 1` returns 1.
pub fn default_k(n: usize) -> u32 {
    if n == 0 {
        return 0;
    }
    let f = (n as f64).sqrt().ceil() as u32;
    f.clamp(1, 6)
}

/// Execute the plan. Writes additions first, then forgets sources,
/// then writes the status marker. Per `DECISION:compact-safety`, an
/// error after the additions land leaves the bd store with extra
/// memory rather than missing memory — which is recoverable.
pub fn apply(
    bd: &dyn BdClient,
    plan: &CompactPlan,
    cfg: &Config,
    iso_ts: &str,
) -> Result<ApplyReport> {
    let mut report = ApplyReport::default();
    let memories = bd.memories()?;
    let exempt_set: BTreeSet<&str> = cfg.compact.exempt.iter().map(|s| s.as_str()).collect();

    // Phase 1: write all replacement bodies with the provenance suffix
    // appended. Done before any forgets so a mid-apply crash leaves
    // strictly more memory.
    for cluster in &plan.clusters {
        let provenance = build_provenance(&cluster.source_keys);
        for body in &cluster.replacement_bodies {
            let full = format!("{body}\n\n{provenance}");
            bd.remember(&full)?;
            report.added.push(full);
        }
    }

    // Phase 2: forget source keys, honoring exempt + drift-guard.
    for cluster in &plan.clusters {
        for key in &cluster.source_keys {
            if is_exempt(key, &exempt_set) {
                report.exempt_skipped.push(key.clone());
                continue;
            }
            if !plan.allow_recompact && is_already_compacted(key, &memories) {
                report.drift_guard_skipped.push(key.clone());
                continue;
            }
            tasks::forget(bd, key)?;
            report.forgotten.push(key.clone());
        }
    }

    // Phase 3: status marker. Skipped if every cluster was a no-op
    // (nothing actually compacted).
    if !report.forgotten.is_empty() {
        let marker = format!("STATUS:compact:{}:{}", plan.prefix, iso_ts);
        bd.remember(&marker)?;
        report.marker_written = Some(marker);
    }

    Ok(report)
}

fn build_provenance(source_keys: &[String]) -> String {
    let list = source_keys.join(", ");
    format!("{PROVENANCE_TAG} {list}]")
}

fn is_exempt(key: &str, user_exempt: &BTreeSet<&str>) -> bool {
    if user_exempt.contains(key) {
        return true;
    }
    HARDCODED_EXEMPT_PREFIXES.iter().any(|p| key == *p || key.starts_with(&format!("{p}:")))
}

fn is_already_compacted(key: &str, memories: &std::collections::BTreeMap<String, String>) -> bool {
    memories.get(key).is_some_and(|v| v.contains(PROVENANCE_TAG))
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bd::{BdOutput, BdVersion, ReadyTask, StatsSummary};
    use crate::error::HewError;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::ffi::OsStr;

    /// Records the bd surface calls in call order so we can assert
    /// adds-before-forgets.
    #[derive(Debug, Default)]
    struct MockBd {
        memories: BTreeMap<String, String>,
        calls: RefCell<Vec<String>>, // "remember:<body>" or "forget:<key>"
        forget_fails: BTreeMap<String, String>,
    }

    impl MockBd {
        fn with_memories(pairs: &[(&str, &str)]) -> Self {
            let memories: BTreeMap<String, String> =
                pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
            Self { memories, ..Default::default() }
        }

        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl BdClient for MockBd {
        fn version(&self) -> Result<BdVersion> {
            Ok(BdVersion { raw: "test".into(), semver: "0.0.0".into() })
        }
        fn ready(&self) -> Result<Vec<ReadyTask>> {
            Ok(Vec::new())
        }
        fn stats(&self) -> Result<StatsSummary> {
            Ok(StatsSummary::default())
        }
        fn prime_raw(&self) -> Result<String> {
            Ok(String::new())
        }
        fn memories(&self) -> Result<BTreeMap<String, String>> {
            Ok(self.memories.clone())
        }
        fn remember(&self, text: &str) -> Result<()> {
            self.calls.borrow_mut().push(format!("remember:{text}"));
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
            // tasks::forget calls run_raw with ["forget", key]
            if args.len() == 2 && args[0] == OsStr::new("forget") {
                let key = args[1].to_string_lossy().to_string();
                if let Some(msg) = self.forget_fails.get(&key) {
                    return Err(HewError::BdNonZero { code: 1, stderr: msg.clone() });
                }
                self.calls.borrow_mut().push(format!("forget:{key}"));
            }
            Ok(BdOutput { stdout: String::new(), stderr: String::new() })
        }
    }

    fn plan_single_cluster() -> CompactPlan {
        CompactPlan {
            prefix: "CONVENTION".into(),
            target_clusters: 1,
            granularity: Granularity::Broad,
            allow_recompact: false,
            clusters: vec![Cluster {
                topic: "rust-style".into(),
                source_keys: vec!["k1".into(), "k2".into(), "k3".into()],
                replacement_bodies: vec![
                    "CONVENTION:rust-style — formatted by rustfmt; clippy clean.".into(),
                ],
            }],
        }
    }

    // ──────── default_k ────────

    #[test]
    fn default_k_examples() {
        assert_eq!(default_k(0), 0);
        assert_eq!(default_k(1), 1);
        assert_eq!(default_k(4), 2);
        assert_eq!(default_k(9), 3);
        assert_eq!(default_k(25), 5);
        assert_eq!(default_k(36), 6);
        assert_eq!(default_k(49), 6, "cap kicks in at sqrt(49)=7 → 6");
        assert_eq!(default_k(100), 6);
        assert_eq!(default_k(10_000), 6);
    }

    // ──────── validate ────────

    #[test]
    fn validate_empty_clusters_errors() {
        let mut p = plan_single_cluster();
        p.clusters.clear();
        let errs = validate(&p);
        assert!(errs.iter().any(|e| e.contains("no clusters")), "got: {errs:?}");
    }

    #[test]
    fn validate_target_zero_with_clusters_errors() {
        let mut p = plan_single_cluster();
        p.target_clusters = 0;
        let errs = validate(&p);
        assert!(errs.iter().any(|e| e.contains("target_clusters=0")), "got: {errs:?}");
    }

    #[test]
    fn validate_empty_replacement_bodies_errors() {
        let mut p = plan_single_cluster();
        p.clusters[0].replacement_bodies.clear();
        let errs = validate(&p);
        assert!(errs.iter().any(|e| e.contains("no replacement_bodies")), "got: {errs:?}");
    }

    #[test]
    fn validate_clean_plan_returns_no_errors() {
        let errs = validate(&plan_single_cluster());
        assert!(errs.is_empty(), "got: {errs:?}");
    }

    // ──────── apply ordering ────────

    #[test]
    fn apply_writes_adds_before_forgets() {
        let bd = MockBd::with_memories(&[
            ("k1", "CONVENTION:foo — bar"),
            ("k2", "CONVENTION:baz — qux"),
            ("k3", "CONVENTION:quux — quuux"),
        ]);
        let cfg = Config::default();
        let report = apply(&bd, &plan_single_cluster(), &cfg, "2026-05-12T20:00:00Z").unwrap();

        let calls = bd.calls();
        // The remember for the cluster body must come BEFORE any forget.
        let first_forget =
            calls.iter().position(|c| c.starts_with("forget:")).expect("at least one forget");
        let last_remember_before_forget =
            calls.iter().take(first_forget).filter(|c| c.starts_with("remember:")).count();
        assert!(
            last_remember_before_forget >= 1,
            "expected ≥1 remember before first forget; calls: {calls:?}"
        );
        assert_eq!(report.forgotten.len(), 3);
        assert_eq!(report.added.len(), 1);
    }

    #[test]
    fn apply_appends_provenance_suffix() {
        let bd = MockBd::with_memories(&[("k1", "x"), ("k2", "y"), ("k3", "z")]);
        let cfg = Config::default();
        let report = apply(&bd, &plan_single_cluster(), &cfg, "T").unwrap();
        let body = &report.added[0];
        assert!(body.contains("[compacted-from: k1, k2, k3]"), "body: {body}");
        // Original body still present:
        assert!(body.contains("CONVENTION:rust-style"));
    }

    #[test]
    fn apply_writes_status_marker_with_iso_ts() {
        let bd = MockBd::with_memories(&[("k1", "a"), ("k2", "b"), ("k3", "c")]);
        let cfg = Config::default();
        let report = apply(&bd, &plan_single_cluster(), &cfg, "2026-05-12T20:00:00Z").unwrap();
        let marker = report.marker_written.expect("marker written");
        assert_eq!(marker, "STATUS:compact:CONVENTION:2026-05-12T20:00:00Z");
        // ...and that exact marker was sent to bd.remember:
        assert!(bd.calls().contains(&format!("remember:{marker}")));
    }

    #[test]
    fn apply_skips_status_marker_when_nothing_forgotten() {
        let bd = MockBd::with_memories(&[("STATUS:scan", "done")]);
        let cfg = Config::default();
        let plan = CompactPlan {
            prefix: "STATUS".into(),
            target_clusters: 1,
            granularity: Granularity::Broad,
            allow_recompact: false,
            clusters: vec![Cluster {
                topic: "phases".into(),
                source_keys: vec!["STATUS:scan".into()], // exempt — skipped
                replacement_bodies: vec!["STATUS:phases — all done".into()],
            }],
        };
        let report = apply(&bd, &plan, &cfg, "T").unwrap();
        assert!(report.forgotten.is_empty());
        assert!(report.marker_written.is_none(), "no forgets → no marker");
    }

    // ──────── apply exempt + drift-guard ────────

    #[test]
    fn apply_skips_hardcoded_exempt_prefixes() {
        let bd = MockBd::with_memories(&[
            ("STATUS:scan", "ts"),
            ("STATUS:convention", "ts"),
            ("STATUS:plan", "ts"),
            ("STATUS:decompose", "ts"),
            ("STATUS:other-thing", "ts"),
        ]);
        let cfg = Config::default();
        let plan = CompactPlan {
            prefix: "STATUS".into(),
            target_clusters: 1,
            granularity: Granularity::Broad,
            allow_recompact: false,
            clusters: vec![Cluster {
                topic: "phases".into(),
                source_keys: vec![
                    "STATUS:scan".into(),
                    "STATUS:convention".into(),
                    "STATUS:plan".into(),
                    "STATUS:decompose".into(),
                    "STATUS:other-thing".into(),
                ],
                replacement_bodies: vec!["STATUS:phases — collapsed".into()],
            }],
        };
        let report = apply(&bd, &plan, &cfg, "T").unwrap();
        assert_eq!(report.exempt_skipped.len(), 4);
        assert!(report.exempt_skipped.contains(&"STATUS:scan".to_string()));
        // STATUS:other-thing isn't in the hardcoded list — it should
        // have been forgotten.
        assert_eq!(report.forgotten, vec!["STATUS:other-thing".to_string()]);
    }

    #[test]
    fn apply_skips_user_exempt_keys() {
        let bd = MockBd::with_memories(&[("custom-key", "x"), ("k2", "y")]);
        let mut cfg = Config::default();
        cfg.compact.exempt = vec!["custom-key".into()];
        let plan = CompactPlan {
            prefix: "CONVENTION".into(),
            target_clusters: 1,
            granularity: Granularity::Broad,
            allow_recompact: false,
            clusters: vec![Cluster {
                topic: "t".into(),
                source_keys: vec!["custom-key".into(), "k2".into()],
                replacement_bodies: vec!["CONVENTION:t — merged".into()],
            }],
        };
        let report = apply(&bd, &plan, &cfg, "T").unwrap();
        assert_eq!(report.exempt_skipped, vec!["custom-key"]);
        assert_eq!(report.forgotten, vec!["k2"]);
    }

    #[test]
    fn apply_drift_guards_already_compacted_memories() {
        let bd = MockBd::with_memories(&[
            ("k1", "CONVENTION:foo — bar\n\n[compacted-from: old1, old2]"),
            ("k2", "CONVENTION:fresh — baz"),
        ]);
        let cfg = Config::default();
        let plan = CompactPlan {
            prefix: "CONVENTION".into(),
            target_clusters: 1,
            granularity: Granularity::Broad,
            allow_recompact: false,
            clusters: vec![Cluster {
                topic: "t".into(),
                source_keys: vec!["k1".into(), "k2".into()],
                replacement_bodies: vec!["CONVENTION:t — re-merged".into()],
            }],
        };
        let report = apply(&bd, &plan, &cfg, "T").unwrap();
        assert_eq!(report.drift_guard_skipped, vec!["k1"]);
        assert_eq!(report.forgotten, vec!["k2"]);
    }

    #[test]
    fn apply_allow_recompact_overrides_drift_guard() {
        let bd = MockBd::with_memories(&[("k1", "CONVENTION:foo — bar\n\n[compacted-from: old1]")]);
        let cfg = Config::default();
        let plan = CompactPlan {
            prefix: "CONVENTION".into(),
            target_clusters: 1,
            granularity: Granularity::Broad,
            allow_recompact: true,
            clusters: vec![Cluster {
                topic: "t".into(),
                source_keys: vec!["k1".into()],
                replacement_bodies: vec!["CONVENTION:t — second pass".into()],
            }],
        };
        let report = apply(&bd, &plan, &cfg, "T").unwrap();
        assert!(report.drift_guard_skipped.is_empty());
        assert_eq!(report.forgotten, vec!["k1"]);
    }

    // ──────── serde / schema ────────

    #[test]
    fn granularity_serializes_kebab_case() {
        assert_eq!(serde_json::to_string(&Granularity::Broad).unwrap(), "\"broad\"");
        assert_eq!(serde_json::to_string(&Granularity::Fine).unwrap(), "\"fine\"");
        let back: Granularity = serde_json::from_str("\"fine\"").unwrap();
        assert_eq!(back, Granularity::Fine);
    }

    #[test]
    fn compact_plan_round_trips_through_json() {
        let p = plan_single_cluster();
        let json = serde_json::to_string(&p).unwrap();
        let back: CompactPlan = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
    }

    #[test]
    fn schemas_are_emittable() {
        // schema_for! macro must produce non-empty schemas for every
        // public type; if any derive breaks, this fails to compile or
        // emits an empty schema.
        use schemars::schema_for;
        let s = serde_json::to_string(&schema_for!(CompactPlan)).unwrap();
        assert!(s.contains("CompactPlan"));
        let s = serde_json::to_string(&schema_for!(ApplyReport)).unwrap();
        assert!(s.contains("ApplyReport"));
    }
}
