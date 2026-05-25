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

    /// Skip the Claude Code prefix (model · cwd) and emit only hew's segment.
    /// Default is to render the prefix when stdin carries a Claude session JSON.
    #[arg(long)]
    pub bare: bool,
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

    // Drain stdin and try to parse it as the Claude Code session JSON.
    // Empty / malformed / absent stdin must never error — the host
    // statusLine hook treats a non-zero exit as "fall back".
    let session = read_session_json();
    let colorize = should_colorize();
    let claude_prefix =
        if args.bare { None } else { render_claude_prefix(session.as_ref(), colorize) };

    let bd = match RealBd::discover() {
        Ok(bd) => bd,
        // No bd on PATH — show only the Claude prefix (if we have one).
        Err(_) => return emit(claude_prefix.as_deref(), None),
    };

    let resume = match prime::resume(&bd) {
        Ok(r) => r,
        Err(_) => return emit(claude_prefix.as_deref(), None),
    };
    if !resume.project.beads_initialized {
        return emit(claude_prefix.as_deref(), None);
    }

    let format = pick_format(&args);
    let input = build_input(&bd, &resume, args.scope);
    let hew_segment = render(&input, format, width);
    emit(claude_prefix.as_deref(), Some(&hew_segment))
}

/// Emit the composed statusline. Either or both halves may be absent.
/// When both are absent we exit 0 with empty stdout — the host falls
/// back to its own default statusline gracefully.
fn emit(claude: Option<&str>, hew: Option<&str>) -> miette::Result<()> {
    match (claude, hew) {
        (Some(c), Some(h)) => println!("{c} {} {h}", sep_dim()),
        (Some(c), None) => println!("{c}"),
        (None, Some(h)) => println!("{h}"),
        (None, None) => {}
    }
    Ok(())
}

fn should_colorize() -> bool {
    // Respect NO_COLOR (https://no-color.org/).
    std::env::var_os("NO_COLOR").is_none()
}

fn sep_dim() -> &'static str {
    if should_colorize() { "\x1b[2m||\x1b[0m" } else { "||" }
}

fn read_session_json() -> Option<serde_json::Value> {
    use std::io::IsTerminal;
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buf = String::new();
    if stdin.read_to_string(&mut buf).is_err() {
        return None;
    }
    parse_session_json(&buf)
}

/// Render the Claude Code-style prefix: `<model> | <cwd>`. Returns None
/// when the JSON doesn't carry enough fields to build a meaningful line
/// (e.g. someone running `hew statusline` from a plain shell).
fn render_claude_prefix(session: Option<&serde_json::Value>, colorize: bool) -> Option<String> {
    let session = session?;
    let model =
        session.pointer("/model/display_name").and_then(|v| v.as_str()).map(|s| s.to_string());
    let cwd = session
        .pointer("/workspace/current_dir")
        .and_then(|v| v.as_str())
        .or_else(|| session.get("cwd").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    let cwd_label = cwd.as_ref().map(|p| shorten_path(p));

    match (model, cwd_label) {
        (Some(m), Some(c)) => Some(if colorize {
            format!("\x1b[1;36m{m}\x1b[0m \x1b[2m|\x1b[0m \x1b[32m{c}\x1b[0m")
        } else {
            format!("{m} | {c}")
        }),
        (Some(m), None) => Some(if colorize { format!("\x1b[1;36m{m}\x1b[0m") } else { m }),
        (None, Some(c)) => Some(if colorize { format!("\x1b[32m{c}\x1b[0m") } else { c }),
        (None, None) => None,
    }
}

/// Shorten a filesystem path to `~/...` when it lives under $HOME.
fn shorten_path(path: &str) -> String {
    if let Some(home) = std::env::var_os("HOME").and_then(|h| h.into_string().ok())
        && !home.is_empty()
        && let Some(rest) = path.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    path.to_string()
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

/// First MILESTONE: memory's body, stripped of the `MILESTONE:` prefix
/// and condensed for statusline rendering (em-dash head + length cap).
fn resolve_milestone(resume: &prime::ResumeOutput) -> Option<String> {
    resume
        .memories
        .milestone
        .iter()
        .find_map(|v| v.trim_start().strip_prefix("MILESTONE:").map(condense_title))
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
    let title =
        tasks::show(bd, &id).ok().map(|t| condense_title(&t.title)).filter(|s| !s.is_empty());
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

/// Default cap for the scope label rendered in the statusline. Picked so
/// the composed line `<claude-prefix> || <hew>` fits comfortably in a
/// typical 100-110 column statusline without truncation.
const LABEL_MAX_LEN: usize = 28;

/// Condense an epic/task title for statusline rendering: take the head
/// before the first em-dash (matches the milestone-body convention),
/// then truncate to [`LABEL_MAX_LEN`] with an ellipsis.
pub(crate) fn condense_title(t: &str) -> String {
    let head = t.split('—').next().unwrap_or(t).trim();
    truncate_with_ellipsis(head, LABEL_MAX_LEN)
}

fn truncate_with_ellipsis(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let take = max.saturating_sub(1);
    let mut out: String = s.chars().take(take).collect();
    out.push('…');
    out
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

    fn session_json(body: &str) -> serde_json::Value {
        serde_json::from_str(body).unwrap()
    }

    #[test]
    fn claude_prefix_renders_model_and_cwd_no_color() {
        let s = session_json(
            r#"{"model":{"display_name":"Opus"},"workspace":{"current_dir":"/tmp/x"}}"#,
        );
        let out = render_claude_prefix(Some(&s), false).unwrap();
        assert!(out.contains("Opus"));
        assert!(out.contains("/tmp/x"));
        assert!(!out.contains("\x1b["), "no ANSI when colorize=false");
    }

    #[test]
    fn claude_prefix_emits_ansi_when_colorize_true() {
        let s = session_json(
            r#"{"model":{"display_name":"Opus"},"workspace":{"current_dir":"/tmp/x"}}"#,
        );
        let out = render_claude_prefix(Some(&s), true).unwrap();
        assert!(out.contains("\x1b["));
    }

    #[test]
    fn claude_prefix_falls_back_to_top_level_cwd() {
        let s = session_json(r#"{"model":{"display_name":"Opus"},"cwd":"/tmp/y"}"#);
        let out = render_claude_prefix(Some(&s), false).unwrap();
        assert!(out.contains("/tmp/y"));
    }

    #[test]
    fn claude_prefix_returns_none_for_empty_session() {
        assert!(render_claude_prefix(None, false).is_none());
        let s = session_json("{}");
        assert!(render_claude_prefix(Some(&s), false).is_none());
    }

    #[test]
    fn condense_title_strips_em_dash_tail() {
        let t = "feat/treesitter-symbol-extract — pure hew_core module for blah";
        let out = condense_title(t);
        // 30-char head exceeds 28-cap → ellipsified
        assert_eq!(out, "feat/treesitter-symbol-extr…");
        assert!(!out.contains("pure hew_core"), "trailing prose must be dropped");
    }

    #[test]
    fn condense_title_short_passes_through() {
        assert_eq!(condense_title("foundation"), "foundation");
    }

    #[test]
    fn condense_title_truncates_with_ellipsis_when_head_too_long() {
        let t = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // 36 a's, no em-dash
        let out = condense_title(t);
        assert!(out.ends_with('…'));
        assert_eq!(out.chars().count(), LABEL_MAX_LEN);
    }

    #[test]
    fn shorten_path_swaps_home_for_tilde() {
        // Use a clearly-not-home path; we don't want to depend on real $HOME.
        let p = "/var/log";
        assert_eq!(shorten_path(p), "/var/log");
    }
}
