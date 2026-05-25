//! Tree-sitter symbol extraction (skeleton — TS.1).
//!
//! Public surface for diff-driven symbol extraction. The full implementation
//! lands across TS.2 (diff intersection), TS.3 (per-language extraction), and
//! TS.4 (integration). This file only nails down the contract types so the
//! downstream slices can compile against a stable API.
//!
//! Gating: all tree-sitter crate access lives in the `grammars` submodule
//! behind `#[cfg(feature = "treesitter")]`. The contract types and the
//! `diff` line-math helpers compile under default features so TS.2 can be
//! tested without paying the grammar build cost.
//!
//! See DECISION:treesitter-feature-gating, DECISION:treesitter-v1-langs,
//! DECISION:treesitter-capture-convention, DECISION:treesitter-abi-pinning.

#[cfg(feature = "treesitter")]
pub mod grammars;

pub mod diff;

/// V1 supported languages. Exhaustive on purpose — adding a language is a
/// deliberate code change (DECISION:treesitter-v1-langs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Java,
}

/// Kind of definition extracted from a source file.
///
/// Mirrors the tree-sitter org `tags.scm` capture convention:
/// `@definition.{function,method,class,interface,module}`
/// (DECISION:treesitter-capture-convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Module,
}

/// A single symbol extracted from source. Line numbers are 1-based and
/// inclusive on both ends (matches `git diff` line semantics).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[allow(dead_code)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub language: Language,
    pub start_line: u32,
    pub end_line: u32,
}

/// Failure modes for tree-sitter operations. Variants will grow as TS.3
/// lands per-language extractors.
#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum TreesitterError {
    #[error("language not supported: {0:?}")]
    UnsupportedLanguage(Language),
    #[error("parse failed for {language:?}: {reason}")]
    ParseFailed { language: Language, reason: String },
    #[error("query compilation failed for {language:?}: {reason}")]
    QueryFailed { language: Language, reason: String },
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    /// Wiring smoke test — exists purely to assert the feature gate
    /// compiles. If the optional deps don't resolve, this file won't build.
    #[test]
    #[cfg(feature = "treesitter")]
    fn smoke_build_with_feature_compiles() {
        // Touch the gated submodule path so a stale gate fails the build.
        let _ = std::any::type_name::<super::grammars::Marker>();
    }

    /// Default-feature guard — the grammars submodule must NOT be in scope
    /// under default features. `cfg(not(feature = "treesitter"))` on the
    /// test itself proves the cfg gating is symmetric; the body sanity-
    /// checks the contract types still exist on the pure side.
    #[test]
    #[cfg(not(feature = "treesitter"))]
    fn default_build_omits_treesitter_grammars() {
        use super::{Language, SymbolKind};
        let _ = Language::Rust;
        let _ = SymbolKind::Function;
    }
}
