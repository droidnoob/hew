//! Test-only helpers shared by the lib's `#[cfg(test)]` modules **and**
//! the binary's integration tests under `hew/tests/`. Lives in
//! non-test code because Rust integration tests can't reach
//! `#[cfg(test)]` items from a depended-on crate.
//!
//! Every helper here exists to make the test harness less flaky. None
//! of them are intended for production callers.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::time::Duration;

/// Retry `f` on `ETXTBSY` ("Text file busy") — the canonical call-site
/// wrapper for test code that exec's a freshly-installed stub.
///
/// Production code goes through [`crate::process::spawn_with_etxtbsy_retry`]
/// at the `Command::spawn` boundary. Tests that exec stubs via
/// `Command::new(...).output()` / `.status()` skip that path, so they
/// need their own retry wrapper — even after
/// [`install_executable_stub`]'s atomic-rename + dir fsync, the Linux
/// kernel can briefly report `ExecutableFileBusy` at the exec syscall
/// when a sibling test thread's writer fd to a similar inode has just
/// closed.
///
/// Five attempts with exponential backoff (5, 10, 20, 40, 80ms ≈ 155ms
/// worst-case wall clock). Mirrors the production retry shape in
/// [`crate::process::spawn_with_etxtbsy_retry`].
///
/// Errors other than `ErrorKind::ExecutableFileBusy` propagate
/// immediately — this helper exists to absorb the kernel race, not to
/// paper over real failures.
///
/// ```ignore
/// use std::process::Command;
/// use hew_core::testing::{install_executable_stub, retry_etxtbsy};
/// install_executable_stub(dir, "bd", "#!/bin/sh\necho hi\n").unwrap();
/// let out = retry_etxtbsy(|| Command::new(dir.join("bd")).output()).unwrap();
/// ```
pub fn retry_etxtbsy<T, F>(mut f: F) -> io::Result<T>
where
    F: FnMut() -> io::Result<T>,
{
    let mut delay_ms = 5u64;
    for _ in 0..5 {
        match f() {
            Ok(v) => return Ok(v),
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(delay_ms));
                delay_ms *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    f()
}

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
/// Callers can immediately spawn `<dir>/<name>` afterward — but if
/// they exec via `Command::new(...).output()` (not via
/// [`crate::process::spawn_with_etxtbsy_retry`]), they should wrap the
/// call in [`retry_etxtbsy`] so a kernel-side `ExecutableFileBusy`
/// race doesn't turn into a flaky panic.
pub fn install_executable_stub(dir: &Path, name: &str, body: &str) -> io::Result<()> {
    // PID + a per-call counter + monotonic nanos buys a unique temp
    // path even when multiple test threads write the same stub in
    // parallel. The counter is the load-bearing piece — two threads
    // can sample identical nanos on the same machine.
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let tmp_name = format!(
        ".{name}.tmp.{}.{}.{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0),
    );
    let tmp = dir.join(&tmp_name);
    let dest = dir.join(name);

    // Create with the final mode set in one syscall. Doing chmod
    // *after* the write would dirty the inode just before we exec
    // it, which on Linux can trip the writer-vs-exec race even
    // though our own fd is already closed (see ETXTBSY notes below).
    // `std::fs::File` already sets `O_CLOEXEC` by default, so we
    // don't need a `custom_flags` call here.
    {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o755).open(&tmp)?;
        f.write_all(body.as_bytes())?;
        // Flush data + metadata to disk before close so the inode's
        // i_writecount drop is fully durable when exec consults it.
        f.sync_all()?;
    } // explicit close before rename — drop releases the only write fd.

    // Atomic on POSIX. If `dest` already exists it's replaced; old
    // inode hangs around for any process still exec'ing it.
    fs::rename(&tmp, &dest)?;

    // fsync the parent dir so the rename is durable + visible to a
    // subsequent exec. Without this, Linux can keep the directory
    // entry change buffered long enough that an exec() finds an inode
    // the kernel still considers "writer-busy". Best-effort: failure
    // here doesn't change correctness for tmpfs/in-memory paths.
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::process::Command;

    #[test]
    fn installed_stub_is_executable_and_writes_expected_body() {
        let tmp = tempfile::tempdir().unwrap();
        install_executable_stub(tmp.path(), "echo-hi", "#!/bin/sh\necho hi\n").unwrap();

        let stub = tmp.path().join("echo-hi");
        let out = retry_etxtbsy(|| Command::new(&stub).output()).unwrap();
        assert!(out.status.success());
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[test]
    fn install_overwrites_an_existing_stub() {
        let tmp = tempfile::tempdir().unwrap();
        install_executable_stub(tmp.path(), "v", "#!/bin/sh\necho first\n").unwrap();
        install_executable_stub(tmp.path(), "v", "#!/bin/sh\necho second\n").unwrap();
        let stub = tmp.path().join("v");
        let out = retry_etxtbsy(|| Command::new(&stub).output()).unwrap();
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

    fn etxtbsy_err() -> io::Error {
        io::Error::from(io::ErrorKind::ExecutableFileBusy)
    }

    #[test]
    fn retry_etxtbsy_succeeds_on_first_call_when_no_busy() {
        let calls = Cell::new(0u32);
        let got = retry_etxtbsy(|| {
            calls.set(calls.get() + 1);
            Ok::<_, io::Error>(42)
        })
        .unwrap();
        assert_eq!(got, 42);
        assert_eq!(calls.get(), 1, "happy path must not retry");
    }

    #[test]
    fn retry_etxtbsy_eventually_succeeds_when_busy_clears() {
        let calls = Cell::new(0u32);
        let got = retry_etxtbsy(|| {
            let n = calls.get();
            calls.set(n + 1);
            if n < 2 { Err(etxtbsy_err()) } else { Ok(7) }
        })
        .unwrap();
        assert_eq!(got, 7);
        assert_eq!(calls.get(), 3, "should retry past two ETXTBSY hits");
    }

    #[test]
    fn retry_etxtbsy_propagates_other_io_errors() {
        let calls = Cell::new(0u32);
        let err = retry_etxtbsy(|| {
            calls.set(calls.get() + 1);
            Err::<(), _>(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(calls.get(), 1, "non-ETXTBSY errors must not retry");
    }

    #[test]
    fn retry_etxtbsy_gives_up_after_attempts_with_last_etxtbsy() {
        let calls = Cell::new(0u32);
        let err = retry_etxtbsy(|| {
            calls.set(calls.get() + 1);
            Err::<(), _>(etxtbsy_err())
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ExecutableFileBusy);
        // 5 retries inside the loop + 1 final attempt = 6 total calls.
        assert_eq!(calls.get(), 6, "must not loop forever");
    }
}
