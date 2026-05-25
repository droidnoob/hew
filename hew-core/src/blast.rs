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
use crate::treesitter::{
    Symbol, SymbolKind, detect_language, diff::changed_symbols, extract_symbols,
};

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
    compute_blast_with(&git, base)
}

/// Same as [`compute_blast`] but accepts an externally-managed
/// [`GitClient`]. Used by callers that already hold one (e.g.
/// `hew_core::review::bundle` threading a mock through tests).
pub fn compute_blast_with(git: &dyn GitClient, base: Option<&str>) -> Result<Vec<FileEntry>> {
    let base = resolve_base(git, base)?;
    let files = diff_file_set(git, &base)?;

    let mut out = Vec::new();
    for file in &files {
        let ranges = match per_file_diff_ranges(git, &base, file) {
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

/// A single changed symbol with its source bytes attached. The shape
/// `hew_core::review::bundle` ships to the review skill so the agent
/// can read just the changed regions instead of whole files.
#[derive(Debug, Clone, Serialize)]
pub struct ChangedSymbolForReview {
    pub file: String,
    pub language: String,
    pub name: String,
    pub kind: SymbolKind,
    pub line_start: u32,
    pub line_end: u32,
    /// The literal bytes of the symbol's definition, sliced from the
    /// file. May be empty if the byte_range is degenerate or the file
    /// can't be read at bundle time.
    pub source_slice: String,
}

/// Like [`compute_blast_with`] but flattens results into one entry per
/// changed symbol, attaching the source slice for each. Returns an
/// empty Vec on any error so a misbehaving git can't break the review
/// bundle — the caller still has the full diff as a fallback.
pub fn collect_for_review(git: &dyn GitClient, base: Option<&str>) -> Vec<ChangedSymbolForReview> {
    let entries = match compute_blast_with(git, base) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries {
        let source = std::fs::read_to_string(&entry.path).unwrap_or_default();
        for sym in entry.symbols {
            let slice = source.get(sym.byte_range.clone()).unwrap_or("").to_string();
            out.push(ChangedSymbolForReview {
                file: entry.path.clone(),
                language: entry.language.clone(),
                name: sym.name,
                kind: sym.kind,
                line_start: sym.line_range.start,
                line_end: sym.line_range.end.saturating_sub(1),
                source_slice: slice,
            });
        }
    }
    out
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
pub fn resolve_base(git: &dyn GitClient, given: Option<&str>) -> Result<String> {
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
pub fn diff_file_set(git: &dyn GitClient, base: &str) -> Result<Vec<PathBuf>> {
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
    git: &dyn GitClient,
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
