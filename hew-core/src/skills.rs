//! Compile-time registry of every skill shipped with this binary.
//!
//! Skill source markdown lives at `<repo>/skills/` and is embedded via
//! `include_str!`. `hew init` writes these out to the target runtime's
//! skill directory; `hew prime` injects the relevant skill body into
//! the agent's context.

use std::fmt;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Category {
    Core,
    Brownfield,
    Optional,
    Index,
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Core => "core",
            Self::Brownfield => "brownfield",
            Self::Optional => "optional",
            Self::Index => "index",
        };
        f.write_str(s)
    }
}

#[derive(Copy, Clone, Debug)]
pub struct Skill {
    /// Canonical name (matches the markdown file stem, e.g. `hew-execute`).
    pub name: &'static str,
    /// Relative path under the installed skill root (e.g. `core/hew-execute.md`).
    pub relative_path: &'static str,
    pub category: Category,
    /// Raw markdown body, embedded at compile time.
    pub body: &'static str,
}

impl Skill {
    /// Parse the `<!-- hew:version=... -->` line that every skill carries.
    pub fn version(&self) -> Option<&str> {
        let first = self.body.lines().next()?.trim();
        let inner = first.strip_prefix("<!--")?.strip_suffix("-->")?.trim();
        let rest = inner.strip_prefix("hew:version=")?;
        Some(rest.trim())
    }
}

macro_rules! skill {
    ($name:expr, $cat:expr, $relpath:expr) => {
        Skill {
            name: $name,
            relative_path: $relpath,
            category: $cat,
            body: include_str!(concat!("../../skills/", $relpath)),
        }
    };
}

pub const INDEX: Skill = skill!("SKILL", Category::Index, "SKILL.md");

pub const CORE: &[Skill] = &[
    skill!("hew-plan", Category::Core, "core/hew-plan.md"),
    skill!("hew-decompose", Category::Core, "core/hew-decompose.md"),
    skill!("hew-execute", Category::Core, "core/hew-execute.md"),
    skill!("hew-verify", Category::Core, "core/hew-verify.md"),
    skill!("hew-guard", Category::Core, "core/hew-guard.md"),
    skill!("hew-checkpoint", Category::Core, "core/hew-checkpoint.md"),
];

pub const BROWNFIELD: &[Skill] = &[
    skill!("hew-scan", Category::Brownfield, "brownfield/hew-scan.md"),
    skill!("hew-convention", Category::Brownfield, "brownfield/hew-convention.md"),
    skill!("hew-audit", Category::Brownfield, "brownfield/hew-audit.md"),
    skill!("hew-boundary", Category::Brownfield, "brownfield/hew-boundary.md"),
    skill!("hew-migrate", Category::Brownfield, "brownfield/hew-migrate.md"),
];

pub const OPTIONAL: &[Skill] = &[
    skill!("hew-deps", Category::Optional, "optional/hew-deps.md"),
    skill!("hew-research", Category::Optional, "optional/hew-research.md"),
    skill!("hew-quick", Category::Optional, "optional/hew-quick.md"),
    skill!("hew-security", Category::Optional, "optional/hew-security.md"),
];

/// All shipped skills, including the SKILL.md index.
pub fn all() -> Vec<Skill> {
    let mut v = Vec::with_capacity(1 + CORE.len() + BROWNFIELD.len() + OPTIONAL.len());
    v.push(INDEX);
    v.extend_from_slice(CORE);
    v.extend_from_slice(BROWNFIELD);
    v.extend_from_slice(OPTIONAL);
    v
}

/// Look up a skill by canonical name.
pub fn find(name: &str) -> Option<Skill> {
    all().into_iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ship_one_index_plus_fifteen_skills() {
        let all = all();
        assert_eq!(all.len(), 1 + 15, "expected SKILL.md + 15 skills, got {}", all.len());
    }

    #[test]
    fn category_counts_match_spec() {
        assert_eq!(CORE.len(), 6);
        assert_eq!(BROWNFIELD.len(), 5);
        assert_eq!(OPTIONAL.len(), 4);
    }

    #[test]
    fn every_skill_has_a_version_marker() {
        for s in all() {
            assert!(s.version().is_some(), "{} missing <!-- hew:version=... --> on line 1", s.name);
        }
    }

    #[test]
    fn every_skill_body_is_nonempty() {
        for s in all() {
            assert!(!s.body.trim().is_empty(), "{} has empty body", s.name);
        }
    }

    #[test]
    fn skill_names_are_unique() {
        let mut names: Vec<&str> = all().iter().map(|s| s.name).collect();
        names.sort();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate skill names detected");
    }

    #[test]
    fn relative_paths_are_unique() {
        let mut paths: Vec<&str> = all().iter().map(|s| s.relative_path).collect();
        paths.sort();
        let before = paths.len();
        paths.dedup();
        assert_eq!(before, paths.len());
    }

    #[test]
    fn find_returns_known_skill() {
        let s = find("hew-execute").expect("hew-execute must exist");
        assert_eq!(s.category, Category::Core);
        assert!(s.body.contains("hew-execute"));
    }

    #[test]
    fn find_returns_none_for_unknown() {
        assert!(find("hew-nope").is_none());
    }

    #[test]
    fn index_is_special_category() {
        assert_eq!(INDEX.category, Category::Index);
        assert_eq!(INDEX.name, "SKILL");
    }
}
