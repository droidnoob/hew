//! Test-only helpers shared by the lib's `#[cfg(test)]` modules **and**
//! the binary's integration tests under `hew/tests/`. Lives in
//! non-test code because Rust integration tests can't reach
//! `#[cfg(test)]` items from a depended-on crate.
//!
//! Every helper here exists to make the test harness less flaky. None
//! of them are intended for production callers.

use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Install an executable stub script at `<dir>/<name>` in an
/// ETXTBSY-safe way.
///
/// On Linux, opening a file for write while another process is executing
/// it returns `ETXTBSY` ("Text file busy"). The reverse race also bites:
/// some parallel-test schedulers tickle a kernel state where exec'ing a
/// file that was *just* closed for write fails the same way. We hit
/// this regularly on `ubuntu-latest / stable` CI on stubs written via
/// the naive `fs::write + chmod + spawn` sequence — see
/// `GOTCHA:linux-etxtbsy-stub` and hew-6vc / PR #22 CI flake.
///
/// The fix is the standard atomic-install dance:
///
/// 1. Write the body to a sibling temp path that the running test owns
///    by PID — the inode is never opened-for-write under the final
///    name, so the exec syscall can't race the open-for-write.
/// 2. chmod the temp path 0o755.
/// 3. `fs::rename` it to the destination — rename is atomic on POSIX,
///    so no one ever observes the file partially written or with the
///    wrong mode.
///
/// Callers can immediately spawn `<dir>/<name>` afterward.
pub fn install_executable_stub(dir: &Path, name: &str, body: &str) -> io::Result<()> {
    // PID + monotonic nanos buys us a unique temp path even when
    // multiple test threads write the same stub in parallel.
    let tmp_name = format!(
        ".{name}.tmp.{}.{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let tmp = dir.join(&tmp_name);
    let dest = dir.join(name);

    // Use a fresh write — explicitly creating then closing the file
    // before chmod + rename. fs::write does this in one shot.
    fs::write(&tmp, body)?;

    let mut perms = fs::metadata(&tmp)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&tmp, perms)?;

    // Atomic on POSIX. If `dest` already exists it's replaced; old
    // inode hangs around for any process still exec'ing it.
    fs::rename(&tmp, &dest)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn installed_stub_is_executable_and_writes_expected_body() {
        let tmp = tempfile::tempdir().unwrap();
        install_executable_stub(tmp.path(), "echo-hi", "#!/bin/sh\necho hi\n").unwrap();

        let out = Command::new(tmp.path().join("echo-hi")).output().unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn install_overwrites_an_existing_stub() {
        let tmp = tempfile::tempdir().unwrap();
        install_executable_stub(tmp.path(), "v", "#!/bin/sh\necho first\n").unwrap();
        install_executable_stub(tmp.path(), "v", "#!/bin/sh\necho second\n").unwrap();
        let out = Command::new(tmp.path().join("v")).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "second");
    }

    #[test]
    fn install_leaves_no_temp_files_behind() {
        let tmp = tempfile::tempdir().unwrap();
        install_executable_stub(tmp.path(), "stub", "#!/bin/sh\n").unwrap();
        let mut names: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, vec!["stub".to_string()], "leftover .tmp file in {names:?}");
    }
}
