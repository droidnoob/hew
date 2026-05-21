//! `hew forget <KEY>` — top-level forget with LINK: cascade.
//!
//! When a memory dies, its **outbound** edges die with it (they're
//! factually wrong — the source no longer exists). **Inbound** edges
//! are deliberately left in place so that `hew memories --links` can
//! surface them as dangling references, giving authors a chance to
//! notice and rewire.
//!
//! Algorithm:
//! 1. Read the memory set up front so the cascade target list is
//!    locked in before any forget fires.
//! 2. Forget the primary memory `<key>`.
//! 3. Forget each memory whose body is a `LINK:` row with
//!    `from == <key>`. Inbound rows (`to == <key>`) are untouched.
//!
//! Step 1 first means a failure in step 2 doesn't leave LINK sidecars
//! orphaned. (`bd forget` is the only mutation; if step 2 fails we
//! never reach step 3.) The `hew memories --forget <KEY>` flag does
//! NOT cascade — it's the existing escape hatch for callers who want
//! the lower-level single-key forget.

use clap::Args as ClapArgs;
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::memories::links::outbound_link_keys;
use hew_core::tasks;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Memory key to forget.
    #[arg(value_name = "KEY")]
    pub key: String,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover()?;
    run_with_bd(ctx, &bd, &args.key)
}

fn run_with_bd(ctx: &Ctx, bd: &dyn BdClient, key: &str) -> miette::Result<()> {
    let memories = bd.memories()?;
    let pairs: Vec<(&str, &str)> = memories.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let cascade_keys = outbound_link_keys(key, &pairs);

    tasks::forget(bd, key)?;
    for link_key in &cascade_keys {
        tasks::forget(bd, link_key)?;
    }

    if !ctx.quiet {
        println!("forgot {key}");
        if !cascade_keys.is_empty() {
            let n = cascade_keys.len();
            let m = if n == 1 { "row" } else { "rows" };
            println!("  purged {n} outbound LINK: {m}");
            for link_key in &cascade_keys {
                println!("    - {link_key}");
            }
        }
    }
    Ok(())
}
