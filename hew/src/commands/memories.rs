use std::path::PathBuf;

use clap::Args as ClapArgs;
use hew_core::bd::{BdClient, RealBd};
use hew_core::tasks;
use hew_core::{Ctx, OutputMode};

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Filter to memories whose value starts with this prefix (e.g.
    /// CONVENTION, BOUNDARY, AUDIT, SECURITY, MIGRATION, DEP,
    /// STATUS, CHECKPOINT).
    #[arg(long, conflicts_with_all = ["recall", "forget"])]
    pub prefix: Option<String>,

    /// Filter to memories whose value contains this substring (case-insensitive).
    #[arg(long, conflicts_with_all = ["recall", "forget"])]
    pub grep: Option<String>,

    /// Sugar for `--prefix=RESEARCH --grep=<topic>`. Conflicts with --prefix.
    #[arg(long, value_name = "TOPIC", conflicts_with_all = ["prefix", "recall", "forget"])]
    pub research: Option<String>,

    /// Print a single memory by key.
    #[arg(long, value_name = "KEY", conflicts_with = "forget")]
    pub recall: Option<String>,

    /// Remove a single memory by key.
    #[arg(long, value_name = "KEY")]
    pub forget: Option<String>,

    /// Export filtered memories to a file (JSON by default). Pair with
    /// `-o <PATH>` for an explicit destination; without `-o`, the file
    /// lands at `<projname>-memories-<iso-ts>.<ext>` in the current
    /// directory. Use `--plaintext` for human-readable text instead of
    /// JSON.
    #[arg(long, conflicts_with_all = ["recall", "forget"])]
    pub export: bool,

    /// With `--export`: explicit output path. When omitted, the
    /// default `<projname>-memories-<iso-ts>.<ext>` is used.
    #[arg(short = 'o', long = "out", value_name = "PATH", requires = "export")]
    pub out: Option<PathBuf>,

    /// With `--export`: write human-readable text instead of JSON.
    #[arg(long, requires = "export")]
    pub plaintext: bool,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let client = RealBd::discover()?;

    if let Some(key) = args.recall.as_deref() {
        return run_recall(ctx, &client, key);
    }
    if let Some(key) = args.forget.as_deref() {
        return run_forget(ctx, &client, key);
    }

    let memories = client.memories()?;

    // Resolve --research sugar into the underlying prefix/grep filters.
    let (prefix, grep) = if let Some(topic) = args.research.as_ref() {
        (Some("RESEARCH".to_string()), Some(topic.clone()))
    } else {
        (args.prefix.clone(), args.grep.clone())
    };

    let needle = grep.as_ref().map(|s| s.to_lowercase());
    let pfx = prefix.as_ref().map(|p| format!("{}:", p.trim_end_matches(':')));

    let mut hits: Vec<(&String, &String)> = memories
        .iter()
        .filter(|(_, v)| pfx.as_ref().is_none_or(|p| v.trim_start().starts_with(p)))
        .filter(|(_, v)| needle.as_ref().is_none_or(|n| v.to_lowercase().contains(n)))
        .collect();
    hits.sort_by(|a, b| a.0.cmp(b.0));

    if args.export {
        let resolved = resolve_export_path(args.out.as_deref(), args.plaintext);
        let body = if args.plaintext { render_plaintext(&hits) } else { render_json(&hits)? };
        std::fs::write(&resolved, body)
            .map_err(|e| miette::miette!("writing {}: {}", resolved.display(), e))?;
        if !ctx.quiet {
            println!("exported {} memories to {}", hits.len(), resolved.display());
        }
        return Ok(());
    }

    if matches!(ctx.output, OutputMode::Json) {
        let obj: serde_json::Map<String, serde_json::Value> = hits
            .iter()
            .map(|(k, v)| ((*k).clone(), serde_json::Value::String((*v).clone())))
            .collect();
        println!("{}", serde_json::to_string_pretty(&serde_json::Value::Object(obj)).unwrap());
    } else if hits.is_empty() {
        println!("(no memories match)");
    } else {
        for (k, v) in &hits {
            println!("- {k}");
            for line in v.lines() {
                println!("    {line}");
            }
        }
        println!();
        println!("{} memories", hits.len());
    }
    Ok(())
}

fn run_recall(ctx: &Ctx, bd: &dyn BdClient, key: &str) -> miette::Result<()> {
    match tasks::recall(bd, key)? {
        Some(body) => {
            println!("{body}");
            Ok(())
        }
        None => {
            if !ctx.quiet {
                eprintln!("no memory with key `{key}`");
            }
            Err(miette::miette!("no memory with key `{key}`"))
        }
    }
}

fn run_forget(ctx: &Ctx, bd: &dyn BdClient, key: &str) -> miette::Result<()> {
    tasks::forget(bd, key)?;
    if !ctx.quiet {
        println!("forgot {key}");
    }
    Ok(())
}

/// Resolve `-o <PATH>` to a concrete path. When `None`, falls back to
/// `<projname>-memories-<isoTS>.<ext>` in the current working directory.
fn resolve_export_path(provided: Option<&std::path::Path>, plaintext: bool) -> PathBuf {
    if let Some(p) = provided {
        return p.to_path_buf();
    }
    let projname = current_project_name();
    let ts = fs_safe_iso_now_utc();
    let ext = if plaintext { "txt" } else { "json" };
    PathBuf::from(format!("{projname}-memories-{ts}.{ext}"))
}

fn current_project_name() -> String {
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "hew".to_string())
}

/// Filesystem-safe ISO-8601 UTC: `YYYY-MM-DDTHH-MM-SSZ` (colons
/// replaced with dashes so the path is safe on every OS hew supports).
fn fs_safe_iso_now_utc() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    fs_safe_iso_from_unix(secs)
}

fn fs_safe_iso_from_unix(secs: i64) -> String {
    // Civil-from-days per Howard Hinnant's public-domain calendar code —
    // duplicates the helper in hew/src/commands/compact.rs::iso_from_unix
    // so this module stays self-contained.
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y_shift = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y_shift + if month <= 2 { 1 } else { 0 };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}Z")
}

fn render_json(hits: &[(&String, &String)]) -> miette::Result<String> {
    let obj: serde_json::Map<String, serde_json::Value> =
        hits.iter().map(|(k, v)| ((*k).clone(), serde_json::Value::String((*v).clone()))).collect();
    serde_json::to_string_pretty(&serde_json::Value::Object(obj))
        .map_err(|e| miette::miette!("serializing JSON: {e}"))
}

fn render_plaintext(hits: &[(&String, &String)]) -> String {
    let mut out = String::new();
    for (k, v) in hits {
        out.push_str("- ");
        out.push_str(k);
        out.push('\n');
        for line in v.lines() {
            out.push_str("    ");
            out.push_str(line);
            out.push('\n');
        }
    }
    if hits.is_empty() {
        out.push_str("(no memories)\n");
    } else {
        out.push_str(&format!("\n{} memories\n", hits.len()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_filename_uses_dashes_not_colons() {
        let s = fs_safe_iso_from_unix(0);
        assert_eq!(s, "1970-01-01T00-00-00Z");
        assert!(!s.contains(':'), "colons are unsafe on Windows paths");
    }

    #[test]
    fn iso_filename_known_epoch() {
        // 2026-05-16T12:00:00Z → secs = 1_778_932_800.
        let s = fs_safe_iso_from_unix(1_778_932_800);
        assert_eq!(s, "2026-05-16T12-00-00Z");
    }

    #[test]
    fn resolve_export_path_uses_provided() {
        let provided = PathBuf::from("/tmp/foo.json");
        let r = resolve_export_path(Some(&provided), false);
        assert_eq!(r, PathBuf::from("/tmp/foo.json"));
    }

    #[test]
    fn resolve_export_path_default_includes_proj_and_ext() {
        let r = resolve_export_path(None, false);
        let s = r.to_string_lossy();
        assert!(s.ends_with(".json"), "default JSON ext: {s}");
        assert!(s.contains("-memories-"), "default name shape: {s}");

        let r = resolve_export_path(None, true);
        let s = r.to_string_lossy();
        assert!(s.ends_with(".txt"), "default plaintext ext: {s}");
    }

    #[test]
    fn render_json_preserves_keys_and_values() {
        let k1 = "a".to_string();
        let v1 = "alpha".to_string();
        let k2 = "b".to_string();
        let v2 = "bravo".to_string();
        let hits: Vec<(&String, &String)> = vec![(&k1, &v1), (&k2, &v2)];
        let s = render_json(&hits).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["a"], "alpha");
        assert_eq!(parsed["b"], "bravo");
    }

    #[test]
    fn render_plaintext_shape() {
        let k = "convention-foo".to_string();
        let v = "CONVENTION:foo — bar".to_string();
        let hits: Vec<(&String, &String)> = vec![(&k, &v)];
        let s = render_plaintext(&hits);
        assert!(s.contains("- convention-foo"));
        assert!(s.contains("    CONVENTION:foo — bar"));
        assert!(s.contains("1 memories"));
    }

    #[test]
    fn render_plaintext_empty_says_so() {
        let s = render_plaintext(&[]);
        assert!(s.contains("(no memories)"));
    }
}
