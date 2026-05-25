//! Library-side core of the `hew blast` symbol-changelog feature.
//!
//! Feature-gated on `treesitter`. The CLI in `hew/src/commands/blast.rs`
//! is a thin wrapper around the helpers here; the close-note attach
//! path (`hew-execute` → `hew task close` when `craft.symbol_trace=true`)
//! also calls in here directly so it doesn't have to re-exec the
//! binary.
//!
//! All errors flow through [`HewError`] for parity with the rest of
//! `hew_core`. Per-file failures (couldn't read, parse failed) are
//! swallowed silently — callers want the partial result, not an abort
//! on the first unreadable file.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::diff_hunks::parse_changed_ranges;
use crate::error::{HewError, Result};
use crate::git::{GitClient, RealGit};
use crate::treesitter::{Symbol, detect_language, diff::changed_symbols, extract_symbols};

/// A single file's symbol-level changelog entry.
#[derive(Debug, Clone, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub language: String,
    pub symbols: Vec<Symbol>,
}

/// Run a full symbol-level diff against `base` (defaults to `main`
/// then `master`). Returns one [`FileEntry`] per touched file that
/// has at least one symbol overlapping a diff hunk.
///
/// Files with unknown extensions, unreadable contents, or no
/// overlapping symbols are silently skipped.
pub fn compute_blast(base: Option<&str>) -> Result<Vec<FileEntry>> {
    let git = RealGit::discover()?;
    let base = resolve_base(&git, base)?;
    let files = diff_file_set(&git, &base)?;

    let mut out = Vec::new();
    for file in &files {
        let ranges = match per_file_diff_ranges(&git, &base, file) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if ranges.is_empty() {
            continue;
        }
        if let Some(entry) = scan_file(file, Some(&ranges)) {
            out.push(entry);
        }
    }
    Ok(out)
}

/// Extract symbols from a single file. If `ranges` is `None`, returns
/// every extracted symbol. If `Some`, returns only the ones that
/// overlap a range. Returns `None` when the file can't be classified
/// or can't be read — the caller treats that as "skip silently."
pub fn scan_file(file: &Path, ranges: Option<&[std::ops::Range<u32>]>) -> Option<FileEntry> {
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

/// Resolve the base ref. Honors an explicit override; otherwise probes
/// `main`, then `master`. Errors with [`HewError::GitNonZero`] if
/// neither exists.
pub fn resolve_base(git: &RealGit, given: Option<&str>) -> Result<String> {
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
    Err(HewError::GitNonZero {
        code: -1,
        stderr: "no `main` or `master` ref found; pass an explicit base".into(),
    })
}

/// The full list of files in `git diff --name-only <base>...HEAD`.
pub fn diff_file_set(git: &RealGit, base: &str) -> Result<Vec<PathBuf>> {
    let out = git.run_raw(&[
        OsStr::new("diff"),
        OsStr::new("--name-only"),
        OsStr::new(&format!("{base}...HEAD")),
    ])?;
    Ok(out.stdout.lines().map(str::trim).filter(|s| !s.is_empty()).map(PathBuf::from).collect())
}

/// Parse the `+` side of every hunk header in `git diff --unified=0
/// <base>...HEAD -- <file>`.
pub fn per_file_diff_ranges(
    git: &RealGit,
    base: &str,
    file: &Path,
) -> Result<Vec<std::ops::Range<u32>>> {
    let out = git.run_raw(&[
        OsStr::new("diff"),
        OsStr::new("--unified=0"),
        OsStr::new(&format!("{base}...HEAD")),
        OsStr::new("--"),
        file.as_os_str(),
    ])?;
    Ok(parse_changed_ranges(&out.stdout))
}

/// Format a list of [`FileEntry`] as a compact human-readable note
/// suitable for `bd update --append-notes`. Each file is one block:
///
/// ```text
/// symbols changed (blast vs main):
///   hew-core/src/treesitter/grammars.rs (Rust)
///     [Method] extract_symbols  lines 57-112
///     [Method] dedupe           lines 117-129
/// ```
pub fn format_note(base: &str, entries: &[FileEntry]) -> String {
    let mut s = format!("symbols changed (blast vs {base}):\n");
    for FileEntry { path, language, symbols } in entries {
        s.push_str(&format!("  {path} ({language})\n"));
        for sym in symbols {
            s.push_str(&format!(
                "    [{kind:?}] {name}  lines {a}-{b}\n",
                kind = sym.kind,
                name = sym.name,
                a = sym.line_range.start,
                b = sym.line_range.end.saturating_sub(1),
            ));
        }
    }
    s
}
