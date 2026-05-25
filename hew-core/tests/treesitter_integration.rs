//! End-to-end integration tests for the `treesitter` feature.
//!
//! Exercises the full path that the future `hew blast` consumer uses:
//!   source file (via include_str!) + synthetic changed-line ranges →
//!   extract_symbols → changed_symbols → assertion on the returned set.
//!
//! Per-language fixtures live alongside this file under
//! `tests/treesitter/fixtures/sample.{ext}`. Each fixture is hand-sized
//! so the expected symbol set is predictable.

#![cfg(feature = "treesitter")]
// Each e2e test builds a single "diff hunk" — one Range — to assert that
// changed_symbols intersects it correctly. The lint that flags
// `[start..end]` as suspicious doesn't apply here.
#![allow(clippy::single_range_in_vec_init)]

use hew_core::treesitter::{Language, Symbol, SymbolKind, diff::changed_symbols, extract_symbols};

fn pairs(symbols: &[Symbol]) -> Vec<(String, SymbolKind)> {
    let mut out: Vec<_> = symbols.iter().map(|s| (s.name.clone(), s.kind)).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn pair(name: &str, kind: SymbolKind) -> (String, SymbolKind) {
    (name.to_string(), kind)
}

/// Pick the symbol with `name` and assert it's present, then return its
/// `line_range` so the caller can build a deterministic changed-range
/// that intersects exactly this symbol.
fn line_range_of(symbols: &[Symbol], name: &str) -> std::ops::Range<u32> {
    symbols
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("expected symbol `{name}` not found"))
        .line_range
        .clone()
}

// --- Rust ---

const RUST_SRC: &str = include_str!("treesitter/fixtures/sample.rs");

#[test]
fn rust_e2e_extract_and_changed_intersection() {
    let syms = extract_symbols(RUST_SRC, Language::Rust).unwrap();
    let want = {
        let mut v = vec![
            pair("alpha_compute", SymbolKind::Function),
            pair("beta_format", SymbolKind::Function),
            pair("Widget", SymbolKind::Class),
            pair("gamma_describe", SymbolKind::Method),
            pair("delta_clone", SymbolKind::Method),
            pair("epsilon_dispatch", SymbolKind::Function),
        ];
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(pairs(&syms), want);

    // Diff hits the gamma_describe method only.
    let gamma = line_range_of(&syms, "gamma_describe");
    let hit = {
        let changes: [std::ops::Range<u32>; 1] = [gamma.start..gamma.start + 1];
        changed_symbols(&syms, &changes)
    };
    let names: Vec<_> = hit.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["gamma_describe"]);
}

// --- Python ---

const PYTHON_SRC: &str = include_str!("treesitter/fixtures/sample.py");

#[test]
fn python_e2e_extract_and_changed_intersection() {
    let syms = extract_symbols(PYTHON_SRC, Language::Python).unwrap();
    let want = {
        let mut v = vec![
            pair("alpha_compute", SymbolKind::Function),
            pair("beta_format", SymbolKind::Function),
            pair("Widget", SymbolKind::Class),
            pair("__init__", SymbolKind::Method),
            pair("gamma_describe", SymbolKind::Method),
            pair("delta_clone", SymbolKind::Method),
            pair("epsilon_dispatch", SymbolKind::Function),
        ];
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(pairs(&syms), want);

    let gamma = line_range_of(&syms, "gamma_describe");
    let hit = {
        let changes: [std::ops::Range<u32>; 1] = [gamma.start..gamma.start + 1];
        changed_symbols(&syms, &changes)
    };
    let names: Vec<_> = hit.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"gamma_describe"));
}

// --- TypeScript ---

const TS_SRC: &str = include_str!("treesitter/fixtures/sample.ts");

#[test]
fn typescript_e2e_extract_and_changed_intersection() {
    let syms = extract_symbols(TS_SRC, Language::TypeScript).unwrap();
    let want = {
        let mut v = vec![
            pair("alphaCompute", SymbolKind::Function),
            pair("betaFormat", SymbolKind::Function),
            pair("Widget", SymbolKind::Class),
            pair("constructor", SymbolKind::Method),
            pair("gammaDescribe", SymbolKind::Method),
            pair("deltaClone", SymbolKind::Method),
            pair("epsilonDispatch", SymbolKind::Function),
        ];
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(pairs(&syms), want);

    let gamma = line_range_of(&syms, "gammaDescribe");
    let hit = {
        let changes: [std::ops::Range<u32>; 1] = [gamma.start..gamma.start + 1];
        changed_symbols(&syms, &changes)
    };
    let names: Vec<_> = hit.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"gammaDescribe"));
}

// --- JavaScript ---

const JS_SRC: &str = include_str!("treesitter/fixtures/sample.js");

#[test]
fn javascript_e2e_extract_and_changed_intersection() {
    let syms = extract_symbols(JS_SRC, Language::JavaScript).unwrap();
    let want = {
        let mut v = vec![
            pair("alphaCompute", SymbolKind::Function),
            pair("betaFormat", SymbolKind::Function),
            pair("Widget", SymbolKind::Class),
            pair("constructor", SymbolKind::Method),
            pair("gammaDescribe", SymbolKind::Method),
            pair("deltaClone", SymbolKind::Method),
            pair("epsilonDispatch", SymbolKind::Function),
        ];
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(pairs(&syms), want);

    let gamma = line_range_of(&syms, "gammaDescribe");
    let hit = {
        let changes: [std::ops::Range<u32>; 1] = [gamma.start..gamma.start + 1];
        changed_symbols(&syms, &changes)
    };
    let names: Vec<_> = hit.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"gammaDescribe"));
}

// --- Go ---

const GO_SRC: &str = include_str!("treesitter/fixtures/sample.go");

#[test]
fn go_e2e_extract_and_changed_intersection() {
    let syms = extract_symbols(GO_SRC, Language::Go).unwrap();
    let want = {
        let mut v = vec![
            pair("AlphaCompute", SymbolKind::Function),
            pair("BetaFormat", SymbolKind::Function),
            pair("Widget", SymbolKind::Class),
            pair("GammaDescribe", SymbolKind::Method),
            pair("DeltaClone", SymbolKind::Method),
            pair("EpsilonDispatch", SymbolKind::Function),
        ];
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };
    assert_eq!(pairs(&syms), want);

    let gamma = line_range_of(&syms, "GammaDescribe");
    let hit = {
        let changes: [std::ops::Range<u32>; 1] = [gamma.start..gamma.start + 1];
        changed_symbols(&syms, &changes)
    };
    let names: Vec<_> = hit.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"GammaDescribe"));
}

// --- Java ---

const JAVA_SRC: &str = include_str!("treesitter/fixtures/sample.java");

#[test]
fn java_e2e_extract_and_changed_intersection() {
    let syms = extract_symbols(JAVA_SRC, Language::Java).unwrap();
    let names: std::collections::HashSet<&str> = syms.iter().map(|s| s.name.as_str()).collect();
    // Java requires class wrappers; check the expected definitions surface.
    for want in [
        "Sample",
        "Widget",
        "alphaCompute",
        "betaFormat",
        "epsilonDispatch",
        "gammaDescribe",
        "deltaClone",
    ] {
        assert!(names.contains(want), "expected `{want}` in Java symbols, got {names:?}");
    }
    // Class vs method classification on the wrapper + a known method.
    let sample = syms.iter().find(|s| s.name == "Sample").unwrap();
    assert_eq!(sample.kind, SymbolKind::Class);
    let gamma = syms.iter().find(|s| s.name == "gammaDescribe").unwrap();
    assert_eq!(gamma.kind, SymbolKind::Method);

    let changes: [std::ops::Range<u32>; 1] = [gamma.line_range.start..gamma.line_range.start + 1];
    let hit = changed_symbols(&syms, &changes);
    let hit_names: Vec<_> = hit.iter().map(|s| s.name.as_str()).collect();
    assert!(hit_names.contains(&"gammaDescribe"));
}

// --- Non-gating perf signal ---

/// Parses the Rust fixture and runs extract_symbols. Reports timing via
/// eprintln!. Only enforces a budget when `HEW_TS_BENCH=1` is set —
/// otherwise we don't want to flap on slow CI runners.
#[test]
fn perf_parse_under_5ms_warm() {
    // Warm-up parse so cold codegen doesn't dominate.
    let _ = extract_symbols(RUST_SRC, Language::Rust).unwrap();

    let start = std::time::Instant::now();
    let syms = extract_symbols(RUST_SRC, Language::Rust).unwrap();
    let elapsed = start.elapsed();
    assert!(!syms.is_empty());

    eprintln!("treesitter perf: parse + extract Rust fixture in {elapsed:?}");

    if std::env::var("HEW_TS_BENCH").as_deref() == Ok("1") {
        assert!(
            elapsed < std::time::Duration::from_millis(5),
            "perf gate: expected <5ms warm, got {elapsed:?}"
        );
    }
}
