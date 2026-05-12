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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq, Hash)]
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
        // py-fastapi is the most heavily seeded stack — at least
        // SOLID, DRY, KISS, SoC, CoI, Clean Arch, DDD, Idempotence,
        // Fail Fast should preselect.
        let picks = for_stack("py-fastapi");
        assert!(picks.len() >= 5, "py-fastapi defaults too sparse: {}", picks.len());
        let ids: std::collections::HashSet<&str> = picks.iter().map(|p| p.id.as_str()).collect();
        assert!(ids.contains("solid"), "py-fastapi should default to SOLID");
        assert!(ids.contains("dry"), "py-fastapi should default to DRY");
    }

    #[test]
    fn catalog_v1_has_full_breadth() {
        let table = load().unwrap();
        // v1 ships 28 principles across the four categories (20 core +
        // 8 added by CR.2.1: meaningful-names, small-functions, SLA,
        // tell-dont-ask, CQS, pure-functions, no-magic-numbers,
        // consistency-with-existing-code).
        assert!(
            table.principles.len() >= 25,
            "v1 catalog should ship at least 25 principles, got {}",
            table.principles.len()
        );
    }

    #[test]
    fn every_category_is_represented() {
        let table = load().unwrap();
        let cats: std::collections::HashSet<CraftCategory> =
            table.principles.iter().map(|p| p.category).collect();
        for required in [
            CraftCategory::CodeLevel,
            CraftCategory::Architecture,
            CraftCategory::Reliability,
            CraftCategory::RuleSet,
        ] {
            assert!(cats.contains(&required), "missing principles for {required:?}");
        }
    }

    #[test]
    fn default_for_stacks_references_seeded_stack_ids() {
        let table = load().unwrap();
        let valid: std::collections::HashSet<String> = crate::stacks::ids().into_iter().collect();
        for p in &table.principles {
            for s in &p.default_for_stacks {
                assert!(
                    valid.contains(s),
                    "principle `{}` defaults for unknown stack `{}` (known: {:?})",
                    p.id,
                    s,
                    valid
                );
            }
        }
    }

    #[test]
    fn no_principle_has_empty_required_text_fields() {
        let table = load().unwrap();
        for p in &table.principles {
            for (field, value) in [
                ("name", &p.name),
                ("summary", &p.summary),
                ("when_to_apply", &p.when_to_apply),
                ("when_not_to_apply", &p.when_not_to_apply),
                ("example", &p.example),
            ] {
                assert!(!value.trim().is_empty(), "principle `{}` has empty {field}", p.id);
            }
        }
    }

    #[test]
    fn signature_principles_are_present() {
        let table = load().unwrap();
        let ids: std::collections::HashSet<&str> =
            table.principles.iter().map(|p| p.id.as_str()).collect();
        for expected in [
            "solid",
            "dry",
            "kiss",
            "yagni",
            "clean-architecture",
            "idempotence",
            // CR.2.1 additions:
            "meaningful-names",
            "small-functions",
            "single-level-of-abstraction",
            "tell-dont-ask",
            "command-query-separation",
            "pure-functions",
            "no-magic-numbers",
            "consistency-with-existing-code",
        ] {
            assert!(ids.contains(expected), "v1 catalog missing `{expected}`");
        }
    }

    #[test]
    fn consistency_with_existing_code_is_universal() {
        // The brownfield deference rule must default for every seeded
        // stack — it's the meta-principle that prevents universal
        // principles from steamrolling existing conventions.
        let p = find("consistency-with-existing-code").expect("must exist");
        for stack in ["py-fastapi", "ts-next", "rust-axum", "go-echo"] {
            assert!(
                p.default_for_stacks.iter().any(|s| s == stack),
                "consistency-with-existing-code must default for `{stack}` (brownfield deference)"
            );
        }
    }
}
