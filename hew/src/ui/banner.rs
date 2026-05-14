//! Compact ASCII banner printed at the top of `hew init`. Suppressed in
//! quiet mode and non-interactive runs so scripts never see it.

use hew_core::Ctx;

/// Render the banner to stdout if `should_show(ctx)` is true.
pub fn render(ctx: &Ctx) {
    if should_show(ctx) {
        println!("{}", banner_text());
    }
}

/// Visibility predicate. Banner is for humans only — suppressed in quiet
/// mode and non-interactive runs (CI, piped invocations).
pub fn should_show(ctx: &Ctx) -> bool {
    !ctx.quiet && ctx.interactive
}

/// Pure banner text. Block-letter wordmark + version + tagline. Trailing
/// blank line gives breathing room before whatever the caller prints next.
pub fn banner_text() -> String {
    let art = "\
██╗  ██╗███████╗██╗    ██╗
██║  ██║██╔════╝██║    ██║
███████║█████╗  ██║ █╗ ██║
██╔══██║██╔══╝  ██║███╗██║
██║  ██║███████╗╚███╔███╔╝
╚═╝  ╚═╝╚══════╝ ╚══╝╚══╝ ";
    format!(
        "\n{art}\n  v{version}  ━  Carve code, not chaos.\n",
        version = env!("CARGO_PKG_VERSION"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(quiet: bool, interactive: bool) -> Ctx {
        Ctx { interactive, output: hew_core::ctx::OutputMode::Text, quiet, verbose: 0 }
    }

    #[test]
    fn version_matches_cargo_toml() {
        let body = banner_text();
        let expected = format!("v{}", env!("CARGO_PKG_VERSION"));
        assert!(body.contains(&expected), "missing {expected} in:\n{body}");
    }

    #[test]
    fn banner_text_has_tagline() {
        assert!(banner_text().contains("Carve code, not chaos."));
    }

    #[test]
    fn shows_when_interactive_and_not_quiet() {
        assert!(should_show(&ctx_with(false, true)));
    }

    #[test]
    fn hidden_when_quiet() {
        assert!(!should_show(&ctx_with(true, true)));
    }

    #[test]
    fn hidden_when_non_interactive() {
        assert!(!should_show(&ctx_with(false, false)));
    }
}
