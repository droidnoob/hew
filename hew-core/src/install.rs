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

#[derive(Debug, Clone)]
pub struct UninstallPlan {
    pub runtime: Runtime,
    pub root: PathBuf,
    pub removed: Vec<PathBuf>,
}

/// Reverse what `install` wrote. Idempotent: missing paths are no-ops.
/// Never touches `.beads/`, `.gitignore`, or any file not owned by hew.
pub fn uninstall(runtime: Runtime, root: &Path) -> Result<UninstallPlan> {
    let removed = match runtime {
        Runtime::Claude => uninstall_claude(root)?,
        Runtime::Cursor => uninstall_single_file(root, ".cursorrules")?,
        Runtime::Windsurf => uninstall_single_file(root, ".windsurfrules")?,
        Runtime::Codex => uninstall_codex(root)?,
        Runtime::Generic => uninstall_generic(root)?,
    };
    Ok(UninstallPlan { runtime, root: root.to_path_buf(), removed })
}

fn uninstall_claude(root: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();
    let skills_dir = root.join(".claude").join("skills").join("hew");
    if skills_dir.exists() {
        fs::remove_dir_all(&skills_dir)?;
        removed.push(skills_dir);
    }
    let commands_dir = root.join(".claude").join("commands").join("hew");
    if commands_dir.exists() {
        fs::remove_dir_all(&commands_dir)?;
        removed.push(commands_dir);
    }
    if let Some(settings) = remove_claude_session_hook(&root.join(".claude"))? {
        removed.push(settings.clone());
        // Same file — `remove_claude_allowlist` will rewrite it in place
        // (or remove it if both helpers ended up emptying it).
    }
    if let Some(settings) = remove_claude_allowlist(&root.join(".claude"))?
        && !removed.contains(&settings)
    {
        removed.push(settings);
    }
    Ok(removed)
}

fn uninstall_single_file(root: &Path, name: &str) -> Result<Vec<PathBuf>> {
    let path = root.join(name);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let stripped = remove_marked_section(&fs::read_to_string(&path)?);
    if stripped.trim().is_empty() {
        // File only contained our section — remove the file entirely.
        fs::remove_file(&path)?;
    } else {
        fs::write(&path, stripped)?;
    }
    Ok(vec![path])
}

fn uninstall_codex(root: &Path) -> Result<Vec<PathBuf>> {
    let mut removed = Vec::new();

    // Agent roles under .codex/agents/.
    let agents_dir = root.join(".codex").join("agents");
    if agents_dir.exists() {
        for entry in fs::read_dir(&agents_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("hew-") && name.ends_with(".toml") {
                fs::remove_file(entry.path())?;
                removed.push(entry.path());
            }
        }
        if fs::read_dir(&agents_dir)?.next().is_none() {
            let _ = fs::remove_dir(&agents_dir);
        }
    }

    // Skills under .agents/skills/hew-*/. Remove each hew-* directory
    // entirely. Leave non-hew skills and the parent dirs alone unless
    // they end up empty.
    let skills_dir = root.join(".agents").join("skills");
    if skills_dir.exists() {
        for entry in fs::read_dir(&skills_dir)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            let path = entry.path();
            if name.starts_with("hew-") && path.is_dir() {
                fs::remove_dir_all(&path)?;
                removed.push(path);
            }
        }
        if fs::read_dir(&skills_dir)?.next().is_none() {
            let _ = fs::remove_dir(&skills_dir);
            let parent = root.join(".agents");
            if parent.exists() && fs::read_dir(&parent)?.next().is_none() {
                let _ = fs::remove_dir(&parent);
            }
        }
    }

    // Strip section from AGENTS.md (or delete if only hew owned it).
    removed.extend(uninstall_single_file(root, "AGENTS.md")?);
    Ok(removed)
}

fn uninstall_generic(root: &Path) -> Result<Vec<PathBuf>> {
    // Generic install overwrites CLAUDE.md wholesale (no marker section).
    // Only safe action is to delete if the body matches what we'd write.
    // Otherwise leave it for the user — never clobber unknown content.
    let path = root.join("CLAUDE.md");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let body = fs::read_to_string(&path)?;
    if body == bundle_all_skills() {
        fs::remove_file(&path)?;
        return Ok(vec![path]);
    }
    // User modified it — leave alone.
    Ok(Vec::new())
}

/// Strip the HEW:BEGIN/END marker section from a file's contents.
/// Returns the remaining content (caller decides whether to write or delete).
fn remove_marked_section(existing: &str) -> String {
    let Some(start) = existing.find(SECTION_START) else {
        return existing.to_string();
    };
    let Some(end) = existing.find(SECTION_END) else {
        return existing.to_string();
    };
    let end_with_marker = end + SECTION_END.len();
    let mut next = String::with_capacity(existing.len());
    next.push_str(&existing[..start]);
    // Skip the section + a single trailing newline.
    let remainder = &existing[end_with_marker..];
    let remainder = remainder.strip_prefix('\n').unwrap_or(remainder);
    next.push_str(remainder);
    // Trim two-or-more trailing newlines that the removal may leave behind.
    while next.ends_with("\n\n") {
        next.pop();
    }
    next
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

    // SessionStart hook so the agent auto-restores hew context on every
    // new session (post-/clear, post-compact, fresh shell).
    let settings = upsert_claude_session_hook(&root.join(".claude"))?;
    written.push(settings.clone());

    // Permissions allowlist so Claude Code doesn't prompt on every bd /
    // hew / safe-git invocation. Same settings.json file — but record it
    // only once in the `written` list.
    let allow_settings = upsert_claude_allowlist(&root.join(".claude"))?;
    if !written.contains(&allow_settings) {
        written.push(allow_settings);
    }

    Ok(written)
}

/// Matcher that fires on every session-entry source Claude Code emits:
/// startup, resume, and clear. (`compact` is handled by its own hook event,
/// not SessionStart.)
const SESSION_HOOK_MATCHER: &str = "startup|resume|clear";
const SESSION_HOOK_COMMAND: &str = "hew prime resume";
/// Discriminator key on the matcher-level entry. On re-install we drop any
/// SessionStart entry carrying this flag, then push a fresh one — so the
/// hook is exactly-once even after repeated `hew init` runs.
const HEW_MANAGED_FLAG: &str = "hew_managed";

/// Inject (or replace) the hew SessionStart hook in `<claude_dir>/settings.json`.
/// Preserves every other top-level key, every non-hew hook entry, and every
/// other hook event. Refuses to clobber malformed JSON.
fn upsert_claude_session_hook(claude_dir: &Path) -> Result<PathBuf> {
    let settings_path = claude_dir.join("settings.json");
    let mut value: serde_json::Value = if settings_path.exists() {
        let body = fs::read_to_string(&settings_path)?;
        if body.trim().is_empty() {
            serde_json::Value::Object(serde_json::Map::new())
        } else {
            serde_json::from_str(&body).map_err(|e| crate::error::HewError::SettingsMalformed {
                path: settings_path.display().to_string(),
                reason: e.to_string(),
            })?
        }
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    let root = value.as_object_mut().ok_or_else(|| crate::error::HewError::SettingsMalformed {
        path: settings_path.display().to_string(),
        reason: "top-level value must be a JSON object".to_string(),
    })?;

    let hooks_entry = root
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let hooks =
        hooks_entry.as_object_mut().ok_or_else(|| crate::error::HewError::SettingsMalformed {
            path: settings_path.display().to_string(),
            reason: "`hooks` must be a JSON object".to_string(),
        })?;

    let session_entry = hooks
        .entry("SessionStart".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let arr =
        session_entry.as_array_mut().ok_or_else(|| crate::error::HewError::SettingsMalformed {
            path: settings_path.display().to_string(),
            reason: "`hooks.SessionStart` must be an array".to_string(),
        })?;

    arr.retain(|v| !v.get(HEW_MANAGED_FLAG).and_then(|f| f.as_bool()).unwrap_or(false));
    arr.push(serde_json::json!({
        "matcher": SESSION_HOOK_MATCHER,
        HEW_MANAGED_FLAG: true,
        "hooks": [
            { "type": "command", "command": SESSION_HOOK_COMMAND }
        ]
    }));

    fs::create_dir_all(claude_dir)?;
    let mut body = serde_json::to_string_pretty(&value)?;
    body.push('\n');
    fs::write(&settings_path, body)?;
    Ok(settings_path)
}

/// Reverse `upsert_claude_session_hook`. Returns `Some(path)` if anything
/// changed on disk; `None` otherwise. Tidies empty containers but never
/// touches non-hew entries.
fn remove_claude_session_hook(claude_dir: &Path) -> Result<Option<PathBuf>> {
    let settings_path = claude_dir.join("settings.json");
    if !settings_path.exists() {
        return Ok(None);
    }
    let body = fs::read_to_string(&settings_path)?;
    if body.trim().is_empty() {
        return Ok(None);
    }
    let mut value: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| crate::error::HewError::SettingsMalformed {
            path: settings_path.display().to_string(),
            reason: e.to_string(),
        })?;

    let Some(root) = value.as_object_mut() else {
        return Ok(None);
    };
    let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return Ok(None);
    };
    let Some(arr) = hooks.get_mut("SessionStart").and_then(|s| s.as_array_mut()) else {
        return Ok(None);
    };

    let before = arr.len();
    arr.retain(|v| !v.get(HEW_MANAGED_FLAG).and_then(|f| f.as_bool()).unwrap_or(false));
    if arr.len() == before {
        return Ok(None);
    }
    if arr.is_empty() {
        hooks.remove("SessionStart");
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }

    if root.is_empty() {
        fs::remove_file(&settings_path)?;
        return Ok(Some(settings_path));
    }

    let mut new_body = serde_json::to_string_pretty(&value)?;
    new_body.push('\n');
    fs::write(&settings_path, new_body)?;
    Ok(Some(settings_path))
}

/// Permission entries injected into `.claude/settings.json` so Claude Code
/// doesn't prompt on every routine `bd` / `hew` / safe-git invocation. See
/// `DECISION:claude-allowlist-scope` memory.
///
/// Excluded by design: `git push`, `git reset --hard`, `git clean -f`,
/// `git rebase`, any force-flagged operation. Those keep their user prompt.
const HEW_ALLOWLIST_ENTRIES: &[&str] = &[
    "Bash(bd:*)",
    "Bash(hew:*)",
    "Bash(git status:*)",
    "Bash(git diff:*)",
    "Bash(git log:*)",
    "Bash(git show:*)",
    "Bash(git branch:*)",
    "Bash(git add:*)",
    "Bash(git commit:*)",
    "Bash(git checkout:*)",
];

/// Sibling key under `permissions` that records which entries hew owns
/// in `allow`. Re-install drops only entries listed here, then re-adds
/// the current `HEW_ALLOWLIST_ENTRIES`. Foreign entries the user added
/// to `allow` are never touched.
const HEW_MANAGED_ALLOWLIST_KEY: &str = "allow_hew_managed";

/// Load (or initialize) the settings.json JSON value. Returns the parsed
/// value + a flag indicating whether the file existed on disk.
fn load_settings_json(path: &Path) -> Result<serde_json::Value> {
    if !path.exists() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    let body = fs::read_to_string(path)?;
    if body.trim().is_empty() {
        return Ok(serde_json::Value::Object(serde_json::Map::new()));
    }
    serde_json::from_str(&body).map_err(|e| crate::error::HewError::SettingsMalformed {
        path: path.display().to_string(),
        reason: e.to_string(),
    })
}

fn write_settings_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = serde_json::to_string_pretty(value)?;
    body.push('\n');
    fs::write(path, body)?;
    Ok(())
}

/// Insert (or replace) the hew-managed allowlist entries in
/// `<claude_dir>/settings.json`. Preserves all other `permissions` keys,
/// every other allow entry, and every other top-level setting. Idempotent.
fn upsert_claude_allowlist(claude_dir: &Path) -> Result<PathBuf> {
    let settings_path = claude_dir.join("settings.json");
    let mut value = load_settings_json(&settings_path)?;

    let root = value.as_object_mut().ok_or_else(|| crate::error::HewError::SettingsMalformed {
        path: settings_path.display().to_string(),
        reason: "top-level value must be a JSON object".to_string(),
    })?;

    let permissions_entry = root
        .entry("permissions".to_string())
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let permissions = permissions_entry.as_object_mut().ok_or_else(|| {
        crate::error::HewError::SettingsMalformed {
            path: settings_path.display().to_string(),
            reason: "`permissions` must be a JSON object".to_string(),
        }
    })?;

    // Read the prior hew-managed set so we know which entries to drop
    // from `allow` before adding the current set.
    let prior: Vec<String> = permissions
        .get(HEW_MANAGED_ALLOWLIST_KEY)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    let allow_entry = permissions
        .entry("allow".to_string())
        .or_insert_with(|| serde_json::Value::Array(Vec::new()));
    let allow =
        allow_entry.as_array_mut().ok_or_else(|| crate::error::HewError::SettingsMalformed {
            path: settings_path.display().to_string(),
            reason: "`permissions.allow` must be an array".to_string(),
        })?;

    // Drop prior-hew entries; preserve everything else.
    allow.retain(|v| match v.as_str() {
        Some(s) => !prior.iter().any(|p| p == s),
        None => true,
    });
    // Add current hew entries that aren't already present (defensive — a
    // user may have manually added one of ours).
    for entry in HEW_ALLOWLIST_ENTRIES {
        let s = (*entry).to_string();
        if !allow.iter().any(|v| v.as_str() == Some(entry)) {
            allow.push(serde_json::Value::String(s));
        }
    }

    permissions.insert(
        HEW_MANAGED_ALLOWLIST_KEY.to_string(),
        serde_json::Value::Array(
            HEW_ALLOWLIST_ENTRIES
                .iter()
                .map(|e| serde_json::Value::String((*e).to_string()))
                .collect(),
        ),
    );

    write_settings_json(&settings_path, &value)?;
    Ok(settings_path)
}

/// Reverse `upsert_claude_allowlist`. Removes only entries listed in the
/// `permissions.allow_hew_managed` sibling key; foreign entries survive.
/// Tidies empty containers; deletes the file if it becomes fully empty.
fn remove_claude_allowlist(claude_dir: &Path) -> Result<Option<PathBuf>> {
    let settings_path = claude_dir.join("settings.json");
    if !settings_path.exists() {
        return Ok(None);
    }
    let mut value = load_settings_json(&settings_path)?;
    let Some(root) = value.as_object_mut() else {
        return Ok(None);
    };
    let Some(permissions) = root.get_mut("permissions").and_then(|p| p.as_object_mut()) else {
        return Ok(None);
    };

    let prior: Vec<String> = permissions
        .get(HEW_MANAGED_ALLOWLIST_KEY)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    if prior.is_empty() {
        return Ok(None);
    }

    if let Some(allow) = permissions.get_mut("allow").and_then(|a| a.as_array_mut()) {
        allow.retain(|v| match v.as_str() {
            Some(s) => !prior.iter().any(|p| p == s),
            None => true,
        });
        if allow.is_empty() {
            permissions.remove("allow");
        }
    }
    permissions.remove(HEW_MANAGED_ALLOWLIST_KEY);

    if permissions.is_empty() {
        root.remove("permissions");
    }

    if root.is_empty() {
        fs::remove_file(&settings_path)?;
        return Ok(Some(settings_path));
    }

    write_settings_json(&settings_path, &value)?;
    Ok(Some(settings_path))
}

/// Marker lines surrounding the hew section in single-file runtime configs.
const SECTION_START: &str =
    "<!-- HEW:BEGIN — do not edit between the markers; managed by `hew install` -->";
const SECTION_END: &str = "<!-- HEW:END -->";

/// Top-of-bundle directive for runtimes without a SessionStart hook
/// equivalent (Cursor, Windsurf, Codex, Generic). Tells the agent to
/// run `hew prime resume` as its first action in every new session so
/// it inherits project state + the latest CHECKPOINT.
const RESUME_DIRECTIVE: &str = "\
# On every new session\n\
\n\
Run `hew prime resume` as your first action. It emits a JSON document with\n\
project state, STATUS flags, categorized memories, and the most recent\n\
CHECKPOINT. Read that before doing anything else — otherwise you are\n\
flying blind after `/clear`, session compaction, or a fresh shell.\n\
\n\
---\n\n";

fn bundle_all_skills() -> String {
    let mut buf = String::new();
    buf.push_str(RESUME_DIRECTIVE);
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

/// Codex adapter: writes three things side-by-side so Codex picks hew up
/// through every primitive it exposes.
///
/// 1. **Agent roles** at `.codex/agents/hew-<name>.toml` — `AgentRoleToml`
///    shape (`name` + `description` + `developer_instructions`). Used when
///    a parent agent spawns a hew persona as a sub-agent.
/// 2. **Skills** at `.agents/skills/hew-<name>/SKILL.md` — Codex's
///    auto-discovered skill primitive (YAML frontmatter `name` +
///    `description`, then the body). This is the canonical way for
///    users to invoke hew in Codex chat.
/// 3. **`AGENTS.md`** at the project root — bundled body for Codex builds
///    that read AGENTS.md directly.
fn write_codex_layout(root: &Path) -> Result<Vec<PathBuf>> {
    let mut written = Vec::new();

    // Agent roles.
    let agents_dir = root.join(".codex").join("agents");
    fs::create_dir_all(&agents_dir)?;
    for s in skills::all() {
        if s.category == Category::Index {
            continue;
        }
        let toml = render_codex_role(s.name, s.body);
        let dest = agents_dir.join(format!("{}.toml", s.name));
        fs::write(&dest, toml)?;
        written.push(dest);
    }

    // Skills (`.agents/skills/<name>/SKILL.md`).
    let skills_root = root.join(".agents").join("skills");
    for s in skills::all() {
        if s.category == Category::Index {
            continue;
        }
        let dir = skills_root.join(s.name);
        fs::create_dir_all(&dir)?;
        let path = dir.join("SKILL.md");
        fs::write(&path, render_codex_skill_md(s.name, s.body))?;
        written.push(path);
    }

    // AGENTS.md at project root as a pointer + bundled body for Codex
    // builds that read AGENTS.md directly.
    let agents_md = root.join("AGENTS.md");
    upsert_marked_section(&agents_md, &bundle_all_skills())?;
    written.push(agents_md);

    Ok(written)
}

/// Render a single Codex agent role TOML for the given skill.
///
/// Codex's `AgentRoleToml` requires three fields: `name`, `description`,
/// and `developer_instructions`. We derive `description` from the first
/// markdown H1 in the skill body (e.g. `# hew-execute — The Work Loop`),
/// falling back to the skill name. Optional fields like
/// `model_reasoning_effort` and `background_terminal_max_timeout` are
/// left to Codex's defaults.
///
/// TOML literal multi-line strings (`'''...'''`) skip backslash processing,
/// so regex chars like `\s` in skill bodies pass through untouched. The
/// only escape-sensitive edge is a literal `'''` in the body — we fall
/// back to a basic string with full escaping in that case.
fn render_codex_role(name: &str, body: &str) -> String {
    let description = codex_role_description(name, body);
    let header =
        format!("name = \"{name}\"\ndescription = \"{}\"\n", toml_basic_escape(&description),);
    if body.contains("'''") {
        // Defensive fallback. Escape backslashes and triple-double-quotes.
        let escaped = body.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"");
        format!("{header}developer_instructions = \"\"\"\n{escaped}\n\"\"\"\n")
    } else {
        format!("{header}developer_instructions = '''\n{body}\n'''\n")
    }
}

/// Pull the first `# ` heading out of a skill body and use it as the
/// Codex `description`. Fallback: the skill name.
fn codex_role_description(name: &str, body: &str) -> String {
    for line in body.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            return rest.trim().to_string();
        }
    }
    name.to_string()
}

/// Minimal escape for a TOML basic (single-line) string.
fn toml_basic_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Render a `SKILL.md` for Codex's skill discovery
/// (`.agents/skills/<name>/SKILL.md`).
///
/// Codex expects YAML frontmatter with at least `name` and `description`,
/// then the natural-language body. Our skill bodies already start with a
/// YAML frontmatter block (our own `name`/`category`/`init` keys); we
/// strip and replace it so Codex sees exactly the two fields it needs.
/// The rest of the body — including the `<!-- hew:version=... -->` line
/// above the frontmatter — is preserved.
fn render_codex_skill_md(name: &str, body: &str) -> String {
    let description = codex_role_description(name, body);
    let safe_desc = yaml_double_quoted_escape(&description);
    let frontmatter = format!("---\nname: {name}\ndescription: \"{safe_desc}\"\n---\n");
    let trimmed = strip_existing_frontmatter(body);
    format!("{frontmatter}{trimmed}")
}

/// Strip a leading YAML frontmatter block (`---\n...\n---\n`) from a
/// skill body. Handles an optional pre-frontmatter HTML comment line
/// (the `<!-- hew:version=X.Y.Z -->` marker every skill carries).
/// Returns the remaining body verbatim. If no frontmatter is found,
/// returns the body unchanged.
fn strip_existing_frontmatter(body: &str) -> String {
    let mut lines = body.lines().peekable();
    let mut prelude: Vec<&str> = Vec::new();

    // Allow a single HTML-comment line ahead of frontmatter — we drop it,
    // because Codex doesn't care about hew's version marker.
    if let Some(first) = lines.peek()
        && first.trim_start().starts_with("<!--")
        && first.trim_end().ends_with("-->")
    {
        lines.next();
    }

    // Look for the opening `---` fence.
    let Some(first) = lines.peek() else {
        return body.to_string();
    };
    if first.trim() != "---" {
        // No frontmatter — return body unchanged (minus the version comment).
        for line in lines {
            prelude.push(line);
        }
        return prelude.join("\n");
    }
    lines.next(); // consume opening `---`

    // Drop everything until the closing `---`.
    for line in lines.by_ref() {
        if line.trim() == "---" {
            break;
        }
    }

    // Collect the rest.
    let remaining: Vec<&str> = lines.collect();
    let mut out = remaining.join("\n");
    // Trim a leading blank line that often sits between frontmatter and body.
    if let Some(rest) = out.strip_prefix('\n') {
        out = rest.to_string();
    }
    out
}

/// Escape a value for embedding inside a YAML double-quoted scalar.
fn yaml_double_quoted_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
        // 1 SKILL.md + 20 skills + 39 slash commands + 1 settings.json = 61 files.
        assert_eq!(plan.written.len(), 61);

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
        // 20 agent roles + 20 SKILL.md skill files + AGENTS.md = 41 files.
        assert_eq!(plan.written.len(), 41);
        let agents = tmp.path().join(".codex").join("agents");
        assert!(agents.join("hew-execute.toml").exists());
        assert!(agents.join("hew-scan.toml").exists());
        assert!(tmp.path().join("AGENTS.md").exists());

        // Every emitted role TOML must be valid TOML and contain the
        // `developer_instructions` key Codex's AgentRoleToml expects.
        // Anything else regresses to the "unknown field `body`" bug.
        for entry in fs::read_dir(&agents).unwrap() {
            let path = entry.unwrap().path();
            let body = fs::read_to_string(&path).unwrap();
            let parsed: toml::Value = toml::from_str(&body)
                .unwrap_or_else(|e| panic!("{} is not valid TOML: {e}", path.display()));
            assert!(
                parsed.get("developer_instructions").is_some(),
                "{} missing developer_instructions key",
                path.display()
            );
            // `name` must be present and non-empty (Codex rejects empty names).
            let name = parsed
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_else(|| panic!("{} missing or non-string `name`", path.display()));
            assert!(!name.is_empty(), "{} has empty `name`", path.display());
            // Schema fields we must NOT emit (regression guard).
            assert!(parsed.get("body").is_none(), "{} has stale `body` field", path.display());
            assert!(
                parsed.get("category").is_none(),
                "{} has stale `category` field",
                path.display()
            );
        }
    }

    #[test]
    fn install_codex_writes_skill_md_per_skill() {
        let tmp = tempfile::tempdir().unwrap();
        install(Runtime::Codex, tmp.path()).expect("install");

        let skills_root = tmp.path().join(".agents").join("skills");
        let mut seen = 0usize;
        for s in skills::all() {
            if s.category == Category::Index {
                continue;
            }
            let path = skills_root.join(s.name).join("SKILL.md");
            assert!(path.exists(), "missing SKILL.md for {}", s.name);
            let body = fs::read_to_string(&path).unwrap();
            assert!(
                body.starts_with("---\n"),
                "{}: SKILL.md must start with YAML frontmatter, got:\n{body}",
                s.name
            );
            assert!(
                body.contains(&format!("name: {}", s.name)),
                "{}: SKILL.md missing `name:` key",
                s.name
            );
            assert!(
                body.contains("description: \""),
                "{}: SKILL.md missing `description:` key",
                s.name
            );
            // Our internal version marker must NOT leak into Codex's view.
            assert!(
                !body.contains("hew:version="),
                "{}: stale hew version marker should be stripped",
                s.name
            );
            // Our internal frontmatter keys (`category`, `init`) must not
            // collide with Codex's expectations — they should be absent.
            assert!(
                !body.contains("category:"),
                "{}: stale `category:` from original frontmatter",
                s.name
            );
            seen += 1;
        }
        assert!(seen >= 19, "expected at least 19 skill SKILL.md files, saw {seen}");
    }

    #[test]
    fn codex_skill_md_strips_pre_frontmatter_version_comment() {
        let body = "<!-- hew:version=0.3.0 -->\n---\nname: hew-execute\ncategory: core\n---\n\n# hew-execute — The Work Loop\n\nbody here";
        let out = render_codex_skill_md("hew-execute", body);
        assert!(
            out.starts_with("---\nname: hew-execute\ndescription:"),
            "expected new frontmatter at top, got:\n{out}"
        );
        assert!(!out.contains("hew:version="));
        assert!(!out.contains("category: core"));
        assert!(out.contains("# hew-execute — The Work Loop"));
        assert!(out.contains("body here"));
    }

    #[test]
    fn codex_skill_md_handles_body_without_frontmatter() {
        let body = "# hew-thing — Standalone\n\njust a body, no frontmatter";
        let out = render_codex_skill_md("hew-thing", body);
        assert!(out.starts_with("---\nname: hew-thing\ndescription:"));
        assert!(out.contains("just a body"));
    }

    #[test]
    fn codex_role_body_with_backslashes_round_trips() {
        // The pre-fix emitter broke on regex chars like `\s` because basic
        // multi-line strings `"""..."""` still process escape sequences.
        let body = "password\\s*=\\s*[\"'][^\"']+[\"']";
        let toml = render_codex_role("hew-guard", body);
        let parsed: toml::Value = toml::from_str(&toml).expect("parse");
        let got = parsed["developer_instructions"].as_str().expect("string");
        assert!(got.contains("\\s*"), "backslash-s lost in round-trip: {got}");
    }

    #[test]
    fn codex_role_body_with_triple_single_quote_uses_basic_string_fallback() {
        let body = "edge case: ''' inside body";
        let toml = render_codex_role("hew-test", body);
        // Fallback uses basic string `"""..."""` for developer_instructions.
        // The body's `'''` may appear inside it — that's fine because basic
        // strings don't treat `'` as a delimiter.
        assert!(
            toml.contains("developer_instructions = \"\"\""),
            "fallback should use basic string, got: {toml}"
        );
        let parsed: toml::Value = toml::from_str(&toml).expect("parse");
        let got = parsed["developer_instructions"].as_str().expect("string");
        assert_eq!(got.trim(), body);
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

    fn parse_settings(path: &Path) -> serde_json::Value {
        let body = fs::read_to_string(path).unwrap();
        serde_json::from_str(&body).expect("settings.json is valid JSON")
    }

    #[test]
    fn install_claude_writes_session_start_hook() {
        let tmp = tempfile::tempdir().unwrap();
        install(Runtime::Claude, tmp.path()).expect("install");

        let settings = tmp.path().join(".claude").join("settings.json");
        assert!(settings.exists(), "settings.json must be written");
        let v = parse_settings(&settings);
        let arr = v["hooks"]["SessionStart"].as_array().expect("SessionStart array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["matcher"], "startup|resume|clear");
        assert_eq!(arr[0]["hew_managed"], true);
        assert_eq!(arr[0]["hooks"][0]["type"], "command");
        assert_eq!(arr[0]["hooks"][0]["command"], "hew prime resume");
    }

    #[test]
    fn install_claude_session_hook_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        install(Runtime::Claude, tmp.path()).unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        let first = fs::read_to_string(&settings).unwrap();
        install(Runtime::Claude, tmp.path()).unwrap();
        let second = fs::read_to_string(&settings).unwrap();
        assert_eq!(first, second, "second install must be byte-identical");
        let v = parse_settings(&settings);
        assert_eq!(v["hooks"]["SessionStart"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn install_claude_preserves_user_settings_and_foreign_hooks() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let pre = serde_json::json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [{ "matcher": "Bash", "hooks": [{ "type": "command", "command": "echo pre" }] }],
                "SessionStart": [
                    { "matcher": "startup", "hooks": [{ "type": "command", "command": "echo user-hook" }] }
                ]
            }
        });
        fs::write(claude.join("settings.json"), serde_json::to_string_pretty(&pre).unwrap())
            .unwrap();

        install(Runtime::Claude, tmp.path()).unwrap();
        let v = parse_settings(&claude.join("settings.json"));

        assert_eq!(v["theme"], "dark", "top-level user keys preserved");
        assert_eq!(v["hooks"]["PreToolUse"][0]["matcher"], "Bash", "other hook events preserved");
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 2, "user SessionStart entry + hew entry");
        let hew_entries: Vec<_> =
            arr.iter().filter(|e| e["hew_managed"].as_bool().unwrap_or(false)).collect();
        assert_eq!(hew_entries.len(), 1);
        let user_entries: Vec<_> =
            arr.iter().filter(|e| !e["hew_managed"].as_bool().unwrap_or(false)).collect();
        assert_eq!(user_entries.len(), 1);
        assert_eq!(user_entries[0]["hooks"][0]["command"], "echo user-hook");
    }

    #[test]
    fn install_claude_fails_on_malformed_settings_json() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let bad = "{ not json";
        fs::write(claude.join("settings.json"), bad).unwrap();

        let err = install(Runtime::Claude, tmp.path()).expect_err("must fail");
        let msg = format!("{err}");
        assert!(msg.contains("malformed") || msg.contains("settings"), "diagnostic: {msg}");
        // File untouched.
        let after = fs::read_to_string(claude.join("settings.json")).unwrap();
        assert_eq!(after, bad);
    }

    #[test]
    fn uninstall_claude_removes_session_hook_and_preserves_others() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let pre = serde_json::json!({
            "theme": "dark",
            "hooks": {
                "SessionStart": [
                    { "matcher": "startup", "hooks": [{ "type": "command", "command": "echo user" }] }
                ]
            }
        });
        fs::write(claude.join("settings.json"), serde_json::to_string_pretty(&pre).unwrap())
            .unwrap();

        install(Runtime::Claude, tmp.path()).unwrap();
        uninstall(Runtime::Claude, tmp.path()).unwrap();

        let v = parse_settings(&claude.join("settings.json"));
        assert_eq!(v["theme"], "dark");
        let arr = v["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(arr.len(), 1, "user hook survives");
        assert_eq!(arr[0]["hooks"][0]["command"], "echo user");
    }

    #[test]
    fn uninstall_claude_removes_settings_file_when_only_hew_owned_it() {
        let tmp = tempfile::tempdir().unwrap();
        install(Runtime::Claude, tmp.path()).unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        assert!(settings.exists());

        uninstall(Runtime::Claude, tmp.path()).unwrap();
        assert!(!settings.exists(), "settings.json removed when fully emptied by uninstall");
    }

    #[test]
    fn uninstall_claude_is_idempotent_with_no_settings() {
        let tmp = tempfile::tempdir().unwrap();
        // No install first — just uninstall.
        uninstall(Runtime::Claude, tmp.path()).expect("uninstall on empty tree is a no-op");
    }

    #[test]
    fn install_claude_writes_allowlist_entries() {
        let tmp = tempfile::tempdir().unwrap();
        install(Runtime::Claude, tmp.path()).expect("install");

        let settings = tmp.path().join(".claude").join("settings.json");
        let v = parse_settings(&settings);
        let allow = v["permissions"]["allow"].as_array().expect("allow array");
        for entry in HEW_ALLOWLIST_ENTRIES {
            assert!(
                allow.iter().any(|e| e.as_str() == Some(entry)),
                "missing allow entry: {entry}"
            );
        }
        // No git push / reset --hard / clean -f leaked in.
        let allow_strs: Vec<&str> = allow.iter().filter_map(|v| v.as_str()).collect();
        assert!(!allow_strs.iter().any(|s| s.contains("git push")), "push must stay user-approved");
        assert!(
            !allow_strs.iter().any(|s| s.contains("reset --hard")),
            "reset --hard must stay user-approved"
        );
        // The tracker sibling key has the same entries.
        let tracked = v["permissions"][HEW_MANAGED_ALLOWLIST_KEY].as_array().expect("tracker");
        assert_eq!(tracked.len(), HEW_ALLOWLIST_ENTRIES.len());
    }

    #[test]
    fn install_claude_allowlist_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        install(Runtime::Claude, tmp.path()).unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        let first = fs::read_to_string(&settings).unwrap();
        install(Runtime::Claude, tmp.path()).unwrap();
        let second = fs::read_to_string(&settings).unwrap();
        assert_eq!(first, second, "second install must be byte-identical");
        let v = parse_settings(&settings);
        // Every entry appears exactly once.
        let allow = v["permissions"]["allow"].as_array().unwrap();
        for entry in HEW_ALLOWLIST_ENTRIES {
            let count = allow.iter().filter(|e| e.as_str() == Some(entry)).count();
            assert_eq!(count, 1, "duplicate entry: {entry}");
        }
    }

    #[test]
    fn install_claude_preserves_foreign_allow_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let pre = serde_json::json!({
            "permissions": {
                "allow": ["Bash(npm:*)", "Bash(make:*)"],
                "deny": ["Bash(rm -rf /:*)"]
            }
        });
        fs::write(claude.join("settings.json"), serde_json::to_string_pretty(&pre).unwrap())
            .unwrap();

        install(Runtime::Claude, tmp.path()).unwrap();
        let v = parse_settings(&claude.join("settings.json"));
        let allow_strs: Vec<&str> = v["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert!(allow_strs.contains(&"Bash(npm:*)"), "user entry preserved");
        assert!(allow_strs.contains(&"Bash(make:*)"), "user entry preserved");
        assert!(allow_strs.contains(&"Bash(bd:*)"), "hew entry added");
        // The deny sibling key is untouched.
        let deny = v["permissions"]["deny"].as_array().unwrap();
        assert_eq!(deny[0], "Bash(rm -rf /:*)");
    }

    #[test]
    fn install_claude_handles_user_who_added_one_of_our_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let pre = serde_json::json!({
            "permissions": {
                "allow": ["Bash(bd:*)"]  // user added one of ours
            }
        });
        fs::write(claude.join("settings.json"), serde_json::to_string_pretty(&pre).unwrap())
            .unwrap();

        install(Runtime::Claude, tmp.path()).unwrap();
        let v = parse_settings(&claude.join("settings.json"));
        let allow_strs: Vec<&str> = v["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        let bd_count = allow_strs.iter().filter(|s| **s == "Bash(bd:*)").count();
        assert_eq!(bd_count, 1, "no duplicate of Bash(bd:*) even when user pre-added it");
    }

    #[test]
    fn uninstall_claude_removes_only_hew_managed_allow_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        let pre = serde_json::json!({
            "permissions": {
                "allow": ["Bash(npm:*)", "Bash(make:*)"],
                "deny": ["Bash(rm -rf /:*)"]
            }
        });
        fs::write(claude.join("settings.json"), serde_json::to_string_pretty(&pre).unwrap())
            .unwrap();

        install(Runtime::Claude, tmp.path()).unwrap();
        uninstall(Runtime::Claude, tmp.path()).unwrap();

        let v = parse_settings(&claude.join("settings.json"));
        let allow_strs: Vec<&str> = v["permissions"]["allow"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|x| x.as_str())
            .collect();
        assert_eq!(allow_strs, vec!["Bash(npm:*)", "Bash(make:*)"], "only hew entries removed");
        assert!(v["permissions"].get(HEW_MANAGED_ALLOWLIST_KEY).is_none(), "tracker cleared");
        assert_eq!(v["permissions"]["deny"][0], "Bash(rm -rf /:*)", "deny preserved");
    }

    #[test]
    fn install_claude_allowlist_rejects_malformed_settings() {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        fs::create_dir_all(&claude).unwrap();
        fs::write(claude.join("settings.json"), "{ not json").unwrap();

        let err = install(Runtime::Claude, tmp.path()).expect_err("must fail on malformed json");
        let msg = format!("{err}");
        assert!(msg.contains("malformed") || msg.contains("settings"), "diagnostic: {msg}");
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
