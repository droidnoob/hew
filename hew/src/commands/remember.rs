//! `hew remember --type=<prefix> "<body>"` — write a memory with an
//! enforced allowlist. The `--raw` escape hatch skips validation for
//! migrations.

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::RealBd;
use hew_core::error::HewError;
use hew_core::tasks::{self, MEMORY_PREFIXES, validate_memory_type};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Memory type — one of: convention, boundary, security, audit,
    /// decision, status, gotcha, project, milestone, roadmap,
    /// research, dep, factual. The canonical UPPER prefix is prepended
    /// to the body before write.
    #[arg(long = "type", conflicts_with = "raw")]
    pub kind: Option<String>,

    /// Bare body. With `--type=foo`, written as `FOO:<body>`. With
    /// `--raw`, written verbatim.
    pub body: String,

    /// Optional explicit key for upsert-by-key semantics.
    #[arg(long)]
    pub key: Option<String>,

    /// Skip allowlist validation; write `body` verbatim. Use only for
    /// migrations or temporarily-unknown prefixes.
    #[arg(long)]
    pub raw: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;

    let payload = if args.raw {
        args.body
    } else {
        let kind = args.kind.as_deref().ok_or_else(|| HewError::MissingFlag {
            flag: format!("type (one of: {})", MEMORY_PREFIXES.join(", ")),
        })?;
        let upper = validate_memory_type(kind)?;
        if body_already_has_known_prefix(&args.body) {
            return Err(miette::miette!(
                "body already starts with a known prefix — pass either `--type=<x>` with a bare \
                 body, or `--raw` with the full prefixed string (got: {:?})",
                args.body.chars().take(40).collect::<String>(),
            ));
        }
        format!("{upper}:{}", args.body)
    };

    tasks::remember(&bd, &payload, args.key.as_deref())?;
    if !ctx.quiet {
        match args.key.as_deref() {
            Some(k) => println!("remembered ({k})"),
            None => println!("remembered"),
        }
    }
    Ok(())
}

fn body_already_has_known_prefix(body: &str) -> bool {
    let trimmed = body.trim_start();
    let Some(colon) = trimmed.find(':') else {
        return false;
    };
    let prefix = &trimmed[..colon];
    validate_memory_type(prefix).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_known_prefix() {
        assert!(body_already_has_known_prefix("CONVENTION:foo"));
        assert!(body_already_has_known_prefix("  DECISION:bar"));
        assert!(body_already_has_known_prefix("dep:something")); // case-insensitive via validate
    }

    #[test]
    fn ignores_bodies_without_prefix() {
        assert!(!body_already_has_known_prefix("just a note"));
        assert!(!body_already_has_known_prefix("UNKNOWN:thing"));
        assert!(!body_already_has_known_prefix(""));
    }
}
