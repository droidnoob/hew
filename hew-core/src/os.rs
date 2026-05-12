//! Host OS / distro detection + git-install hints.
//!
//! Powers `hew init`'s "git is missing — install it?" path. We never invoke
//! sudo ourselves; on Linux we surface the right command for the user to run.
//! macOS is the only platform with a sudo-free auto-install path today
//! (Homebrew if present).

use std::ffi::OsStr;
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::debug;
use wait_timeout::ChildExt;

use crate::error::{HewError, Result};

const BREW_INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsKind {
    MacOs,
    Linux(Distro),
    Windows,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Distro {
    Debian,
    Ubuntu,
    Fedora,
    Rhel,
    Arch,
    Alpine,
    OpenSuse,
    Other,
}

/// Detect the host OS. On Linux, parses `/etc/os-release` ID field.
pub fn detect_os() -> OsKind {
    detect_os_with(|| std::fs::read_to_string("/etc/os-release").ok())
}

/// Test-friendly variant. `os_release` returns the contents of
/// `/etc/os-release` (only consulted on Linux hosts).
pub fn detect_os_with<F>(os_release: F) -> OsKind
where
    F: FnOnce() -> Option<String>,
{
    match std::env::consts::OS {
        "macos" => OsKind::MacOs,
        "windows" => OsKind::Windows,
        "linux" => OsKind::Linux(parse_distro(os_release().as_deref().unwrap_or(""))),
        _ => OsKind::Unknown,
    }
}

/// Parse the ID= field from /etc/os-release into a `Distro`.
pub fn parse_distro(os_release: &str) -> Distro {
    for line in os_release.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("ID=") {
            let id = rest.trim_matches('"').to_ascii_lowercase();
            return match id.as_str() {
                "debian" => Distro::Debian,
                "ubuntu" => Distro::Ubuntu,
                "fedora" => Distro::Fedora,
                "rhel" | "centos" | "rocky" | "almalinux" => Distro::Rhel,
                "arch" | "manjaro" | "endeavouros" => Distro::Arch,
                "alpine" => Distro::Alpine,
                "opensuse" | "opensuse-leap" | "opensuse-tumbleweed" | "sles" => Distro::OpenSuse,
                _ => Distro::Other,
            };
        }
    }
    Distro::Other
}

/// Return a human-runnable one-liner for installing git on this OS.
///
/// On macOS, prefers `brew install git` when brew is on PATH; otherwise
/// falls back to `xcode-select --install` (the system-provided path).
pub fn git_install_hint(os: &OsKind) -> String {
    match os {
        OsKind::MacOs => {
            if which::which("brew").is_ok() {
                "brew install git".to_string()
            } else {
                "xcode-select --install (opens GUI prompt)".to_string()
            }
        }
        OsKind::Linux(d) => match d {
            Distro::Debian | Distro::Ubuntu => "sudo apt install git".to_string(),
            Distro::Fedora => "sudo dnf install git".to_string(),
            Distro::Rhel => "sudo dnf install git  # or yum install git".to_string(),
            Distro::Arch => "sudo pacman -S git".to_string(),
            Distro::Alpine => "apk add git".to_string(),
            Distro::OpenSuse => "sudo zypper install git".to_string(),
            Distro::Other => "use your package manager to install git".to_string(),
        },
        OsKind::Windows => "winget install --id Git.Git -e".to_string(),
        OsKind::Unknown => "install git via your platform's package manager".to_string(),
    }
}

/// Attempt to install git via a sudo-free path. Returns:
/// - `Ok(true)` if git was installed.
/// - `Ok(false)` if no sudo-free path is available on this OS.
/// - `Err(_)` if a sudo-free path was attempted and failed.
pub fn try_install_git_sudo_free(os: &OsKind) -> Result<bool> {
    try_install_git_sudo_free_with(os, || which::which("brew").ok())
}

/// Test-friendly variant: caller supplies the brew lookup. Production code
/// uses [`try_install_git_sudo_free`].
pub fn try_install_git_sudo_free_with<F>(os: &OsKind, brew_lookup: F) -> Result<bool>
where
    F: FnOnce() -> Option<std::path::PathBuf>,
{
    match os {
        OsKind::MacOs => match brew_lookup() {
            Some(brew) => {
                run_brew_install_git(&brew)?;
                Ok(true)
            }
            None => Ok(false),
        },
        // All other platforms need sudo or user action; no sudo-free path.
        _ => Ok(false),
    }
}

fn run_brew_install_git(brew: &std::path::Path) -> Result<()> {
    debug!(?brew, "attempting `brew install git`");
    let mut cmd = Command::new(brew);
    cmd.args([OsStr::new("install"), OsStr::new("git")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    let status = match child.wait_timeout(BREW_INSTALL_TIMEOUT)? {
        Some(s) => s,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(HewError::GitNonZero {
                code: -1,
                stderr: format!("`brew install git` timed out after {BREW_INSTALL_TIMEOUT:?}"),
            });
        }
    };
    if !status.success() {
        use std::io::Read;
        let mut stderr = String::new();
        if let Some(mut s) = child.stderr.take() {
            s.read_to_string(&mut stderr)?;
        }
        return Err(HewError::GitNonZero {
            code: status.code().unwrap_or(-1),
            stderr: if stderr.is_empty() { "brew install git failed".to_string() } else { stderr },
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn detect_os_smoke() {
        // Host-dependent — just assert it doesn't panic and matches platform.
        let os = detect_os();
        match std::env::consts::OS {
            "macos" => assert_eq!(os, OsKind::MacOs),
            "linux" => assert!(matches!(os, OsKind::Linux(_))),
            "windows" => assert_eq!(os, OsKind::Windows),
            _ => assert_eq!(os, OsKind::Unknown),
        }
    }

    #[test]
    fn parse_distro_table() {
        let cases: &[(&str, Distro)] = &[
            ("ID=debian\n", Distro::Debian),
            ("ID=ubuntu\nVERSION=22.04\n", Distro::Ubuntu),
            ("ID=fedora\n", Distro::Fedora),
            ("ID=rhel\n", Distro::Rhel),
            ("ID=centos\n", Distro::Rhel),
            ("ID=rocky\n", Distro::Rhel),
            ("ID=arch\n", Distro::Arch),
            ("ID=manjaro\n", Distro::Arch),
            ("ID=alpine\n", Distro::Alpine),
            ("ID=opensuse-leap\n", Distro::OpenSuse),
            ("ID=sles\n", Distro::OpenSuse),
            ("ID=\"ubuntu\"\n", Distro::Ubuntu),
            ("ID=void\n", Distro::Other),
            ("# nothing\n", Distro::Other),
            ("", Distro::Other),
        ];
        for (input, expected) in cases {
            assert_eq!(parse_distro(input), *expected, "{input:?}");
        }
    }

    #[test]
    fn git_install_hint_table() {
        let pairs: &[(OsKind, &str)] = &[
            (OsKind::Linux(Distro::Debian), "sudo apt install git"),
            (OsKind::Linux(Distro::Ubuntu), "sudo apt install git"),
            (OsKind::Linux(Distro::Fedora), "sudo dnf install git"),
            (OsKind::Linux(Distro::Arch), "sudo pacman -S git"),
            (OsKind::Linux(Distro::Alpine), "apk add git"),
            (OsKind::Linux(Distro::OpenSuse), "sudo zypper install git"),
            (OsKind::Windows, "winget install --id Git.Git -e"),
        ];
        for (os, expected) in pairs {
            assert_eq!(git_install_hint(os), *expected, "{os:?}");
        }
        // RHEL contains both 'dnf' and the yum fallback comment.
        let rhel = git_install_hint(&OsKind::Linux(Distro::Rhel));
        assert!(rhel.contains("dnf install git"), "{rhel}");
    }

    fn write_stub(dir: &std::path::Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&path, perms).unwrap();
    }

    #[test]
    fn install_returns_false_when_no_path_available() {
        // Non-macOS hosts have no sudo-free path.
        assert!(!try_install_git_sudo_free_with(&OsKind::Unknown, || None).unwrap());
        assert!(!try_install_git_sudo_free_with(&OsKind::Windows, || None).unwrap());
        assert!(!try_install_git_sudo_free_with(&OsKind::Linux(Distro::Debian), || None).unwrap());
    }

    #[test]
    fn macos_without_brew_returns_false() {
        assert!(!try_install_git_sudo_free_with(&OsKind::MacOs, || None).unwrap());
    }

    #[test]
    fn macos_with_brew_invokes_install_git() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("args.log");
        write_stub(
            tmp.path(),
            "brew",
            &format!("#!/bin/sh\necho \"$@\" > {}\nexit 0\n", log.display()),
        );
        let brew_path = tmp.path().join("brew");
        let result =
            try_install_git_sudo_free_with(&OsKind::MacOs, || Some(brew_path.clone())).unwrap();
        assert!(result);
        let recorded = fs::read_to_string(&log).unwrap();
        assert_eq!(recorded.trim(), "install git");
    }

    #[test]
    fn macos_with_brew_propagates_failure() {
        let tmp = tempfile::tempdir().unwrap();
        write_stub(tmp.path(), "brew", "#!/bin/sh\necho 'no formula' >&2\nexit 1\n");
        let brew_path = tmp.path().join("brew");
        let err = try_install_git_sudo_free_with(&OsKind::MacOs, || Some(brew_path.clone()))
            .expect_err("must surface brew failure");
        match err {
            HewError::GitNonZero { code, stderr } => {
                assert_eq!(code, 1);
                assert!(stderr.contains("no formula"), "{stderr}");
            }
            other => panic!("expected GitNonZero, got {other:?}"),
        }
    }
}
