//! Pure-data layer for the `hew statusline` agent statusline.
//!
//! The CLI computes a [`StatuslineInput`] from project state and hands
//! it to [`render`], which is a pure function over `(input, format,
//! width)`. No I/O happens here — that's the point. All formatting and
//! progress-bar math is unit-tested with synthetic inputs.
//!
//! Mirrors the `hew_core::compact` / `hew_core::guard` pattern: the
//! data structures are `Serialize + Deserialize + JsonSchema` so a
//! future `hew schema statusline-input` can dump the contract.

use serde::{Deserialize, Serialize};

/// Output verbosity for [`render`].
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum StatuslineFormat {
    /// `<label> <bar> <pct>%`
    Compact,
    /// Compact plus phase + epic-fraction.
    #[default]
    Medium,
    /// Medium plus user/owner segment.
    Full,
}

/// Workflow phase derived from `STATUS:*` markers + task counts.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default,
)]
#[serde(rename_all = "kebab-case")]
pub enum Phase {
    #[default]
    Planning,
    Executing,
    Verifying,
}

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Planning => "planning",
            Phase::Executing => "executing",
            Phase::Verifying => "verifying",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct EpicSnapshot {
    pub id: String,
    pub title: String,
    pub tasks_done: u32,
    pub tasks_total: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
pub struct StatuslineInput {
    pub project_label: Option<String>,
    pub milestone: Option<String>,
    pub current_epic: Option<EpicSnapshot>,
    pub phase: Phase,
    pub tasks_done: u32,
    pub tasks_total: u32,
    pub user: Option<String>,
    pub user_done: u32,
    pub user_total: u32,
}

/// Minimum / maximum bar width. Width is clamped, never panicked.
const BAR_MIN: u32 = 1;
const BAR_MAX: u32 = 80;

const BAR_FILL: char = '\u{2588}'; // █
const BAR_EMPTY: char = '\u{2591}'; // ░

/// Render a progress bar of `width` cells.
///
/// - `total == 0` → all empty.
/// - `done == 0` → all empty.
/// - `done >= total` → all filled.
/// - `width` clamped to `[1, 80]`.
pub fn render_bar(done: u32, total: u32, width: u32) -> String {
    let width = width.clamp(BAR_MIN, BAR_MAX) as usize;
    if total == 0 || done == 0 {
        return BAR_EMPTY.to_string().repeat(width);
    }
    let done = done.min(total) as u64;
    let total = total as u64;
    let w = width as u64;
    let filled = ((done * w) / total) as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let mut out = String::with_capacity(width * 3);
    for _ in 0..filled {
        out.push(BAR_FILL);
    }
    for _ in 0..empty {
        out.push(BAR_EMPTY);
    }
    out
}

fn percent(done: u32, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    let done = done.min(total) as u64;
    let total = total as u64;
    // round half-up
    (((done * 100) + total / 2) / total) as u32
}

/// Pick the scope label shown next to the progress bar.
///
/// Priority: explicit milestone → current epic title → `"(no scope)"`.
pub fn pick_scope_label(input: &StatuslineInput) -> String {
    if let Some(m) = input.milestone.as_deref().filter(|s| !s.is_empty()) {
        return m.to_string();
    }
    if let Some(e) = &input.current_epic
        && !e.title.is_empty()
    {
        return e.title.clone();
    }
    "(no scope)".to_string()
}

/// Walk `STATUS:*` marker keys plus the current task counts and return
/// the inferred [`Phase`].
///
/// - `plan` + `decompose` set, tasks remaining → [`Phase::Executing`]
/// - `plan` + `decompose` set, all tasks done, no `verify` marker → [`Phase::Verifying`]
/// - otherwise → [`Phase::Planning`]
///
/// `status_markers` is a slice of marker keys (e.g. `"STATUS:plan:..."`).
/// Matching is by substring on `":plan"`, `":decompose"`, `":verify"`
/// to tolerate timestamped suffixes.
pub fn detect_phase(status_markers: &[&str], tasks_done: u32, tasks_total: u32) -> Phase {
    let has = |needle: &str| status_markers.iter().any(|m| m.contains(needle));
    let plan = has("plan");
    let decompose = has("decompose");
    let verify = has("verify");
    if plan && decompose {
        if tasks_total > 0 && tasks_done >= tasks_total && !verify {
            return Phase::Verifying;
        }
        return Phase::Executing;
    }
    Phase::Planning
}

/// Render the full statusline.
pub fn render(input: &StatuslineInput, format: StatuslineFormat, width: u32) -> String {
    let label = pick_scope_label(input);
    let bar = render_bar(input.tasks_done, input.tasks_total, width);
    let pct = percent(input.tasks_done, input.tasks_total);
    let mut out = format!("{label} {bar} {pct}%");

    if matches!(format, StatuslineFormat::Medium | StatuslineFormat::Full) {
        out.push_str(" • ");
        out.push_str(input.phase.label());
        if let Some(epic) = &input.current_epic {
            out.push_str(" • ");
            out.push_str(&format!("{}/{}", epic.tasks_done, epic.tasks_total));
        }
    }

    if matches!(format, StatuslineFormat::Full)
        && let Some(user) = input.user.as_deref().filter(|s| !s.is_empty())
    {
        out.push_str(" • ");
        out.push_str(&format!("{user} {}/{}", input.user_done, input.user_total));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell_count(s: &str) -> usize {
        s.chars().count()
    }

    #[test]
    fn render_bar_empty_when_done_zero() {
        let b = render_bar(0, 10, 10);
        assert_eq!(cell_count(&b), 10);
        assert!(b.chars().all(|c| c == BAR_EMPTY));
    }

    #[test]
    fn render_bar_full_when_done_equals_total() {
        let b = render_bar(10, 10, 10);
        assert_eq!(cell_count(&b), 10);
        assert!(b.chars().all(|c| c == BAR_FILL));
    }

    #[test]
    fn render_bar_half() {
        let b = render_bar(5, 10, 10);
        let filled = b.chars().filter(|c| *c == BAR_FILL).count();
        let empty = b.chars().filter(|c| *c == BAR_EMPTY).count();
        assert_eq!(filled, 5);
        assert_eq!(empty, 5);
    }

    #[test]
    fn render_bar_three_over_seven_floors() {
        // floor(3 * 10 / 7) = 4
        let b = render_bar(3, 7, 10);
        let filled = b.chars().filter(|c| *c == BAR_FILL).count();
        let empty = b.chars().filter(|c| *c == BAR_EMPTY).count();
        assert_eq!(filled, 4);
        assert_eq!(empty, 6);
    }

    #[test]
    fn render_bar_total_zero_all_empty() {
        let b = render_bar(5, 0, 8);
        assert_eq!(cell_count(&b), 8);
        assert!(b.chars().all(|c| c == BAR_EMPTY));
    }

    #[test]
    fn render_bar_width_one() {
        // empty case
        let b = render_bar(0, 10, 1);
        assert_eq!(cell_count(&b), 1);
        assert_eq!(b.chars().next().unwrap(), BAR_EMPTY);
        // full case
        let b = render_bar(10, 10, 1);
        assert_eq!(cell_count(&b), 1);
        assert_eq!(b.chars().next().unwrap(), BAR_FILL);
    }

    #[test]
    fn render_bar_width_eighty() {
        let b = render_bar(40, 80, 80);
        assert_eq!(cell_count(&b), 80);
    }

    #[test]
    fn render_bar_width_clamped_at_extremes() {
        // width = 0 → clamp to 1
        let b = render_bar(0, 10, 0);
        assert_eq!(cell_count(&b), 1);
        // width = 999 → clamp to 80
        let b = render_bar(0, 10, 999);
        assert_eq!(cell_count(&b), 80);
    }

    #[test]
    fn render_bar_done_exceeds_total_saturates() {
        let b = render_bar(99, 10, 10);
        assert!(b.chars().all(|c| c == BAR_FILL));
    }

    fn sample_input() -> StatuslineInput {
        StatuslineInput {
            project_label: Some("hew".into()),
            milestone: None,
            current_epic: Some(EpicSnapshot {
                id: "hew-4hk".into(),
                title: "feat/statusline".into(),
                tasks_done: 1,
                tasks_total: 4,
            }),
            phase: Phase::Executing,
            tasks_done: 5,
            tasks_total: 10,
            user: Some("ak".into()),
            user_done: 2,
            user_total: 7,
        }
    }

    #[test]
    fn render_compact_shape() {
        let s = render(&sample_input(), StatuslineFormat::Compact, 10);
        // `<label> <bar> <pct>%`
        assert!(s.starts_with("feat/statusline "));
        assert!(s.ends_with(" 50%"));
        assert!(!s.contains("executing"));
    }

    #[test]
    fn render_medium_includes_phase_and_epic_fraction() {
        let s = render(&sample_input(), StatuslineFormat::Medium, 10);
        assert!(s.contains("executing"));
        assert!(s.contains("1/4"));
        assert!(!s.contains("ak 2/7"));
    }

    #[test]
    fn render_full_appends_user_segment() {
        let s = render(&sample_input(), StatuslineFormat::Full, 10);
        assert!(s.contains("executing"));
        assert!(s.contains("1/4"));
        assert!(s.contains("ak 2/7"));
    }

    #[test]
    fn detect_phase_all_combinations() {
        // no markers → Planning
        assert_eq!(detect_phase(&[], 0, 0), Phase::Planning);
        // only plan → Planning (decompose missing)
        assert_eq!(detect_phase(&["STATUS:plan:2026"], 0, 5), Phase::Planning);
        // plan + decompose, tasks remaining → Executing
        assert_eq!(detect_phase(&["STATUS:plan:x", "STATUS:decompose:x"], 2, 5), Phase::Executing);
        // plan + decompose, all done, no verify → Verifying
        assert_eq!(detect_phase(&["STATUS:plan:x", "STATUS:decompose:x"], 5, 5), Phase::Verifying);
        // plan + decompose + verify, all done → Executing (post-verify falls back)
        assert_eq!(
            detect_phase(&["STATUS:plan:x", "STATUS:decompose:x", "STATUS:verify:x"], 5, 5,),
            Phase::Executing
        );
    }

    #[test]
    fn pick_scope_label_fallback_order() {
        // milestone wins
        let mut i = sample_input();
        i.milestone = Some("m1".into());
        assert_eq!(pick_scope_label(&i), "m1");
        // empty milestone falls through to epic title
        i.milestone = Some(String::new());
        assert_eq!(pick_scope_label(&i), "feat/statusline");
        // no milestone, no epic → "(no scope)"
        i.milestone = None;
        i.current_epic = None;
        assert_eq!(pick_scope_label(&i), "(no scope)");
    }

    #[test]
    fn percent_rounds_half_up_and_handles_zero_total() {
        assert_eq!(percent(0, 0), 0);
        assert_eq!(percent(0, 10), 0);
        assert_eq!(percent(1, 2), 50);
        assert_eq!(percent(1, 3), 33);
        assert_eq!(percent(2, 3), 67);
        assert_eq!(percent(10, 10), 100);
    }

    #[test]
    fn default_format_is_medium() {
        assert_eq!(StatuslineFormat::default(), StatuslineFormat::Medium);
    }

    #[test]
    fn default_phase_is_planning() {
        assert_eq!(Phase::default(), Phase::Planning);
    }
}
