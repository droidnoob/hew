//! `hew prime <skill>` — assemble agent context JSON.
//!
//! The output shape is the contract between the binary and any agent
//! that consumes it. Adding fields is backwards-compatible; renaming
//! or removing is not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bd::{BdClient, ReadyTask, StatsSummary};
use crate::error::Result;
use crate::skills::{self, Skill};

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PrimeOutput {
    pub schema_version: u32,
    pub skill: String,
    pub project: ProjectInfo,
    pub status: StatusMap,
    pub prerequisites: Prerequisites,
    pub tasks: TaskInfo,
    pub memories: MemoryBuckets,
    pub skill_instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<UpdateAvailable>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateAvailable {
    pub current: String,
    pub latest: String,
    pub message: String,
}

/// Skill-agnostic prime output used by SessionStart hooks to restore
/// agent context after `/clear` or a new session.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResumeOutput {
    pub schema_version: u32,
    pub project: ProjectInfo,
    pub status: StatusMap,
    pub tasks: TaskInfo,
    pub memories: MemoryBuckets,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_checkpoint: Option<Checkpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<UpdateAvailable>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Checkpoint {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct ProjectInfo {
    pub beads_initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bd_version: Option<String>,
}

pub type StatusMap = BTreeMap<String, StatusEntry>;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StatusEntry {
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Prerequisites {
    pub met: bool,
    pub missing: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskInfo {
    pub total: u64,
    pub done: u64,
    pub in_progress: u64,
    pub ready: u64,
    pub blocked: u64,
    pub ready_list: Vec<ReadyTask>,
}

#[derive(Debug, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct MemoryBuckets {
    pub conventions: Vec<String>,
    pub boundaries: Vec<String>,
    pub audit: Vec<String>,
    pub security: Vec<String>,
    pub migration: Vec<String>,
    pub dep: Vec<String>,
    pub factual: Vec<String>,
}

/// Categorize a raw memory map (key -> value) into prefix buckets +
/// a STATUS map parsed out of `STATUS:<phase>:complete — <timestamp>`.
pub fn categorize(memories: &BTreeMap<String, String>) -> (MemoryBuckets, StatusMap) {
    let mut buckets = MemoryBuckets::default();
    let mut status: StatusMap = BTreeMap::new();

    for value in memories.values() {
        let trimmed = value.trim_start();
        if let Some(rest) = trimmed.strip_prefix("STATUS:") {
            if let Some((phase, payload)) = rest.split_once(':') {
                let complete = payload.trim_start().starts_with("complete");
                let timestamp = payload.split('—').nth(1).map(|s| s.trim().to_string());
                status.insert(phase.trim().to_string(), StatusEntry { complete, timestamp });
            }
            continue;
        }
        if trimmed.starts_with("CONVENTION:") {
            buckets.conventions.push(value.clone());
        } else if trimmed.starts_with("BOUNDARY:") {
            buckets.boundaries.push(value.clone());
        } else if trimmed.starts_with("AUDIT:") {
            buckets.audit.push(value.clone());
        } else if trimmed.starts_with("SECURITY:") {
            buckets.security.push(value.clone());
        } else if trimmed.starts_with("MIGRATION:") {
            buckets.migration.push(value.clone());
        } else if trimmed.starts_with("DEP:") {
            buckets.dep.push(value.clone());
        } else {
            buckets.factual.push(value.clone());
        }
    }

    (buckets, status)
}

/// Prerequisite chain — the agent stops or warns if these are absent.
pub fn prerequisites_for(skill: &str, status: &StatusMap) -> Prerequisites {
    let needs: &[&str] = match skill {
        "decompose" | "hew-decompose" => &["plan"],
        "execute" | "hew-execute" => &["plan"],
        "verify" | "hew-verify" => &["plan"],
        "guard" | "hew-guard" => &["plan"],
        "convention" | "hew-convention" => &["scan"],
        "boundary" | "hew-boundary" => &["scan"],
        "audit" | "hew-audit" => &["scan"],
        "migrate" | "hew-migrate" => &["scan"],
        _ => &[],
    };

    let missing: Vec<String> = needs
        .iter()
        .filter(|p| !status.get(**p).map(|s| s.complete).unwrap_or(false))
        .map(|p| (*p).to_string())
        .collect();

    Prerequisites { met: missing.is_empty(), missing }
}

/// Build the prime JSON for `skill_name`.
pub fn build(client: &dyn BdClient, skill_name: &str) -> Result<PrimeOutput> {
    let skill = resolve_skill(skill_name)?;

    let stats: StatsSummary = client.stats().unwrap_or_default();
    let ready = client.ready().unwrap_or_default();
    let memories = client.memories().unwrap_or_default();
    let bd_version = client.version().ok().map(|v| v.semver);

    let (buckets, status) = categorize(&memories);
    let prereqs = prerequisites_for(skill.name, &status);

    let tasks = TaskInfo {
        total: stats.total_issues,
        done: stats.closed_issues,
        in_progress: stats.in_progress_issues,
        ready: stats.ready_issues,
        blocked: stats.blocked_issues,
        ready_list: ready.into_iter().take(20).collect(),
    };

    // Kick off (best-effort) the passive update check and surface any
    // cached notice from a previous run.
    crate::notify::schedule_if_stale(env!("CARGO_PKG_VERSION"));
    let update_available =
        crate::notify::read_cached_notice().ok().flatten().map(|n| UpdateAvailable {
            current: n.current,
            latest: n.latest.clone(),
            message: format!("Run `hew update` to upgrade to {}.", n.latest),
        });

    Ok(PrimeOutput {
        schema_version: 1,
        skill: skill.name.to_string(),
        project: ProjectInfo { beads_initialized: bd_version.is_some(), bd_version },
        status,
        prerequisites: prereqs,
        tasks,
        memories: buckets,
        skill_instructions: skill.body.to_string(),
        update_available,
    })
}

/// Find the most-recent `CHECKPOINT:` memory by parsing the ISO-8601
/// timestamp out of the value prefix. ISO-8601 strings sort
/// lexicographically, so a string max gives chronological max.
pub fn latest_checkpoint(memories: &BTreeMap<String, String>) -> Option<Checkpoint> {
    memories
        .iter()
        .filter(|(_, v)| v.trim_start().starts_with("CHECKPOINT:"))
        .map(|(k, v)| {
            let rest = v.trim_start().strip_prefix("CHECKPOINT:").unwrap_or("");
            let timestamp =
                rest.split_whitespace().next().filter(|s| !s.is_empty()).map(|s| s.to_string());
            Checkpoint { key: k.clone(), timestamp, body: v.clone() }
        })
        .max_by(|a, b| a.timestamp.cmp(&b.timestamp))
}

/// Build the skill-agnostic resume JSON. Returned by `hew prime resume`
/// and consumed by SessionStart hooks.
pub fn resume(client: &dyn BdClient) -> Result<ResumeOutput> {
    let stats: StatsSummary = client.stats().unwrap_or_default();
    let ready = client.ready().unwrap_or_default();
    let memories = client.memories().unwrap_or_default();
    let bd_version = client.version().ok().map(|v| v.semver);

    let (buckets, status) = categorize(&memories);
    let checkpoint = latest_checkpoint(&memories);

    let tasks = TaskInfo {
        total: stats.total_issues,
        done: stats.closed_issues,
        in_progress: stats.in_progress_issues,
        ready: stats.ready_issues,
        blocked: stats.blocked_issues,
        ready_list: ready.into_iter().take(20).collect(),
    };

    crate::notify::schedule_if_stale(env!("CARGO_PKG_VERSION"));
    let update_available =
        crate::notify::read_cached_notice().ok().flatten().map(|n| UpdateAvailable {
            current: n.current,
            latest: n.latest.clone(),
            message: format!("Run `hew update` to upgrade to {}.", n.latest),
        });

    Ok(ResumeOutput {
        schema_version: 1,
        project: ProjectInfo { beads_initialized: bd_version.is_some(), bd_version },
        status,
        tasks,
        memories: buckets,
        latest_checkpoint: checkpoint,
        update_available,
    })
}

fn resolve_skill(name: &str) -> Result<Skill> {
    // Accept `execute`, `hew-execute`, or the canonical name.
    if let Some(s) = skills::find(name) {
        return Ok(s);
    }
    let prefixed = format!("hew-{name}");
    if let Some(s) = skills::find(&prefixed) {
        return Ok(s);
    }
    Err(crate::error::HewError::MissingFlag { flag: format!("skill (unknown: {name})") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn categorize_routes_each_prefix() {
        let m = map(&[
            ("a", "CONVENTION:errors — wrap"),
            ("b", "BOUNDARY: POST /users"),
            ("c", "AUDIT: jose deprecated"),
            ("d", "SECURITY: JWT 15m"),
            ("e", "MIGRATION: add col"),
            ("f", "DEP: chrono 0.4"),
            ("g", "Backend: FastAPI + Postgres"),
        ]);
        let (b, _) = categorize(&m);
        assert_eq!(b.conventions.len(), 1);
        assert_eq!(b.boundaries.len(), 1);
        assert_eq!(b.audit.len(), 1);
        assert_eq!(b.security.len(), 1);
        assert_eq!(b.migration.len(), 1);
        assert_eq!(b.dep.len(), 1);
        assert_eq!(b.factual.len(), 1);
    }

    #[test]
    fn status_memory_is_parsed_out_and_not_bucketed() {
        let m = map(&[("k", "STATUS:scan:complete — 2026-05-11T14:30:00")]);
        let (b, s) = categorize(&m);
        assert!(b.factual.is_empty(), "STATUS must not leak into factual");
        assert!(s.get("scan").map(|e| e.complete).unwrap_or(false));
        assert_eq!(s["scan"].timestamp.as_deref(), Some("2026-05-11T14:30:00"));
    }

    #[test]
    fn status_in_progress_is_not_complete() {
        let m = map(&[("k", "STATUS:plan:in-progress")]);
        let (_, s) = categorize(&m);
        assert!(!s["plan"].complete);
    }

    #[test]
    fn execute_needs_plan() {
        let mut status: StatusMap = BTreeMap::new();
        let p = prerequisites_for("hew-execute", &status);
        assert!(!p.met);
        assert_eq!(p.missing, vec!["plan"]);

        status.insert("plan".into(), StatusEntry { complete: true, timestamp: None });
        let p = prerequisites_for("hew-execute", &status);
        assert!(p.met);
    }

    #[test]
    fn plan_has_no_prerequisites() {
        let status: StatusMap = BTreeMap::new();
        let p = prerequisites_for("hew-plan", &status);
        assert!(p.met);
        assert!(p.missing.is_empty());
    }

    #[test]
    fn unknown_skill_errors() {
        assert!(resolve_skill("nope").is_err());
    }

    #[test]
    fn resolve_accepts_short_name() {
        let s = resolve_skill("execute").unwrap();
        assert_eq!(s.name, "hew-execute");
    }

    #[test]
    fn latest_checkpoint_returns_none_when_absent() {
        let m = map(&[("a", "CONVENTION:errors — wrap"), ("b", "Backend: FastAPI")]);
        assert!(latest_checkpoint(&m).is_none());
    }

    #[test]
    fn latest_checkpoint_returns_single_when_one_present() {
        let m = map(&[
            ("a", "CONVENTION:errors — wrap"),
            ("checkpoint-08-35", "CHECKPOINT:2026-05-12T08:35 — Mid auth work."),
        ]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert_eq!(c.key, "checkpoint-08-35");
        assert_eq!(c.timestamp.as_deref(), Some("2026-05-12T08:35"));
        assert!(c.body.contains("Mid auth work"));
    }

    #[test]
    fn latest_checkpoint_picks_most_recent_by_timestamp() {
        let m = map(&[
            ("ck-early", "CHECKPOINT:2026-05-10T08:00 — early"),
            ("ck-late", "CHECKPOINT:2026-05-12T14:30 — late"),
            ("ck-mid", "CHECKPOINT:2026-05-11T12:00 — mid"),
        ]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert_eq!(c.key, "ck-late");
        assert_eq!(c.timestamp.as_deref(), Some("2026-05-12T14:30"));
    }

    #[test]
    fn latest_checkpoint_handles_missing_timestamp() {
        let m = map(&[("ck", "CHECKPOINT:")]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert!(c.timestamp.is_none());
    }
}
