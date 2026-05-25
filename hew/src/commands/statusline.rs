//! `hew statusline` — emit the agent statusline as a single line on
//! stdout. Thin wrapper around [`hew_core::statusline::render`]; this
//! file owns the side-effects (stdin parse, bd queries, env lookups).
//!
//! Contract: stdout reserved for the statusline itself. Errors go to
//! stderr (CONVENTION:cli-stdout-contract). When bd isn't initialized
//! we exit 0 with empty stdout so Claude Code's hook falls back to its
//! default surface gracefully (this is correct behavior, not an error).

use std::io::Read;

use clap::{Args as ClapArgs, ValueEnum};
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::prime;
use hew_core::statusline::{EpicSnapshot, StatuslineFormat, StatuslineInput, detect_phase, render};
use hew_core::tasks;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Compact format: `<label> <bar> <pct>%` only. Mutually exclusive with --full.
    #[arg(long, conflicts_with = "full")]
    pub compact: bool,

    /// Full format: medium plus user/owner segment.
    #[arg(long)]
    pub full: bool,

    /// Which scope label to show. `auto` falls back through milestone → epic → "(no scope)".
    #[arg(long, value_enum, default_value_t = ScopeArg::Auto)]
    pub scope: ScopeArg,

    /// Progress-bar width in cells. Clamped to [1, 80].
    #[arg(long, default_value_t = 10)]
    pub width: u32,
}

#[derive(Debug, Copy, Clone, ValueEnum)]
pub enum ScopeArg {
    Auto,
    Project,
    Milestone,
    Epic,
}

pub fn run(_ctx: &Ctx, args: Args) -> miette::Result<()> {
    // Validate early — fail fast on bad width before any bd work.
    let width = args.width.clamp(1, 80);

    // Drain stdin tolerantly. The Claude Code SessionStart hook pipes
    // a JSON document; we currently don't consume any field from it
    // (project label and user come from env / memories), but we MUST
    // not error if stdin is empty, malformed, or absent.
    consume_stdin();

    let bd = match RealBd::discover() {
        Ok(bd) => bd,
        // No bd on PATH or no .beads/ — exit 0 silently.
        Err(_) => return Ok(()),
    };

    // The bd graph may not be initialized in this dir. `version()` is
    // cheap and works without an init; `stats()` fails on an uninited
    // graph. If stats fails we treat it as "not initialized" and exit
    // quietly — same contract.
    let resume = match prime::resume(&bd) {
        Ok(r) => r,
        Err(_) => return Ok(()),
    };
    if !resume.project.beads_initialized {
        return Ok(());
    }

    let format = pick_format(&args);
    let input = build_input(&bd, &resume, args.scope);
    println!("{}", render(&input, format, width));
    Ok(())
}

fn pick_format(args: &Args) -> StatuslineFormat {
    if args.compact {
        StatuslineFormat::Compact
    } else if args.full {
        StatuslineFormat::Full
    } else {
        StatuslineFormat::Medium
    }
}

fn consume_stdin() {
    // Detect TTY so we don't block waiting for input the user will
    // never type. When stdin is a TTY: skip. When piped: drain.
    use std::io::IsTerminal;
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return;
    }
    let mut buf = String::new();
    let _ = stdin.read_to_string(&mut buf);
    // Lenient JSON peek: parse if possible, ignore if not.
    let _ = parse_session_json(&buf);
}

/// Lenient parser for the Claude Code SessionStart JSON. Returns None on
/// any failure; never panics. Exposed for inline tests.
fn parse_session_json(s: &str) -> Option<serde_json::Value> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    serde_json::from_str(trimmed).ok()
}

fn build_input(
    bd: &dyn BdClient,
    resume: &prime::ResumeOutput,
    scope: ScopeArg,
) -> StatuslineInput {
    let tasks_done = clamp_u32(resume.tasks.done);
    let tasks_total = clamp_u32(resume.tasks.total);

    let current_epic = resolve_current_epic(bd, resume);
    let milestone = resolve_milestone(resume);
    let project_label = resolve_project_label(&milestone);
    let user = std::env::var("USER").ok().filter(|s| !s.is_empty());

    let (user_done, user_total) = (0, 0); // not surfaced in MVP; reserved for Full format.

    // Apply explicit --scope override by zeroing the non-selected slots
    // so pick_scope_label falls through to the right one. Cheap and
    // keeps the pure render fn unaware of scope-arg semantics.
    let (milestone_eff, epic_eff) = match scope {
        ScopeArg::Auto => (milestone.clone(), current_epic.clone()),
        ScopeArg::Project => (None, None),
        ScopeArg::Milestone => (milestone.clone(), None),
        ScopeArg::Epic => (None, current_epic.clone()),
    };

    let markers: Vec<&str> =
        resume.status.iter().filter(|(_, e)| e.complete).map(|(k, _)| k.as_str()).collect();
    let phase = detect_phase(&markers, tasks_done, tasks_total);

    StatuslineInput {
        project_label,
        milestone: milestone_eff,
        current_epic: epic_eff,
        phase,
        tasks_done,
        tasks_total,
        user,
        user_done,
        user_total,
    }
}

fn clamp_u32(n: u64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// First MILESTONE: memory's body, stripped of the `MILESTONE:` prefix.
fn resolve_milestone(resume: &prime::ResumeOutput) -> Option<String> {
    resume
        .memories
        .milestone
        .iter()
        .find_map(|v| v.trim_start().strip_prefix("MILESTONE:").map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
}

/// Project label: milestone's first phrase before `—` (em-dash) or `-`
/// fallback, otherwise the current dir's file name.
pub(crate) fn resolve_project_label(milestone: &Option<String>) -> Option<String> {
    if let Some(m) = milestone {
        let head = m.split('—').next().unwrap_or(m).trim();
        if !head.is_empty() {
            return Some(head.to_string());
        }
    }
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
}

/// Pick the current epic: the parent of the first in_progress task,
/// fallback to the first ready epic. Returns `None` if neither exists.
///
/// Epic title comes from `bd show <id>`; failures degrade to using the
/// id as the title. Tasks_done / tasks_total come from `bd children`.
fn resolve_current_epic(bd: &dyn BdClient, resume: &prime::ResumeOutput) -> Option<EpicSnapshot> {
    let id = epic_id_from_in_progress(resume).or_else(|| epic_id_from_ready(resume))?;

    // Best-effort enrichment. Any failure → id-only snapshot.
    let title = tasks::show(bd, &id).ok().map(|t| t.title).filter(|s| !s.is_empty());
    let children = tasks::children(bd, &id).ok().unwrap_or_default();
    let total = children.len() as u32;
    let done = children.iter().filter(|c| c.status == "closed").count() as u32;
    Some(EpicSnapshot {
        id: id.clone(),
        title: title.unwrap_or(id),
        tasks_done: done,
        tasks_total: total,
    })
}

fn epic_id_from_in_progress(resume: &prime::ResumeOutput) -> Option<String> {
    resume.in_progress.iter().find_map(|t| t.parent.clone())
}

fn epic_id_from_ready(resume: &prime::ResumeOutput) -> Option<String> {
    resume.tasks.ready_list.iter().find(|t| t.issue_type == "epic").map(|t| t.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_session_json_empty_is_none() {
        assert!(parse_session_json("").is_none());
        assert!(parse_session_json("   \n").is_none());
    }

    #[test]
    fn parse_session_json_malformed_is_none() {
        assert!(parse_session_json("{not json").is_none());
        assert!(parse_session_json("garbage").is_none());
    }

    #[test]
    fn parse_session_json_ok() {
        let v = parse_session_json(r#"{"model": {"display_name": "Opus"}}"#).unwrap();
        assert_eq!(v.pointer("/model/display_name").and_then(|v| v.as_str()), Some("Opus"));
    }

    #[test]
    fn project_label_strips_milestone_em_dash() {
        let m = Some("foundation — walking skeleton".to_string());
        assert_eq!(resolve_project_label(&m).as_deref(), Some("foundation"));
    }

    #[test]
    fn project_label_falls_back_to_cwd_when_milestone_absent() {
        let label = resolve_project_label(&None);
        // CWD-derived; only assert it's *something*.
        assert!(label.is_some());
        assert!(!label.unwrap().is_empty());
    }

    #[test]
    fn project_label_falls_back_when_milestone_empty_head() {
        // milestone with empty pre-emdash chunk — should fall through.
        let m = Some("— body only".to_string());
        let label = resolve_project_label(&m);
        // current_dir().file_name() picks up.
        assert!(label.is_some());
    }
}
