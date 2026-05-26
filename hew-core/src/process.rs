//! Subprocess spawn helpers shared across hew-core.
//!
//! The single helper here, [`spawn_with_etxtbsy_retry`], wraps
//! [`std::process::Command::spawn`] with a brief retry loop on
//! `ETXTBSY` (Linux's "Text file busy" — `ErrorKind::ExecutableFileBusy`).
//!
//! ## Why this exists
//!
//! When parallel threads each write+exec their own stub binary (the
//! test harness pattern used in `hew_core::testing::install_executable_stub`),
//! one thread's writable fd to its stub temp file leaks into another
//! thread's child via `fork()` even when the fd is `O_CLOEXEC` —
//! `O_CLOEXEC` only fires on `exec`, not `fork`. The kernel then sees
//! an outstanding writer on the inode and the child's `exec` trips
//! `ETXTBSY` (errno 26).
//!
//! Same race can happen in production any time `hew init` rewrites
//! bundled artifacts during concurrent reads. Exponential backoff up to
//! ~150ms total handles the transient window without callers needing
//! to care.
//!
//! Defense in depth: every spawn path in hew-core (git, bd, os
//! installer probes) should go through this helper. The cost on the
//! happy path is one extra `match`; the cost on the failing path is a
//! handful of millisecond-scale sleeps that resolve a race the caller
//! would otherwise surface as a confusing test flake.

use std::process::{Child, Command};
use std::time::Duration;

/// Spawn `cmd`, retrying briefly on `ETXTBSY` (Linux "Text file busy").
/// Returns the spawned `Child` on success or the underlying io error
/// on the final attempt.
pub(crate) fn spawn_with_etxtbsy_retry(cmd: &mut Command) -> std::io::Result<Child> {
    use std::io::ErrorKind;
    let mut delay_ms = 5u64;
    for _ in 0..5 {
        match cmd.spawn() {
            Ok(c) => return Ok(c),
            Err(e) if e.kind() == ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(delay_ms));
                delay_ms *= 2;
            }
            Err(e) => return Err(e),
        }
    }
    cmd.spawn()
}
