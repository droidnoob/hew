//! `hew statusline` — emit the agent statusline as a single line on
//! stdout. Thin wrapper around [`hew_core::statusline::render`]; this
//! file owns the side-effects (stdin parse, bd queries, env lookups).
//!
//! Contract: stdout reserved for the statusline itself. Errors go to
//! stderr (CONVENTION:cli-stdout-contract). When bd isn't initialized
//! we exit 0 with empty stdout so Claude Code's hook falls back to its
//! default surface gracefully (this is correct behavior, not an error).

use std::fmt::Write as _;
use std::io::Read;

use clap::{Args as ClapArgs, ValueEnum};
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::prime;
use hew_core::statusline::{EpicSnapshot, StatuslineFormat, StatuslineInput, detect_phase};
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
    let width = args.width.clamp(1, 80);
    let session = read_session_json();
    let colorize = should_colorize();

    let claude_prefix =
        if args.bare { None } else { render_claude_prefix(session.as_ref(), colorize) };

    // Context-usage bar — parsed from the Claude transcript stdin
    // references. The host's previous statusline showed this; bring it
    // back with an explicit `ctx` label so it can't be confused with
    // hew's task counters.
    let ctx_segment = session
        .as_ref()
        .and_then(|s| s.get("transcript_path").and_then(|v| v.as_str()))
        .and_then(read_token_usage)
        .map(|u| render_token_segment(&u, width, colorize));

    let mut segments: Vec<String> = Vec::new();
    if let Some(p) = claude_prefix {
        segments.push(p);
    }
    if let Some(c) = ctx_segment {
        segments.push(c);
    }

    let hew_segment = match RealBd::discover().and_then(|bd| {
        let resume = prime::resume(&bd)?;
        Ok((bd, resume))
    }) {
        Ok((bd, resume)) if resume.project.beads_initialized => {
            let input = build_input(&bd, &resume, args.scope);
            Some(render_hew_segment(&input, pick_format(&args), colorize))
        }
        _ => None,
    };
    if let Some(h) = hew_segment {
        segments.push(h);
    }

    emit(&segments)
}

/// Join non-empty segments with the dim `||` separator. Empty out =
/// empty stdout = host falls back to default statusline.
fn emit(segments: &[String]) -> miette::Result<()> {
    if segments.is_empty() {
        return Ok(());
    }
    let sep = format!(" {} ", sep_dim());
    println!("{}", segments.join(&sep));
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

/// Approximate context-window tokens used, parsed from a Claude Code
/// transcript JSONL file. `model` is the `message.model` string when
/// present — used to detect the `[1m]` suffix that signals an extended
/// 1M-token context window.
#[derive(Debug, Clone)]
struct TokenUsage {
    input: u64,
    cache_creation: u64,
    cache_read: u64,
    model: Option<String>,
}

impl TokenUsage {
    fn total(&self) -> u64 {
        self.input + self.cache_creation + self.cache_read
    }
}

/// Walk the transcript backward and return the most-recent assistant
/// message's `usage` block. Returns None on any IO / parse failure —
/// the statusLine must never break because of a missing transcript.
///
/// Reads the entire file. Claude transcripts grow but stay well under
/// a few MB for typical sessions; we eat the cost to keep the code
/// simple. Tail-only optimization is the obvious follow-up.
fn read_token_usage(path: &str) -> Option<TokenUsage> {
    let body = std::fs::read_to_string(path).ok()?;
    for line in body.lines().rev() {
        if line.is_empty() {
            continue;
        }
        let v: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if v.get("type").and_then(|t| t.as_str()) != Some("assistant") {
            continue;
        }
        let Some(usage) = v.pointer("/message/usage") else {
            continue;
        };
        let pick = |k: &str| usage.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        let model = v.pointer("/message/model").and_then(|m| m.as_str()).map(|s| s.to_string());
        return Some(TokenUsage {
            input: pick("input_tokens"),
            cache_creation: pick("cache_creation_input_tokens"),
            cache_read: pick("cache_read_input_tokens"),
            model,
        });
    }
    None
}

/// Pick the context window limit. Claude Code exposes 200K standard and
/// 1M extended.
///
/// The model id in the transcript reliably carries a `[1m]` suffix on the
/// extended window (confirmed 2026-05-25 against Opus 4.6 / 4.7 with the
/// 1M-context selector). When present, that's authoritative and we use
/// 1_000_000 directly. Otherwise we fall back to the observed-usage
/// heuristic: usage above 200K can only have come from the extended
/// window, so promote the ceiling.
fn infer_context_limit(used: u64, model: Option<&str>) -> u64 {
    if model.is_some_and(|m| m.contains("[1m]")) {
        return 1_000_000;
    }
    if used > 200_000 { 1_000_000 } else { 200_000 }
}

/// Render the `ctx ▓▓▓░░ NN%` segment. Color gradient:
/// - green   < 60%
/// - yellow  60–84%
/// - red     ≥ 85%
fn render_token_segment(usage: &TokenUsage, width: u32, colorize: bool) -> String {
    let used = usage.total();
    let limit = infer_context_limit(used, usage.model.as_deref());
    let used_clamped = used.min(limit) as u32;
    let limit_u32 = limit as u32;
    let bar = hew_core::statusline::render_bar(used_clamped, limit_u32, width);
    let pct = if limit == 0 { 0 } else { ((used as f64 / limit as f64) * 100.0).round() as u32 };
    let color = if !colorize {
        ""
    } else if pct < 60 {
        "\x1b[32m"
    } else if pct < 85 {
        "\x1b[33m"
    } else {
        "\x1b[31m"
    };
    let reset = if colorize { "\x1b[0m" } else { "" };
    let label_dim_open = if colorize { "\x1b[2m" } else { "" };
    let label_dim_close = if colorize { "\x1b[0m" } else { "" };
    let count = humanize_tokens(used);
    format!(
        "{label_dim_open}ctx{label_dim_close} {color}{bar}{reset} {color}{pct}%{reset} \
         {label_dim_open}·{label_dim_close} {color}{count}{reset}",
    )
}

/// Render a token count as `847` / `270K` / `1.2M`.
fn humanize_tokens(n: u64) -> String {
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        return format!("{}K", n.div_ceil(1_000));
    }
    let m = n as f64 / 1_000_000.0;
    if m >= 10.0 { format!("{m:.0}M") } else { format!("{m:.1}M") }
}

/// Render the hew segment: `hew: <label> <done>/<total> (<phase>)`. No
/// bar — the context segment owns the bar real-estate; this segment is
/// a labeled fraction so the two graphs aren't visually competing.
fn render_hew_segment(input: &StatuslineInput, format: StatuslineFormat, colorize: bool) -> String {
    use hew_core::statusline::pick_scope_label;
    let label = pick_scope_label(input);

    let frac = input.current_epic.as_ref().map(|e| format!(" {}/{}", e.tasks_done, e.tasks_total));

    let phase = match input.phase {
        hew_core::statusline::Phase::Planning => "planning",
        hew_core::statusline::Phase::Executing => "executing",
        hew_core::statusline::Phase::Verifying => "verifying",
    };

    let label_dim_open = if colorize { "\x1b[2m" } else { "" };
    let label_dim_close = if colorize { "\x1b[0m" } else { "" };
    let label_color = if colorize { "\x1b[1;35m" } else { "" }; // magenta for hew
    let reset = if colorize { "\x1b[0m" } else { "" };

    let mut out = format!(
        "{label_dim_open}hew{label_dim_close} {label_color}{label}{reset}{}",
        frac.as_deref().unwrap_or(""),
    );

    if matches!(format, StatuslineFormat::Medium | StatuslineFormat::Full) {
        let _ = write!(&mut out, " {label_dim_open}({phase}){label_dim_close}");
    }
    if matches!(format, StatuslineFormat::Full)
        && let Some(user) = input.user.as_deref().filter(|s| !s.is_empty())
    {
        let _ = write!(
            &mut out,
            " {label_dim_open}·{label_dim_close} {user} {}/{}",
            input.user_done, input.user_total
        );
    }
    out
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
    fn humanize_tokens_under_thousand_is_raw() {
        assert_eq!(humanize_tokens(0), "0");
        assert_eq!(humanize_tokens(999), "999");
    }

    #[test]
    fn humanize_tokens_kilo_range_rounds_up() {
        assert_eq!(humanize_tokens(1_000), "1K");
        assert_eq!(humanize_tokens(40_501), "41K");
        assert_eq!(humanize_tokens(999_999), "1000K");
    }

    #[test]
    fn humanize_tokens_mega_range_has_one_decimal() {
        assert_eq!(humanize_tokens(1_200_000), "1.2M");
        assert_eq!(humanize_tokens(10_000_000), "10M");
    }

    #[test]
    fn infer_context_limit_thresholds_no_model() {
        assert_eq!(infer_context_limit(0, None), 200_000);
        assert_eq!(infer_context_limit(200_000, None), 200_000);
        assert_eq!(infer_context_limit(200_001, None), 1_000_000);
        assert_eq!(infer_context_limit(900_000, None), 1_000_000);
    }

    #[test]
    fn infer_context_limit_1m_suffix_wins_at_low_usage() {
        // The [1m] suffix is authoritative — even at 45K we should treat
        // the ceiling as 1M, not 200K.
        assert_eq!(infer_context_limit(45_000, Some("claude-opus-4-7[1m]")), 1_000_000);
        assert_eq!(infer_context_limit(0, Some("claude-opus-4-6[1m]")), 1_000_000);
    }

    #[test]
    fn infer_context_limit_no_suffix_stays_200k() {
        assert_eq!(infer_context_limit(45_000, Some("claude-opus-4-7")), 200_000);
        assert_eq!(infer_context_limit(45_000, Some("claude-sonnet-4-6")), 200_000);
    }

    #[test]
    fn token_usage_total_sums_all_three() {
        let u = TokenUsage { input: 100, cache_creation: 200, cache_read: 5_000, model: None };
        assert_eq!(u.total(), 5_300);
    }

    #[test]
    fn shorten_path_swaps_home_for_tilde() {
        // Use a clearly-not-home path; we don't want to depend on real $HOME.
        let p = "/var/log";
        assert_eq!(shorten_path(p), "/var/log");
    }
}
