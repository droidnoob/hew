//! Run-scope selector for `hew loop`.
//!
//! [`Scope`] names *which* set of bd tasks counts as the queue for a
//! single `hew loop run` invocation. v1 ships two shapes:
//!
//! - [`Scope::Ready`] — every bd-ready task (current behavior).
//! - [`Scope::Epics`] — restricted to children of the listed epics.
//!
//! Downstream consumers (dispatcher, runner, loop_log, summary, CLI)
//! all read this single type so the "what counts" boundary lives in
//! one place. Serialized form is a tagged JSON union mirroring
//! `hew_core::external_gate::GateKind`:
//!
//! ```json
//! {"kind": "ready"}
//! {"kind": "epics", "epic_ids": ["hew-6az"]}
//! ```
//!
//! [`Default`] is `Ready` so legacy callers that omit the field keep
//! the pre-scope behavior.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::bd::BdClient;
use crate::error::Result;
use crate::tasks;

/// Which tasks count as the loop's queue.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Scope {
    /// Every bd-ready task — no scope filter.
    #[default]
    Ready,
    /// Only tasks transitively under one of `epic_ids`.
    Epics { epic_ids: Vec<String> },
}

impl Scope {
    /// True when `task_id` belongs to this scope.
    ///
    /// `epic_descendant_set` is the pre-resolved set of every task id
    /// transitively under any selected epic (including the epics
    /// themselves). Callers build it once per run via
    /// [`resolve_descendants`] and pass it in for every filter check.
    pub fn includes(&self, task_id: &str, epic_descendant_set: &HashSet<String>) -> bool {
        match self {
            Self::Ready => true,
            Self::Epics { .. } => epic_descendant_set.contains(task_id),
        }
    }
}

/// Walk every epic in `epic_ids` and return the union of all their
/// transitive descendants plus the epics themselves.
///
/// Uses `bd children <id>` (via [`tasks::children`]) to resolve one
/// level at a time and BFS-walks via a visited set, so a graph with
/// shared descendants or accidental cycles still terminates.
pub fn resolve_descendants(bd: &dyn BdClient, epic_ids: &[String]) -> Result<HashSet<String>> {
    let mut out: HashSet<String> = HashSet::new();
    let mut queue: Vec<String> = Vec::new();

    for id in epic_ids {
        if out.insert(id.clone()) {
            queue.push(id.clone());
        }
    }

    while let Some(parent) = queue.pop() {
        let kids = tasks::children(bd, &parent)?;
        for c in kids {
            if out.insert(c.id.clone()) {
                queue.push(c.id);
            }
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
    use crate::error::HewError;
    use std::cell::RefCell;
    use std::collections::BTreeMap;
    use std::ffi::OsStr;

    #[test]
    fn scope_default_is_ready() {
        assert_eq!(Scope::default(), Scope::Ready);
    }

    #[test]
    fn scope_ready_includes_everything() {
        let empty: HashSet<String> = HashSet::new();
        let s = Scope::Ready;
        assert!(s.includes("hew-anything", &empty));
        assert!(s.includes("foo", &empty));
    }

    #[test]
    fn scope_epics_filters_to_descendant_set() {
        let s = Scope::Epics { epic_ids: vec!["hew-6az".into()] };
        let set: HashSet<String> =
            ["hew-6az", "hew-child-1", "hew-child-2"].iter().map(|s| s.to_string()).collect();
        assert!(s.includes("hew-6az", &set));
        assert!(s.includes("hew-child-1", &set));
        assert!(!s.includes("hew-stranger", &set));
    }

    #[test]
    fn scope_serde_roundtrip_ready() {
        let s = Scope::Ready;
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"ready"}"#);
        let back: Scope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, Scope::Ready);
    }

    #[test]
    fn scope_serde_roundtrip_epics() {
        let s = Scope::Epics { epic_ids: vec!["hew-6az".into()] };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"kind":"epics","epic_ids":["hew-6az"]}"#);
        let back: Scope = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn scope_serde_backward_compat_missing_field() {
        // Legacy RunConfig JSON without a scope key should deserialize
        // to the default — we model this here by deserializing a
        // wrapper that has scope: Option<Scope> with #[serde(default)].
        #[derive(Deserialize)]
        struct RunConfigCompat {
            #[serde(default)]
            scope: Scope,
        }
        let body = "{}";
        let cfg: RunConfigCompat = serde_json::from_str(body).unwrap();
        assert_eq!(cfg.scope, Scope::default());
    }

    // ── per-parent fake BdClient ────────────────────────────────────
    //
    // The shared MockBd in `tasks::tests` keys on the first argv token,
    // so every `bd children <id>` call returns the same body — that's
    // fine for direct-children tests but useless for a transitive walk.
    // We need a fake that maps each parent id to its own children list.

    #[derive(Debug, Default)]
    struct PerParentBd {
        children: BTreeMap<String, String>, // parent_id → JSON array body
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl PerParentBd {
        fn new() -> Self {
            Self::default()
        }
        fn with_children(mut self, parent: &str, ids: &[&str]) -> Self {
            let body = ids
                .iter()
                .map(|id| {
                    format!(
                        r#"{{"id":"{id}","title":"t-{id}","description":"","status":"open","priority":2,"issue_type":"task","closed_at":"","close_reason":null,"parent":"{parent}"}}"#
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            self.children.insert(parent.to_string(), format!("[{body}]"));
            self
        }
    }

    impl BdClient for PerParentBd {
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
            Ok(BTreeMap::new())
        }
        fn remember(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
            let captured: Vec<String> =
                args.iter().map(|a| a.to_string_lossy().to_string()).collect();
            self.calls.borrow_mut().push(captured.clone());
            if captured.first().map(|s| s.as_str()) == Some("children") {
                let parent = captured.get(1).cloned().unwrap_or_default();
                let body = self.children.get(&parent).cloned().unwrap_or_else(|| "[]".into());
                return Ok(BdOutput { stdout: body, stderr: String::new() });
            }
            Err(HewError::BdNonZero { code: 1, stderr: format!("unexpected call: {captured:?}") })
        }
    }

    #[test]
    fn resolve_descendants_includes_self_and_transitive() {
        // epic-a → [c-1, c-2]; c-1 → [g-1]; c-2 → []; g-1 → [].
        let bd = PerParentBd::new()
            .with_children("epic-a", &["c-1", "c-2"])
            .with_children("c-1", &["g-1"])
            .with_children("c-2", &[])
            .with_children("g-1", &[]);
        let set = resolve_descendants(&bd, &["epic-a".into()]).unwrap();
        let expected: HashSet<String> =
            ["epic-a", "c-1", "c-2", "g-1"].iter().map(|s| s.to_string()).collect();
        assert_eq!(set, expected);
    }

    #[test]
    fn resolve_descendants_unions_multiple_epics_and_dedupes() {
        // Two epics that share a descendant ("c-shared").
        let bd = PerParentBd::new()
            .with_children("epic-a", &["c-shared", "c-a-only"])
            .with_children("epic-b", &["c-shared", "c-b-only"])
            .with_children("c-shared", &[])
            .with_children("c-a-only", &[])
            .with_children("c-b-only", &[]);
        let set = resolve_descendants(&bd, &["epic-a".to_string(), "epic-b".to_string()]).unwrap();
        let expected: HashSet<String> = ["epic-a", "epic-b", "c-shared", "c-a-only", "c-b-only"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(set, expected);
    }

    #[test]
    fn resolve_descendants_empty_input_returns_empty_set() {
        let bd = PerParentBd::new();
        let set = resolve_descendants(&bd, &[]).unwrap();
        assert!(set.is_empty());
    }
}
