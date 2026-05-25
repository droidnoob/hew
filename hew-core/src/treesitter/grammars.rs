//! Per-language tree-sitter grammar loaders (skeleton — TS.1).
//!
//! Feature-gated. Real loader implementations land in TS.3. This file
//! exists today so the feature wiring is exercised end-to-end: enabling
//! `--features treesitter` must compile a path that touches every grammar
//! crate.

use super::{Language, TreesitterError};

/// Marker type — referenced by the wiring smoke test in `mod.rs` so that a
/// broken cfg gate fails the build instead of silently dropping the module.
#[allow(dead_code)]
pub struct Marker;

/// Load the tree-sitter `Language` handle for `lang`.
///
/// Each arm forces the corresponding grammar crate to be linked when the
/// `treesitter` feature is on. TS.3 will replace the `todo!()` bodies with
/// the actual `tree_sitter::Language::new(...)` calls plus the
/// per-language `tags.scm` query loading.
#[allow(dead_code)]
pub fn load(lang: Language) -> Result<tree_sitter::Language, TreesitterError> {
    match lang {
        Language::Rust => {
            let _ = tree_sitter_rust::LANGUAGE;
            todo!("TS.3: wire tree_sitter_rust::LANGUAGE")
        }
        Language::Python => {
            let _ = tree_sitter_python::LANGUAGE;
            todo!("TS.3: wire tree_sitter_python::LANGUAGE")
        }
        Language::TypeScript => {
            let _ = tree_sitter_typescript::LANGUAGE_TYPESCRIPT;
            todo!("TS.3: wire tree_sitter_typescript::LANGUAGE_TYPESCRIPT")
        }
        Language::JavaScript => {
            let _ = tree_sitter_javascript::LANGUAGE;
            todo!("TS.3: wire tree_sitter_javascript::LANGUAGE")
        }
        Language::Go => {
            let _ = tree_sitter_go::LANGUAGE;
            todo!("TS.3: wire tree_sitter_go::LANGUAGE")
        }
        Language::Java => {
            let _ = tree_sitter_java::LANGUAGE;
            todo!("TS.3: wire tree_sitter_java::LANGUAGE")
        }
    }
}
