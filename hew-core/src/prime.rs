//! `hew prime <skill>` — assemble agent context JSON.
//!
//! The output shape is the contract between the binary and any agent
//! that consumes it. Adding fields is backwards-compatible; renaming
//! or removing is not.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::bd::{BdClient, ReadyTask, StatsSummary};
use crate::error::Result;
use crate::skills::{self, Skill};

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct PrimeOutput {
    pub schema_version: u32,
    pub skill: String,
    pub project: ProjectInfo,
    pub status: StatusMap,
    pub prerequisites: Prerequisites,
    pub tasks: TaskInfo,
    pub memories: MemoryBuckets,
    pub skill_instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<UpdateAvailable>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct UpdateAvailable {
    pub current: String,
    pub latest: String,
    pub message: String,
}

/// Skill-agnostic prime output used by SessionStart hooks to restore
/// agent context after `/clear` or a new session.
#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ResumeOutput {
    pub schema_version: u32,
    pub project: ProjectInfo,
    pub status: StatusMap,
    pub tasks: TaskInfo,
    pub memories: MemoryBuckets,
    /// Behavior-shaping `hew config` knobs rendered as agent
    /// instructions. Always present (defaults used if the config file
    /// is missing or malformed).
    pub config: ConfigInstructions,
    /// Tasks the user has claimed (status=in_progress). The "what was
    /// I doing?" signal at session start.
    #[serde(default)]
    pub in_progress: Vec<ReadyTask>,
    /// Working-tree state: branch, dirty/clean, ahead/behind upstream.
    /// `None` when not inside a git repo or git is unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitState>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_checkpoint: Option<Checkpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_available: Option<UpdateAvailable>,
}

/// Snapshot of the working tree at session-start time. All counts are
/// best-effort; any single git call failing degrades the field to its
/// default rather than dropping the whole struct.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct GitState {
    /// Current branch name, or `None` if HEAD is detached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Any tracked file modified, staged, or partially staged.
    pub dirty: bool,
    /// Untracked file count (`?? ` lines from `git status --porcelain`).
    pub untracked_count: u32,
    /// Commits on the local branch not in the upstream.
    pub ahead: u32,
    /// Commits on the upstream not in the local branch.
    pub behind: u32,
    /// Upstream ref (`origin/main` etc.), if the branch tracks one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
}

/// Subset of `hew config` that shapes agent behavior at session-time.
/// Knobs that only affect CLI defaults at run-time (update-check,
/// default-runtime, compact dry-run, etc.) are intentionally omitted —
/// the agent doesn't need them to make decisions.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ConfigInstructions {
    /// `none` | `epic` | `always` — when hew-execute auto-creates a
    /// branch on first claim.
    pub branching_strategy: String,
    /// When true, hew-guard fails the close if a behavior-changing
    /// task ships without a test.
    pub testing_required: bool,
    /// Soft-warn threshold for changed function size. `0` disables.
    pub craft_max_function_lines: u32,
    /// Soft-warn on unused imports / dead code surfaced by lints.
    pub craft_warn_on_unused: bool,
    /// Fire the Step 10 review picker after this many closed tasks.
    /// `0` disables this trigger.
    pub review_after_n_tasks: u32,
    /// Fire the Step 10 review picker on epic close.
    pub review_after_epic: bool,
    /// `ask` | `auto-skip` | `auto-run` — default at the
    /// hew-plan research-or-decompose picker.
    pub research_default: String,
    /// Optional skill picker defaults. `yes` / `no` / `ask`.
    pub optional_skills: OptionalSkillsView,
    /// Whether `.beads/` is tracked in git.
    pub git_track: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct OptionalSkillsView {
    pub deps: String,
    pub research: String,
    pub security: String,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Checkpoint {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct ProjectInfo {
    pub beads_initialized: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bd_version: Option<String>,
}

pub type StatusMap = BTreeMap<String, StatusEntry>;

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StatusEntry {
    pub complete: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Prerequisites {
    pub met: bool,
    pub missing: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskInfo {
    pub total: u64,
    pub done: u64,
    pub in_progress: u64,
    pub ready: u64,
    pub blocked: u64,
    pub ready_list: Vec<ReadyTask>,
}

#[derive(Debug, Serialize, Deserialize, Default, schemars::JsonSchema)]
pub struct MemoryBuckets {
    pub conventions: Vec<String>,
    pub boundaries: Vec<String>,
    pub audit: Vec<String>,
    pub security: Vec<String>,
    pub migration: Vec<String>,
    pub dep: Vec<String>,
    /// `PROJECT:` — high-level project facts (what we're building, for
    /// whom, hard constraints). Written by hew-new-project.
    #[serde(default)]
    pub project: Vec<String>,
    /// `MILESTONE:` — current milestone identity + acceptance window.
    /// Written by hew-new-project and updated as milestones progress.
    #[serde(default)]
    pub milestone: Vec<String>,
    /// `ROADMAP:` — the milestone chain (ordered list of epics).
    /// Written by hew-new-project.
    #[serde(default)]
    pub roadmap: Vec<String>,
    /// `RESEARCH:` — investigation findings with provenance tags
    /// (`[VERIFIED]` / `[CITED]` / `[ASSUMED]`). Written by hew-research.
    #[serde(default)]
    pub research: Vec<String>,
    /// `DECISION:` — locked architectural choices. CLAUDE.md: "Cite when
    /// relevant." Previously buried in `factual`.
    #[serde(default)]
    pub decisions: Vec<String>,
    /// `GOTCHA:` — hard-won discoveries. CLAUDE.md: "Read these before
    /// debugging anything weird." Previously buried in `factual`.
    #[serde(default)]
    pub gotchas: Vec<String>,
    /// `FEEDBACK:` — user-stated preferences. CLAUDE.md: "Honor every
    /// time." Previously buried in `factual`.
    #[serde(default)]
    pub feedback: Vec<String>,
    pub factual: Vec<String>,
}

/// Categorize a raw memory map (key -> value) into prefix buckets +
/// a STATUS map parsed out of `STATUS:<phase>:complete — <timestamp>`.
pub fn categorize(memories: &BTreeMap<String, String>) -> (MemoryBuckets, StatusMap) {
    let mut buckets = MemoryBuckets::default();
    let mut status: StatusMap = BTreeMap::new();

    for value in memories.values() {
        let trimmed = value.trim_start();
        if let Some(rest) = trimmed.strip_prefix("STATUS:") {
            if let Some((phase, payload)) = rest.split_once(':') {
                let complete = payload.trim_start().starts_with("complete");
                let timestamp = payload.split('—').nth(1).map(|s| s.trim().to_string());
                status.insert(phase.trim().to_string(), StatusEntry { complete, timestamp });
            }
            continue;
        }
        if trimmed.starts_with("CONVENTION:") {
            buckets.conventions.push(value.clone());
        } else if trimmed.starts_with("BOUNDARY:") {
            buckets.boundaries.push(value.clone());
        } else if trimmed.starts_with("AUDIT:") {
            buckets.audit.push(value.clone());
        } else if trimmed.starts_with("SECURITY:") {
            buckets.security.push(value.clone());
        } else if trimmed.starts_with("MIGRATION:") {
            buckets.migration.push(value.clone());
        } else if trimmed.starts_with("DEP:") {
            buckets.dep.push(value.clone());
        } else if trimmed.starts_with("PROJECT:") {
            buckets.project.push(value.clone());
        } else if trimmed.starts_with("MILESTONE:") {
            buckets.milestone.push(value.clone());
        } else if trimmed.starts_with("ROADMAP:") {
            buckets.roadmap.push(value.clone());
        } else if trimmed.starts_with("RESEARCH:") {
            buckets.research.push(value.clone());
        } else if trimmed.starts_with("DECISION:") {
            buckets.decisions.push(value.clone());
        } else if trimmed.starts_with("GOTCHA:") {
            buckets.gotchas.push(value.clone());
        } else if trimmed.starts_with("FEEDBACK:") {
            buckets.feedback.push(value.clone());
        } else {
            buckets.factual.push(value.clone());
        }
    }

    (buckets, status)
}

/// Prerequisite chain — the agent stops or warns if these are absent.
pub fn prerequisites_for(skill: &str, status: &StatusMap) -> Prerequisites {
    let needs: &[&str] = match skill {
        "decompose" | "hew-decompose" => &["plan"],
        "execute" | "hew-execute" => &["plan"],
        "verify" | "hew-verify" => &["plan"],
        "guard" | "hew-guard" => &["plan"],
        "convention" | "hew-convention" => &["scan"],
        "boundary" | "hew-boundary" => &["scan"],
        "audit" | "hew-audit" => &["scan"],
        "migrate" | "hew-migrate" => &["scan"],
        _ => &[],
    };

    let missing: Vec<String> = needs
        .iter()
        .filter(|p| !status.get(**p).map(|s| s.complete).unwrap_or(false))
        .map(|p| (*p).to_string())
        .collect();

    Prerequisites { met: missing.is_empty(), missing }
}

/// Build the prime JSON for `skill_name`.
pub fn build(client: &dyn BdClient, skill_name: &str) -> Result<PrimeOutput> {
    let skill = resolve_skill(skill_name)?;

    let stats: StatsSummary = client.stats().unwrap_or_default();
    let ready = client.ready().unwrap_or_default();
    let memories = client.memories().unwrap_or_default();
    let bd_version = client.version().ok().map(|v| v.semver);

    let (buckets, status) = categorize(&memories);
    let prereqs = prerequisites_for(skill.name, &status);

    let tasks = TaskInfo {
        total: stats.total_issues,
        done: stats.closed_issues,
        in_progress: stats.in_progress_issues,
        ready: stats.ready_issues,
        blocked: stats.blocked_issues,
        ready_list: ready.into_iter().take(20).collect(),
    };

    // Kick off (best-effort) the passive update check and surface any
    // cached notice from a previous run.
    crate::notify::schedule_if_stale(env!("CARGO_PKG_VERSION"));
    let update_available =
        crate::notify::read_cached_notice().ok().flatten().map(|n| UpdateAvailable {
            current: n.current,
            latest: n.latest.clone(),
            message: format!("Run `hew update` to upgrade to {}.", n.latest),
        });

    Ok(PrimeOutput {
        schema_version: 1,
        skill: skill.name.to_string(),
        project: ProjectInfo { beads_initialized: bd_version.is_some(), bd_version },
        status,
        prerequisites: prereqs,
        tasks,
        memories: buckets,
        skill_instructions: skill.body.to_string(),
        update_available,
    })
}

/// Find the most-recent `CHECKPOINT:` memory by parsing the ISO-8601
/// timestamp out of the value prefix. ISO-8601 strings sort
/// lexicographically, so a string max gives chronological max.
///
/// Defense-in-depth: a body whose first whitespace-token doesn't look
/// like an ISO date is treated as having no timestamp (sorts last),
/// so a malformed checkpoint (e.g. `CHECKPOINT:practice-svc — …`
/// from GitHub #40) can't shadow a properly-shaped newer one. The
/// canonical fix is on the write side — `hew checkpoint` always
/// emits the right shape — but the resume path stays robust either
/// way.
pub fn latest_checkpoint(memories: &BTreeMap<String, String>) -> Option<Checkpoint> {
    memories
        .iter()
        .filter(|(_, v)| v.trim_start().starts_with("CHECKPOINT:"))
        .map(|(k, v)| {
            let rest = v.trim_start().strip_prefix("CHECKPOINT:").unwrap_or("");
            let timestamp = rest
                .split_whitespace()
                .next()
                .filter(|s| !s.is_empty())
                .filter(|s| crate::time::looks_like_iso_date(s))
                .map(|s| s.to_string());
            Checkpoint { key: k.clone(), timestamp, body: v.clone() }
        })
        .max_by(|a, b| a.timestamp.cmp(&b.timestamp))
}

/// Build the skill-agnostic resume JSON. Returned by `hew prime resume`
/// and consumed by SessionStart hooks.
pub fn resume(client: &dyn BdClient) -> Result<ResumeOutput> {
    let stats: StatsSummary = client.stats().unwrap_or_default();
    let ready = client.ready().unwrap_or_default();
    let memories = client.memories().unwrap_or_default();
    let bd_version = client.version().ok().map(|v| v.semver);

    let (buckets, status) = categorize(&memories);
    let checkpoint = latest_checkpoint(&memories);

    let tasks = TaskInfo {
        total: stats.total_issues,
        done: stats.closed_issues,
        in_progress: stats.in_progress_issues,
        ready: stats.ready_issues,
        blocked: stats.blocked_issues,
        ready_list: ready.into_iter().take(20).collect(),
    };

    crate::notify::schedule_if_stale(env!("CARGO_PKG_VERSION"));
    let update_available =
        crate::notify::read_cached_notice().ok().flatten().map(|n| UpdateAvailable {
            current: n.current,
            latest: n.latest.clone(),
            message: format!("Run `hew update` to upgrade to {}.", n.latest),
        });

    let in_progress = collect_in_progress(client);
    let git = crate::git::RealGit::discover().ok().and_then(|g| collect_git_state(&g));

    // Self-heal pre-statusline installs. Silent / best-effort — the
    // SessionStart hook must never break because of a migration misfire.
    if let Ok(cwd) = std::env::current_dir() {
        let _ = crate::install::auto_migrate_claude_statusline(&cwd);
    }

    Ok(ResumeOutput {
        schema_version: 1,
        project: ProjectInfo { beads_initialized: bd_version.is_some(), bd_version },
        status,
        tasks,
        memories: buckets,
        config: load_config_instructions(),
        in_progress,
        git,
        latest_checkpoint: checkpoint,
        update_available,
    })
}

/// Load and project the persistent `hew config` into the
/// session-facing instruction view. Errors fall back to defaults — the
/// SessionStart hook must not break on a missing or malformed config.
pub fn load_config_instructions() -> ConfigInstructions {
    let cfg = crate::config::load().unwrap_or_default();
    ConfigInstructions {
        branching_strategy: cfg.branching.strategy,
        testing_required: cfg.testing.require,
        craft_max_function_lines: cfg.craft.max_function_lines,
        craft_warn_on_unused: cfg.craft.warn_on_unused,
        review_after_n_tasks: cfg.review.after_n_tasks,
        review_after_epic: cfg.review.after_epic,
        research_default: cfg.research.default,
        optional_skills: OptionalSkillsView {
            deps: cfg.optional_skills.deps.as_str().to_string(),
            research: cfg.optional_skills.research.as_str().to_string(),
            security: cfg.optional_skills.security.as_str().to_string(),
        },
        git_track: cfg.git_track,
    }
}

/// Collect tasks the user has currently claimed (status=in_progress).
/// Falls back to an empty list on any error — the SessionStart hook
/// must never break because the bd query hiccupped.
///
/// Routes through `run_to_file` (not the read-after-wait pipe) because
/// `bd list --json` can exceed the OS pipe buffer on large graphs —
/// see `GOTCHA:pipe-deadlock`.
pub fn collect_in_progress(client: &dyn BdClient) -> Vec<ReadyTask> {
    use std::ffi::OsStr;

    let args = [
        OsStr::new("list"),
        OsStr::new("--status"),
        OsStr::new("in_progress"),
        OsStr::new("--json"),
        OsStr::new("--limit"),
        OsStr::new("0"),
    ];
    let tmp = std::env::temp_dir().join(format!(
        "hew-in-progress-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    ));
    if client.run_to_file(&args, &tmp).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return Vec::new();
    }
    let body = std::fs::read_to_string(&tmp).unwrap_or_default();
    let _ = std::fs::remove_file(&tmp);
    serde_json::from_str(body.trim()).unwrap_or_default()
}

/// Best-effort working-tree state for the cwd. Returns `None` when not
/// inside a git repo, when git is unavailable, or when the top-level
/// `rev-parse --git-dir` fails. Individual sub-queries degrade silently
/// (a missing upstream zeros out `ahead`/`behind` rather than dropping
/// the whole struct).
pub fn collect_git_state(git: &dyn crate::git::GitClient) -> Option<GitState> {
    use std::ffi::OsStr;

    // Cheap "are we in a repo?" gate.
    git.run_raw(&[OsStr::new("rev-parse"), OsStr::new("--git-dir")]).ok()?;

    let branch = git.current_branch().ok().flatten();

    let (dirty, untracked_count) = git
        .run_raw(&[OsStr::new("status"), OsStr::new("--porcelain")])
        .ok()
        .map(|o| {
            let mut dirty = false;
            let mut untracked = 0u32;
            for line in o.stdout.lines() {
                if line.is_empty() {
                    continue;
                }
                if let Some(stripped) = line.strip_prefix("?? ") {
                    let _ = stripped;
                    untracked += 1;
                } else {
                    dirty = true;
                }
            }
            (dirty, untracked)
        })
        .unwrap_or((false, 0));

    let upstream = git
        .run_raw(&[
            OsStr::new("rev-parse"),
            OsStr::new("--abbrev-ref"),
            OsStr::new("--symbolic-full-name"),
            OsStr::new("@{upstream}"),
        ])
        .ok()
        .map(|o| o.stdout.trim().to_string())
        .filter(|s| !s.is_empty());

    let (ahead, behind) = if upstream.is_some() {
        git.run_raw(&[
            OsStr::new("rev-list"),
            OsStr::new("--left-right"),
            OsStr::new("--count"),
            OsStr::new("HEAD...@{upstream}"),
        ])
        .ok()
        .and_then(|o| {
            let mut parts = o.stdout.split_whitespace();
            let a: u32 = parts.next()?.parse().ok()?;
            let b: u32 = parts.next()?.parse().ok()?;
            Some((a, b))
        })
        .unwrap_or((0, 0))
    } else {
        (0, 0)
    };

    Some(GitState { branch, dirty, untracked_count, ahead, behind, upstream })
}

/// Render `ConfigInstructions` as a list of agent-readable directive
/// lines. Each line is a self-contained instruction, not a config dump.
pub fn render_config_instructions(c: &ConfigInstructions) -> Vec<String> {
    let mut lines = Vec::new();

    match c.branching_strategy.as_str() {
        "epic" => lines.push(
            "Branching: auto-create a feature branch on first task claim per epic. \
             Use `hew branch new --prefix=<feat|fix|chore|...> --slug=<short-kebab>` \
             when starting work outside an epic."
                .into(),
        ),
        "always" => lines.push(
            "Branching: every task claim opens a new branch automatically. \
             Never commit directly to main."
                .into(),
        ),
        "none" => lines.push(
            "Branching: manual. Create branches yourself with \
             `hew branch new --prefix=… --slug=…` before substantive work."
                .into(),
        ),
        other => lines.push(format!("Branching strategy: {other} (custom).")),
    }

    if c.testing_required {
        lines.push(
            "Tests required: every behavior-changing close must ship a test. \
             hew-guard will fail the close otherwise (testing.require=true)."
                .into(),
        );
    } else {
        lines.push(
            "Tests: soft-warn only. hew-guard will flag missing tests but won't \
             block the close (testing.require=false)."
                .into(),
        );
    }

    if c.craft_max_function_lines > 0 {
        lines.push(format!(
            "Craft: soft-warn when a changed function exceeds {} lines \
             (craft.max_function_lines).",
            c.craft_max_function_lines
        ));
    }
    if c.craft_warn_on_unused {
        lines.push(
            "Craft: soft-warn on unused imports and dead code surfaced by \
             language lints (craft.warn_on_unused=true)."
                .into(),
        );
    }

    if c.review_after_n_tasks > 0 {
        lines.push(format!(
            "Review trigger: fire the Step 10 review picker after {} closed tasks \
             since the last review marker.",
            c.review_after_n_tasks
        ));
    }
    if c.review_after_epic {
        lines.push("Review trigger: fire the Step 10 review picker on every epic close.".into());
    }

    lines.push(format!(
        "Research picker default: `{}` at the hew-plan research-or-decompose fork.",
        c.research_default
    ));

    let mut skills_off = Vec::new();
    let mut skills_ask = Vec::new();
    for (name, mode) in [
        ("deps", &c.optional_skills.deps),
        ("research", &c.optional_skills.research),
        ("security", &c.optional_skills.security),
    ] {
        match mode.as_str() {
            "no" => skills_off.push(name),
            "ask" => skills_ask.push(name),
            _ => {}
        }
    }
    if !skills_off.is_empty() {
        lines.push(format!(
            "Optional skills disabled (do not invoke unless explicitly requested): {}.",
            skills_off.join(", ")
        ));
    }
    if !skills_ask.is_empty() {
        lines.push(format!(
            "Optional skills gated on user prompt: {}. Ask before invoking.",
            skills_ask.join(", ")
        ));
    }

    if c.git_track {
        lines.push("`.beads/` is tracked in git — task-graph changes show up in diffs.".into());
    } else {
        lines.push(
            "`.beads/` is NOT tracked in git — task graph is local-only \
             (not shared via the repo)."
                .into(),
        );
    }

    lines
}

/// Render a `ResumeOutput` as a human-readable summary for the
/// SessionStart hook. Mirrors `status::render_text` but adds the
/// latest CHECKPOINT body and any cached update notice.
pub fn render_resume_text(out: &ResumeOutput) -> String {
    use std::fmt::Write;

    let mut s = String::new();
    let _ = writeln!(s, "hew resume");
    let _ = writeln!(s, "──────────────────────────────────");
    match out.project.bd_version.as_deref() {
        Some(v) => {
            let _ = writeln!(s, "  bd:        v{v}");
        }
        None => {
            let _ = writeln!(s, "  bd:        not detected");
        }
    }
    if let Some(g) = &out.git {
        let branch = g.branch.as_deref().unwrap_or("(detached HEAD)");
        let cleanliness = if g.dirty { "dirty" } else { "clean" };
        let _ = writeln!(s, "  branch:    {branch} ({cleanliness})");
        if g.untracked_count > 0 {
            let _ = writeln!(s, "  untracked: {} file(s)", g.untracked_count);
        }
        if let Some(up) = &g.upstream {
            let drift = match (g.ahead, g.behind) {
                (0, 0) => "in sync".to_string(),
                (a, 0) => format!("{a} ahead"),
                (0, b) => format!("{b} behind"),
                (a, b) => format!("{a} ahead, {b} behind"),
            };
            let _ = writeln!(s, "  upstream:  {up} ({drift})");
        }
    }
    if let Some(u) = &out.update_available {
        let _ = writeln!(s, "  update:    {} → {} ({})", u.current, u.latest, u.message);
    }
    let _ = writeln!(s);

    let known_phases = ["scan", "convention", "audit", "boundary", "plan", "decompose", "verify"];
    let _ = writeln!(s, "Phases");
    let _ = writeln!(s, "──────────────────────────────────");
    for name in known_phases {
        let entry = out.status.get(name);
        let complete = entry.map(|e| e.complete).unwrap_or(false);
        let ts = entry.and_then(|e| e.timestamp.as_deref()).unwrap_or("");
        let mark = if complete { "✓" } else { "○" };
        let _ = writeln!(s, "  {mark} {:<10}  {ts}", name);
    }
    // Surface any phases beyond the known set (e.g., review, compact).
    let mut extras: Vec<(&String, &StatusEntry)> =
        out.status.iter().filter(|(k, _)| !known_phases.contains(&k.as_str())).collect();
    extras.sort_by(|a, b| a.0.cmp(b.0));
    for (name, entry) in extras {
        let mark = if entry.complete { "✓" } else { "○" };
        let ts = entry.timestamp.as_deref().unwrap_or("");
        let _ = writeln!(s, "  {mark} {:<10}  {ts}", name);
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Tasks");
    let _ = writeln!(s, "──────────────────────────────────");
    let _ = writeln!(
        s,
        "  {} total │ {} done │ {} in progress │ {} ready │ {} blocked",
        out.tasks.total, out.tasks.done, out.tasks.in_progress, out.tasks.ready, out.tasks.blocked,
    );
    if !out.tasks.ready_list.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "  Next up:");
        for t in out.tasks.ready_list.iter().take(5) {
            let _ = writeln!(s, "    • [P{}] {} {}", t.priority, t.id, t.title);
        }
    }
    let _ = writeln!(s);

    if !out.in_progress.is_empty() {
        let _ = writeln!(s, "Claimed (in-flight)");
        let _ = writeln!(s, "──────────────────────────────────");
        for t in &out.in_progress {
            let _ = writeln!(s, "  • [P{}] {} {}", t.priority, t.id, t.title);
            if !t.description.is_empty() {
                // Indent each line of the body; trim trailing whitespace to
                // keep block compact. First ~20 lines is plenty for context.
                for line in t.description.lines().take(20) {
                    let _ = writeln!(s, "      {}", line.trim_end());
                }
                let extra = t.description.lines().count().saturating_sub(20);
                if extra > 0 {
                    let _ = writeln!(s, "      … ({extra} more line(s) truncated)");
                }
            }
        }
        let _ = writeln!(s);
    }

    let _ = writeln!(s, "Memories");
    let _ = writeln!(s, "──────────────────────────────────");
    let m = &out.memories;
    let _ = writeln!(
        s,
        "  {} CONVENTION │ {} BOUNDARY │ {} AUDIT │ {} SECURITY │ {} MIGRATION │ {} DEP │ {} factual",
        m.conventions.len(),
        m.boundaries.len(),
        m.audit.len(),
        m.security.len(),
        m.migration.len(),
        m.dep.len(),
        m.factual.len(),
    );
    if !m.decisions.is_empty() || !m.gotchas.is_empty() || !m.feedback.is_empty() {
        let _ = writeln!(
            s,
            "  {} DECISION │ {} GOTCHA │ {} FEEDBACK",
            m.decisions.len(),
            m.gotchas.len(),
            m.feedback.len(),
        );
    }
    if !m.project.is_empty()
        || !m.milestone.is_empty()
        || !m.roadmap.is_empty()
        || !m.research.is_empty()
    {
        let _ = writeln!(
            s,
            "  {} PROJECT │ {} MILESTONE │ {} ROADMAP │ {} RESEARCH",
            m.project.len(),
            m.milestone.len(),
            m.roadmap.len(),
            m.research.len(),
        );
    }

    // Short labels for conventions.
    let labels: Vec<String> = m
        .conventions
        .iter()
        .filter_map(|line| {
            let after = line.trim().strip_prefix("CONVENTION:")?;
            let label = after.split('—').next()?.trim();
            if label.is_empty() { None } else { Some(label.to_string()) }
        })
        .collect();
    if !labels.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "  Conventions: {}", labels.join(", "));
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Project config — read as standing instructions");
    let _ = writeln!(s, "──────────────────────────────────");
    for line in render_config_instructions(&out.config) {
        let _ = writeln!(s, "  • {line}");
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Latest CHECKPOINT");
    let _ = writeln!(s, "──────────────────────────────────");
    match &out.latest_checkpoint {
        Some(c) => {
            let _ = writeln!(s, "  key:       {}", c.key);
            if let Some(ts) = &c.timestamp {
                let _ = writeln!(s, "  timestamp: {ts}");
            }
            let _ = writeln!(s);
            for line in c.body.lines() {
                let _ = writeln!(s, "  {line}");
            }
        }
        None => {
            let _ = writeln!(s, "  (none — no CHECKPOINT: memories on file)");
        }
    }
    s
}

/// Pretty-print a [`PrimeOutput`] as labeled sections, mirroring
/// [`render_resume_text`]. The skill body is intentionally elided from
/// the rendered text — it can be many KB and agents that need the body
/// already load it from the skill index. Pass `--json` for the full
/// JSON shape including `skill_instructions`.
pub fn render_prime_text(out: &PrimeOutput) -> String {
    use std::fmt::Write;

    let mut s = String::new();
    let _ = writeln!(s, "hew prime {}", out.skill);
    let _ = writeln!(s, "──────────────────────────────────");
    match out.project.bd_version.as_deref() {
        Some(v) => {
            let _ = writeln!(s, "  bd:        v{v}");
        }
        None => {
            let _ = writeln!(s, "  bd:        not detected");
        }
    }
    if let Some(u) = &out.update_available {
        let _ = writeln!(s, "  update:    {} → {} ({})", u.current, u.latest, u.message);
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Prerequisites");
    let _ = writeln!(s, "──────────────────────────────────");
    if out.prerequisites.met {
        let _ = writeln!(s, "  ✓ all prerequisites met");
    } else {
        let _ = writeln!(s, "  ✗ missing: {}", out.prerequisites.missing.join(", "));
    }
    let _ = writeln!(s);

    let known_phases = ["scan", "convention", "audit", "boundary", "plan", "decompose", "verify"];
    let _ = writeln!(s, "Phases");
    let _ = writeln!(s, "──────────────────────────────────");
    for name in known_phases {
        let entry = out.status.get(name);
        let complete = entry.map(|e| e.complete).unwrap_or(false);
        let ts = entry.and_then(|e| e.timestamp.as_deref()).unwrap_or("");
        let mark = if complete { "✓" } else { "○" };
        let _ = writeln!(s, "  {mark} {:<10}  {ts}", name);
    }
    let mut extras: Vec<(&String, &StatusEntry)> =
        out.status.iter().filter(|(k, _)| !known_phases.contains(&k.as_str())).collect();
    extras.sort_by(|a, b| a.0.cmp(b.0));
    for (name, entry) in extras {
        let mark = if entry.complete { "✓" } else { "○" };
        let ts = entry.timestamp.as_deref().unwrap_or("");
        let _ = writeln!(s, "  {mark} {:<10}  {ts}", name);
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Tasks");
    let _ = writeln!(s, "──────────────────────────────────");
    let _ = writeln!(
        s,
        "  {} total │ {} done │ {} in progress │ {} ready │ {} blocked",
        out.tasks.total, out.tasks.done, out.tasks.in_progress, out.tasks.ready, out.tasks.blocked,
    );
    if !out.tasks.ready_list.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "  Next up:");
        for t in out.tasks.ready_list.iter().take(5) {
            let _ = writeln!(s, "    • [P{}] {} {}", t.priority, t.id, t.title);
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Memories");
    let _ = writeln!(s, "──────────────────────────────────");
    let m = &out.memories;
    let _ = writeln!(
        s,
        "  {} CONVENTION │ {} BOUNDARY │ {} AUDIT │ {} SECURITY │ {} MIGRATION │ {} DEP │ {} factual",
        m.conventions.len(),
        m.boundaries.len(),
        m.audit.len(),
        m.security.len(),
        m.migration.len(),
        m.dep.len(),
        m.factual.len(),
    );
    if !m.decisions.is_empty() || !m.gotchas.is_empty() || !m.feedback.is_empty() {
        let _ = writeln!(
            s,
            "  {} DECISION │ {} GOTCHA │ {} FEEDBACK",
            m.decisions.len(),
            m.gotchas.len(),
            m.feedback.len(),
        );
    }
    if !m.project.is_empty()
        || !m.milestone.is_empty()
        || !m.roadmap.is_empty()
        || !m.research.is_empty()
    {
        let _ = writeln!(
            s,
            "  {} PROJECT │ {} MILESTONE │ {} ROADMAP │ {} RESEARCH",
            m.project.len(),
            m.milestone.len(),
            m.roadmap.len(),
            m.research.len(),
        );
    }

    let labels: Vec<String> = m
        .conventions
        .iter()
        .filter_map(|line| {
            let after = line.trim().strip_prefix("CONVENTION:")?;
            let label = after.split('—').next()?.trim();
            if label.is_empty() { None } else { Some(label.to_string()) }
        })
        .collect();
    if !labels.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "  Conventions: {}", labels.join(", "));
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Skill instructions");
    let _ = writeln!(s, "──────────────────────────────────");
    let _ = writeln!(
        s,
        "  (skill body elided — {} bytes; pass --json to include verbatim)",
        out.skill_instructions.len(),
    );
    s
}

fn resolve_skill(name: &str) -> Result<Skill> {
    // Accept `execute`, `hew-execute`, or the canonical name.
    if let Some(s) = skills::find(name) {
        return Ok(s);
    }
    let prefixed = format!("hew-{name}");
    if let Some(s) = skills::find(&prefixed) {
        return Ok(s);
    }
    Err(crate::error::HewError::MissingFlag { flag: format!("skill (unknown: {name})") })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(items: &[(&str, &str)]) -> BTreeMap<String, String> {
        items.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    #[test]
    fn categorize_routes_each_prefix() {
        let m = map(&[
            ("a", "CONVENTION:errors — wrap"),
            ("b", "BOUNDARY: POST /users"),
            ("c", "AUDIT: jose deprecated"),
            ("d", "SECURITY: JWT 15m"),
            ("e", "MIGRATION: add col"),
            ("f", "DEP: chrono 0.4"),
            ("g", "Backend: FastAPI + Postgres"),
        ]);
        let (b, _) = categorize(&m);
        assert_eq!(b.conventions.len(), 1);
        assert_eq!(b.boundaries.len(), 1);
        assert_eq!(b.audit.len(), 1);
        assert_eq!(b.security.len(), 1);
        assert_eq!(b.migration.len(), 1);
        assert_eq!(b.dep.len(), 1);
        assert_eq!(b.factual.len(), 1);
    }

    #[test]
    fn categorize_routes_project_milestone_roadmap_research() {
        let m = map(&[
            ("p", "PROJECT: building a CRM for solo founders. Stack: TS+Next.js."),
            ("m", "MILESTONE:foundation — walking skeleton + auth slice."),
            ("r", "ROADMAP: foundation -> MVP -> hardening -> launch."),
            ("s", "RESEARCH:auth [VERIFIED] passwordless TTL 15m. Source: NIST."),
        ]);
        let (b, _) = categorize(&m);
        assert_eq!(b.project.len(), 1);
        assert_eq!(b.milestone.len(), 1);
        assert_eq!(b.roadmap.len(), 1);
        assert_eq!(b.research.len(), 1);
        assert!(
            b.factual.is_empty(),
            "PROJECT/MILESTONE/ROADMAP/RESEARCH must not leak into factual"
        );
    }

    #[test]
    fn categorize_routes_decision_gotcha_feedback() {
        let m = map(&[
            ("d", "DECISION:db — Postgres."),
            ("g", "GOTCHA:pipe-deadlock — bd JSON > 16KB pipe."),
            ("f", "FEEDBACK:no-json-piping — never pipe --json through jq."),
            ("x", "untagged factual entry"),
        ]);
        let (b, _) = categorize(&m);
        assert_eq!(b.decisions.len(), 1, "DECISION should route to its own bucket");
        assert_eq!(b.gotchas.len(), 1, "GOTCHA should route to its own bucket");
        assert_eq!(b.feedback.len(), 1, "FEEDBACK should route to its own bucket");
        assert_eq!(b.factual.len(), 1, "only the untagged entry stays factual");
    }

    #[test]
    fn status_memory_is_parsed_out_and_not_bucketed() {
        let m = map(&[("k", "STATUS:scan:complete — 2026-05-11T14:30:00")]);
        let (b, s) = categorize(&m);
        assert!(b.factual.is_empty(), "STATUS must not leak into factual");
        assert!(s.get("scan").map(|e| e.complete).unwrap_or(false));
        assert_eq!(s["scan"].timestamp.as_deref(), Some("2026-05-11T14:30:00"));
    }

    #[test]
    fn status_in_progress_is_not_complete() {
        let m = map(&[("k", "STATUS:plan:in-progress")]);
        let (_, s) = categorize(&m);
        assert!(!s["plan"].complete);
    }

    #[test]
    fn execute_needs_plan() {
        let mut status: StatusMap = BTreeMap::new();
        let p = prerequisites_for("hew-execute", &status);
        assert!(!p.met);
        assert_eq!(p.missing, vec!["plan"]);

        status.insert("plan".into(), StatusEntry { complete: true, timestamp: None });
        let p = prerequisites_for("hew-execute", &status);
        assert!(p.met);
    }

    #[test]
    fn plan_has_no_prerequisites() {
        let status: StatusMap = BTreeMap::new();
        let p = prerequisites_for("hew-plan", &status);
        assert!(p.met);
        assert!(p.missing.is_empty());
    }

    #[test]
    fn unknown_skill_errors() {
        assert!(resolve_skill("nope").is_err());
    }

    #[test]
    fn resolve_accepts_short_name() {
        let s = resolve_skill("execute").unwrap();
        assert_eq!(s.name, "hew-execute");
    }

    #[test]
    fn latest_checkpoint_returns_none_when_absent() {
        let m = map(&[("a", "CONVENTION:errors — wrap"), ("b", "Backend: FastAPI")]);
        assert!(latest_checkpoint(&m).is_none());
    }

    #[test]
    fn latest_checkpoint_returns_single_when_one_present() {
        let m = map(&[
            ("a", "CONVENTION:errors — wrap"),
            ("checkpoint-08-35", "CHECKPOINT:2026-05-12T08:35 — Mid auth work."),
        ]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert_eq!(c.key, "checkpoint-08-35");
        assert_eq!(c.timestamp.as_deref(), Some("2026-05-12T08:35"));
        assert!(c.body.contains("Mid auth work"));
    }

    #[test]
    fn latest_checkpoint_picks_most_recent_by_timestamp() {
        let m = map(&[
            ("ck-early", "CHECKPOINT:2026-05-10T08:00 — early"),
            ("ck-late", "CHECKPOINT:2026-05-12T14:30 — late"),
            ("ck-mid", "CHECKPOINT:2026-05-11T12:00 — mid"),
        ]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert_eq!(c.key, "ck-late");
        assert_eq!(c.timestamp.as_deref(), Some("2026-05-12T14:30"));
    }

    #[test]
    fn latest_checkpoint_handles_missing_timestamp() {
        let m = map(&[("ck", "CHECKPOINT:")]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert!(c.timestamp.is_none());
    }

    // Regression for GitHub issue #40: a malformed CHECKPOINT body
    // (no ISO timestamp after the prefix) was sorting *above* a
    // properly-formed newer one, because its non-ISO first token
    // ("practice-svc-l3.2-checkpoint-…") lex-sorted with whatever
    // shape was present. The recogniser now treats non-ISO tokens
    // as "no timestamp" so the malformed entry sorts last.
    #[test]
    fn latest_checkpoint_ignores_non_iso_first_token() {
        let m = map(&[
            ("malformed", "CHECKPOINT:practice-svc-l3.2 — newer in wall-clock but mis-shaped"),
            ("good", "CHECKPOINT:2026-05-20T08:00:00Z — properly shaped"),
        ]);
        let c = latest_checkpoint(&m).expect("checkpoint present");
        assert_eq!(c.key, "good", "well-formed checkpoint must beat malformed one");
        assert_eq!(c.timestamp.as_deref(), Some("2026-05-20T08:00:00Z"));
    }

    fn instr(c: &ConfigInstructions) -> String {
        render_config_instructions(c).join("\n")
    }

    fn default_instructions() -> ConfigInstructions {
        ConfigInstructions {
            branching_strategy: "epic".into(),
            testing_required: false,
            craft_max_function_lines: 0,
            craft_warn_on_unused: true,
            review_after_n_tasks: 0,
            review_after_epic: false,
            research_default: "ask".into(),
            optional_skills: OptionalSkillsView {
                deps: "yes".into(),
                research: "yes".into(),
                security: "yes".into(),
            },
            git_track: false,
        }
    }

    #[test]
    fn config_instructions_render_branching_strategies() {
        let mut c = default_instructions();
        for (strat, expect) in [
            ("epic", "auto-create a feature branch on first task claim per epic"),
            ("always", "every task claim opens a new branch"),
            ("none", "manual"),
        ] {
            c.branching_strategy = strat.into();
            let text = instr(&c);
            assert!(text.contains(expect), "branching={strat} missing `{expect}`:\n{text}");
        }
    }

    #[test]
    fn config_instructions_call_out_required_tests() {
        let mut c = default_instructions();
        c.testing_required = true;
        let text = instr(&c);
        assert!(text.contains("Tests required"), "missing required-tests line:\n{text}");
        assert!(text.contains("hew-guard will fail"));
    }

    #[test]
    fn config_instructions_soft_warn_when_tests_optional() {
        let c = default_instructions();
        let text = instr(&c);
        assert!(text.contains("Tests: soft-warn"), "missing soft-warn line:\n{text}");
    }

    #[test]
    fn config_instructions_omit_zero_threshold_for_function_size() {
        let c = default_instructions();
        let text = instr(&c);
        assert!(
            !text.contains("max_function_lines"),
            "zero threshold should be omitted, got:\n{text}"
        );
    }

    #[test]
    fn config_instructions_surface_review_triggers() {
        let mut c = default_instructions();
        c.review_after_n_tasks = 8;
        c.review_after_epic = true;
        let text = instr(&c);
        assert!(text.contains("after 8 closed tasks"), "missing N-tasks line:\n{text}");
        assert!(text.contains("on every epic close"), "missing per-epic line:\n{text}");
    }

    #[test]
    fn config_instructions_call_out_disabled_optional_skills() {
        let mut c = default_instructions();
        c.optional_skills.security = "no".into();
        c.optional_skills.deps = "ask".into();
        let text = instr(&c);
        assert!(
            text.contains("Optional skills disabled") && text.contains("security"),
            "missing disabled-skills line:\n{text}"
        );
        assert!(
            text.contains("gated on user prompt") && text.contains("deps"),
            "missing ask-gated line:\n{text}"
        );
    }

    #[test]
    fn config_instructions_signal_git_tracking_state() {
        let mut c = default_instructions();
        let text = instr(&c);
        assert!(text.contains("NOT tracked"), "missing untracked line:\n{text}");
        c.git_track = true;
        let text = instr(&c);
        assert!(
            text.contains("tracked in git") && !text.contains("NOT tracked"),
            "missing tracked-in-git line:\n{text}"
        );
    }

    // ---- collect_git_state ---------------------------------------------

    use crate::error::Result as HResult;
    use crate::git::{GitClient, GitOutput};
    use std::collections::HashMap;
    use std::ffi::OsStr;

    /// In-process GitClient that returns pre-canned stdout per first-arg
    /// match. Any unmapped first-arg returns an error so missing setup
    /// surfaces as a test failure, not silent degradation.
    #[derive(Debug)]
    struct CannedGit {
        responses: HashMap<String, std::result::Result<String, ()>>,
        branch: std::result::Result<Option<String>, ()>,
    }

    impl Default for CannedGit {
        fn default() -> Self {
            Self { responses: HashMap::new(), branch: Ok(None) }
        }
    }

    impl CannedGit {
        fn with(mut self, first_arg: &str, body: &str) -> Self {
            self.responses.insert(first_arg.into(), Ok(body.into()));
            self
        }
        fn err(mut self, first_arg: &str) -> Self {
            self.responses.insert(first_arg.into(), Err(()));
            self
        }
        fn branch(mut self, b: Option<&str>) -> Self {
            self.branch = Ok(b.map(str::to_string));
            self
        }
    }

    impl GitClient for CannedGit {
        fn current_branch(&self) -> HResult<Option<String>> {
            match &self.branch {
                Ok(b) => Ok(b.clone()),
                Err(()) => Err(crate::error::HewError::GitNotFound),
            }
        }
        fn checkout_new_branch(&self, _: &str, _: Option<&str>) -> HResult<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> HResult<GitOutput> {
            let first = args.first().map(|a| a.to_string_lossy().into_owned()).unwrap_or_default();
            match self.responses.get(&first) {
                Some(Ok(body)) => Ok(GitOutput { stdout: body.clone(), stderr: String::new() }),
                Some(Err(())) | None => Err(crate::error::HewError::GitNotFound),
            }
        }
    }

    #[test]
    fn git_state_none_when_not_a_repo() {
        let g = CannedGit::default().err("rev-parse");
        assert!(collect_git_state(&g).is_none());
    }

    #[test]
    fn git_state_marks_dirty_and_counts_untracked() {
        let porcelain = " M src/lib.rs\nA  src/new.rs\n?? target/foo\n?? notes.md\n";
        let g = CannedGit::default()
            .with("rev-parse", "/repo/.git")
            .branch(Some("feat/x"))
            .with("status", porcelain);
        let s = collect_git_state(&g).expect("inside repo");
        assert_eq!(s.branch.as_deref(), Some("feat/x"));
        assert!(s.dirty, "tracked changes must mark dirty");
        assert_eq!(s.untracked_count, 2, "?? lines counted as untracked");
    }

    #[test]
    fn git_state_detached_head_branch_is_none() {
        let g =
            CannedGit::default().with("rev-parse", "/repo/.git").branch(None).with("status", "");
        let s = collect_git_state(&g).expect("inside repo");
        assert!(s.branch.is_none(), "detached HEAD → branch=None");
    }
}
