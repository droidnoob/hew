//! Compile-time stack-conventions table for `hew-new-project`.
//!
//! `hew-new-project` detects the chosen tech stack and writes one
//! `CONVENTION:<key>` memory per entry under the matching `stack_id`.
//! The source TOML lives at `<repo>/skills/data/stack-conventions.toml`
//! and is embedded via `include_str!` so the binary ships with the
//! seed set baked in.
//!
//! Two contracts live here:
//!
//! 1. Stable agent-facing shapes: [`Stack`] + [`StackConvention`] are
//!    schemars-derived. Adding fields is fine; renaming or removing
//!    breaks the contract.
//! 2. Drift safety: [`load`] is total — the embedded TOML must parse
//!    or `cargo test` fails (`stacks_table_parses`).

use serde::{Deserialize, Serialize};

const EMBEDDED: &str = include_str!("../../skills/data/stack-conventions.toml");

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct StackTable {
    pub stacks: Vec<Stack>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct Stack {
    pub stack_id: String,
    pub language: String,
    pub framework: String,
    pub conventions: Vec<StackConvention>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct StackConvention {
    pub key: String,
    pub body: String,
}

/// Parse the embedded stack-conventions table.
///
/// Returns an `Err(toml::de::Error)` only if the table file is
/// malformed; in practice the build fails before this can happen at
/// runtime because the drift test [`stacks_table_parses`] runs on
/// every `cargo test`.
pub fn load() -> Result<StackTable, toml::de::Error> {
    toml::from_str(EMBEDDED)
}

/// Look up a stack by id. Returns `None` if not seeded.
pub fn find(stack_id: &str) -> Option<Stack> {
    load().ok().and_then(|t| t.stacks.into_iter().find(|s| s.stack_id == stack_id))
}

/// List every seeded stack id (for picker UIs and CLI help).
pub fn ids() -> Vec<String> {
    load().map(|t| t.stacks.into_iter().map(|s| s.stack_id).collect()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn embedded_table_parses() {
        let t = load().expect("embedded stack-conventions.toml must parse");
        assert!(!t.stacks.is_empty(), "table must have at least one stack");
    }

    #[test]
    fn seed_stacks_are_present() {
        let table = load().unwrap();
        let ids: HashSet<&str> = table.stacks.iter().map(|s| s.stack_id.as_str()).collect();
        for expected in ["ts-next", "py-fastapi", "rust-axum", "go-echo"] {
            assert!(ids.contains(expected), "missing seed stack `{expected}`");
        }
    }

    #[test]
    fn stack_ids_are_unique() {
        let table = load().unwrap();
        let mut seen: HashSet<&str> = HashSet::new();
        for s in &table.stacks {
            assert!(seen.insert(s.stack_id.as_str()), "duplicate stack_id `{}`", s.stack_id);
        }
    }

    #[test]
    fn convention_keys_are_unique_within_each_stack() {
        let table = load().unwrap();
        for stack in &table.stacks {
            let mut seen: HashSet<&str> = HashSet::new();
            for c in &stack.conventions {
                assert!(
                    seen.insert(c.key.as_str()),
                    "stack `{}` has duplicate convention key `{}`",
                    stack.stack_id,
                    c.key
                );
            }
        }
    }

    #[test]
    fn every_stack_has_at_least_three_conventions() {
        let table = load().unwrap();
        for stack in &table.stacks {
            assert!(
                stack.conventions.len() >= 3,
                "stack `{}` has too few conventions ({}); seed needs >= 3 to be useful",
                stack.stack_id,
                stack.conventions.len(),
            );
        }
    }

    #[test]
    fn no_convention_body_is_empty() {
        let table = load().unwrap();
        for stack in &table.stacks {
            for c in &stack.conventions {
                assert!(
                    !c.body.trim().is_empty(),
                    "stack `{}` key `{}` has empty body",
                    stack.stack_id,
                    c.key,
                );
            }
        }
    }

    #[test]
    fn find_returns_seeded_stack() {
        let s = find("py-fastapi").expect("py-fastapi should be seeded");
        assert_eq!(s.language, "Python 3.12+");
    }

    #[test]
    fn find_returns_none_for_unknown_stack() {
        assert!(find("klingon-rocket-on-rails").is_none());
    }

    #[test]
    fn ids_lists_all_seeds() {
        let v = ids();
        assert!(v.contains(&"ts-next".to_string()));
        assert!(v.contains(&"py-fastapi".to_string()));
        assert!(v.contains(&"rust-axum".to_string()));
        assert!(v.contains(&"go-echo".to_string()));
    }
}
