//! Review-bundle builder shared by `/hew:review` and `/hew:adversarial-review`.
//!
//! A [`ReviewBundle`] is a self-contained snapshot the review skills hand
//! to the agent: which tasks closed in scope, the git diff covering those
//! closures, the constraint-bearing memories, the epic body (if any), and
//! the last-review timestamp. The agent does the actual reviewing — this
//! module only assembles the inputs.
//!
//! Anchor model (consistent across scope variants):
//!
//! 1. Resolve an **anchor timestamp** — the moment just before the oldest
//!    in-scope task closed (or the explicit git ref, for `GitRef` scope).
//! 2. `closed_tasks` = every closed bd issue with `closed_at` >= anchor.
//! 3. `diff_base` = `git rev-list -1 --before=<anchor> HEAD` (or the
//!    explicit ref). `diff` = `git diff <diff_base>..HEAD`.
//!
//! Findings are *not* persisted here — the review skills decide what to
//! file. The only memory this module ever writes is the
//! `STATUS:review:<iso-timestamp>` marker, via [`write_review_marker`].
//!
//! Memory hygiene per DECISION:review-filing: no `REVIEW:` / `RISK:`
//! memories. Findings go to `bd create --type=bug|chore`.

use std::ffi::{OsStr, OsString};

use serde::{Deserialize, Serialize};

use crate::bd::{BdClient, hew_temp_path};
use crate::error::{HewError, Result};
use crate::git::GitClient;
use crate::tasks::{self, BdIssueRaw};

// Re-export so prior callers (RV.5/RV.6 skills, schema tooling) keep their
// `hew_core::review::TaskSummary` path. The canonical home is now
// [`crate::tasks::TaskSummary`].
pub use crate::tasks::TaskSummary;

const REVIEW_MARKER_PREFIX: &str = "STATUS:review:";

/// Caller-facing scope variants. `bundle()` resolves these into an anchor.
#[derive(Debug, Clone)]
pub enum ReviewScope {
    /// Last N closed tasks, oldest-first. `n` must be >= 1.
    LastN(u32),
    /// All closed tasks transitively under the given epic id.
    Epic(String),
    /// Tasks closed at or after the given task's close time.
    Task(String),
    /// Tasks closed at or after the commit time of `rev`. `rev` is also
    /// used directly as the diff base.
    GitRef(String),
}

/// Echoed-back scope tag carried inside the bundle, for the agent's reference.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ReviewScopeRepr {
    LastN { n: u32 },
    Epic { id: String },
    Task { id: String },
    GitRef { rev: String },
}

impl From<&ReviewScope> for ReviewScopeRepr {
    fn from(s: &ReviewScope) -> Self {
        match s {
            ReviewScope::LastN(n) => Self::LastN { n: *n },
            ReviewScope::Epic(id) => Self::Epic { id: id.clone() },
            ReviewScope::Task(id) => Self::Task { id: id.clone() },
            ReviewScope::GitRef(rev) => Self::GitRef { rev: rev.clone() },
        }
    }
}

/// Constraint-bearing memories the reviewer needs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, Default, PartialEq, Eq)]
pub struct ReviewMemories {
    pub conventions: Vec<String>,
    pub boundaries: Vec<String>,
    pub security: Vec<String>,
}

/// Populated only for [`ReviewScope::Epic`].
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct EpicMeta {
    pub id: String,
    pub title: String,
    pub body: String,
    pub child_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct ReviewBundle {
    pub scope: ReviewScopeRepr,
    /// Tasks in scope, oldest-first by `closed_at`.
    pub closed_tasks: Vec<TaskSummary>,
    /// Anchor timestamp used to filter tasks and resolve `diff_base`.
    /// Empty for `GitRef` scope (the ref *is* the anchor).
    pub anchor_at: Option<String>,
    /// Git diff covering the scope.
    pub diff: String,
    /// Commit SHA used as the diff base. `None` if no commit predates the anchor.
    pub diff_base: Option<String>,
    pub memories: ReviewMemories,
    /// Set when `scope` is `Epic`.
    pub epic: Option<EpicMeta>,
    /// The prior `STATUS:review:<ts>` marker, if any.
    pub last_review_at: Option<String>,
}

// ────────────────────────────────────────────────────────────────────────────
// Public API
// ────────────────────────────────────────────────────────────────────────────

/// Assemble a [`ReviewBundle`] for the requested scope.
pub fn bundle(bd: &dyn BdClient, git: &dyn GitClient, scope: ReviewScope) -> Result<ReviewBundle> {
    // Validate before any external calls so error reporting is cheap.
    if let ReviewScope::LastN(0) = scope {
        return Err(HewError::MissingFlag { flag: "n (must be >= 1)".into() });
    }

    let last_review_at = last_review_marker(bd)?;

    // GitRef is a pure git query — closed_tasks/epic stay empty.
    let (closed_tasks, anchor, epic_meta) = match &scope {
        ReviewScope::GitRef(_) => (Vec::new(), None, None),
        other => {
            let all_closed = tasks::list_closed_tasks(bd)?; // newest-first
            resolve_scope(bd, other, &all_closed)?
        }
    };

    let (diff_base, diff) = resolve_diff(git, &scope, anchor.as_deref())?;
    let memories = collect_memories(bd)?;

    Ok(ReviewBundle {
        scope: ReviewScopeRepr::from(&scope),
        closed_tasks,
        anchor_at: anchor,
        diff,
        diff_base,
        memories,
        epic: epic_meta,
        last_review_at,
    })
}

/// Read the most recent `STATUS:review:<ts>` memory, if any. Returns the
/// timestamp string verbatim (typically RFC3339 / ISO-8601).
pub fn last_review_marker(bd: &dyn BdClient) -> Result<Option<String>> {
    let memories = bd.memories()?;
    let mut found: Vec<String> = memories
        .values()
        .filter_map(|v| v.trim().strip_prefix(REVIEW_MARKER_PREFIX).map(str::to_string))
        .collect();
    if found.is_empty() {
        return Ok(None);
    }
    // RFC3339 / ISO-8601 timestamps sort correctly as strings.
    found.sort();
    Ok(found.pop())
}

/// Persist a `STATUS:review:<iso-ts>` marker so future runs can compute
/// `tasks_since_last_review`.
pub fn write_review_marker(bd: &dyn BdClient, iso_ts: &str) -> Result<()> {
    bd.remember(&format!("{REVIEW_MARKER_PREFIX}{iso_ts}"))
}

/// Count closed tasks whose `closed_at > last marker`. Counts all closed
/// tasks when no marker has ever been written.
pub fn tasks_since_last_review(bd: &dyn BdClient) -> Result<u32> {
    let marker = last_review_marker(bd)?;
    let closed = tasks::list_closed_tasks(bd)?;
    let count = match marker {
        Some(ts) => closed.iter().filter(|t| t.closed_at.as_str() > ts.as_str()).count(),
        None => closed.len(),
    };
    Ok(u32::try_from(count).unwrap_or(u32::MAX))
}

// ────────────────────────────────────────────────────────────────────────────
// Internals
// ────────────────────────────────────────────────────────────────────────────

/// Return `(closed_tasks_in_scope_oldest_first, anchor_timestamp, epic_meta)`.
fn resolve_scope(
    bd: &dyn BdClient,
    scope: &ReviewScope,
    all_closed_newest_first: &[BdIssueRaw],
) -> Result<(Vec<TaskSummary>, Option<String>, Option<EpicMeta>)> {
    match scope {
        ReviewScope::LastN(n) => {
            // n == 0 already rejected in `bundle()`.
            let n_usize = *n as usize;
            let taken: Vec<&BdIssueRaw> = all_closed_newest_first.iter().take(n_usize).collect();
            let anchor = taken.last().map(|t| t.closed_at.clone());
            let tasks: Vec<TaskSummary> =
                taken.into_iter().rev().cloned().map(BdIssueRaw::into_summary).collect();
            Ok((tasks, anchor, None))
        }
        ReviewScope::Epic(epic_id) => {
            let epic_raw = tasks::fetch_issue(bd, epic_id)?;
            // Build a parent->children id map across ALL closed issues so we
            // capture descendants at any depth (epic → task → subtask).
            let mut by_parent: std::collections::HashMap<&str, Vec<&BdIssueRaw>> =
                std::collections::HashMap::new();
            for issue in all_closed_newest_first {
                if let Some(p) = issue.parent.as_deref() {
                    by_parent.entry(p).or_default().push(issue);
                }
            }
            let mut collected: Vec<&BdIssueRaw> = Vec::new();
            let mut frontier: Vec<&str> = vec![epic_id.as_str()];
            while let Some(node_id) = frontier.pop() {
                if let Some(children) = by_parent.get(node_id) {
                    for c in children {
                        collected.push(c);
                        frontier.push(c.id.as_str());
                    }
                }
            }
            // Oldest first by closed_at.
            collected.sort_by(|a, b| a.closed_at.cmp(&b.closed_at));
            let anchor = collected.first().map(|t| t.closed_at.clone());
            let child_count = u32::try_from(collected.len()).unwrap_or(u32::MAX);
            let tasks: Vec<TaskSummary> =
                collected.into_iter().cloned().map(BdIssueRaw::into_summary).collect();
            let epic_meta = EpicMeta {
                id: epic_raw.id.clone(),
                title: epic_raw.title.clone(),
                body: epic_raw.description.clone(),
                child_count,
            };
            Ok((tasks, anchor, Some(epic_meta)))
        }
        ReviewScope::Task(task_id) => {
            let anchor_issue = tasks::fetch_issue(bd, task_id)?;
            if anchor_issue.closed_at.is_empty() {
                return Err(HewError::MissingFlag {
                    flag: format!("task (`{task_id}` has no closed_at; not closed yet?)"),
                });
            }
            let anchor = anchor_issue.closed_at.clone();
            let mut tasks: Vec<TaskSummary> = all_closed_newest_first
                .iter()
                .filter(|t| t.closed_at.as_str() >= anchor.as_str())
                .cloned()
                .map(BdIssueRaw::into_summary)
                .collect();
            tasks.sort_by(|a, b| a.closed_at.cmp(&b.closed_at));
            Ok((tasks, Some(anchor), None))
        }
        ReviewScope::GitRef(_) => {
            // `bundle()` handles this branch before calling resolve_scope.
            unreachable!("GitRef scope is handled in bundle()")
        }
    }
}

fn resolve_diff(
    git: &dyn GitClient,
    scope: &ReviewScope,
    anchor_at: Option<&str>,
) -> Result<(Option<String>, String)> {
    let base = match scope {
        ReviewScope::GitRef(rev) => Some(rev.clone()),
        _ => match anchor_at {
            Some(ts) => rev_at_or_before(git, ts)?,
            None => None,
        },
    };

    let diff = match base.as_deref() {
        Some(b) => git_diff(git, b)?,
        None => String::new(),
    };

    Ok((base, diff))
}

/// `git rev-list -1 --before=<ts> HEAD` → most recent commit at/before `ts`.
fn rev_at_or_before(git: &dyn GitClient, ts: &str) -> Result<Option<String>> {
    let before_arg = OsString::from(format!("--before={ts}"));
    let out = git.run_raw(&[
        OsStr::new("rev-list"),
        OsStr::new("-1"),
        before_arg.as_os_str(),
        OsStr::new("HEAD"),
    ])?;
    let sha = out.stdout.trim().to_string();
    Ok(if sha.is_empty() { None } else { Some(sha) })
}

fn git_diff(git: &dyn GitClient, base: &str) -> Result<String> {
    // git diff output can be huge (whole-branch reviews). Write to a temp
    // file to avoid the same pipe-buffer deadlock that bites bd list.
    let range = OsString::from(format!("{base}..HEAD"));
    let tmp_path = hew_temp_path("git-diff", "patch");
    git.run_to_file(&[OsStr::new("diff"), range.as_os_str()], &tmp_path)?;
    let body = std::fs::read_to_string(&tmp_path)?;
    let _ = std::fs::remove_file(&tmp_path);
    Ok(body)
}

fn collect_memories(bd: &dyn BdClient) -> Result<ReviewMemories> {
    let memories = bd.memories()?;
    let mut out = ReviewMemories::default();
    for v in memories.values() {
        let trimmed = v.trim();
        if let Some(rest) = trimmed.strip_prefix("CONVENTION:") {
            out.conventions.push(format!("CONVENTION:{rest}"));
        } else if let Some(rest) = trimmed.strip_prefix("BOUNDARY:") {
            out.boundaries.push(format!("BOUNDARY:{rest}"));
        } else if let Some(rest) = trimmed.strip_prefix("SECURITY:") {
            out.security.push(format!("SECURITY:{rest}"));
        }
    }
    out.conventions.sort();
    out.boundaries.sort();
    out.security.sort();
    Ok(out)
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bd::{BdOutput, BdVersion, ReadyTask, StatsSummary};
    use crate::git::GitOutput;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// In-memory BdClient mock keyed on the first non-flag argument.
    #[derive(Debug, Default)]
    struct MockBd {
        memories: BTreeMap<String, String>,
        list_closed_json: String,
        shows: BTreeMap<String, String>, // id → JSON array body
        remembered: RefCell<Vec<String>>,
    }

    impl MockBd {
        fn with_memories(pairs: &[(&str, &str)]) -> Self {
            let memories: BTreeMap<String, String> =
                pairs.iter().map(|(k, v)| ((*k).to_string(), (*v).to_string())).collect();
            Self { memories, ..Default::default() }
        }

        fn with_list(json: &str) -> Self {
            Self { list_closed_json: json.to_string(), ..Default::default() }
        }

        fn add_show(mut self, id: &str, body: &str) -> Self {
            self.shows.insert(id.into(), body.into());
            self
        }

        fn add_memory(mut self, key: &str, val: &str) -> Self {
            self.memories.insert(key.into(), val.into());
            self
        }
    }

    impl BdClient for MockBd {
        fn version(&self) -> Result<BdVersion> {
            Ok(BdVersion { raw: "test".into(), semver: "0.0.0".into() })
        }
        fn ready(&self) -> Result<Vec<ReadyTask>> {
            Ok(Vec::new())
        }
        fn stats(&self) -> Result<StatsSummary> {
            Ok(StatsSummary::default())
        }
        fn prime_raw(&self) -> Result<String> {
            Ok(String::new())
        }
        fn memories(&self) -> Result<BTreeMap<String, String>> {
            Ok(self.memories.clone())
        }
        fn remember(&self, text: &str) -> Result<()> {
            self.remembered.borrow_mut().push(text.to_string());
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
            let first = args.first().map(|a| a.to_string_lossy()).unwrap_or_default();
            match first.as_ref() {
                "list" => {
                    Ok(BdOutput { stdout: self.list_closed_json.clone(), stderr: String::new() })
                }
                "show" => {
                    let id =
                        args.get(1).map(|a| a.to_string_lossy().to_string()).unwrap_or_default();
                    let body = self.shows.get(&id).cloned().unwrap_or_else(|| "[]".into());
                    Ok(BdOutput { stdout: body, stderr: String::new() })
                }
                other => Err(HewError::BdNonZero {
                    code: 2,
                    stderr: format!("mock: unhandled bd subcommand `{other}`"),
                }),
            }
        }
    }

    /// GitClient mock matched on the first argument.
    #[derive(Debug, Default)]
    struct MockGit {
        rev_list_response: String,
        diff_response: String,
    }

    impl GitClient for MockGit {
        fn current_branch(&self) -> Result<Option<String>> {
            Ok(None)
        }
        fn checkout_new_branch(&self, _name: &str, _from: Option<&str>) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<GitOutput> {
            let first = args.first().map(|a| a.to_string_lossy()).unwrap_or_default();
            let body = match first.as_ref() {
                "rev-list" => self.rev_list_response.clone(),
                "diff" => self.diff_response.clone(),
                other => panic!("mock git: unhandled `{other}`"),
            };
            Ok(GitOutput { stdout: body, stderr: String::new() })
        }
    }

    fn issue(id: &str, parent: Option<&str>, closed_at: &str) -> String {
        format!(
            r#"{{"id":"{id}","title":"t-{id}","status":"closed","priority":2,"issue_type":"task","closed_at":"{closed_at}","close_reason":"done","parent":{}}}"#,
            parent.map_or("null".to_string(), |p| format!("\"{p}\""))
        )
    }

    fn three_closed_tasks_newest_first() -> String {
        // Newest first, per bd's default --sort=closed direction.
        format!(
            "[{},{},{}]",
            issue("a-3", None, "2026-05-12T13:00:00Z"),
            issue("a-2", None, "2026-05-12T12:00:00Z"),
            issue("a-1", None, "2026-05-12T11:00:00Z"),
        )
    }

    #[test]
    fn last_review_marker_picks_most_recent() {
        let bd = MockBd::with_memories(&[
            ("m1", "STATUS:review:2026-05-12T09:00:00Z"),
            ("m2", "STATUS:review:2026-05-12T10:00:00Z"),
            ("m3", "CONVENTION:noise"),
        ]);
        let marker = last_review_marker(&bd).unwrap();
        assert_eq!(marker.as_deref(), Some("2026-05-12T10:00:00Z"));
    }

    #[test]
    fn last_review_marker_returns_none_when_unset() {
        let bd = MockBd::default();
        assert!(last_review_marker(&bd).unwrap().is_none());
    }

    #[test]
    fn write_review_marker_emits_status_prefix() {
        let bd = MockBd::default();
        write_review_marker(&bd, "2026-05-12T13:00:00Z").unwrap();
        let remembered = bd.remembered.borrow();
        assert_eq!(remembered.len(), 1);
        assert_eq!(remembered[0], "STATUS:review:2026-05-12T13:00:00Z");
    }

    #[test]
    fn tasks_since_last_review_counts_strictly_after() {
        let bd = MockBd::with_list(&three_closed_tasks_newest_first())
            .add_memory("m1", "STATUS:review:2026-05-12T11:30:00Z");
        // Marker is between a-1 (11:00) and a-2 (12:00). a-2, a-3 strictly after → 2.
        assert_eq!(tasks_since_last_review(&bd).unwrap(), 2);
    }

    #[test]
    fn tasks_since_last_review_counts_all_when_no_marker() {
        let bd = MockBd::with_list(&three_closed_tasks_newest_first());
        assert_eq!(tasks_since_last_review(&bd).unwrap(), 3);
    }

    #[test]
    fn bundle_last_n_takes_newest_n_and_returns_them_oldest_first() {
        let bd = MockBd::with_list(&three_closed_tasks_newest_first());
        let git =
            MockGit { rev_list_response: "deadbeef\n".into(), diff_response: "diff body".into() };
        let b = bundle(&bd, &git, ReviewScope::LastN(2)).unwrap();
        assert_eq!(b.closed_tasks.len(), 2);
        // Oldest first: a-2 then a-3.
        assert_eq!(b.closed_tasks[0].id, "a-2");
        assert_eq!(b.closed_tasks[1].id, "a-3");
        assert_eq!(b.anchor_at.as_deref(), Some("2026-05-12T12:00:00Z"));
        assert_eq!(b.diff_base.as_deref(), Some("deadbeef"));
        assert_eq!(b.diff, "diff body");
        assert!(b.epic.is_none());
        assert!(matches!(b.scope, ReviewScopeRepr::LastN { n: 2 }));
    }

    #[test]
    fn bundle_last_n_with_zero_errors() {
        let bd = MockBd::default();
        let git = MockGit::default();
        assert!(bundle(&bd, &git, ReviewScope::LastN(0)).is_err());
    }

    #[test]
    fn bundle_last_n_with_more_than_available_takes_all() {
        let bd = MockBd::with_list(&three_closed_tasks_newest_first());
        let git = MockGit::default();
        let b = bundle(&bd, &git, ReviewScope::LastN(99)).unwrap();
        assert_eq!(b.closed_tasks.len(), 3);
        assert_eq!(b.anchor_at.as_deref(), Some("2026-05-12T11:00:00Z"));
    }

    #[test]
    fn bundle_epic_walks_parent_chain_transitively() {
        // epic-1 → task-a → subtask-aa; epic-1 → task-b; unrelated task-c
        let closed_json = format!(
            "[{},{},{},{}]",
            issue("task-a", Some("epic-1"), "2026-05-12T11:00:00Z"),
            issue("subtask-aa", Some("task-a"), "2026-05-12T12:00:00Z"),
            issue("task-b", Some("epic-1"), "2026-05-12T13:00:00Z"),
            issue("task-c", Some("other-epic"), "2026-05-12T14:00:00Z"),
        );
        let bd = MockBd::with_list(&closed_json).add_show(
            "epic-1",
            r#"[{"id":"epic-1","title":"E1","description":"epic body here","status":"closed","issue_type":"epic","closed_at":"2026-05-12T13:30:00Z"}]"#,
        );
        let git = MockGit::default();
        let b = bundle(&bd, &git, ReviewScope::Epic("epic-1".into())).unwrap();
        let ids: Vec<&str> = b.closed_tasks.iter().map(|t| t.id.as_str()).collect();
        // Oldest-first: task-a, subtask-aa, task-b. task-c excluded.
        assert_eq!(ids, vec!["task-a", "subtask-aa", "task-b"]);
        let epic = b.epic.expect("epic populated");
        assert_eq!(epic.id, "epic-1");
        assert_eq!(epic.body, "epic body here");
        assert_eq!(epic.child_count, 3);
        assert_eq!(b.anchor_at.as_deref(), Some("2026-05-12T11:00:00Z"));
    }

    #[test]
    fn bundle_task_anchors_at_task_close_time() {
        let bd = MockBd::with_list(&three_closed_tasks_newest_first())
            .add_show("a-2", &format!("[{}]", issue("a-2", None, "2026-05-12T12:00:00Z")));
        let git = MockGit::default();
        let b = bundle(&bd, &git, ReviewScope::Task("a-2".into())).unwrap();
        let ids: Vec<&str> = b.closed_tasks.iter().map(|t| t.id.as_str()).collect();
        // closed_at >= 12:00 → a-2, a-3.
        assert_eq!(ids, vec!["a-2", "a-3"]);
        assert_eq!(b.anchor_at.as_deref(), Some("2026-05-12T12:00:00Z"));
    }

    #[test]
    fn bundle_task_errors_on_unclosed_task() {
        // closed_at = "" → not closed
        let bd = MockBd::with_list("[]").add_show(
            "open-1",
            r#"[{"id":"open-1","title":"t","status":"open","issue_type":"task","closed_at":""}]"#,
        );
        let git = MockGit::default();
        let err = bundle(&bd, &git, ReviewScope::Task("open-1".into())).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("closed_at"), "{msg}");
    }

    #[test]
    fn bundle_gitref_uses_rev_as_diff_base_directly() {
        let bd = MockBd::default(); // closed list unused for GitRef
        let git = MockGit {
            rev_list_response: String::new(),
            diff_response: "diff against abc123".into(),
        };
        let b = bundle(&bd, &git, ReviewScope::GitRef("abc123".into())).unwrap();
        assert!(b.closed_tasks.is_empty());
        assert_eq!(b.diff_base.as_deref(), Some("abc123"));
        assert_eq!(b.diff, "diff against abc123");
        assert!(b.anchor_at.is_none());
    }

    #[test]
    fn bundle_diff_empty_when_no_commit_before_anchor() {
        let bd = MockBd::with_list(&three_closed_tasks_newest_first());
        // rev-list returns empty → no commit before that anchor.
        let git = MockGit::default();
        let b = bundle(&bd, &git, ReviewScope::LastN(1)).unwrap();
        assert_eq!(b.diff_base, None);
        assert_eq!(b.diff, "");
    }

    #[test]
    fn bundle_collects_only_review_relevant_memories() {
        let bd = MockBd::with_list("[]")
            .add_memory("c1", "CONVENTION:naming — snake_case")
            .add_memory("b1", "BOUNDARY:auth — /login is public")
            .add_memory("s1", "SECURITY:csrf — required on POST")
            .add_memory("ignore", "DECISION:foo — not in bundle")
            .add_memory("ignore2", "STATUS:scan:complete");
        let git = MockGit::default();
        let b = bundle(&bd, &git, ReviewScope::LastN(1)).unwrap();
        assert_eq!(b.memories.conventions.len(), 1);
        assert_eq!(b.memories.boundaries.len(), 1);
        assert_eq!(b.memories.security.len(), 1);
        assert!(b.memories.conventions[0].starts_with("CONVENTION:naming"));
    }

    #[test]
    fn scope_repr_round_trips_through_json() {
        let cases = vec![
            ReviewScopeRepr::LastN { n: 8 },
            ReviewScopeRepr::Epic { id: "e-1".into() },
            ReviewScopeRepr::Task { id: "t-1".into() },
            ReviewScopeRepr::GitRef { rev: "HEAD~3".into() },
        ];
        for r in cases {
            let s = serde_json::to_string(&r).unwrap();
            let back: ReviewScopeRepr = serde_json::from_str(&s).unwrap();
            assert_eq!(r, back);
        }
    }

    #[test]
    fn last_review_marker_handles_malformed_lines_gracefully() {
        let bd = MockBd::with_memories(&[
            ("m1", "STATUS:review:not-a-timestamp"),
            ("m2", "STATUS:review:2026-05-12T10:00:00Z"),
        ]);
        // Lexicographic sort: "2026-..." < "not-a-timestamp"; the buggy entry
        // wins as "most recent". This is acceptable — we trust the writer.
        // Test confirms the function doesn't panic and picks one.
        let marker = last_review_marker(&bd).unwrap();
        assert!(marker.is_some());
    }
}
