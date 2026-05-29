//! Per-worker git worktrees for the parallel hew loop.
//!
//! Layout (out-of-tree per `DECISION:loop-worktree-location`):
//!
//! ```text
//! ~/.hew/wt/
//!   <run-id>/
//!     <n>/            ← worker N's checkout, branch `loop/<run-id>/w<n>`
//! ```
//!
//! `~/.hew/wt/` is resolved via [`etcetera`] (same strategy used in
//! `config::config_path` for `~/.config/`). The dispatcher owns the
//! lifecycle: it picks `run_id`, calls [`create`] per slot fill, and
//! calls [`prune`] on worker completion. [`list_orphans`] enumerates
//! anything still on disk that no live run claims.
//!
//! Everything that talks to `git` goes through the [`GitClient`] trait
//! so tests inject a recording fake instead of spawning a real binary —
//! the per-task spec calls out "no actual git command run in unit
//! tests".

use std::collections::HashSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::error::{HewError, Result};
use crate::git::GitClient;

/// A single worker's checkout under `~/.hew/wt/<run-id>/<n>/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeHandle {
    pub run_id: String,
    pub worker_n: u32,
    pub branch: String,
    pub path: PathBuf,
}

/// Canonical root for hew-owned worktrees: `~/.hew/wt/`.
///
/// Resolved via `etcetera::choose_base_strategy().home_dir().join(".hew/wt")`
/// so it follows the same home discovery as the rest of hew. All other
/// functions in this module take an explicit `root` to keep tests free
/// of process-wide env mutation.
pub fn root() -> Result<PathBuf> {
    use etcetera::BaseStrategy;
    let strategy = etcetera::choose_base_strategy()
        .map_err(|e| HewError::Io(std::io::Error::other(e.to_string())))?;
    Ok(strategy.home_dir().join(".hew").join("wt"))
}

/// Recommended branch name for a worker's worktree: `loop/<run-id>/w<n>`.
///
/// The dispatcher always uses this pattern; exposing it as a helper
/// keeps the convention in one place and makes the
/// `create_uses_branch_name_pattern_loop_run_id_w_n` test trivial.
pub fn branch_name(run_id: &str, worker_n: u32) -> String {
    format!("loop/{run_id}/w{worker_n}")
}

/// `<root>/<run-id>/<n>` — the on-disk path for one worker.
pub fn worker_path(root: &Path, run_id: &str, worker_n: u32) -> PathBuf {
    root.join(run_id).join(worker_n.to_string())
}

/// Lay down a fresh worktree for worker `worker_n` of `run_id`.
///
/// Shells `git -C <project_root> worktree add -b <branch> <wt_path> <base_sha>`.
/// Creates any missing parent dirs first so the branch / run-id namespace
/// materialises lazily.
pub fn create(
    git: &dyn GitClient,
    project_root: &Path,
    root: &Path,
    run_id: &str,
    worker_n: u32,
    base_sha: &str,
    branch: &str,
) -> Result<WorktreeHandle> {
    let path = worker_path(root, run_id, worker_n);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let project_root_s = project_root.as_os_str();
    let path_s = path.as_os_str();
    let branch_os = OsStr::new(branch);
    let base_os = OsStr::new(base_sha);
    git.run_raw(&[
        OsStr::new("-C"),
        project_root_s,
        OsStr::new("worktree"),
        OsStr::new("add"),
        OsStr::new("-b"),
        branch_os,
        path_s,
        base_os,
    ])?;
    Ok(WorktreeHandle { run_id: run_id.to_string(), worker_n, branch: branch.to_string(), path })
}

/// Remove worker `worker_n` of `run_id` — both the git record and the
/// directory.
///
/// Tolerant of partial state: the git `worktree remove` is best-effort
/// (the dir may already be gone, or git may have lost track of it), and
/// a stale directory is force-removed afterward. Finishes with
/// `git worktree prune` so the project's `.git/worktrees/` admin dir
/// reflects reality.
pub fn prune(
    git: &dyn GitClient,
    project_root: &Path,
    root: &Path,
    run_id: &str,
    worker_n: u32,
) -> Result<()> {
    let path = worker_path(root, run_id, worker_n);
    let project_root_s = project_root.as_os_str();
    let path_s = path.as_os_str();

    // Best-effort: git may have already lost track of the worktree
    // (manual delete, prior crash). We still want the dir wiped below.
    let _ = git.run_raw(&[
        OsStr::new("-C"),
        project_root_s,
        OsStr::new("worktree"),
        OsStr::new("remove"),
        OsStr::new("--force"),
        path_s,
    ]);

    if path.exists() {
        std::fs::remove_dir_all(&path)?;
    }
    // Drop the parent run-id dir iff empty — best-effort, errors are fine
    // when other workers still live under the same run.
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir(parent);
    }

    // Sync git's internal worktree admin records to the on-disk truth.
    let _ = git.run_raw(&[
        OsStr::new("-C"),
        project_root_s,
        OsStr::new("worktree"),
        OsStr::new("prune"),
    ]);
    Ok(())
}

/// Enumerate every `<root>/<run-id>/<n>/` currently on disk.
///
/// Returns an empty Vec — not an error — when `root` is absent: a fresh
/// machine has no worktrees yet and that's expected, not a fault.
pub fn list_all(root: &Path) -> Result<Vec<WorktreeHandle>> {
    let mut out = Vec::new();
    let run_iter = match std::fs::read_dir(root) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(HewError::Io(e)),
    };
    for run_entry in run_iter.flatten() {
        if !run_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let run_id = match run_entry.file_name().into_string() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let worker_iter = match std::fs::read_dir(run_entry.path()) {
            Ok(it) => it,
            Err(_) => continue,
        };
        for w_entry in worker_iter.flatten() {
            if !w_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            let worker_n = match w_entry.file_name().to_str().and_then(|s| s.parse::<u32>().ok()) {
                Some(n) => n,
                None => continue,
            };
            out.push(WorktreeHandle {
                run_id: run_id.clone(),
                worker_n,
                branch: branch_name(&run_id, worker_n),
                path: w_entry.path(),
            });
        }
    }
    Ok(out)
}

/// Worktrees whose `run_id` is **not** in `active`.
///
/// The dispatcher passes the set of currently-live run IDs (from
/// `.hew/loop/<run-id>/` markers in the project tree); anything else
/// under `~/.hew/wt/` is garbage from a prior crashed run and safe to
/// `prune`.
pub fn list_orphans(root: &Path, active: &HashSet<String>) -> Result<Vec<WorktreeHandle>> {
    Ok(list_all(root)?.into_iter().filter(|h| !active.contains(&h.run_id)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Result as HewResult;
    use crate::git::{GitClient, GitOutput};
    use std::cell::RefCell;
    use std::ffi::OsString;

    /// Records every call so assertions can inspect the exact argv. The
    /// real `git worktree add` would create the dir as a side effect;
    /// the fake does that explicitly to mirror the post-condition tests
    /// rely on.
    #[derive(Debug, Default)]
    struct RecordingGit {
        calls: RefCell<Vec<Vec<OsString>>>,
    }

    impl GitClient for RecordingGit {
        fn current_branch(&self) -> HewResult<Option<String>> {
            Ok(None)
        }
        fn checkout_new_branch(&self, _: &str, _: Option<&str>) -> HewResult<()> {
            Ok(())
        }
        fn run_raw(&self, args: &[&OsStr]) -> HewResult<GitOutput> {
            let owned: Vec<OsString> = args.iter().map(|a| a.to_os_string()).collect();
            // Mirror `git worktree add <path>`'s on-disk side effect so
            // the post-call path-exists assertion holds without a real git.
            if owned.iter().any(|a| a == "add")
                && let Some(pos) = owned.iter().position(|a| a == "-b")
                && let Some(wt_path) = owned.get(pos + 2)
            {
                std::fs::create_dir_all(Path::new(wt_path)).expect("mkdir worktree path");
            }
            self.calls.borrow_mut().push(owned);
            Ok(GitOutput { stdout: String::new(), stderr: String::new() })
        }
    }

    fn os(s: &str) -> OsString {
        OsString::from(s)
    }

    #[test]
    fn branch_name_uses_loop_run_id_w_n_pattern() {
        assert_eq!(branch_name("loop-2026-05-29-abc", 0), "loop/loop-2026-05-29-abc/w0");
        assert_eq!(branch_name("r1", 7), "loop/r1/w7");
    }

    #[test]
    fn worker_path_nests_run_id_then_worker_n() {
        let root = Path::new("/tmp/wt");
        assert_eq!(worker_path(root, "r1", 0), PathBuf::from("/tmp/wt/r1/0"));
        assert_eq!(worker_path(root, "r1", 12), PathBuf::from("/tmp/wt/r1/12"));
    }

    #[test]
    fn create_lays_down_directory_and_returns_handle() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        let git = RecordingGit::default();
        let branch = branch_name("r1", 0);
        let handle = create(&git, &project, &root, "r1", 0, "deadbeef", &branch).unwrap();

        assert_eq!(handle.run_id, "r1");
        assert_eq!(handle.worker_n, 0);
        assert_eq!(handle.branch, "loop/r1/w0");
        assert_eq!(handle.path, root.join("r1").join("0"));
        assert!(handle.path.is_dir(), "worktree dir should exist after create");
    }

    #[test]
    fn create_invokes_git_worktree_add_with_expected_argv() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();

        let git = RecordingGit::default();
        create(&git, &project, &root, "r1", 2, "abc123", "loop/r1/w2").unwrap();

        let calls = git.calls.borrow();
        assert_eq!(calls.len(), 1, "exactly one git call (worktree add)");
        let argv = &calls[0];
        let expected: Vec<OsString> = vec![
            os("-C"),
            project.clone().into_os_string(),
            os("worktree"),
            os("add"),
            os("-b"),
            os("loop/r1/w2"),
            root.join("r1").join("2").into_os_string(),
            os("abc123"),
        ];
        assert_eq!(argv, &expected);
    }

    #[test]
    fn create_uses_branch_name_pattern_loop_run_id_w_n() {
        // Caller-side convention: pass `branch_name(run_id, n)` to create.
        // The handle preserves the branch verbatim. This protects the
        // contract the dispatcher relies on.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let git = RecordingGit::default();

        for n in [0u32, 3, 11] {
            let b = branch_name("R", n);
            let h = create(&git, &project, &root, "R", n, "HEAD", &b).unwrap();
            assert_eq!(h.branch, format!("loop/R/w{n}"));
        }
    }

    #[test]
    fn prune_removes_directory_and_git_record() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let git = RecordingGit::default();

        // Create then prune.
        create(&git, &project, &root, "r1", 0, "HEAD", &branch_name("r1", 0)).unwrap();
        let wt_path = worker_path(&root, "r1", 0);
        assert!(wt_path.exists());

        prune(&git, &project, &root, "r1", 0).unwrap();

        assert!(!wt_path.exists(), "worker dir gone");
        // Run-id dir is empty → also gone.
        assert!(!root.join("r1").exists(), "run-id dir gone when empty");

        // Two follow-up calls were issued: `worktree remove --force` then
        // `worktree prune`.
        let calls = git.calls.borrow();
        assert!(calls.iter().any(|c| c.iter().any(|a| a == "remove")));
        assert!(calls.iter().any(|c| {
            // The bare `worktree prune` invocation (not `remove`).
            c.iter().any(|a| a == "prune") && !c.iter().any(|a| a == "remove")
        }));
    }

    #[test]
    fn prune_tolerates_missing_worktree_directory() {
        // Crash recovery: dir already gone, git record also gone.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let git = RecordingGit::default();

        // Should not error even though nothing exists.
        prune(&git, &project, &root, "ghost", 9).unwrap();
    }

    #[test]
    fn list_orphans_returns_nothing_when_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt"); // never created
        let active: HashSet<String> = HashSet::new();
        assert!(list_orphans(&root, &active).unwrap().is_empty());

        // And when the root exists but is empty:
        std::fs::create_dir_all(&root).unwrap();
        assert!(list_orphans(&root, &active).unwrap().is_empty());
    }

    #[test]
    fn list_orphans_filters_by_active_run_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        // Three workers across two run IDs.
        for (run, n) in [("r1", 0u32), ("r1", 1), ("r2", 0)] {
            std::fs::create_dir_all(worker_path(&root, run, n)).unwrap();
        }

        let mut active = HashSet::new();
        active.insert("r1".to_string());
        let orphans = list_orphans(&root, &active).unwrap();

        assert_eq!(orphans.len(), 1, "only r2/0 is orphaned");
        assert_eq!(orphans[0].run_id, "r2");
        assert_eq!(orphans[0].worker_n, 0);
        assert_eq!(orphans[0].branch, "loop/r2/w0");
    }

    #[test]
    fn list_all_skips_non_numeric_worker_dirs() {
        // Defensive: a stray file or bad dir under a run-id namespace
        // shouldn't panic the lister.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("wt");
        std::fs::create_dir_all(root.join("r1").join("0")).unwrap();
        std::fs::create_dir_all(root.join("r1").join("not-a-number")).unwrap();
        std::fs::write(root.join("r1").join("loose-file"), b"x").unwrap();

        let all = list_all(&root).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].worker_n, 0);
    }

    #[test]
    fn root_resolves_under_etcetera_home() {
        // Smoke: just confirm the path ends with `.hew/wt`. Don't assert
        // an absolute prefix because etcetera's home varies per platform
        // and CI sandbox.
        let r = root().unwrap();
        assert!(r.ends_with(".hew/wt"), "expected …/.hew/wt, got {}", r.display());
    }
}
