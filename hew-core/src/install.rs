//! Write skill files into an agent runtime's directory layout.
//!
//! Each adapter knows where its runtime expects skills. The init flow
//! resolves which adapter to use, then hands it `(target_root, skills)`.
//! Only the Claude adapter is implemented at v0.1; the others stub out
//! cleanly so their dedicated tasks can fill them in without changing
//! the public surface.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{HewError, Result};
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
        Runtime::Cursor => return not_yet(runtime, "hew-3xq.3.2"),
        Runtime::Codex => return not_yet(runtime, "hew-3xq.3.3"),
        Runtime::Windsurf => return not_yet(runtime, "hew-3xq.3.4"),
        Runtime::Generic => write_generic_claude_md(root)?,
    };
    Ok(InstallPlan { runtime, root: root.to_path_buf(), written })
}

fn not_yet(runtime: Runtime, tracked_id: &str) -> Result<InstallPlan> {
    Err(HewError::MissingFlag {
        flag: format!(
            "runtime `{}` adapter not yet implemented (tracked: {})",
            runtime.as_str(),
            tracked_id
        ),
    })
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

    Ok(written)
}

fn write_generic_claude_md(root: &Path) -> Result<Vec<PathBuf>> {
    // Bundle every skill body into one CLAUDE.md as fallback.
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
    let path = root.join("CLAUDE.md");
    fs::write(&path, buf)?;
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
    fn install_claude_writes_every_skill() {
        let tmp = tempfile::tempdir().unwrap();
        let plan = install(Runtime::Claude, tmp.path()).expect("install");
        assert_eq!(plan.runtime, Runtime::Claude);
        // 1 SKILL.md + 14 skills.
        assert_eq!(plan.written.len(), 15);
        let hew_root = tmp.path().join(".claude").join("skills").join("hew");
        assert!(hew_root.join("SKILL.md").exists());
        assert!(hew_root.join("core").join("hew-execute.md").exists());
        assert!(hew_root.join("brownfield").join("hew-scan.md").exists());
        assert!(hew_root.join("optional").join("hew-quick.md").exists());
        assert!(hew_root.join("custom").exists(), "custom/ dir reserved for team skills");
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
    fn install_other_runtimes_errors_with_tracked_id() {
        let tmp = tempfile::tempdir().unwrap();
        let err = install(Runtime::Cursor, tmp.path()).expect_err("cursor not yet");
        assert!(err.to_string().contains("hew-3xq.3.2"));
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
