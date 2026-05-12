//! Compile-time craft-principles catalog.
//!
//! Hew's methodology surfaces craft principles (SOLID, DRY, KISS, Clean
//! Architecture, etc.) but doesn't enforce them universally — projects
//! pick the subset that fits their domain at bootstrap. The catalog
//! lives in `skills/data/craft-principles.toml` and is embedded via
//! `include_str!` so the binary ships with the full reference set.
//!
//! Each [`CraftPrinciple`] is:
//!
//! - `id` — stable kebab-case slug; `CONVENTION:craft.<id>` memories
//!   pin a project's chosen principles.
//! - `category` — code-level, architecture, reliability, or rule-set.
//! - `default_for_stacks` — which seeded stack_ids should preselect
//!   this principle in `hew-new-project` Phase C.
//! - `conflicts_with` — sibling principle ids that fight this one
//!   (e.g., `event-sourcing` vs `crud-simplicity`); the picker
//!   warns when both are chosen.
//!
//! CR.2 fills the table with the v1 catalog; CR.1 just stands up the
//! types + a placeholder entry so the schema + load API are usable.

use serde::{Deserialize, Serialize};

const EMBEDDED: &str = include_str!("../../skills/data/craft-principles.toml");

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct CraftTable {
    pub principles: Vec<CraftPrinciple>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct CraftPrinciple {
    pub id: String,
    pub name: String,
    pub category: CraftCategory,
    pub summary: String,
    pub when_to_apply: String,
    pub when_not_to_apply: String,
    #[serde(default)]
    pub conflicts_with: Vec<String>,
    pub example: String,
    #[serde(default)]
    pub default_for_stacks: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CraftCategory {
    /// SOLID, CUPID, DRY, KISS, YAGNI, SoC, Composition-over-Inheritance,
    /// Law of Demeter, etc.
    CodeLevel,
    /// Clean Architecture, Hexagonal, DDD, CQRS, Event Sourcing.
    Architecture,
    /// Immutability, Idempotence, Fail Fast.
    Reliability,
    /// Rule of Three, Unix Philosophy, Boy Scout Rule,
    /// Principle of Least Astonishment.
    RuleSet,
}

/// Parse the embedded craft-principles table.
///
/// In practice the build fails before this can fail at runtime —
/// [`embedded_table_parses`] runs on every `cargo test`.
pub fn load() -> Result<CraftTable, toml::de::Error> {
    toml::from_str(EMBEDDED)
}

/// Look up a principle by id. Returns `None` if not in the catalog.
pub fn find(id: &str) -> Option<CraftPrinciple> {
    load().ok().and_then(|t| t.principles.into_iter().find(|p| p.id == id))
}

/// Every principle id (stable, kebab-case).
pub fn ids() -> Vec<String> {
    load().map(|t| t.principles.into_iter().map(|p| p.id).collect()).unwrap_or_default()
}

/// Pre-selected principles for a stack id. Empty when the stack isn't
/// in any principle's `default_for_stacks` list.
pub fn for_stack(stack_id: &str) -> Vec<CraftPrinciple> {
    load()
        .map(|t| {
            t.principles
                .into_iter()
                .filter(|p| p.default_for_stacks.iter().any(|s| s == stack_id))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_table_parses() {
        let t = load().expect("embedded craft-principles.toml must parse");
        assert!(!t.principles.is_empty(), "table must have at least one entry");
    }

    #[test]
    fn ids_are_unique() {
        let table = load().unwrap();
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for p in &table.principles {
            assert!(seen.insert(p.id.as_str()), "duplicate principle id `{}`", p.id);
        }
    }

    #[test]
    fn ids_are_kebab_case() {
        let table = load().unwrap();
        for p in &table.principles {
            assert!(
                p.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
                "id `{}` is not kebab-case",
                p.id
            );
        }
    }

    #[test]
    fn conflicts_with_references_exist() {
        let table = load().unwrap();
        let ids: std::collections::HashSet<&str> =
            table.principles.iter().map(|p| p.id.as_str()).collect();
        for p in &table.principles {
            for c in &p.conflicts_with {
                assert!(
                    ids.contains(c.as_str()),
                    "principle `{}` declares conflict with unknown id `{}`",
                    p.id,
                    c
                );
            }
        }
    }

    #[test]
    fn find_returns_some_for_known_id() {
        let table = load().unwrap();
        let first = &table.principles[0];
        let id = first.id.clone();
        assert_eq!(find(&id).unwrap().id, id);
    }

    #[test]
    fn find_returns_none_for_unknown() {
        assert!(find("klingon-rocket-pattern").is_none());
    }

    #[test]
    fn for_stack_filters_by_default_list() {
        // CR.2 will seed real defaults; for the placeholder we just
        // assert the function doesn't panic and returns an iterable.
        let _ = for_stack("py-fastapi");
    }
}
