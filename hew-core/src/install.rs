//! Write skill files into an agent runtime's directory layout.
//!
//! Each adapter knows where its runtime expects skills. The init flow
//! resolves which adapter to use, then hands it `(target_root, skills)`.
//! Only the Claude adapter is implemented at v0.1; the others stub out
//! cleanly so their dedicated tasks can fill them in without changing
//! the public surface.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::skills::{self, Category, Skill};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Runtime {
    Claude,
    Cursor,
    Codex,
    Windsurf,
    Generic,
}

impl Runtime {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Cursor => "cursor",
            Self::Codex => "codex",
            Self::Windsurf => "windsurf",
            Self::Generic => "generic",
        }
    }

    /// Cheap marker file/directory that signals this runtime is present.
    pub fn marker(self) -> &'static str {
        match self {
            Self::Claude => ".claude",
            Self::Cursor => ".cursor",
            Self::Codex => ".codex",
            Self::Windsurf => ".windsurf",
            Self::Generic => "",
        }
    }
}

/// Inspect a project root for runtime markers. Returns every runtime found.
pub fn detect_runtimes(project_root: &Path) -> Vec<Runtime> {
    [Runtime::Claude, Runtime::Cursor, Runtime::Codex, Runtime::Windsurf]
        .into_iter()
        .filter(|r| project_root.join(r.marker()).exists())
        .collect()
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub runtime: Runtime,
    pub root: PathBuf,
    pub written: Vec<PathBuf>,
}

/// Write every shipped skill into the runtime's directory.
pub fn install(runtime: Runtime, root: &Path) -> Result<InstallPlan> {
    let written = match runtime {
        Runtime::Claude => write_claude_layout(root)?,
        Runtime::Cursor => write_cursorrules(root)?,
        Runtime::Codex => write_codex_layout(root)?,
        Runtime::Windsurf => write_windsurfrules(root)?,
        Runtime::Generic => write_generic_claude_md(root)?,
    };
    Ok(InstallPlan { runtime, root: root.to_path_buf(), written })
}

fn write_claude_layout(root: &Path) -> Result<Vec<PathBuf>> {
    let base = root.join(".claude").join("skills").join("hew");
    for sub in ["core", "brownfield", "optional", "custom"] {
        fs::create_dir_all(base.join(sub))?;
    }

    let mut written = Vec::new();

    // SKILL.md at the hew root.
    let index_path = base.join("SKILL.md");
    fs::write(&index_path, skills::INDEX.body)?;
    written.push(index_path);

    for s in skills::all() {
        if s.category == Category::Index {
            continue;
        }
        let dest = base.join(category_dir(s.category)).join(file_name(&s));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&dest, s.body)?;
        written.push(dest);
    }

    // Slash commands at .claude/commands/hew/<name>.md so they become
    // /hew:<name> inside Claude Code.
    let commands_dir = root.join(".claude").join("commands").join("hew");
    fs::create_dir_all(&commands_dir)?;
    for c in crate::slash::ALL {
        let dest = commands_dir.join(format!("{}.md", c.name));
        fs::write(&dest, c.body)?;
        written.push(dest);
    }

    Ok(written)
}

/// Marker lines surrounding the hew section in single-file runtime configs.
const SECTION_START: &str =
    "<!-- HEW:BEGIN — do not edit between the markers; managed by `hew install` -->";
const SECTION_END: &str = "<!-- HEW:END -->";

fn bundle_all_skills() -> String {
    let mut buf = String::new();
    buf.push_str(skills::INDEX.body);
    buf.push_str("\n\n---\n\n");
    for s in skills::all() {
        if s.category == Category::Index {
            continue;
        }
        buf.push_str("\n\n");
        buf.push_str(s.body);
        buf.push('\n');
    }
    buf
}

/// Inject (or replace) the hew section in a single-file rules document.
/// Idempotent: subsequent installs replace only the section between the
/// markers, preserving anything the user added outside.
fn upsert_marked_section(path: &Path, body: &str) -> Result<()> {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let new_section = format!("{SECTION_START}\n{body}\n{SECTION_END}\n");
    let updated = if let (Some(start), Some(end)) =
        (existing.find(SECTION_START), existing.find(SECTION_END))
    {
        let end_with_marker = end + SECTION_END.len();
        // Trim a trailing newline after the end marker, if present, then re-add ours.
        let mut next = String::with_capacity(existing.len() + new_section.len());
        next.push_str(&existing[..start]);
        next.push_str(&new_section);
        // Skip past the original end marker + any single trailing newline.
        let remainder = &existing[end_with_marker..];
        let remainder = remainder.strip_prefix('\n').unwrap_or(remainder);
        next.push_str(remainder);
        next
    } else {
        let mut next = existing;
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        if !next.is_empty() {
            next.push('\n');
        }
        next.push_str(&new_section);
        next
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, updated)?;
    Ok(())
}

fn write_cursorrules(root: &Path) -> Result<Vec<PathBuf>> {
    let path = root.join(".cursorrules");
    upsert_marked_section(&path, &bundle_all_skills())?;
    Ok(vec![path])
}

fn write_windsurfrules(root: &Path) -> Result<Vec<PathBuf>> {
    let path = root.join(".windsurfrules");
    upsert_marked_section(&path, &bundle_all_skills())?;
    Ok(vec![path])
}

/// Codex adapter: drop each skill into `.codex/agents/hew-<name>.toml`
/// with the minimal frontmatter Codex expects, plus an AGENTS.md
/// pointer at the project root.
fn write_codex_layout(root: &Path) -> Result<Vec<PathBuf>> {
    let agents_dir = root.join(".codex").join("agents");
    fs::create_dir_all(&agents_dir)?;
    let mut written = Vec::new();

    for s in skills::all() {
        if s.category == Category::Index {
            continue;
        }
        let toml = format!(
            "name = \"{}\"\ncategory = \"{}\"\n\nbody = \"\"\"\n{}\n\"\"\"\n",
            s.name,
            s.category,
            s.body.replace("\"\"\"", "\\\"\\\"\\\""),
        );
        let dest = agents_dir.join(format!("{}.toml", s.name));
        fs::write(&dest, toml)?;
        written.push(dest);
    }

    // AGENTS.md at project root as a pointer + bundled body for Codex
    // builds that read AGENTS.md directly.
    let agents_md = root.join("AGENTS.md");
    upsert_marked_section(&agents_md, &bundle_all_skills())?;
    written.push(agents_md);

    Ok(written)
}

fn write_generic_claude_md(root: &Path) -> Result<Vec<PathBuf>> {
    let path = root.join("CLAUDE.md");
    fs::write(&path, bundle_all_skills())?;
    Ok(vec![path])
}

fn category_dir(c: Category) -> &'static str {
    match c {
        Category::Core => "core",
        Category::Brownfield => "brownfield",
        Category::Optional => "optional",
        Category::Index => "",
    }
}

fn file_name(s: &Skill) -> String {
    // The registry already stores e.g. `core/hew-execute.md`. Strip the prefix.
    s.relative_path.rsplit('/').next().unwrap_or(s.relative_path).to_string()
}

/// Ensure `.beads/` is listed in the project `.gitignore`. Idempotent.
pub fn ensure_beads_gitignored(project_root: &Path) -> Result<bool> {
    let path = project_root.join(".gitignore");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if existing.lines().any(|l| l.trim() == ".beads/" || l.trim() == ".beads") {
        return Ok(false);
    }
    let mut new = existing;
    if !new.is_empty() && !new.ends_with('\n') {
        new.push('\n');
    }
    new.push_str(".beads/\n");
    fs::write(&path, new)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_finds_known_markers() {
        let tmp = tempfile::tempdir().unwrap();
        fs::create_dir(tmp.path().join(".claude")).unwrap();
        fs::create_dir(tmp.path().join(".cursor")).unwrap();
        let found = detect_runtimes(tmp.path());
        assert!(found.contains(&Runtime::Claude));
        assert!(found.contains(&Runtime::Cursor));
        assert!(!found.contains(&Runtime::Codex));
    }

    #[test]
    fn install_claude_writes_every_skill_and_slash_command() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = install(Runtime::Claude, tmp.path()).expect("install");
        assert_eq!(plan.runtime, Runtime::Claude);
        // 1 SKILL.md + 14 skills + 23 slash commands = 38 files.
        assert_eq!(plan.written.len(), 38);

        let hew_root = tmp.path().join(".claude").join("skills").join("hew");
        assert!(hew_root.join("SKILL.md").exists());
        assert!(hew_root.join("core").join("hew-execute.md").exists());
        assert!(hew_root.join("brownfield").join("hew-scan.md").exists());
        assert!(hew_root.join("optional").join("hew-quick.md").exists());
        assert!(hew_root.join("custom").exists(), "custom/ dir reserved for team skills");

        // Slash commands.
        let cmd_root = tmp.path().join(".claude").join("commands").join("hew");
        for name in ["do", "next", "auto", "plan", "execute-loop", "doctor"]
            .iter()
            .filter(|n| **n != "execute-loop")
        {
            assert!(cmd_root.join(format!("{name}.md")).exists(), "/{}", name);
        }
        // Spot-check that a known command body landed verbatim.
        let plan_body = fs::read_to_string(cmd_root.join("plan.md")).unwrap();
        assert!(plan_body.contains("hew-plan skill"));
    }

    #[test]
    fn install_generic_writes_single_claude_md() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = install(Runtime::Generic, tmp.path()).expect("install");
        assert_eq!(plan.written.len(), 1);
        let body = fs::read_to_string(tmp.path().join("CLAUDE.md")).unwrap();
        assert!(body.contains("hew-execute"));
        assert!(body.contains("hew-scan"));
    }

    #[test]
    fn install_cursor_creates_marked_cursorrules() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = install(Runtime::Cursor, tmp.path()).expect("install");
        assert_eq!(plan.written.len(), 1);
        let body = fs::read_to_string(tmp.path().join(".cursorrules")).unwrap();
        assert!(body.contains("HEW:BEGIN"));
        assert!(body.contains("HEW:END"));
        assert!(body.contains("hew-execute"));
    }

    #[test]
    fn install_cursor_is_idempotent_and_preserves_user_content() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join(".cursorrules");
        fs::write(&path, "# user content\n\nkeep me\n").unwrap();

        install(Runtime::Cursor, tmp.path()).unwrap();
        let first = fs::read_to_string(&path).unwrap();
        install(Runtime::Cursor, tmp.path()).unwrap();
        let second = fs::read_to_string(&path).unwrap();

        assert_eq!(first, second, "second install must be idempotent");
        assert!(second.contains("keep me"), "user content preserved");
        assert_eq!(
            second.matches("HEW:BEGIN").count(),
            1,
            "exactly one hew section even after re-install"
        );
    }

    #[test]
    fn install_windsurf_writes_marked_windsurfrules() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = install(Runtime::Windsurf, tmp.path()).expect("install");
        assert_eq!(plan.written.len(), 1);
        let body = fs::read_to_string(tmp.path().join(".windsurfrules")).unwrap();
        assert!(body.contains("HEW:BEGIN"));
        assert!(body.contains("hew-plan"));
    }

    #[test]
    fn install_codex_writes_per_skill_toml_and_agents_md() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = install(Runtime::Codex, tmp.path()).expect("install");
        // 14 skills + AGENTS.md
        assert_eq!(plan.written.len(), 15);
        let agents = tmp.path().join(".codex").join("agents");
        assert!(agents.join("hew-execute.toml").exists());
        assert!(agents.join("hew-scan.toml").exists());
        let body = fs::read_to_string(agents.join("hew-execute.toml")).unwrap();
        assert!(body.starts_with("name = \"hew-execute\""));
        assert!(body.contains("category = \"core\""));
        assert!(body.contains("hew-execute")); // body field present
        assert!(tmp.path().join("AGENTS.md").exists());
    }

    #[test]
    fn gitignore_added_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let changed = ensure_beads_gitignored(tmp.path()).unwrap();
        assert!(changed);
        let body = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert!(body.contains(".beads/"));
    }

    #[test]
    fn gitignore_noop_when_already_present() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".gitignore"), "foo\n.beads/\nbar\n").unwrap();
        let changed = ensure_beads_gitignored(tmp.path()).unwrap();
        assert!(!changed);
    }

    #[test]
    fn gitignore_appends_with_proper_newline() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join(".gitignore"), "foo").unwrap(); // no trailing newline
        ensure_beads_gitignored(tmp.path()).unwrap();
        let body = fs::read_to_string(tmp.path().join(".gitignore")).unwrap();
        assert_eq!(body, "foo\n.beads/\n");
    }
}
