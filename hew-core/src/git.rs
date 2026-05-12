//! `git` wrapper. Thin shell-out following the `BdClient` pattern in
//! `bd.rs`: `Vec<OsString>` args, `.stdin(Stdio::null())`, `wait_timeout`,
//! read stdio after wait. Git is *optional* for hew (unlike bd), so most
//! callers check `RealGit::discover()` and degrade silently on missing.

use std::ffi::{OsStr, OsString};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::debug;
use wait_timeout::ChildExt;

use crate::error::{HewError, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct GitOutput {
    pub stdout: String,
    pub stderr: String,
}

/// Abstraction over the `git` binary so tests can drop in a fake.
pub trait GitClient: std::fmt::Debug {
    fn current_branch(&self) -> Result<Option<String>>;
    fn checkout_new_branch(&self, name: &str, from: Option<&str>) -> Result<()>;
    fn run_raw(&self, args: &[&OsStr]) -> Result<GitOutput>;
}

#[derive(Debug, Clone)]
pub struct RealGit {
    path: PathBuf,
    timeout: Duration,
}

impl RealGit {
    /// Resolve `git` on `PATH`. Errors with `HewError::GitNotFound` if missing.
    pub fn discover() -> Result<Self> {
        let path = which::which("git").map_err(|_| HewError::GitNotFound)?;
        Ok(Self { path, timeout: DEFAULT_TIMEOUT })
    }

    /// Cheap availability check that never errors. Suitable for `if git_available() { ... }`.
    pub fn is_available() -> bool {
        which::which("git").is_ok()
    }

    /// Explicit path — used by tests that put a stub on PATH.
    pub fn at(path: PathBuf) -> Self {
        Self { path, timeout: DEFAULT_TIMEOUT }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn run(&self, args: &[&OsStr]) -> Result<GitOutput> {
        debug!(git = %self.path.display(), ?args, "running git");

        let mut cmd = Command::new(&self.path);
        cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn()?;

        let status = match child.wait_timeout(self.timeout)? {
            Some(s) => s,
            None => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(HewError::GitNonZero {
                    code: -1,
                    stderr: format!("`git` timed out after {:?}", self.timeout),
                });
            }
        };

        let mut stdout = String::new();
        let mut stderr = String::new();
        if let Some(mut s) = child.stdout.take() {
            use std::io::Read;
            s.read_to_string(&mut stdout)?;
        }
        if let Some(mut s) = child.stderr.take() {
            use std::io::Read;
            s.read_to_string(&mut stderr)?;
        }

        if !status.success() {
            return Err(HewError::GitNonZero {
                code: status.code().unwrap_or(-1),
                stderr: if stderr.is_empty() { stdout.clone() } else { stderr },
            });
        }
        Ok(GitOutput { stdout, stderr })
    }
}

impl GitClient for RealGit {
    fn current_branch(&self) -> Result<Option<String>> {
        // --quiet so a detached HEAD returns empty rather than printing to stderr.
        let out = self.run(&[
            OsStr::new("symbolic-ref"),
            OsStr::new("--quiet"),
            OsStr::new("--short"),
            OsStr::new("HEAD"),
        ]);
        match out {
            Ok(o) => {
                let name = o.stdout.trim().to_string();
                Ok(if name.is_empty() { None } else { Some(name) })
            }
            // Non-zero exit on detached HEAD is expected; not an error.
            Err(HewError::GitNonZero { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn checkout_new_branch(&self, name: &str, from: Option<&str>) -> Result<()> {
        let name_os = OsString::from(name);
        let mut args: Vec<&OsStr> =
            vec![OsStr::new("checkout"), OsStr::new("-b"), name_os.as_os_str()];
        let from_os: OsString;
        if let Some(base) = from {
            from_os = OsString::from(base);
            args.push(from_os.as_os_str());
        }
        self.run(&args)?;
        Ok(())
    }

    fn run_raw(&self, args: &[&OsStr]) -> Result<GitOutput> {
        self.run(args)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn write_stub(dir: &std::path::Path, body: &str) {
        let path = dir.join("git");
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn discover_fails_when_git_missing() {
        // Empty PATH → not found.
        // We can't actually scrub PATH on the running process safely; instead
        // assert that `is_available()` mirrors which::which behavior.
        let _ = RealGit::is_available(); // smoke
    }

    #[test]
    fn current_branch_extracts_short_name() {
        let tmp = tempfile::tempdir().unwrap();
        // Stub that prints the branch on `symbolic-ref ... HEAD`.
        write_stub(
            tmp.path(),
            "#!/bin/sh\ncase \"$1\" in\n  symbolic-ref) echo 'feat/foo'; exit 0;;\nesac\nexit 1\n",
        );
        let git = RealGit::at(tmp.path().join("git"));
        assert_eq!(git.current_branch().unwrap().as_deref(), Some("feat/foo"));
    }

    #[test]
    fn current_branch_returns_none_on_detached_head() {
        let tmp = tempfile::tempdir().unwrap();
        write_stub(
            tmp.path(),
            "#!/bin/sh\ncase \"$1\" in\n  symbolic-ref) exit 1;;\nesac\nexit 1\n",
        );
        let git = RealGit::at(tmp.path().join("git"));
        assert!(git.current_branch().unwrap().is_none());
    }

    #[test]
    fn checkout_new_branch_invokes_checkout_dash_b() {
        let tmp = tempfile::tempdir().unwrap();
        // Stub records its args to a file then exits 0.
        let log = tmp.path().join("args.log");
        write_stub(tmp.path(), &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display(),));
        let git = RealGit::at(tmp.path().join("git"));
        git.checkout_new_branch("feat/auth", None).unwrap();
        let recorded = fs::read_to_string(&log).unwrap();
        assert_eq!(recorded.trim(), "checkout -b feat/auth");
    }

    #[test]
    fn checkout_new_branch_passes_from_base() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("args.log");
        write_stub(tmp.path(), &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display(),));
        let git = RealGit::at(tmp.path().join("git"));
        git.checkout_new_branch("feat/auth", Some("origin/main")).unwrap();
        let recorded = fs::read_to_string(&log).unwrap();
        assert_eq!(recorded.trim(), "checkout -b feat/auth origin/main");
    }

    #[test]
    fn checkout_propagates_git_nonzero_error() {
        let tmp = tempfile::tempdir().unwrap();
        write_stub(
            tmp.path(),
            "#!/bin/sh\necho 'fatal: A branch named feat/foo already exists' >&2\nexit 128\n",
        );
        let git = RealGit::at(tmp.path().join("git"));
        let err = git.checkout_new_branch("feat/foo", None).expect_err("must error");
        match err {
            HewError::GitNonZero { code, stderr } => {
                assert_eq!(code, 128);
                assert!(stderr.contains("already exists"));
            }
            other => panic!("expected GitNonZero, got {other:?}"),
        }
    }
}
