//! Pure per-task model resolver for the `hew loop` dynamic-model epic.
//!
//! Precedence (first match wins):
//!   1. Description tag `<!-- hew:model=X -->`.
//!   2. Label `model:X`.
//!   3. `cfg.by_priority` keyed by `P{n}`.
//!   4. `cfg.by_type` keyed by task issue type.
//!   5. `cfg.default`.
//!   6. `None` → spawner falls back to the runtime's own default.
//!
//! No I/O, no allocation if every rule misses.
//!
//! See `DECISION:loop-fallback-policy` for fallback runtime; this resolver
//! only chooses the *model name*, leaving runtime selection to the loop.

use crate::config::LoopModelConfig;

/// Input shape for the resolver. Callers adapt their own task struct
/// (e.g. `bd::ReadyTask`) into this borrow-only view.
#[derive(Debug, Clone, Copy)]
pub struct TaskRecord<'a> {
    pub description: &'a str,
    pub labels: &'a [String],
    pub priority: u8,
    pub issue_type: &'a str,
}

const TAG_OPEN: &str = "<!-- hew:model=";
const TAG_CLOSE: &str = "-->";
const LABEL_PREFIX: &str = "model:";

/// Resolve the model name for a task. See module docs for precedence.
pub fn resolve_model(task: &TaskRecord<'_>, cfg: &LoopModelConfig) -> Option<String> {
    if let Some(m) = extract_tag(task.description) {
        return Some(m);
    }
    if let Some(m) = extract_label(task.labels) {
        return Some(m);
    }
    let prio_key = format!("P{}", task.priority);
    if let Some(m) = cfg.by_priority.get(&prio_key) {
        return Some(m.clone());
    }
    if !task.issue_type.is_empty()
        && let Some(m) = cfg.by_type.get(task.issue_type)
    {
        return Some(m.clone());
    }
    cfg.default.clone()
}

fn extract_tag(description: &str) -> Option<String> {
    let start = description.find(TAG_OPEN)?;
    let rest = &description[start + TAG_OPEN.len()..];
    let end = rest.find(TAG_CLOSE)?;
    let value = rest[..end].trim();
    if value.is_empty() { None } else { Some(value.to_string()) }
}

fn extract_label(labels: &[String]) -> Option<String> {
    for raw in labels {
        let l = raw.trim();
        if let Some(rest) = l.strip_prefix(LABEL_PREFIX) {
            let v = rest.trim();
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn cfg(
        default: Option<&str>,
        by_priority: &[(&str, &str)],
        by_type: &[(&str, &str)],
    ) -> LoopModelConfig {
        let mut bp = BTreeMap::new();
        for (k, v) in by_priority {
            bp.insert((*k).to_string(), (*v).to_string());
        }
        let mut bt = BTreeMap::new();
        for (k, v) in by_type {
            bt.insert((*k).to_string(), (*v).to_string());
        }
        LoopModelConfig { default: default.map(str::to_string), by_priority: bp, by_type: bt }
    }

    fn task<'a>(
        description: &'a str,
        labels: &'a [String],
        priority: u8,
        issue_type: &'a str,
    ) -> TaskRecord<'a> {
        TaskRecord { description, labels, priority, issue_type }
    }

    #[test]
    fn tag_wins_over_label_and_config() {
        let labels = vec!["model:label-pick".to_string()];
        let t = task("body <!-- hew:model=tag-pick --> more", &labels, 0, "task");
        let c = cfg(Some("cfg-default"), &[("P0", "cfg-prio")], &[("task", "cfg-type")]);
        assert_eq!(resolve_model(&t, &c), Some("tag-pick".to_string()));
    }

    #[test]
    fn label_wins_over_config() {
        let labels = vec!["model:label-pick".to_string()];
        let t = task("no tag here", &labels, 0, "task");
        let c = cfg(Some("cfg-default"), &[("P0", "cfg-prio")], &[("task", "cfg-type")]);
        assert_eq!(resolve_model(&t, &c), Some("label-pick".to_string()));
    }

    #[test]
    fn by_priority_beats_by_type() {
        let labels: Vec<String> = vec![];
        let t = task("", &labels, 0, "task");
        let c = cfg(Some("cfg-default"), &[("P0", "cfg-prio")], &[("task", "cfg-type")]);
        assert_eq!(resolve_model(&t, &c), Some("cfg-prio".to_string()));
    }

    #[test]
    fn by_type_used_when_priority_misses() {
        let labels: Vec<String> = vec![];
        let t = task("", &labels, 2, "bug");
        let c = cfg(Some("cfg-default"), &[("P0", "cfg-prio")], &[("bug", "cfg-bug")]);
        assert_eq!(resolve_model(&t, &c), Some("cfg-bug".to_string()));
    }

    #[test]
    fn default_used_when_no_rule_matches() {
        let labels: Vec<String> = vec![];
        let t = task("", &labels, 4, "chore");
        let c = cfg(Some("cfg-default"), &[("P0", "cfg-prio")], &[("bug", "cfg-bug")]);
        assert_eq!(resolve_model(&t, &c), Some("cfg-default".to_string()));
    }

    #[test]
    fn all_empty_returns_none() {
        let labels: Vec<String> = vec![];
        let t = task("", &labels, 2, "task");
        let c = cfg(None, &[], &[]);
        assert_eq!(resolve_model(&t, &c), None);
    }

    #[test]
    fn malformed_empty_tag_falls_through_to_label() {
        let labels = vec!["model:label-pick".to_string()];
        let t = task("body <!-- hew:model= --> more", &labels, 0, "task");
        let c = cfg(None, &[], &[]);
        assert_eq!(resolve_model(&t, &c), Some("label-pick".to_string()));
    }

    #[test]
    fn malformed_unterminated_tag_falls_through() {
        let labels: Vec<String> = vec![];
        let t = task("body <!-- hew:model=oops no close", &labels, 0, "task");
        let c = cfg(Some("cfg-default"), &[], &[]);
        assert_eq!(resolve_model(&t, &c), Some("cfg-default".to_string()));
    }

    #[test]
    fn empty_label_value_is_ignored() {
        let labels = vec!["model:".to_string(), "model:   ".to_string()];
        let t = task("", &labels, 0, "task");
        let c = cfg(Some("cfg-default"), &[], &[]);
        assert_eq!(resolve_model(&t, &c), Some("cfg-default".to_string()));
    }

    #[test]
    fn non_model_labels_ignored() {
        let labels = vec!["area:loop".to_string(), "needs-tests".to_string()];
        let t = task("", &labels, 0, "task");
        let c = cfg(Some("cfg-default"), &[], &[]);
        assert_eq!(resolve_model(&t, &c), Some("cfg-default".to_string()));
    }

    #[test]
    fn priority_formatted_with_p_prefix() {
        let labels: Vec<String> = vec![];
        let t = task("", &labels, 3, "task");
        let c = cfg(None, &[("P3", "haiku-4-5")], &[]);
        assert_eq!(resolve_model(&t, &c), Some("haiku-4-5".to_string()));
    }

    #[test]
    fn empty_issue_type_skips_by_type() {
        let labels: Vec<String> = vec![];
        let t = task("", &labels, 0, "");
        // by_type has an "" key which should still be skipped because issue_type is empty.
        let mut by_type = std::collections::BTreeMap::new();
        by_type.insert(String::new(), "should-not-pick".to_string());
        let c = LoopModelConfig {
            default: Some("cfg-default".into()),
            by_priority: Default::default(),
            by_type,
        };
        assert_eq!(resolve_model(&t, &c), Some("cfg-default".to_string()));
    }

    #[test]
    fn tag_value_is_trimmed() {
        let labels: Vec<String> = vec![];
        let t = task("<!-- hew:model=   opus-4-7   -->", &labels, 0, "task");
        let c = cfg(None, &[], &[]);
        assert_eq!(resolve_model(&t, &c), Some("opus-4-7".to_string()));
    }
}
