//! Curated `bd`-issue helpers shared by [`crate::review`] and the `hew`
//! binary's `commands/*` modules.
//!
//! Two contracts live here:
//!
//! 1. Stable agent-facing types: [`TaskSummary`] and [`EpicSummary`] are
//!    schemars-derived and frozen by `hew schema task` / `hew schema epic`.
//!    Adding fields is fine; renaming or removing breaks the contract.
//! 2. Pipe-deadlock-safe queries: every helper that can return large bd
//!    output (`list`, `children`, `search`, `dep_tree`) routes through
//!    [`BdClient::run_to_file`] via [`crate::bd::hew_temp_path`].
//!
//! Memory hygiene: writes go through [`validate_memory_type`] which
//! enforces the [`MEMORY_PREFIXES`] allowlist. Callers that genuinely need
//! a raw prefix should call [`BdClient::remember`] directly.

use std::ffi::{OsStr, OsString};

use serde::{Deserialize, Serialize};

use crate::bd::{BdClient, hew_temp_path};
use crate::error::{HewError, Result};

// ────────────────────────────────────────────────────────────────────────────
// Public types
// ────────────────────────────────────────────────────────────────────────────

/// One task summary, agent-friendly. Mirrors the curated subset of `bd
/// show --json` output; ignores fields the agent never needs.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub issue_type: String,
    pub priority: u8,
    /// `open` / `in_progress` / `blocked` / `closed` / `deferred` —
    /// populated from `bd show --json` `status` field.
    #[serde(default)]
    pub status: String,
    /// Full task body (markdown). Empty for issues that haven't been
    /// fleshed out yet.
    #[serde(default)]
    pub description: String,
    pub closed_at: String,
    pub close_reason: Option<String>,
    pub parent: Option<String>,
}

/// Epic + first-level children. Use [`children`] or repeated [`show_epic`]
/// calls to walk deeper.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
pub struct EpicSummary {
    pub id: String,
    pub title: String,
    pub body: String,
    pub status: String,
    pub closed_at: String,
    pub child_count: u32,
    pub children: Vec<TaskSummary>,
}

/// Filter for [`list`]. Mirrors the most-used `bd list` flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct TaskListFilter {
    /// Comma-joined onto `--status`. Empty means no status filter.
    #[serde(default)]
    pub status: Vec<String>,
    /// Single-value `--type` filter (bd doesn't accept comma lists here).
    #[serde(default)]
    pub issue_type: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    /// Routed as `--closed-after <since>` — pair with `status=["closed"]`.
    #[serde(default)]
    pub since: Option<String>,
    /// `0` means unlimited (matches `bd list --limit 0`).
    #[serde(default)]
    pub n: u32,
    /// `true` = newest-first (bd default for `--sort closed`); `false`
    /// adds `--reverse` for oldest-first.
    #[serde(default = "default_true")]
    pub newest_first: bool,
}

fn default_true() -> bool {
    true
}

/// Args for [`new_task`]. Fields with `None` fall back to `bd q` defaults
/// (type=task, priority=2, no labels). `parent` triggers a follow-up
/// `bd update --parent` since `bd q` doesn't accept the flag.
#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema)]
pub struct NewTaskArgs {
    pub title: String,
    #[serde(default)]
    pub issue_type: Option<String>,
    #[serde(default)]
    pub priority: Option<u8>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub parent: Option<String>,
}

// ────────────────────────────────────────────────────────────────────────────
// Memory-prefix allowlist (per DECISION:hew-remember-type-allowlist)
// ────────────────────────────────────────────────────────────────────────────

/// Allowlisted memory-type prefixes (lower-case). The wrapper UPPER-cases
/// before writing. Anything outside this list is rejected so the memory
/// store stays grep-able.
pub const MEMORY_PREFIXES: &[&str] = &[
    "convention",
    "boundary",
    "security",
    "audit",
    "decision",
    "status",
    "gotcha",
    "feedback",
    "project",
    "milestone",
    "roadmap",
    "research",
    "dep",
    "factual",
    // ML.8 (hew-uxf): `link` joins the allowlist so `hew remember
    // --type=link --raw "LINK:a->relates_to:memory:b"` works without
    // --raw escape hatch. Non-raw form prepends `LINK:` to the body
    // and produces `LINK:<body>` — useful for power-users who pre-
    // format the row; the curated path stays `hew remember --related`.
    "link",
];

/// Validate a `--type` argument against [`MEMORY_PREFIXES`]. Accepts any
/// case (e.g. `Convention`, `CONVENTION`) and returns the canonical UPPER
/// form. Rejects with [`HewError::MissingFlag`] so the CLI surface emits a
/// consistent missing/invalid-flag diagnostic.
pub fn validate_memory_type(t: &str) -> Result<&'static str> {
    let lower = t.trim().to_ascii_lowercase();
    for &p in MEMORY_PREFIXES {
        if lower == p {
            return Ok(canonical_upper(p));
        }
    }
    Err(HewError::MissingFlag {
        flag: format!("type (got `{t}`; allowed: {})", MEMORY_PREFIXES.join(", ")),
    })
}

fn canonical_upper(p: &str) -> &'static str {
    match p {
        "convention" => "CONVENTION",
        "boundary" => "BOUNDARY",
        "security" => "SECURITY",
        "audit" => "AUDIT",
        "decision" => "DECISION",
        "status" => "STATUS",
        "gotcha" => "GOTCHA",
        "feedback" => "FEEDBACK",
        "project" => "PROJECT",
        "milestone" => "MILESTONE",
        "roadmap" => "ROADMAP",
        "research" => "RESEARCH",
        "dep" => "DEP",
        "factual" => "FACTUAL",
        "link" => "LINK",
        _ => unreachable!("checked against MEMORY_PREFIXES"),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Raw bd JSON shape (permissive: `serde(default)` on every field).
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub(crate) struct BdIssueRaw {
    #[serde(default)]
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) description: String,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) priority: u8,
    #[serde(default, rename = "issue_type")]
    pub(crate) issue_type: String,
    #[serde(default)]
    pub(crate) closed_at: String,
    #[serde(default)]
    pub(crate) close_reason: Option<String>,
    #[serde(default)]
    pub(crate) parent: Option<String>,
}

impl BdIssueRaw {
    pub(crate) fn into_summary(self) -> TaskSummary {
        TaskSummary {
            id: self.id,
            title: self.title,
            issue_type: self.issue_type,
            priority: self.priority,
            status: self.status,
            description: self.description,
            closed_at: self.closed_at,
            close_reason: self.close_reason,
            parent: self.parent,
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Reads
// ────────────────────────────────────────────────────────────────────────────

/// `bd show <id> --json` → first element of the `[issue, ...dependents]` array.
pub fn show(bd: &dyn BdClient, id: &str) -> Result<TaskSummary> {
    let raw = fetch_issue(bd, id)?;
    Ok(raw.into_summary())
}

/// Fetch the epic itself plus its direct children (`bd children`). Walks
/// only one level — deeper recursion is the caller's job.
pub fn show_epic(bd: &dyn BdClient, id: &str) -> Result<EpicSummary> {
    let raw = fetch_issue(bd, id)?;
    let children = children(bd, id)?;
    let child_count = u32::try_from(children.len()).unwrap_or(u32::MAX);
    Ok(EpicSummary {
        id: raw.id,
        title: raw.title,
        body: raw.description,
        status: raw.status,
        closed_at: raw.closed_at,
        child_count,
        children,
    })
}

/// `bd list` with the given filter. Routes through `run_to_file` since
/// `--limit 0` on mature projects can exceed the OS pipe buffer.
pub fn list(bd: &dyn BdClient, filter: &TaskListFilter) -> Result<Vec<TaskSummary>> {
    let mut argv: Vec<OsString> = vec![OsString::from("list")];

    if !filter.status.is_empty() {
        argv.push(OsString::from("--status"));
        argv.push(OsString::from(filter.status.join(",")));
    }
    if let Some(t) = &filter.issue_type {
        argv.push(OsString::from("--type"));
        argv.push(OsString::from(t));
    }
    if let Some(p) = &filter.parent {
        argv.push(OsString::from("--parent"));
        argv.push(OsString::from(p));
    }
    if let Some(s) = &filter.since {
        argv.push(OsString::from("--closed-after"));
        argv.push(OsString::from(s));
    }
    argv.push(OsString::from("--sort"));
    argv.push(OsString::from("closed"));
    if !filter.newest_first {
        argv.push(OsString::from("--reverse"));
    }
    argv.push(OsString::from("--limit"));
    argv.push(OsString::from(filter.n.to_string()));
    argv.push(OsString::from("--json"));

    let body = read_json_via_temp(bd, &argv, "bd-list")?;
    let issues: Vec<BdIssueRaw> = serde_json::from_str(body.trim())?;
    Ok(issues.into_iter().map(BdIssueRaw::into_summary).collect())
}

/// `bd children <parent> --json`.
pub fn children(bd: &dyn BdClient, parent_id: &str) -> Result<Vec<TaskSummary>> {
    let argv = [OsString::from("children"), OsString::from(parent_id), OsString::from("--json")];
    let body = read_json_via_temp(bd, &argv, "bd-children")?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let issues: Vec<BdIssueRaw> = serde_json::from_str(trimmed)?;
    Ok(issues.into_iter().map(BdIssueRaw::into_summary).collect())
}

/// `bd search <query> --status all --json` (exclude-closed-by-default is
/// the wrong default for an agent-facing search).
pub fn search(bd: &dyn BdClient, query: &str) -> Result<Vec<TaskSummary>> {
    let argv = [
        OsString::from("search"),
        OsString::from(query),
        OsString::from("--status"),
        OsString::from("all"),
        OsString::from("--json"),
    ];
    let body = read_json_via_temp(bd, &argv, "bd-search")?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let issues: Vec<BdIssueRaw> = serde_json::from_str(trimmed)?;
    Ok(issues.into_iter().map(BdIssueRaw::into_summary).collect())
}

/// `bd list --status blocked --json`. Convenience for "what's stuck."
pub fn blocked(bd: &dyn BdClient) -> Result<Vec<TaskSummary>> {
    list(bd, &TaskListFilter { status: vec!["blocked".into()], ..Default::default() })
}

// ────────────────────────────────────────────────────────────────────────────
// Writes
// ────────────────────────────────────────────────────────────────────────────

/// Atomically claim an issue: `bd update <id> --claim` sets status to
/// `in_progress` and assignee to the current actor in one bd call.
/// Idempotent if you already own it.
pub fn claim(bd: &dyn BdClient, id: &str) -> Result<()> {
    let id_os = OsString::from(id);
    bd.run_raw(&[OsStr::new("update"), id_os.as_os_str(), OsStr::new("--claim")])?;
    Ok(())
}

/// `bd close <id> -r <reason>`. Use [`reopen`] to undo.
pub fn close_with_reason(bd: &dyn BdClient, id: &str, reason: &str) -> Result<()> {
    close_with_reason_force(bd, id, reason, false)
}

/// `bd close <id> -r <reason> [--force]`. When `force` is `true`, the
/// dep-blocker check inside `bd` is bypassed. Useful when a planner
/// added an over-conservative dep that didn't actually gate the work
/// — see GH issue #17.
pub fn close_with_reason_force(
    bd: &dyn BdClient,
    id: &str,
    reason: &str,
    force: bool,
) -> Result<()> {
    let id_os = OsString::from(id);
    let reason_os = OsString::from(reason);
    let mut argv: Vec<&OsStr> =
        vec![OsStr::new("close"), id_os.as_os_str(), OsStr::new("-r"), reason_os.as_os_str()];
    if force {
        argv.push(OsStr::new("--force"));
    }
    bd.run_raw(&argv)?;
    Ok(())
}

/// Mutable fields supported by `hew task update` / [`update_task`].
/// `None` means "leave unchanged"; `Some` is passed through to the
/// corresponding `bd update` flag.
#[derive(Debug, Default, Clone)]
pub struct UpdateTaskArgs<'a> {
    pub title: Option<&'a str>,
    pub description: Option<&'a str>,
    /// Path passed to `bd update --body-file`. Mutually exclusive with
    /// [`description`] at the CLI layer.
    pub description_file: Option<&'a std::path::Path>,
    pub acceptance: Option<&'a str>,
}

impl UpdateTaskArgs<'_> {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.description_file.is_none()
            && self.acceptance.is_none()
    }
}

/// `bd update <id> [--title …] [--description … | --body-file …] [--acceptance …]`.
/// Edits one or more existing task fields. Returns the requested-field
/// count so callers can short-circuit on no-op invocations.
pub fn update_task(bd: &dyn BdClient, id: &str, args: &UpdateTaskArgs<'_>) -> Result<u32> {
    if args.is_empty() {
        return Ok(0);
    }
    let id_os = OsString::from(id);
    let mut argv: Vec<OsString> = vec![OsString::from("update"), id_os];

    let mut changed = 0u32;
    if let Some(t) = args.title {
        argv.push(OsString::from("--title"));
        argv.push(OsString::from(t));
        changed += 1;
    }
    if let Some(d) = args.description {
        argv.push(OsString::from("--description"));
        argv.push(OsString::from(d));
        changed += 1;
    }
    if let Some(p) = args.description_file {
        argv.push(OsString::from("--body-file"));
        argv.push(OsString::from(p));
        changed += 1;
    }
    if let Some(a) = args.acceptance {
        argv.push(OsString::from("--acceptance"));
        argv.push(OsString::from(a));
        changed += 1;
    }

    let ref_args: Vec<&OsStr> = argv.iter().map(|s| s.as_os_str()).collect();
    bd.run_raw(&ref_args)?;
    Ok(changed)
}

/// `bd reopen <id>` (optionally with `-r <reason>`). Clears `closed_at`
/// and emits a Reopened event.
pub fn reopen(bd: &dyn BdClient, id: &str, reason: Option<&str>) -> Result<()> {
    let id_os = OsString::from(id);
    if let Some(r) = reason {
        let r_os = OsString::from(r);
        bd.run_raw(&[OsStr::new("reopen"), id_os.as_os_str(), OsStr::new("-r"), r_os.as_os_str()])?;
    } else {
        bd.run_raw(&[OsStr::new("reopen"), id_os.as_os_str()])?;
    }
    Ok(())
}

/// Create a new task via `bd q` (outputs ID only on stdout). If `parent`
/// is set, follows up with `bd update <id> --parent <p>` since `bd q`
/// doesn't expose `--parent`. Returns the new issue ID.
pub fn new_task(bd: &dyn BdClient, args: NewTaskArgs) -> Result<String> {
    let mut argv: Vec<OsString> = vec![OsString::from("q"), OsString::from(&args.title)];
    if let Some(t) = &args.issue_type {
        argv.push(OsString::from("-t"));
        argv.push(OsString::from(t));
    }
    if let Some(p) = args.priority {
        argv.push(OsString::from("-p"));
        argv.push(OsString::from(p.to_string()));
    }
    if !args.labels.is_empty() {
        argv.push(OsString::from("-l"));
        argv.push(OsString::from(args.labels.join(",")));
    }
    let argv_refs: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
    let out = bd.run_raw(&argv_refs)?;
    let id = out.stdout.trim().to_string();
    if id.is_empty() {
        return Err(HewError::BdNonZero { code: 0, stderr: "`bd q` returned an empty id".into() });
    }

    if let Some(parent) = args.parent {
        let id_os = OsString::from(&id);
        let p_os = OsString::from(&parent);
        bd.run_raw(&[
            OsStr::new("update"),
            id_os.as_os_str(),
            OsStr::new("--parent"),
            p_os.as_os_str(),
        ])?;
    }

    Ok(id)
}

/// `bd note <id> <text>`.
pub fn note(bd: &dyn BdClient, id: &str, text: &str) -> Result<()> {
    let id_os = OsString::from(id);
    let text_os = OsString::from(text);
    bd.run_raw(&[OsStr::new("note"), id_os.as_os_str(), text_os.as_os_str()])?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Dependencies
// ────────────────────────────────────────────────────────────────────────────

/// `bd dep add <dependent> <dependency>` — i.e. *dependent* is blocked by
/// *dependency*. Matches bd's own convention.
pub fn dep_add(bd: &dyn BdClient, dependent: &str, dependency: &str) -> Result<()> {
    let a = OsString::from(dependent);
    let b = OsString::from(dependency);
    bd.run_raw(&[OsStr::new("dep"), OsStr::new("add"), a.as_os_str(), b.as_os_str()])?;
    Ok(())
}

/// `bd dep remove <dependent> <dependency>`. The escape hatch for an
/// accidental `bd mol bond` (see `GOTCHA:bd-mol-bond`).
pub fn dep_remove(bd: &dyn BdClient, dependent: &str, dependency: &str) -> Result<()> {
    let a = OsString::from(dependent);
    let b = OsString::from(dependency);
    bd.run_raw(&[OsStr::new("dep"), OsStr::new("remove"), a.as_os_str(), b.as_os_str()])?;
    Ok(())
}

/// `bd dep tree <id> --json` — returned as-is so agents see bd's native
/// tree shape (depth, edges, status badges).
pub fn dep_tree(bd: &dyn BdClient, id: &str) -> Result<serde_json::Value> {
    let argv = [
        OsString::from("dep"),
        OsString::from("tree"),
        OsString::from(id),
        OsString::from("--json"),
    ];
    let body = read_json_via_temp(bd, &argv, "bd-dep-tree")?;
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Ok(serde_json::Value::Null);
    }
    Ok(serde_json::from_str(trimmed)?)
}

// ────────────────────────────────────────────────────────────────────────────
// Memories
// ────────────────────────────────────────────────────────────────────────────

/// `bd remember <body>` (optionally `--key <k>`). For the no-key case the
/// trait's [`BdClient::remember`] would also work, but routing through
/// `run_raw` keeps the with-key and without-key paths uniform in tests.
pub fn remember(bd: &dyn BdClient, body: &str, key: Option<&str>) -> Result<()> {
    let body_os = OsString::from(body);
    if let Some(k) = key {
        let key_os = OsString::from(k);
        bd.run_raw(&[
            OsStr::new("remember"),
            body_os.as_os_str(),
            OsStr::new("--key"),
            key_os.as_os_str(),
        ])?;
    } else {
        bd.run_raw(&[OsStr::new("remember"), body_os.as_os_str()])?;
    }
    Ok(())
}

/// `bd recall <key>`. Returns `Ok(None)` when bd reports `No memory with
/// key "<key>"` (exit 1) — every other bd failure propagates.
pub fn recall(bd: &dyn BdClient, key: &str) -> Result<Option<String>> {
    let key_os = OsString::from(key);
    match bd.run_raw(&[OsStr::new("recall"), key_os.as_os_str()]) {
        Ok(out) => Ok(Some(out.stdout.trim().to_string())),
        Err(HewError::BdNonZero { stderr, .. }) if stderr.contains("No memory with key") => {
            Ok(None)
        }
        Err(e) => Err(e),
    }
}

/// `bd forget <key>`.
pub fn forget(bd: &dyn BdClient, key: &str) -> Result<()> {
    let key_os = OsString::from(key);
    bd.run_raw(&[OsStr::new("forget"), key_os.as_os_str()])?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Internals
// ────────────────────────────────────────────────────────────────────────────

/// Fetch every closed bd issue. Returns newest-first by `closed_at`. Used
/// by [`crate::review`].
pub(crate) fn list_closed_tasks(bd: &dyn BdClient) -> Result<Vec<BdIssueRaw>> {
    let argv = [
        OsString::from("list"),
        OsString::from("--status=closed"),
        OsString::from("--sort=closed"),
        OsString::from("--limit"),
        OsString::from("0"),
        OsString::from("--json"),
    ];
    let body = read_json_via_temp(bd, &argv, "bd-list-closed")?;
    let issues: Vec<BdIssueRaw> = serde_json::from_str(body.trim())?;
    Ok(issues)
}

/// Fetch a single issue. `bd show --json` returns `[issue, ...dependents]`
/// — we take the first.
pub(crate) fn fetch_issue(bd: &dyn BdClient, id: &str) -> Result<BdIssueRaw> {
    let id_os = OsString::from(id);
    let out = bd.run_raw(&[OsStr::new("show"), id_os.as_os_str(), OsStr::new("--json")])?;
    let arr: Vec<BdIssueRaw> = serde_json::from_str(out.stdout.trim())?;
    arr.into_iter().next().ok_or_else(|| HewError::BdNonZero {
        code: 0,
        stderr: format!("`bd show {id} --json` returned empty array"),
    })
}

/// Route a bd query through `run_to_file` to dodge the pipe-buffer
/// deadlock at `~16KB` macOS / `~64KB` Linux.
fn read_json_via_temp(bd: &dyn BdClient, argv: &[OsString], label: &str) -> Result<String> {
    let argv_refs: Vec<&OsStr> = argv.iter().map(OsString::as_os_str).collect();
    let tmp_path = hew_temp_path(label, "json");
    bd.run_to_file(&argv_refs, &tmp_path)?;
    let body = std::fs::read_to_string(&tmp_path)?;
    let _ = std::fs::remove_file(&tmp_path);
    Ok(body)
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bd::{BdOutput, BdVersion, ReadyTask, StatsSummary};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// MockBd records every argv it sees and returns canned bodies keyed
    /// on the first non-flag argument (or `<first>/<second>` for `dep`).
    #[derive(Debug, Default)]
    struct MockBd {
        // arg-key → stdout body
        responses: BTreeMap<String, String>,
        // arg-key → stderr (used to drive Err paths)
        errors: BTreeMap<String, (i32, String)>,
        calls: RefCell<Vec<Vec<String>>>,
    }

    impl MockBd {
        fn new() -> Self {
            Self::default()
        }

        fn with(mut self, key: &str, body: &str) -> Self {
            self.responses.insert(key.into(), body.into());
            self
        }

        fn err(mut self, key: &str, code: i32, stderr: &str) -> Self {
            self.errors.insert(key.into(), (code, stderr.into()));
            self
        }

        fn last_call(&self) -> Vec<String> {
            self.calls.borrow().last().cloned().unwrap_or_default()
        }

        fn nth_call(&self, n: usize) -> Vec<String> {
            self.calls.borrow().get(n).cloned().unwrap_or_default()
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }

        fn lookup_key(args: &[&OsStr]) -> String {
            let first = args.first().map(|a| a.to_string_lossy().to_string()).unwrap_or_default();
            // `dep add|remove|tree` dispatches on the second token.
            if first == "dep" {
                let second =
                    args.get(1).map(|a| a.to_string_lossy().to_string()).unwrap_or_default();
                return format!("dep {second}");
            }
            first
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
            Ok(BTreeMap::new())
        }
        fn remember(&self, _text: &str) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
            let captured: Vec<String> =
                args.iter().map(|a| a.to_string_lossy().to_string()).collect();
            self.calls.borrow_mut().push(captured);
            let key = Self::lookup_key(args);
            if let Some((code, stderr)) = self.errors.get(&key) {
                return Err(HewError::BdNonZero { code: *code, stderr: stderr.clone() });
            }
            let body = self.responses.get(&key).cloned().unwrap_or_default();
            Ok(BdOutput { stdout: body, stderr: String::new() })
        }
    }

    fn issue_json(id: &str, parent: Option<&str>, status: &str, closed_at: &str) -> String {
        format!(
            r#"{{"id":"{id}","title":"t-{id}","description":"body-{id}","status":"{status}","priority":2,"issue_type":"task","closed_at":"{closed_at}","close_reason":null,"parent":{}}}"#,
            parent.map_or("null".into(), |p| format!("\"{p}\""))
        )
    }

    // ── show / show_epic ────────────────────────────────────────────────

    #[test]
    fn show_parses_first_array_element() {
        let bd = MockBd::new().with(
            "show",
            &format!(
                "[{},{}]",
                issue_json("a-1", None, "closed", "2026-05-12T10:00:00Z"),
                // The trailing element (a "dependent") must be ignored.
                issue_json("a-2", None, "open", ""),
            ),
        );
        let t = show(&bd, "a-1").unwrap();
        assert_eq!(t.id, "a-1");
        assert_eq!(t.title, "t-a-1");
        assert_eq!(t.issue_type, "task");
        assert_eq!(t.closed_at, "2026-05-12T10:00:00Z");
    }

    #[test]
    fn show_errors_on_empty_array() {
        let bd = MockBd::new().with("show", "[]");
        let err = show(&bd, "missing").unwrap_err();
        assert!(matches!(err, HewError::BdNonZero { .. }));
    }

    #[test]
    fn show_tolerates_missing_optional_fields() {
        let bd = MockBd::new().with("show", r#"[{"id":"x","title":"y"}]"#);
        let t = show(&bd, "x").unwrap();
        assert_eq!(t.id, "x");
        assert_eq!(t.parent, None);
        assert_eq!(t.priority, 0);
    }

    #[test]
    fn show_epic_pulls_body_and_children() {
        let bd =
            MockBd::new().with("show", &format!("[{}]", issue_json("e-1", None, "open", ""))).with(
                "children",
                &format!(
                    "[{},{}]",
                    issue_json("e-1.1", Some("e-1"), "open", ""),
                    issue_json("e-1.2", Some("e-1"), "open", ""),
                ),
            );
        let e = show_epic(&bd, "e-1").unwrap();
        assert_eq!(e.id, "e-1");
        assert_eq!(e.body, "body-e-1");
        assert_eq!(e.child_count, 2);
        let ids: Vec<&str> = e.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["e-1.1", "e-1.2"]);
    }

    // ── list / children / search / blocked ──────────────────────────────

    #[test]
    fn list_emits_full_argv_with_filters() {
        let bd = MockBd::new().with("list", "[]");
        let filter = TaskListFilter {
            status: vec!["open".into(), "in_progress".into()],
            issue_type: Some("task".into()),
            parent: Some("e-1".into()),
            since: Some("2026-05-12T00:00:00Z".into()),
            n: 20,
            newest_first: true,
        };
        list(&bd, &filter).unwrap();
        let argv = bd.last_call();
        assert_eq!(argv[0], "list");
        let joined = argv.join(" ");
        assert!(joined.contains("--status open,in_progress"), "{joined}");
        assert!(joined.contains("--type task"), "{joined}");
        assert!(joined.contains("--parent e-1"), "{joined}");
        assert!(joined.contains("--closed-after 2026-05-12T00:00:00Z"), "{joined}");
        assert!(joined.contains("--sort closed"), "{joined}");
        assert!(joined.contains("--limit 20"), "{joined}");
        assert!(joined.contains("--json"), "{joined}");
        assert!(!joined.contains("--reverse"), "{joined}");
    }

    #[test]
    fn list_with_oldest_first_adds_reverse() {
        let bd = MockBd::new().with("list", "[]");
        let filter = TaskListFilter { newest_first: false, ..Default::default() };
        list(&bd, &filter).unwrap();
        assert!(bd.last_call().iter().any(|a| a == "--reverse"));
    }

    #[test]
    fn list_with_n_zero_passes_unlimited() {
        let bd = MockBd::new().with("list", "[]");
        list(&bd, &TaskListFilter::default()).unwrap();
        let argv = bd.last_call();
        let limit_pos = argv.iter().position(|a| a == "--limit").unwrap();
        assert_eq!(argv[limit_pos + 1], "0");
    }

    #[test]
    fn list_parses_response_into_summaries() {
        let bd = MockBd::new().with(
            "list",
            &format!(
                "[{},{}]",
                issue_json("a-1", None, "closed", "2026-05-12T10:00:00Z"),
                issue_json("a-2", None, "closed", "2026-05-12T11:00:00Z"),
            ),
        );
        let out = list(&bd, &TaskListFilter::default()).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "a-1");
    }

    #[test]
    fn children_uses_bd_children_with_parent() {
        let bd = MockBd::new()
            .with("children", &format!("[{}]", issue_json("c-1", Some("p-1"), "open", "")));
        let out = children(&bd, "p-1").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "c-1");
        let argv = bd.last_call();
        assert_eq!(argv[0], "children");
        assert_eq!(argv[1], "p-1");
        assert!(argv.contains(&"--json".to_string()));
    }

    #[test]
    fn search_includes_status_all_and_json() {
        let bd = MockBd::new().with("search", "[]");
        search(&bd, "auth").unwrap();
        let joined = bd.last_call().join(" ");
        assert!(joined.starts_with("search auth"));
        assert!(joined.contains("--status all"));
        assert!(joined.contains("--json"));
    }

    #[test]
    fn blocked_calls_list_with_status_blocked() {
        let bd = MockBd::new().with("list", "[]");
        blocked(&bd).unwrap();
        let joined = bd.last_call().join(" ");
        assert!(joined.contains("--status blocked"), "{joined}");
    }

    // ── writes: claim / close / reopen / new / note ─────────────────────

    #[test]
    fn claim_sends_update_with_claim_flag() {
        let bd = MockBd::new().with("update", "");
        claim(&bd, "t-1").unwrap();
        let argv = bd.last_call();
        assert_eq!(argv, vec!["update", "t-1", "--claim"]);
    }

    #[test]
    fn close_with_reason_passes_reason() {
        let bd = MockBd::new().with("close", "");
        close_with_reason(&bd, "t-1", "shipped via abc123").unwrap();
        let argv = bd.last_call();
        assert_eq!(argv, vec!["close", "t-1", "-r", "shipped via abc123"]);
    }

    #[test]
    fn close_with_force_appends_force_flag() {
        let bd = MockBd::new().with("close", "");
        close_with_reason_force(&bd, "t-1", "dep was bogus", true).unwrap();
        assert_eq!(bd.last_call(), vec!["close", "t-1", "-r", "dep was bogus", "--force"]);
    }

    #[test]
    fn close_without_force_omits_force_flag() {
        let bd = MockBd::new().with("close", "");
        close_with_reason_force(&bd, "t-1", "done", false).unwrap();
        assert_eq!(bd.last_call(), vec!["close", "t-1", "-r", "done"]);
    }

    #[test]
    fn update_task_skips_when_no_fields_set() {
        let bd = MockBd::new();
        let n = update_task(&bd, "t-1", &UpdateTaskArgs::default()).unwrap();
        assert_eq!(n, 0);
        assert_eq!(bd.call_count(), 0, "should not shell out for no-op update");
    }

    #[test]
    fn update_task_passes_each_field_flag() {
        let bd = MockBd::new().with("update", "");
        let n = update_task(
            &bd,
            "t-1",
            &UpdateTaskArgs {
                title: Some("new title"),
                description: Some("new body"),
                description_file: None,
                acceptance: Some("new accept"),
            },
        )
        .unwrap();
        assert_eq!(n, 3);
        let argv = bd.last_call();
        assert!(argv.starts_with(&["update".to_string(), "t-1".to_string()]));
        let joined = argv.join(" ");
        assert!(joined.contains("--title new title"), "{joined}");
        assert!(joined.contains("--description new body"), "{joined}");
        assert!(joined.contains("--acceptance new accept"), "{joined}");
    }

    #[test]
    fn update_task_routes_description_file_to_body_file_flag() {
        let bd = MockBd::new().with("update", "");
        let path = std::path::PathBuf::from("/tmp/spec.md");
        update_task(
            &bd,
            "t-1",
            &UpdateTaskArgs { description_file: Some(&path), ..Default::default() },
        )
        .unwrap();
        let joined = bd.last_call().join(" ");
        assert!(joined.contains("--body-file /tmp/spec.md"), "{joined}");
    }

    #[test]
    fn reopen_without_reason_skips_flag() {
        let bd = MockBd::new().with("reopen", "");
        reopen(&bd, "t-1", None).unwrap();
        assert_eq!(bd.last_call(), vec!["reopen", "t-1"]);
    }

    #[test]
    fn reopen_with_reason_passes_flag() {
        let bd = MockBd::new().with("reopen", "");
        reopen(&bd, "t-1", Some("not done after all")).unwrap();
        assert_eq!(bd.last_call(), vec!["reopen", "t-1", "-r", "not done after all"]);
    }

    #[test]
    fn new_task_parses_bd_q_id_and_skips_parent_call() {
        let bd = MockBd::new().with("q", "hew-9zz\n");
        let id = new_task(&bd, NewTaskArgs { title: "Do the thing".into(), ..Default::default() })
            .unwrap();
        assert_eq!(id, "hew-9zz");
        assert_eq!(bd.call_count(), 1);
        let argv = bd.last_call();
        assert_eq!(argv[0], "q");
        assert_eq!(argv[1], "Do the thing");
    }

    #[test]
    fn new_task_with_parent_chases_with_update() {
        let bd = MockBd::new().with("q", "hew-9zz\n").with("update", "");
        let id = new_task(
            &bd,
            NewTaskArgs {
                title: "Sub".into(),
                issue_type: Some("task".into()),
                priority: Some(1),
                labels: vec!["foo".into(), "bar".into()],
                parent: Some("hew-9aa".into()),
            },
        )
        .unwrap();
        assert_eq!(id, "hew-9zz");
        assert_eq!(bd.call_count(), 2);
        let q_argv = bd.nth_call(0);
        let joined = q_argv.join(" ");
        assert!(joined.contains("-t task"), "{joined}");
        assert!(joined.contains("-p 1"), "{joined}");
        assert!(joined.contains("-l foo,bar"), "{joined}");
        let update_argv = bd.nth_call(1);
        assert_eq!(update_argv, vec!["update", "hew-9zz", "--parent", "hew-9aa"]);
    }

    #[test]
    fn new_task_errors_on_empty_bd_q_output() {
        let bd = MockBd::new().with("q", "");
        let err =
            new_task(&bd, NewTaskArgs { title: "Empty".into(), ..Default::default() }).unwrap_err();
        assert!(matches!(err, HewError::BdNonZero { .. }));
    }

    #[test]
    fn note_sends_id_and_text() {
        let bd = MockBd::new().with("note", "");
        note(&bd, "t-1", "saw a flake").unwrap();
        assert_eq!(bd.last_call(), vec!["note", "t-1", "saw a flake"]);
    }

    // ── deps ────────────────────────────────────────────────────────────

    #[test]
    fn dep_add_argv_order() {
        let bd = MockBd::new().with("dep add", "");
        dep_add(&bd, "t-1", "t-2").unwrap();
        assert_eq!(bd.last_call(), vec!["dep", "add", "t-1", "t-2"]);
    }

    #[test]
    fn dep_remove_argv_order() {
        let bd = MockBd::new().with("dep remove", "");
        dep_remove(&bd, "t-1", "t-2").unwrap();
        assert_eq!(bd.last_call(), vec!["dep", "remove", "t-1", "t-2"]);
    }

    #[test]
    fn dep_tree_returns_passthrough_json() {
        let bd = MockBd::new().with("dep tree", r#"{"root":"t-1","children":[]}"#);
        let v = dep_tree(&bd, "t-1").unwrap();
        assert_eq!(v["root"], "t-1");
        let argv = bd.last_call();
        assert!(argv.contains(&"--json".to_string()));
    }

    // ── memories ────────────────────────────────────────────────────────

    #[test]
    fn recall_returns_some_on_hit() {
        let bd = MockBd::new().with("recall", "the value\n");
        let v = recall(&bd, "some-key").unwrap();
        assert_eq!(v.as_deref(), Some("the value"));
    }

    #[test]
    fn recall_returns_none_when_key_missing() {
        let bd = MockBd::new().err("recall", 1, "No memory with key \"x\"\n");
        let v = recall(&bd, "x").unwrap();
        assert!(v.is_none());
    }

    #[test]
    fn recall_propagates_other_bd_errors() {
        let bd = MockBd::new().err("recall", 2, "some other failure");
        let err = recall(&bd, "x").unwrap_err();
        assert!(matches!(err, HewError::BdNonZero { .. }));
    }

    #[test]
    fn forget_sends_key() {
        let bd = MockBd::new().with("forget", "");
        forget(&bd, "k").unwrap();
        assert_eq!(bd.last_call(), vec!["forget", "k"]);
    }

    // ── validate_memory_type ────────────────────────────────────────────

    #[test]
    fn validate_memory_type_accepts_every_allowlisted_value() {
        for &p in MEMORY_PREFIXES {
            let upper = validate_memory_type(p).unwrap();
            assert_eq!(upper, p.to_ascii_uppercase());
            assert_eq!(validate_memory_type(&p.to_ascii_uppercase()).unwrap(), upper);
        }
    }

    #[test]
    fn validate_memory_type_rejects_unknown() {
        let err = validate_memory_type("review").unwrap_err();
        assert!(matches!(err, HewError::MissingFlag { .. }));
        let msg = err.to_string();
        assert!(msg.contains("type"), "{msg}");
    }

    #[test]
    fn validate_memory_type_trims_whitespace() {
        assert_eq!(validate_memory_type("  decision\n").unwrap(), "DECISION");
    }
}
