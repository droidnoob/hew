//! TTY + non-interactive detection.
//!
//! Precedence (highest wins):
//!   1. explicit `--non-interactive` flag (caller passes `force_non_interactive=true`)
//!   2. `HEW_NON_INTERACTIVE=1`
//!   3. `CI=true`
//!   4. stderr is not a TTY  (NOT stdout — stdout is commonly piped to jq)

use std::io::IsTerminal;

pub fn is_non_interactive(force: bool) -> bool {
    if force {
        return true;
    }
    if env_flag("HEW_NON_INTERACTIVE") {
        return true;
    }
    if env_flag("CI") {
        return true;
    }
    !std::io::stderr().is_terminal()
}

fn env_flag(key: &str) -> bool {
    match std::env::var(key) {
        Ok(v) => matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on"),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_overrides_everything() {
        assert!(is_non_interactive(true));
    }
}
