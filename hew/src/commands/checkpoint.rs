//! `hew checkpoint "<body>"` — write a properly-shaped `CHECKPOINT:`
//! memory in one shot.
//!
//! Before this subcommand existed, the `hew-checkpoint` skill body
//! told the agent to call `hew remember --raw "CHECKPOINT:…" --key …`.
//! Easy to get wrong — and getting it wrong (no ISO timestamp directly
//! after the `CHECKPOINT:` prefix) silently shadowed newer good
//! checkpoints in `hew prime resume` (GitHub issue #40). This
//! subcommand removes the foot-gun: pass the body, get a correct
//! `CHECKPOINT:<ISO> — <body>` row with an auto-generated key.

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::checkpoint::{build_checkpoint_body, build_checkpoint_key};
use hew_core::error::HewError;
use hew_core::memories::links::{LinkKind, build_link_row_body};
use hew_core::tasks;
use hew_core::time::iso_now_utc;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Checkpoint body. `CHECKPOINT:<ISO-8601-now> — ` is prepended
    /// automatically if missing. If the body already starts with a
    /// correctly-shaped `CHECKPOINT:<ISO>` prefix it's written
    /// verbatim; a malformed prefix (e.g. `CHECKPOINT:practice-svc-…`)
    /// is rewritten to the canonical shape.
    pub body: Option<String>,

    /// Override the auto-generated key. Default: `checkpoint-<ISO>`
    /// with colons replaced by `-`.
    #[arg(long)]
    pub key: Option<String>,

    /// Override the timestamp embedded in the body / key. Useful for
    /// tests and for back-dating an after-the-fact checkpoint. Format:
    /// `YYYY-MM-DDTHH:MM:SSZ`.
    #[arg(long = "timestamp", value_name = "ISO")]
    pub timestamp: Option<String>,

    /// Emit a `LINK:<key>->relates_to:memory:<related>` sidecar after
    /// the primary write. Repeatable.
    #[arg(long = "related", value_name = "KEY")]
    pub related: Vec<String>,

    /// Emit a `LINK:<key>->relates_to:task:<id>` sidecar after the
    /// primary write. Repeatable.
    #[arg(long = "related-task", value_name = "ID")]
    pub related_task: Vec<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    let body = args.body.ok_or_else(|| HewError::MissingFlag { flag: "body".to_string() })?;
    if body.trim().is_empty() {
        return Err(miette::miette!("checkpoint body is empty — pass at least a one-line summary"));
    }

    let now_iso = args.timestamp.unwrap_or_else(iso_now_utc);
    let key = args.key.unwrap_or_else(|| build_checkpoint_key(&now_iso));
    let payload = build_checkpoint_body(&body, &now_iso);

    for v in &args.related {
        validate_link_target("--related", v)?;
    }
    for v in &args.related_task {
        validate_link_target("--related-task", v)?;
    }

    tasks::remember(&bd, &payload, Some(&key))?;
    if !ctx.quiet {
        println!("checkpoint saved (CHECKPOINT:{now_iso}, key={key}). Safe to /clear.");
    }

    if !args.related.is_empty() || !args.related_task.is_empty() {
        write_link_sidecars(&bd, ctx, &key, &args.related, &args.related_task)?;
    }
    Ok(())
}

fn write_link_sidecars(
    bd: &dyn BdClient,
    ctx: &Ctx,
    from: &str,
    related: &[String],
    related_task: &[String],
) -> miette::Result<()> {
    let mut written = 0usize;
    for to in related {
        let body = build_link_row_body(from, LinkKind::Memory, to);
        tasks::remember(bd, &body, None)?;
        written += 1;
    }
    for to in related_task {
        let body = build_link_row_body(from, LinkKind::Task, to);
        tasks::remember(bd, &body, None)?;
        written += 1;
    }
    if !ctx.quiet && written > 0 {
        let m = if written == 1 { "memory" } else { "memories" };
        println!("emitted {written} LINK: sidecar {m}");
    }
    Ok(())
}

fn validate_link_target(flag: &str, value: &str) -> miette::Result<()> {
    if value.is_empty() {
        return Err(miette::miette!("{flag} requires a non-empty value"));
    }
    let ok = value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.'));
    if !ok {
        return Err(miette::miette!(
            "{flag}={value:?} is not a valid LINK target — must match `[a-z0-9._-]+`"
        ));
    }
    Ok(())
}
