//! Per-worker branch consolidation at parallel-loop shutdown.
//!
//! At run end, each per-worker branch (`loop/<run-id>/w<n>`) merges back
//! onto a single base branch with `git merge --no-ff --no-edit`. Linear
//! cases succeed silently; conflicts are aborted (`git merge --abort`),
//! recorded as [`ConflictReport`]s, and filed as `[merge-conflict]` bug
//! tasks so a human can resolve them under the still-on-disk worktree
//! (per `DECISION:loop-parallel-overlap-policy`).
//!
//! `--no-ff` preserves worker history for archaeology — every worker
//! branch lands as a merge commit even when fast-forward would have been
//! possible.
//!
//! Tests inject a [`GitClient`] fake; no real `git` runs.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use crate::bd::BdClient;
use crate::error::{HewError, Result};
use crate::git::GitClient;
use crate::tasks::{self, NewTaskArgs, UpdateTaskArgs};

/// Outcome of [`merge_back`]: each input branch ends up in exactly one of
/// `merged` or `conflicts`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MergeReport {
    pub merged: Vec<String>,
    pub conflicts: Vec<ConflictReport>,
}

/// One worker branch whose `git merge` produced conflicts. `files` are
/// the paths git reported as unmerged (`diff --diff-filter=U`). `hint`
/// is a human-readable resolution suggestion that points at the worktree
/// path the worker used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictReport {
    pub branch: String,
    pub files: Vec<PathBuf>,
    pub hint: String,
}

/// Sequentially merge each `worker_branches` entry onto `base_branch`
/// under `project_root`. Stops only on a *non-conflict* git failure
/// (e.g. missing branch, broken repo); conflicts are recorded and the
/// loop continues to the next branch so a single bad merge doesn't
/// strand sibling work.
///
/// Caller is responsible for being on `base_branch` before this runs.
/// We don't `git checkout` for you — the dispatcher already owns the
/// main worktree, and a stray checkout here would clash with any
/// concurrent worker still holding a worktree.
pub fn merge_back(
    git: &dyn GitClient,
    project_root: &Path,
    base_branch: &str,
    worker_branches: &[String],
) -> Result<MergeReport> {
    let mut report = MergeReport::default();
    let _ = base_branch; // surfaced in the API for callers; git uses HEAD implicitly.

    for branch in worker_branches {
        match attempt_merge(git, project_root, branch) {
            Ok(()) => report.merged.push(branch.clone()),
            Err(MergeOutcome::Conflict { files }) => {
                // Abort the in-progress merge so the next iteration can
                // start clean. If the abort itself fails, surface that
                // — the working tree is in an unknown state and the
                // operator must intervene.
                abort_merge(git, project_root)?;
                let hint = format!(
                    "Worker branch `{branch}` conflicts with prior merges. \
                     Inspect under `~/.hew/wt/<run-id>/<n>/`, resolve, then re-run \
                     `git -C {} merge --no-ff --no-edit {branch}` manually.",
                    project_root.display()
                );
                report.conflicts.push(ConflictReport { branch: branch.clone(), files, hint });
            }
            Err(MergeOutcome::Fatal(e)) => return Err(e),
        }
    }

    Ok(report)
}

/// File one `[merge-conflict]` bug task per [`ConflictReport`] via `bd q`,
/// then attach a description listing the conflicting files. Returns the
/// new issue IDs in the same order as `conflicts`.
///
/// Title shape: `[merge-conflict] hew loop run <run-id> worker-<n>` when
/// `branch` parses as the standard `loop/<run-id>/w<n>` pattern;
/// otherwise the branch name appears verbatim so the operator can still
/// trace it.
pub fn file_conflict_bug_tasks(
    bd: &dyn BdClient,
    run_id: &str,
    conflicts: &[ConflictReport],
) -> Result<Vec<String>> {
    let mut ids = Vec::with_capacity(conflicts.len());
    for c in conflicts {
        let title = match parse_worker_n(&c.branch) {
            Some(n) => format!("[merge-conflict] hew loop run {run_id} worker-{n}"),
            None => format!("[merge-conflict] hew loop run {run_id} branch {}", c.branch),
        };
        let id = tasks::new_task(
            bd,
            NewTaskArgs {
                title,
                issue_type: Some("bug".into()),
                priority: Some(2),
                labels: vec!["merge-conflict".into()],
                ..Default::default()
            },
        )?;
        let body = build_conflict_body(run_id, c);
        tasks::update_task(
            bd,
            &id,
            &UpdateTaskArgs { description: Some(&body), ..Default::default() },
        )?;
        ids.push(id);
    }
    Ok(ids)
}

// ────────────────────────────────────────────────────────────────────────────
// Internals
// ────────────────────────────────────────────────────────────────────────────

enum MergeOutcome {
    Conflict { files: Vec<PathBuf> },
    Fatal(HewError),
}

fn attempt_merge(
    git: &dyn GitClient,
    project_root: &Path,
    branch: &str,
) -> std::result::Result<(), MergeOutcome> {
    let project_root_s = project_root.as_os_str();
    let branch_os = OsString::from(branch);
    let res = git.run_raw(&[
        OsStr::new("-C"),
        project_root_s,
        OsStr::new("merge"),
        OsStr::new("--no-ff"),
        OsStr::new("--no-edit"),
        branch_os.as_os_str(),
    ]);
    match res {
        Ok(_) => Ok(()),
        Err(HewError::GitNonZero { .. }) => {
            // Could be a conflict (most common) or a different failure
            // (missing ref, etc). `diff --name-only --diff-filter=U`
            // returns the unmerged paths; an empty list means there's
            // no in-progress merge — i.e. this was a non-conflict
            // failure, propagate it as fatal.
            let files = match unmerged_paths(git, project_root) {
                Ok(f) => f,
                Err(e) => return Err(MergeOutcome::Fatal(e)),
            };
            if files.is_empty() {
                Err(MergeOutcome::Fatal(HewError::GitNonZero {
                    code: 1,
                    stderr: format!("`git merge {branch}` failed without conflict files"),
                }))
            } else {
                Err(MergeOutcome::Conflict { files })
            }
        }
        Err(e) => Err(MergeOutcome::Fatal(e)),
    }
}

fn unmerged_paths(git: &dyn GitClient, project_root: &Path) -> Result<Vec<PathBuf>> {
    let project_root_s = project_root.as_os_str();
    let out = git.run_raw(&[
        OsStr::new("-C"),
        project_root_s,
        OsStr::new("diff"),
        OsStr::new("--name-only"),
        OsStr::new("--diff-filter=U"),
    ])?;
    Ok(out.stdout.lines().filter(|l| !l.is_empty()).map(PathBuf::from).collect())
}

fn abort_merge(git: &dyn GitClient, project_root: &Path) -> Result<()> {
    let project_root_s = project_root.as_os_str();
    git.run_raw(&[OsStr::new("-C"), project_root_s, OsStr::new("merge"), OsStr::new("--abort")])?;
    Ok(())
}

/// Parse `loop/<run-id>/w<n>` and return `n`. Returns `None` for any
/// other branch shape so the bug-task title falls back to verbatim.
fn parse_worker_n(branch: &str) -> Option<u32> {
    let last = branch.rsplit('/').next()?;
    let n = last.strip_prefix('w')?;
    n.parse::<u32>().ok()
}

fn build_conflict_body(run_id: &str, c: &ConflictReport) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "Merge of worker branch `{}` onto base failed during `hew loop run` `{run_id}`.\n\n",
        c.branch
    ));
    s.push_str("**Conflicting files:**\n");
    if c.files.is_empty() {
        s.push_str("- (none reported)\n");
    } else {
        for f in &c.files {
            s.push_str(&format!("- `{}`\n", f.display()));
        }
    }
    s.push_str("\n**Hint:**\n");
    s.push_str(&c.hint);
    s.push('\n');
    s
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bd::{BdClient, BdOutput, BdVersion, ReadyTask, StatsSummary};
    use crate::git::{GitClient, GitOutput};
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// Records every argv. `fail_merge_for` makes `merge --no-ff` of a
    /// listed branch fail with GitNonZero. `unmerged_files` is returned
    /// from `diff --diff-filter=U`. Subsequent merges after an abort
    /// succeed unless their branch is also in `fail_merge_for`.
    #[derive(Debug, Default)]
    struct FakeGit {
        calls: RefCell<Vec<Vec<String>>>,
        fail_merge_for: RefCell<Vec<String>>,
        unmerged_files: RefCell<Vec<String>>,
    }

    impl FakeGit {
        fn new() -> Self {
            Self::default()
        }
        fn with_failing_merge(self, branch: &str) -> Self {
            self.fail_merge_for.borrow_mut().push(branch.into());
            self
        }
        fn with_unmerged_files(self, files: &[&str]) -> Self {
            *self.unmerged_files.borrow_mut() = files.iter().map(|s| (*s).to_string()).collect();
            self
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl GitClient for FakeGit {
        fn current_branch(&self) -> Result<Option<String>> {
            Ok(None)
        }
        fn checkout_new_branch(&self, _: &str, _: Option<&str>) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<GitOutput> {
            let captured: Vec<String> =
                args.iter().map(|a| a.to_string_lossy().to_string()).collect();
            self.calls.borrow_mut().push(captured.clone());

            // `merge --no-ff --no-edit <branch>`
            if let Some(pos) = captured.iter().position(|a| a == "merge")
                && captured.get(pos + 1).map(String::as_str) == Some("--no-ff")
            {
                let branch = captured.get(pos + 3).cloned().unwrap_or_default();
                if self.fail_merge_for.borrow().iter().any(|b| b == &branch) {
                    return Err(HewError::GitNonZero {
                        code: 1,
                        stderr: format!("Automatic merge failed for {branch}"),
                    });
                }
                return Ok(GitOutput { stdout: String::new(), stderr: String::new() });
            }

            // `diff --name-only --diff-filter=U`
            if captured.iter().any(|a| a == "diff")
                && captured.iter().any(|a| a == "--diff-filter=U")
            {
                let body = self.unmerged_files.borrow().join("\n");
                return Ok(GitOutput {
                    stdout: if body.is_empty() { String::new() } else { format!("{body}\n") },
                    stderr: String::new(),
                });
            }

            // `merge --abort` — always succeeds.
            if captured.iter().any(|a| a == "merge") && captured.iter().any(|a| a == "--abort") {
                return Ok(GitOutput { stdout: String::new(), stderr: String::new() });
            }

            Ok(GitOutput { stdout: String::new(), stderr: String::new() })
        }
    }

    #[derive(Debug, Default)]
    struct FakeBd {
        calls: RefCell<Vec<Vec<String>>>,
        next_id: RefCell<u32>,
    }

    impl FakeBd {
        fn new() -> Self {
            Self::default()
        }
        fn calls(&self) -> Vec<Vec<String>> {
            self.calls.borrow().clone()
        }
    }

    impl BdClient for FakeBd {
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
        fn remember(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput> {
            let captured: Vec<String> =
                args.iter().map(|a| a.to_string_lossy().to_string()).collect();
            self.calls.borrow_mut().push(captured.clone());
            // `bd q "<title>" -t bug -p 2 -l merge-conflict` → emits id
            if captured.first().map(String::as_str) == Some("q") {
                let mut next = self.next_id.borrow_mut();
                *next += 1;
                let id = format!("hew-bug-{}", *next);
                return Ok(BdOutput { stdout: format!("{id}\n"), stderr: String::new() });
            }
            Ok(BdOutput { stdout: String::new(), stderr: String::new() })
        }
    }

    #[test]
    fn merge_back_clean_branches_succeeds() {
        let git = FakeGit::new();
        let project = PathBuf::from("/tmp/proj");
        let branches = vec!["loop/r1/w0".to_string(), "loop/r1/w1".to_string()];

        let report = merge_back(&git, &project, "main", &branches).unwrap();

        assert_eq!(report.merged, branches);
        assert!(report.conflicts.is_empty());

        // Confirm merge argv shape (--no-ff --no-edit per craft note).
        let calls = git.calls();
        let merge_calls: Vec<_> =
            calls.iter().filter(|c| c.iter().any(|a| a == "--no-ff")).collect();
        assert_eq!(merge_calls.len(), 2, "one merge per branch");
        for c in &merge_calls {
            assert!(c.iter().any(|a| a == "--no-ff"));
            assert!(c.iter().any(|a| a == "--no-edit"));
            assert!(c.contains(&"-C".to_string()));
        }
        // No abort calls on the clean path.
        assert!(!calls.iter().any(|c| c.iter().any(|a| a == "--abort")));
    }

    #[test]
    fn merge_back_conflicting_branches_files_bug_task() {
        let git = FakeGit::new()
            .with_failing_merge("loop/r1/w0")
            .with_unmerged_files(&["src/foo.rs", "src/bar.rs"]);
        let project = PathBuf::from("/tmp/proj");
        let branches = vec!["loop/r1/w0".to_string()];

        let report = merge_back(&git, &project, "main", &branches).unwrap();

        assert!(report.merged.is_empty());
        assert_eq!(report.conflicts.len(), 1);
        let c = &report.conflicts[0];
        assert_eq!(c.branch, "loop/r1/w0");
        assert_eq!(c.files, vec![PathBuf::from("src/foo.rs"), PathBuf::from("src/bar.rs")]);
        assert!(c.hint.contains("loop/r1/w0"));

        // Confirm git merge --abort fired after the failed merge.
        assert!(git.calls().iter().any(|c| c.iter().any(|a| a == "--abort")));

        // Now drive the bug-task filer and confirm the title + bd argv.
        let bd = FakeBd::new();
        let ids = file_conflict_bug_tasks(&bd, "r1", &report.conflicts).unwrap();
        assert_eq!(ids.len(), 1);

        let bd_calls = bd.calls();
        let q_call = bd_calls.iter().find(|c| c.first().map(String::as_str) == Some("q")).unwrap();
        assert_eq!(q_call[1], "[merge-conflict] hew loop run r1 worker-0");
        let joined = q_call.join(" ");
        assert!(joined.contains("-t bug"), "{joined}");
        assert!(joined.contains("-p 2"), "{joined}");
        assert!(joined.contains("-l merge-conflict"), "{joined}");

        // And the description update lists the files in the body.
        let update_call = bd_calls
            .iter()
            .find(|c| c.first().map(String::as_str) == Some("update"))
            .expect("expected a `bd update` for the description");
        let body = update_call.join(" ");
        assert!(body.contains("src/foo.rs"), "{body}");
        assert!(body.contains("src/bar.rs"), "{body}");
    }

    #[test]
    fn merge_back_continues_after_conflict_to_remaining_branches() {
        // Three workers; the middle one conflicts. Expect the first
        // and third to land in `merged` and only the middle in
        // `conflicts`.
        let git =
            FakeGit::new().with_failing_merge("loop/r1/w1").with_unmerged_files(&["touched.rs"]);
        let project = PathBuf::from("/tmp/proj");
        let branches =
            vec!["loop/r1/w0".to_string(), "loop/r1/w1".to_string(), "loop/r1/w2".to_string()];

        let report = merge_back(&git, &project, "main", &branches).unwrap();

        assert_eq!(report.merged, vec!["loop/r1/w0", "loop/r1/w2"]);
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].branch, "loop/r1/w1");
    }

    #[test]
    fn parse_worker_n_handles_standard_and_fallback() {
        assert_eq!(parse_worker_n("loop/r1/w0"), Some(0));
        assert_eq!(parse_worker_n("loop/some-run/w42"), Some(42));
        assert_eq!(parse_worker_n("feat/foo"), None);
        assert_eq!(parse_worker_n("loop/r1/wzz"), None);
    }

    #[test]
    fn file_conflict_bug_tasks_falls_back_to_branch_name_for_nonstandard_branch() {
        let bd = FakeBd::new();
        let conflicts = vec![ConflictReport {
            branch: "feat/manual".into(),
            files: vec![PathBuf::from("x.rs")],
            hint: "hint".into(),
        }];
        let ids = file_conflict_bug_tasks(&bd, "r1", &conflicts).unwrap();
        assert_eq!(ids.len(), 1);
        let q =
            bd.calls().into_iter().find(|c| c.first().map(String::as_str) == Some("q")).unwrap();
        assert_eq!(q[1], "[merge-conflict] hew loop run r1 branch feat/manual");
    }
}
