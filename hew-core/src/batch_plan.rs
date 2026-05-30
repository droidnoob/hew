//! Per-iter batch artifact written by `hew loop run --jobs N >= 2`.
//!
//! A [`BatchPlan`] names the task ids the dispatcher should consider
//! dispatching on the *next* iter. It is one of three signals
//! (agent-suggested, planner-spawned, or skipped → fall back to
//! trust-the-graph) and persists on disk as
//! `<run-dir>/batch-NNN.json` so a future `hew loop graph` /
//! `hew loop summary` consumer can replay the dispatch decision after
//! the fact.
//!
//! See parent epic `hew-lf40` for the wire-up and the planner-spawn
//! pipeline that produces these files.
//!
//! Schema discipline mirrors [`crate::external_gate::GateKind`]: tagged
//! enum + `snake_case` rename, atomic write via
//! [`crate::loop_log::write_json_atomic`], and a pinned
//! [`SCHEMA_VERSION`] so a newer hew can reject older logs cleanly
//! instead of misparsing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::loop_log::write_json_atomic;
use crate::runner::TokenSpend;

/// Pinned schema version for the on-disk batch-plan format. Bump iff
/// the wire shape changes; readers reject any other value.
pub const SCHEMA_VERSION: u32 = 1;

/// Provenance of a [`BatchPlan`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchSource {
    /// The previous iter's close output named the batch via a
    /// `next_iteration:` tail line.
    Agent,
    /// A dedicated planner subprocess produced the batch between iters.
    Planner,
    /// No batch was produced — dispatcher falls back to trust-the-graph
    /// (`bd ready` order). `reason` on the surrounding [`BatchPlan`]
    /// records why (e.g. budget exhausted, planner declined).
    Skipped,
}

/// Per-iter batch artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPlan {
    pub schema_version: u32,
    pub iter_number: u32,
    pub task_ids: Vec<String>,
    pub source: BatchSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub planner_tokens: Option<TokenSpend>,
}

/// `<run_dir>/batch-NNN.json` with a 3-digit zero-padded iter number.
pub fn path(run_dir: &Path, iter: u32) -> PathBuf {
    run_dir.join(format!("batch-{iter:03}.json"))
}

/// Read the batch plan for `iter` from `run_dir`. Returns `Ok(None)`
/// when the file is absent (the common case for old runs or skipped
/// iters that never wrote one).
pub fn read(run_dir: &Path, iter: u32) -> Result<Option<BatchPlan>> {
    let p = path(run_dir, iter);
    let body = match std::fs::read_to_string(&p) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let plan: BatchPlan = serde_json::from_str(&body)?;
    if plan.schema_version != SCHEMA_VERSION {
        return Err(std::io::Error::other(format!(
            "unsupported batch_plan schema_version {} (expected {})",
            plan.schema_version, SCHEMA_VERSION
        ))
        .into());
    }
    Ok(Some(plan))
}

/// Atomically write `plan` to `<run_dir>/batch-NNN.json`.
pub fn write(run_dir: &Path, plan: &BatchPlan) -> Result<()> {
    write_json_atomic(&path(run_dir, plan.iter_number), plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "hew-batch-plan-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn path_zero_pads_iter_to_three_digits() {
        assert_eq!(path(Path::new("/tmp/r"), 1), Path::new("/tmp/r/batch-001.json"));
        assert_eq!(path(Path::new("/tmp/r"), 42), Path::new("/tmp/r/batch-042.json"));
        assert_eq!(path(Path::new("/tmp/r"), 999), Path::new("/tmp/r/batch-999.json"));
    }

    #[test]
    fn read_returns_none_on_missing_file() {
        let dir = tmpdir();
        assert!(read(&dir, 7).unwrap().is_none());
    }

    #[test]
    fn read_parses_agent_sourced_plan_roundtrip() {
        let dir = tmpdir();
        let plan = BatchPlan {
            schema_version: SCHEMA_VERSION,
            iter_number: 3,
            task_ids: vec!["hew-aaa".into(), "hew-bbb".into()],
            source: BatchSource::Agent,
            reason: None,
            created_at: "2026-05-30T00:00:00Z".into(),
            planner_tokens: None,
        };
        write(&dir, &plan).unwrap();
        let parsed = read(&dir, 3).unwrap().expect("file present");
        assert_eq!(parsed, plan);
    }

    #[test]
    fn read_parses_planner_sourced_plan_with_tokens() {
        let dir = tmpdir();
        let plan = BatchPlan {
            schema_version: SCHEMA_VERSION,
            iter_number: 5,
            task_ids: vec!["hew-ccc".into()],
            source: BatchSource::Planner,
            reason: None,
            created_at: "2026-05-30T00:00:00Z".into(),
            planner_tokens: Some(TokenSpend {
                input: 1000,
                output: 200,
                cache_read: 0,
                cache_create: 0,
            }),
        };
        write(&dir, &plan).unwrap();
        let parsed = read(&dir, 5).unwrap().expect("file present");
        assert_eq!(parsed.source, BatchSource::Planner);
        assert_eq!(parsed.planner_tokens.unwrap().input, 1000);
    }

    #[test]
    fn read_parses_skipped_plan_with_reason() {
        let dir = tmpdir();
        let plan = BatchPlan {
            schema_version: SCHEMA_VERSION,
            iter_number: 9,
            task_ids: Vec::new(),
            source: BatchSource::Skipped,
            reason: Some("planner budget exhausted".into()),
            created_at: "2026-05-30T00:00:00Z".into(),
            planner_tokens: None,
        };
        write(&dir, &plan).unwrap();
        let parsed = read(&dir, 9).unwrap().expect("file present");
        assert_eq!(parsed.source, BatchSource::Skipped);
        assert_eq!(parsed.reason.as_deref(), Some("planner budget exhausted"));
        assert!(parsed.task_ids.is_empty());
    }

    #[test]
    fn write_atomic_temp_then_rename_pattern() {
        let dir = tmpdir();
        let plan = BatchPlan {
            schema_version: SCHEMA_VERSION,
            iter_number: 2,
            task_ids: vec!["hew-zzz".into()],
            source: BatchSource::Agent,
            reason: None,
            created_at: "2026-05-30T00:00:00Z".into(),
            planner_tokens: None,
        };
        write(&dir, &plan).unwrap();
        let final_path = path(&dir, 2);
        assert!(final_path.exists());
        // The temp sibling must not linger after a successful rename.
        let tmp = dir.join(".batch-002.json.tmp");
        assert!(!tmp.exists(), "atomic write must remove its temp sibling: {tmp:?}");
    }

    #[test]
    fn batch_source_serde_snake_case() {
        // Single-word variants render in lower-case on the wire — no
        // PascalCase leakage.
        let cases = [
            (BatchSource::Agent, "\"agent\""),
            (BatchSource::Planner, "\"planner\""),
            (BatchSource::Skipped, "\"skipped\""),
        ];
        for (variant, expected) in cases {
            let s = serde_json::to_string(&variant).unwrap();
            assert_eq!(s, expected, "wire form for {variant:?}");
            let parsed: BatchSource = serde_json::from_str(expected).unwrap();
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn schema_version_pinned_to_1() {
        assert_eq!(SCHEMA_VERSION, 1);
    }

    #[test]
    fn read_rejects_unknown_schema_version_with_clear_error() {
        let dir = tmpdir();
        // Hand-rolled JSON with the wrong schema_version — serde parses
        // it cleanly, the version check should reject it.
        let body = r#"{
            "schema_version": 99,
            "iter_number": 1,
            "task_ids": ["hew-x"],
            "source": "agent",
            "created_at": "2026-05-30T00:00:00Z"
        }"#;
        std::fs::write(path(&dir, 1), body).unwrap();
        let err = read(&dir, 1).expect_err("must reject unknown schema_version");
        let msg = err.to_string();
        assert!(
            msg.contains("schema_version") && msg.contains("99"),
            "error must name the offending version: {msg}"
        );
    }
}
