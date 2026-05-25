//! `hew blast` — symbol-level changelog of the current branch.
//!
//! Three input modes:
//!
//! - **diff** (default): walk `git diff --unified=0 <base>...HEAD`,
//!   extract symbols for each touched file, return the ones that
//!   overlap a hunk.
//! - **no-diff** (`--no-diff`): treat the positional / stdin file list
//!   as the full input. Extract every symbol from each file. Skips git.
//! - **stdin** (`--stdin`): read newline-separated paths from stdin in
//!   addition to (or instead of) positional args.
//!
//! Feature-gated behind `treesitter`. Off this gate, the subcommand
//! exists in the clap surface (so `hew --help` doesn't lie when the
//! binary was built without the feature) but invocation returns a
//! "rebuild with --features treesitter" error.

#[cfg(feature = "treesitter")]
use std::path::PathBuf;

use clap::Args as ClapArgs;
use hew_core::Ctx;

#[derive(Debug, ClapArgs)]
pub struct Args {
    /// Base ref to diff against. Default: `main` (falls back to
    /// `master` if `main` doesn't exist). Ignored under `--no-diff`.
    #[arg(long)]
    pub base: Option<String>,

    /// Restrict the scan to paths matching any of these substrings.
    /// Plain substring match — keep it simple, no glob library.
    /// Applied AFTER the positional + stdin file list, if any.
    #[arg(long)]
    pub path: Vec<String>,

    /// Skip git entirely. Treat the positional / stdin file list as
    /// the full input and emit every symbol from each file. Requires
    /// at least one file via positional args or `--stdin`.
    #[arg(long)]
    pub no_diff: bool,

    /// Also read newline-separated file paths from stdin. Combines
    /// with positional args.
    #[arg(long)]
    pub stdin: bool,

    /// Emit JSON instead of the default text table.
    #[arg(long)]
    pub json: bool,

    /// Files to scope to. In diff mode these intersect with the
    /// git-derived file list. In `--no-diff` mode this is the full
    /// input.
    pub files: Vec<std::path::PathBuf>,
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
    use hew_core::blast::{
        FileEntry, diff_file_set, per_file_diff_ranges, resolve_base, scan_file,
    };
    use hew_core::git::RealGit;

    // --- gather the explicit file list (positional + stdin) ---
    let mut explicit: Vec<PathBuf> = args.files.clone();
    if args.stdin {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| miette::miette!("read --stdin: {e}"))?;
        for line in buf.lines() {
            let s = line.trim();
            if !s.is_empty() {
                explicit.push(PathBuf::from(s));
            }
        }
    }

    if args.no_diff && explicit.is_empty() {
        return Err(miette::miette!(
            "--no-diff requires at least one file (positional arg or --stdin)"
        ));
    }

    let mut entries: Vec<FileEntry> = Vec::new();
    let header_base: String;

    if args.no_diff {
        header_base = "(no-diff)".to_string();
        for file in &explicit {
            if !path_matches(file, &args.path) {
                continue;
            }
            if let Some(entry) = scan_file(file, None) {
                entries.push(entry);
            }
        }
    } else {
        let git = RealGit::discover()
            .map_err(|e| miette::miette!("`git` not on PATH or unusable: {e}"))?;
        let base = resolve_base(&git, args.base.as_deref())
            .map_err(|e| miette::miette!("resolve base: {e}"))?;
        header_base = base.clone();
        let diff_files =
            diff_file_set(&git, &base).map_err(|e| miette::miette!("git diff: {e}"))?;

        let target: Vec<PathBuf> = if explicit.is_empty() {
            diff_files
        } else {
            let want: std::collections::HashSet<PathBuf> = explicit.into_iter().collect();
            diff_files.into_iter().filter(|p| want.contains(p)).collect()
        };

        for file in &target {
            if !path_matches(file, &args.path) {
                continue;
            }
            let ranges = match per_file_diff_ranges(&git, &base, file) {
                Ok(r) => r,
                Err(e) => {
                    if !ctx.quiet {
                        eprintln!("blast: diff for {} failed: {e}", file.display());
                    }
                    continue;
                }
            };
            if ranges.is_empty() {
                continue;
            }
            if let Some(entry) = scan_file(file, Some(&ranges)) {
                entries.push(entry);
            }
        }
    }

    if args.json {
        let v = serde_json::json!({ "base": header_base, "files": entries });
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return Ok(());
    }

    if entries.is_empty() {
        if !ctx.quiet {
            if args.no_diff {
                println!("hew blast: no symbols found in the given files");
            } else {
                println!("hew blast: no symbol-level changes vs {header_base}");
            }
        }
        return Ok(());
    }

    if args.no_diff {
        println!("hew blast (no-diff)");
    } else {
        println!("hew blast vs {header_base}");
    }
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
    println!(
        "\n-- {n} files, {total} {label}",
        n = entries.len(),
        label = if args.no_diff { "symbols" } else { "changed symbols" }
    );
    Ok(())
}

#[cfg(feature = "treesitter")]
fn path_matches(path: &std::path::Path, needles: &[String]) -> bool {
    if needles.is_empty() {
        return true;
    }
    let s = path.to_string_lossy();
    needles.iter().any(|n| s.contains(n.as_str()))
}

#[cfg(all(test, feature = "treesitter"))]
mod tests {
    use super::*;

    #[test]
    fn path_matches_empty_needle_lets_everything_through() {
        assert!(path_matches(std::path::Path::new("a/b.rs"), &[]));
    }

    #[test]
    fn path_matches_any_substring_hits() {
        assert!(path_matches(
            std::path::Path::new("hew-core/src/treesitter/diff.rs"),
            &["treesitter".into()]
        ));
        assert!(!path_matches(
            std::path::Path::new("hew-core/src/install.rs"),
            &["treesitter".into()]
        ));
    }
}
