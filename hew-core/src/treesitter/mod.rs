//! Tree-sitter symbol extraction.
//!
//! Public surface for diff-driven symbol extraction. TS.1 wired the
//! Cargo feature, TS.2 (this slice) locks the public types + pure
//! line-math. Per-language extraction lands in TS.3 (grammars submodule).
//!
//! Gating: all tree-sitter crate access lives in the `grammars` submodule
//! behind `#[cfg(feature = "treesitter")]`. The contract types and the
//! `diff` line-math helpers compile under default features so callers
//! can be tested without paying the grammar build cost.
//!
//! See DECISION:treesitter-feature-gating, DECISION:treesitter-v1-langs,
//! DECISION:treesitter-capture-convention, DECISION:treesitter-abi-pinning.

use std::ops::Range;

use serde::{Deserialize, Serialize};

#[cfg(feature = "treesitter")]
pub mod grammars;

pub mod diff;

/// V1 supported languages. Exhaustive on purpose — adding a language is a
/// deliberate code change (DECISION:treesitter-v1-langs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
/// TS.2 scope: Function / Method / Class only. The wider capture
/// convention (DECISION:treesitter-capture-convention) covers
/// interface + module too; those land in TS.3 when the per-language
/// extractors decide whether each language emits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SymbolKind {
    Function,
    Method,
    Class,
}

/// A single symbol extracted from source.
///
/// `byte_range` is the half-open span into the source bytes; `line_range`
/// is the half-open 1-based line span. Both use `Range<_>` (not
/// `RangeInclusive`) so they compose with `std::ops::Range` and match
/// tree-sitter's `node.byte_range()` shape.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub byte_range: Range<usize>,
    pub line_range: Range<u32>,
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
