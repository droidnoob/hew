//! `hew blast` — symbol-level changelog of the current branch.
//!
//! Feature-gated behind `treesitter`. Off this gate, the subcommand
//! exists in the clap surface (so `hew --help` doesn't lie when the
//! binary was built without the feature) but invocation returns a
//! "rebuild with --features treesitter" error.

#[cfg(feature = "treesitter")]
use std::ffi::OsStr;
#[cfg(feature = "treesitter")]
use std::path::PathBuf;

use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Base ref to diff against. Default: `main` (falls back to
    /// `master` if `main` doesn't exist).
    #[arg(long)]
    pub base: Option<String>,

    /// Restrict the scan to paths matching any of these substrings.
    /// Plain substring match — keep it simple, no glob library.
    #[arg(long)]
    pub path: Vec<String>,

    /// Emit JSON instead of the default text table.
    #[arg(long)]
    pub json: bool,
}

#[cfg(not(feature = "treesitter"))]
pub fn run(_ctx: &Ctx, _args: Args) -> miette::Result<()> {
    Err(miette::miette!(
        "`hew blast` requires the `treesitter` feature. \
         Rebuild with `cargo install hew --features treesitter` or \
         `cargo build -p hew --features treesitter`."
    ))
}

#[cfg(feature = "treesitter")]
pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    use hew_core::diff_hunks::parse_changed_ranges;
    use hew_core::git::{GitClient, RealGit};
    use hew_core::treesitter::{detect_language, diff::changed_symbols, extract_symbols};

    let git =
        RealGit::discover().map_err(|e| miette::miette!("`git` not on PATH or unusable: {e}"))?;

    let base = resolve_base(&git, args.base.as_deref())?;

    // List touched files between base and HEAD (working tree included).
    let name_only = git
        .run_raw(&[
            OsStr::new("diff"),
            OsStr::new("--name-only"),
            OsStr::new(&format!("{base}...HEAD")),
        ])
        .map_err(|e| miette::miette!("git diff --name-only failed: {e}"))?;

    let files: Vec<PathBuf> = name_only
        .stdout
        .lines()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter(|s| args.path.is_empty() || args.path.iter().any(|p| s.contains(p.as_str())))
        .map(PathBuf::from)
        .collect();

    let mut entries: Vec<FileEntry> = Vec::new();
    for file in &files {
        let Some(lang) = detect_language(file) else {
            continue;
        };
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            // Deleted files won't read; that's fine — nothing to scope.
            Err(_) => continue,
        };
        let symbols = match extract_symbols(&source, lang) {
            Ok(s) => s,
            Err(e) => {
                if !ctx.quiet {
                    eprintln!("blast: extract failed for {}: {e}", file.display());
                }
                continue;
            }
        };
        // Per-file diff for hunk headers.
        let diff = git
            .run_raw(&[
                OsStr::new("diff"),
                OsStr::new("--unified=0"),
                OsStr::new(&format!("{base}...HEAD")),
                OsStr::new("--"),
                file.as_os_str(),
            ])
            .map_err(|e| miette::miette!("git diff for {} failed: {e}", file.display()))?;
        let ranges = parse_changed_ranges(&diff.stdout);
        if ranges.is_empty() {
            continue;
        }
        let hit = changed_symbols(&symbols, &ranges);
        if hit.is_empty() {
            continue;
        }
        entries.push(FileEntry {
            path: file.display().to_string(),
            language: format!("{lang:?}"),
            symbols: hit,
        });
    }

    if args.json {
        let v = serde_json::json!({ "base": base, "files": entries });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return Ok(());
    }

    if entries.is_empty() {
        if !ctx.quiet {
            println!("hew blast: no symbol-level changes vs {base}");
        }
        return Ok(());
    }

    println!("hew blast vs {base}");
    let mut total = 0;
    for FileEntry { path, language, symbols } in &entries {
        println!("\n{path} ({language}) — {n} symbols", n = symbols.len());
        for s in symbols {
            println!(
                "  {kind:<9} {name:<32} lines {a}-{b}",
                kind = format!("{:?}", s.kind),
                name = s.name,
                a = s.line_range.start,
                b = s.line_range.end.saturating_sub(1),
            );
            total += 1;
        }
    }
    println!("\n-- {n} files, {total} changed symbols", n = entries.len());
    Ok(())
}

#[cfg(feature = "treesitter")]
#[derive(Debug, serde::Serialize)]
struct FileEntry {
    path: String,
    language: String,
    symbols: Vec<hew_core::treesitter::Symbol>,
}

#[cfg(feature = "treesitter")]
fn resolve_base(git: &hew_core::git::RealGit, given: Option<&str>) -> miette::Result<String> {
    use hew_core::git::GitClient;
    if let Some(g) = given {
        return Ok(g.to_string());
    }
    // Try `main`, then `master`. We're after the *local* branch so
    // `rev-parse --verify` is enough.
    for candidate in ["main", "master"] {
        let out =
            git.run_raw(&[OsStr::new("rev-parse"), OsStr::new("--verify"), OsStr::new(candidate)]);
        if out.is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err(miette::miette!("could not resolve a base ref. Pass --base <ref> explicitly."))
}
