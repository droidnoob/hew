//! External-state gates for the `hew gate` subcommand.
//!
//! A gate is a bd task whose closure waits on some external condition
//! resolving — initially: a GitHub PR being merged. The CLI wraps bd
//! task creation with a typed metadata spec, and `hew gate poll` reads
//! that spec back, queries the external surface (e.g. `gh pr view`),
//! and closes the task when the condition is met.
//!
//! This module is the pure logic half:
//!
//! - [`GateKind`] / [`GateSpec`] — the typed spec stored as task
//!   metadata under the [`METADATA_KEY`] JSON key.
//! - [`classify_gh_pr_view`] — interprets the parsed JSON from
//!   `gh pr view <N> --json state,mergedAt` and returns a
//!   [`PollOutcome`].
//! - No subprocess, no bd, no `gh` invocation lives here — the CLI
//!   layer in `hew/src/commands/gate.rs` owns those side effects.
//!
//! Disambiguation: `hew_core::gate` (the per-iter loop test/lint
//! gate) is a different concept. This module covers external-state
//! gates exposed via the `hew gate` user command.

use serde::{Deserialize, Serialize};

/// Top-level metadata key used on the bd task. The serialized
/// [`GateSpec`] sits at `metadata.<METADATA_KEY>`.
pub const METADATA_KEY: &str = "hew_gate";

/// Label applied to gate tasks for discoverability via `bd list --label`.
pub const GATE_LABEL: &str = "hew-gate";

/// Kinds of external conditions a gate can wait on.
///
/// v1 ships only `GhPr` — the other variants are intentionally
/// deferred until there's a concrete user need (filed as
/// `RESEARCH:gate-backends` for tracking).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum GateKind {
    /// GitHub PR merged. Resolved when `gh pr view <id> --json state`
    /// reports `state = MERGED`.
    #[serde(rename = "gh:pr")]
    GhPr { id: u64 },
}

impl GateKind {
    /// Human-readable label used in CLI output and task titles.
    pub fn short_label(&self) -> String {
        match self {
            Self::GhPr { id } => format!("PR #{id}"),
        }
    }
}

/// Stored on a bd task's metadata; round-trips through JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateSpec {
    pub kind: GateKind,
}

impl GateSpec {
    /// Build the JSON document that bd's `--metadata` flag expects —
    /// wraps `self` under the [`METADATA_KEY`] so we don't collide
    /// with other future hew-managed metadata blocks on the same task.
    pub fn to_metadata_json(&self) -> String {
        // `{"hew_gate": {...}}` — manual wrap to keep the key stable
        // even if we later add sibling keys.
        let inner = serde_json::to_string(self).expect("GateSpec serializes");
        format!(r#"{{"{METADATA_KEY}":{inner}}}"#)
    }

    /// Read a [`GateSpec`] back from a bd-emitted metadata blob.
    /// Expects the wrapping form (`{"hew_gate": {...}}`) that
    /// [`to_metadata_json`](Self::to_metadata_json) produces. If
    /// bd's surfaces ever flatten the wrapper in practice, add a
    /// fallback here — until then we keep the contract strict so a
    /// malformed metadata block fails loudly instead of silently
    /// matching the wrong shape.
    pub fn from_metadata_json(s: &str) -> Result<Self, GateParseError> {
        let v: serde_json::Value = serde_json::from_str(s).map_err(GateParseError::Json)?;
        let inner = v.get(METADATA_KEY).cloned().ok_or(GateParseError::MissingKey)?;
        serde_json::from_value(inner).map_err(GateParseError::Json)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum GateParseError {
    #[error("gate metadata is not valid JSON or doesn't match the GateSpec shape: {0}")]
    Json(#[source] serde_json::Error),
    #[error("gate metadata missing top-level `{}` key", METADATA_KEY)]
    MissingKey,
}

/// What `hew gate poll` should do with a gate task after consulting
/// its external source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// External condition fired — caller closes the task with `reason`.
    Resolved { reason: String },
    /// External condition hasn't fired yet — leave the task open. The
    /// `detail` field is a short status line for CLI output
    /// (e.g. "OPEN").
    StillOpen { detail: String },
    /// We talked to the external surface but couldn't classify the
    /// response (unknown state string, schema mismatch). Caller
    /// surfaces a warning and leaves the task open.
    Indeterminate { detail: String },
}

/// GitHub PR view payload — the subset we care about. Matches
/// `gh pr view <N> --json state,mergedAt`.
#[derive(Debug, Clone, Deserialize)]
pub struct GhPrView {
    pub state: String,
    #[serde(rename = "mergedAt")]
    pub merged_at: Option<String>,
}

/// Classify a `gh pr view --json state,mergedAt` payload.
///
/// GitHub's PR state field is `OPEN | CLOSED | MERGED`. `MERGED`
/// resolves the gate; `OPEN` is still-pending; `CLOSED` (closed
/// without merge) is also still-pending from a gate perspective
/// because the workflow assumes the PR will eventually re-open or be
/// re-pushed — we'd rather leave the gate open than auto-resolve on a
/// state the operator likely didn't intend.
pub fn classify_gh_pr_view(view: &GhPrView) -> PollOutcome {
    match view.state.as_str() {
        "MERGED" => {
            let when = view.merged_at.as_deref().unwrap_or("(unknown time)");
            PollOutcome::Resolved { reason: format!("PR merged at {when}") }
        }
        "OPEN" => PollOutcome::StillOpen { detail: "OPEN".into() },
        "CLOSED" => PollOutcome::StillOpen { detail: "CLOSED (not merged)".into() },
        other => PollOutcome::Indeterminate { detail: format!("unknown state: {other}") },
    }
}

/// Parse the raw `gh pr view` JSON and classify in one step. Returns
/// `Indeterminate` if the payload doesn't deserialize.
pub fn classify_gh_pr_view_json(raw: &str) -> PollOutcome {
    match serde_json::from_str::<GhPrView>(raw) {
        Ok(v) => classify_gh_pr_view(&v),
        Err(e) => PollOutcome::Indeterminate { detail: format!("gh pr view payload: {e}") },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_spec_roundtrips_through_metadata_json() {
        let spec = GateSpec { kind: GateKind::GhPr { id: 49 } };
        let s = spec.to_metadata_json();
        // Wrapping form: top-level "hew_gate" key.
        assert!(s.contains(r#""hew_gate""#), "metadata json must wrap under hew_gate key: {s}");
        let back = GateSpec::from_metadata_json(&s).expect("roundtrip");
        assert_eq!(back, spec);
    }

    #[test]
    fn gate_spec_rejects_missing_wrapper_key() {
        // Strict contract: if the wrapping key is absent we fail loudly
        // rather than silently coercing into the wrong shape.
        let unwrapped = r#"{"kind":{"kind":"gh:pr","id":7}}"#;
        assert!(matches!(GateSpec::from_metadata_json(unwrapped), Err(GateParseError::MissingKey)));
    }

    #[test]
    fn gh_pr_view_merged_resolves() {
        let v = GhPrView { state: "MERGED".into(), merged_at: Some("2026-05-29T12:34:56Z".into()) };
        match classify_gh_pr_view(&v) {
            PollOutcome::Resolved { reason } => {
                assert!(reason.contains("2026-05-29"), "reason carries mergedAt: {reason}");
            }
            other => panic!("expected Resolved, got {other:?}"),
        }
    }

    #[test]
    fn gh_pr_view_merged_without_timestamp_still_resolves() {
        let v = GhPrView { state: "MERGED".into(), merged_at: None };
        assert!(matches!(classify_gh_pr_view(&v), PollOutcome::Resolved { .. }));
    }

    #[test]
    fn gh_pr_view_open_stays_open() {
        let v = GhPrView { state: "OPEN".into(), merged_at: None };
        assert!(matches!(classify_gh_pr_view(&v), PollOutcome::StillOpen { .. }));
    }

    #[test]
    fn gh_pr_view_closed_without_merge_stays_open() {
        // Closed-without-merge → leave the gate alone; operator may reopen.
        let v = GhPrView { state: "CLOSED".into(), merged_at: None };
        match classify_gh_pr_view(&v) {
            PollOutcome::StillOpen { detail } => assert!(detail.contains("CLOSED")),
            other => panic!("expected StillOpen, got {other:?}"),
        }
    }

    #[test]
    fn gh_pr_view_unknown_state_is_indeterminate() {
        let v = GhPrView { state: "DRAFT".into(), merged_at: None };
        assert!(matches!(classify_gh_pr_view(&v), PollOutcome::Indeterminate { .. }));
    }

    #[test]
    fn classify_from_json_handles_malformed_payload() {
        let outcome = classify_gh_pr_view_json("{ not valid json");
        assert!(matches!(outcome, PollOutcome::Indeterminate { .. }));
    }

    #[test]
    fn classify_from_json_parses_typical_gh_output() {
        let raw = r#"{"state":"MERGED","mergedAt":"2026-05-29T03:00:00Z"}"#;
        assert!(matches!(classify_gh_pr_view_json(raw), PollOutcome::Resolved { .. }));
    }

    #[test]
    fn gate_kind_short_label_for_pr() {
        let k = GateKind::GhPr { id: 49 };
        assert_eq!(k.short_label(), "PR #49");
    }
}
