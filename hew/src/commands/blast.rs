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
use std::ffi::OsStr;
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

    // --- resolve the file set + per-file changed ranges ---
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
        // diff mode
        let git = RealGit::discover()
            .map_err(|e| miette::miette!("`git` not on PATH or unusable: {e}"))?;
        let base = resolve_base(&git, args.base.as_deref())?;
        header_base = base.clone();
        let diff_files = git_diff_file_set(&git, &base)?;

        // If explicit files were given, intersect with the diff set.
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

/// Extract symbols from a single file. If `ranges` is `None`, returns
/// every extracted symbol. If `Some`, returns only the ones that
/// overlap a range. Returns `None` when the file can't be classified or
/// can't be read — the caller treats that as "skip silently."
#[cfg(feature = "treesitter")]
fn scan_file(file: &std::path::Path, ranges: Option<&[std::ops::Range<u32>]>) -> Option<FileEntry> {
    use hew_core::treesitter::{detect_language, diff::changed_symbols, extract_symbols};
    let lang = detect_language(file)?;
    let source = std::fs::read_to_string(file).ok()?;
    let symbols = extract_symbols(&source, lang).ok()?;
    let kept = match ranges {
        Some(r) => changed_symbols(&symbols, r),
        None => symbols,
    };
    if kept.is_empty() {
        return None;
    }
    Some(FileEntry {
        path: file.display().to_string(),
        language: format!("{lang:?}"),
        symbols: kept,
    })
}

#[cfg(feature = "treesitter")]
fn git_diff_file_set(git: &hew_core::git::RealGit, base: &str) -> miette::Result<Vec<PathBuf>> {
    use hew_core::git::GitClient;
    let out = git
        .run_raw(&[
            OsStr::new("diff"),
            OsStr::new("--name-only"),
            OsStr::new(&format!("{base}...HEAD")),
        ])
        .map_err(|e| miette::miette!("git diff --name-only failed: {e}"))?;
    Ok(out.stdout.lines().map(|s| s.trim()).filter(|s| !s.is_empty()).map(PathBuf::from).collect())
}

#[cfg(feature = "treesitter")]
fn per_file_diff_ranges(
    git: &hew_core::git::RealGit,
    base: &str,
    file: &std::path::Path,
) -> miette::Result<Vec<std::ops::Range<u32>>> {
    use hew_core::diff_hunks::parse_changed_ranges;
    use hew_core::git::GitClient;
    let out = git
        .run_raw(&[
            OsStr::new("diff"),
            OsStr::new("--unified=0"),
            OsStr::new(&format!("{base}...HEAD")),
            OsStr::new("--"),
            file.as_os_str(),
        ])
        .map_err(|e| miette::miette!("git diff failed: {e}"))?;
    Ok(parse_changed_ranges(&out.stdout))
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
    for candidate in ["main", "master"] {
        let out =
            git.run_raw(&[OsStr::new("rev-parse"), OsStr::new("--verify"), OsStr::new(candidate)]);
        if out.is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err(miette::miette!("could not resolve a base ref. Pass --base <ref> explicitly."))
}

#[cfg(all(test, feature = "treesitter"))]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn path_matches_empty_needle_lets_everything_through() {
        assert!(path_matches(Path::new("a/b.rs"), &[]));
    }

    #[test]
    fn path_matches_any_substring_hits() {
        assert!(path_matches(Path::new("hew-core/src/treesitter/diff.rs"), &["treesitter".into()]));
        assert!(!path_matches(Path::new("hew-core/src/install.rs"), &["treesitter".into()]));
    }

    #[test]
    fn scan_file_no_diff_returns_all_symbols_for_a_rust_file() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sample.rs");
        std::fs::write(&p, "fn alpha() {}\nstruct Widget;\nimpl Widget { fn beta(&self) {} }\n")
            .unwrap();
        let got = scan_file(&p, None).expect("scan_file should find symbols");
        let names: Vec<&str> = got.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"alpha"));
        assert!(names.contains(&"Widget"));
        assert!(names.contains(&"beta"));
    }

    #[test]
    fn scan_file_with_narrow_range_returns_only_overlapping() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("sample.rs");
        // 3 line ranges: alpha 1, Widget 2, impl(beta) 3
        std::fs::write(&p, "fn alpha() {}\nstruct Widget;\nimpl Widget { fn beta(&self) {} }\n")
            .unwrap();
        let all = scan_file(&p, None).expect("baseline scan").symbols;
        let widget = all.iter().find(|s| s.name == "Widget").expect("Widget should be extracted");
        let one_range = [widget.line_range.clone()];
        let scoped = scan_file(&p, Some(&one_range)).expect("scoped scan");
        let names: Vec<&str> = scoped.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Widget"));
        assert!(scoped.symbols.len() < all.len(), "scoped should be a strict subset");
    }
}
