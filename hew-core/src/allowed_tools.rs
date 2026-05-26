//! Per-skill tool allowlist for `--allowedTools` assembly in `hew loop`.
//!
//! Each skill body may declare its tool surface in frontmatter:
//!
//! ```yaml
//! tools: [Read, Edit, Write, Grep, Glob, Bash(cargo:*), Bash(git:*)]
//! ```
//!
//! Skills that omit the line inherit [`DEFAULT_ALLOWED_TOOLS`]. The
//! resolver returns the declared list verbatim (no merging with the
//! default — explicit declarations are total, so a skill can deny
//! itself shell access by declaring an empty list… but the parser
//! rejects empty `[]` so the path is "declare what you need, or omit
//! to inherit").

use crate::skills;

/// Conservative default surface for a skill that doesn't declare its
/// own. Covers read/edit/write file ops + scoped Bash invocations the
/// hew workflow needs (cargo, git, hew, bd, grep, find). Notably
/// excludes unscoped `Bash` — any skill that needs broader shell must
/// say so.
pub const DEFAULT_ALLOWED_TOOLS: &[&str] = &[
    "Read",
    "Edit",
    "Write",
    "Grep",
    "Glob",
    "Bash(cargo:*)",
    "Bash(git:*)",
    "Bash(hew:*)",
    "Bash(bd:*)",
    "Bash(grep:*)",
    "Bash(find:*)",
    "Bash(ls:*)",
    "Bash(cat:*)",
    "Bash(rg:*)",
    "TodoWrite",
];

/// Resolve the tool allowlist for a skill by canonical name. Returns
/// the skill's declared list if any, otherwise [`DEFAULT_ALLOWED_TOOLS`]
/// as owned strings. Unknown skill names get the default.
pub fn for_skill(name: &str) -> Vec<String> {
    if let Some(s) = skills::find(name)
        && let Some(declared) = s.declared_tools()
    {
        return declared;
    }
    DEFAULT_ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect()
}

/// Format an allowlist as a comma-joined string suitable for Claude
/// Code's `--allowedTools` flag.
pub fn format(tools: &[String]) -> String {
    tools.join(",")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_skill_gets_default() {
        let tools = for_skill("not-a-skill");
        assert_eq!(tools.len(), DEFAULT_ALLOWED_TOOLS.len());
        assert!(tools.iter().any(|t| t == "Read"));
    }

    #[test]
    fn default_excludes_unscoped_bash() {
        assert!(!DEFAULT_ALLOWED_TOOLS.contains(&"Bash"));
    }

    #[test]
    fn default_includes_core_file_ops() {
        for t in ["Read", "Edit", "Write", "Grep", "Glob"] {
            assert!(DEFAULT_ALLOWED_TOOLS.contains(&t), "default missing {t}");
        }
    }

    #[test]
    fn default_includes_scoped_bash_for_hew_workflow() {
        for t in ["Bash(cargo:*)", "Bash(git:*)", "Bash(hew:*)", "Bash(bd:*)"] {
            assert!(DEFAULT_ALLOWED_TOOLS.contains(&t), "default missing {t}");
        }
    }

    #[test]
    fn format_joins_with_commas() {
        let tools = vec!["Read".to_string(), "Edit".to_string(), "Bash(cargo:*)".to_string()];
        assert_eq!(format(&tools), "Read,Edit,Bash(cargo:*)");
    }

    #[test]
    fn known_skill_without_declaration_gets_default() {
        // hew-execute doesn't currently declare tools — should fall through.
        let tools = for_skill("hew-execute");
        assert_eq!(tools.len(), DEFAULT_ALLOWED_TOOLS.len());
    }
}
