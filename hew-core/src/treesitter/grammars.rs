//! Per-language tree-sitter symbol extraction.
//!
//! Feature-gated. Implements `extract_symbols` over the six V1 languages
//! using a single shared pipeline (parse → query → walk captures →
//! assemble Symbols). Each language ships a hand-trimmed `tags.scm`
//! query that captures only the symbol kinds we surface today.
//!
//! Capture convention follows the tree-sitter org tags.scm style
//! (DECISION:treesitter-capture-convention):
//!
//! - `@definition.function`   → `SymbolKind::Function`
//! - `@definition.method`     → `SymbolKind::Method`
//! - `@definition.class`      → `SymbolKind::Class`
//! - `@definition.interface`  → `SymbolKind::Class`  (collapsed for V1)
//! - `@definition.module`     → `SymbolKind::Class`  (collapsed for V1)
//! - `@name`                  → `Symbol.name` (the identifier text)
//!
//! Dedupe: a node may match multiple patterns (e.g. a function inside an
//! `impl` block matches both the bare `function_item` and the nested
//! `impl_item` patterns). We dedupe by `byte_range`, preferring the more
//! specific kind: Method > Function > Class.

use std::path::Path;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Language as TsLanguage, Parser, Query, QueryCursor};

use super::{Language, Symbol, SymbolKind, TreesitterError};

/// Marker — see `mod.rs::tests::smoke_build_with_feature_compiles`.
pub struct Marker;

/// Detect a language from a file's extension. Filesystem is NOT touched.
///
/// Recognizes: `.rs`, `.py`, `.ts` / `.tsx`, `.js` / `.jsx` / `.mjs` /
/// `.cjs`, `.go`, `.java`. Returns `None` for any unknown extension.
pub fn detect_language(path: &Path) -> Option<Language> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "rs" => Language::Rust,
        "py" | "pyi" => Language::Python,
        "ts" | "tsx" => Language::TypeScript,
        "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
        "go" => Language::Go,
        "java" => Language::Java,
        _ => return None,
    })
}

/// Extract every supported symbol definition from `source`.
///
/// Returns `Ok` with whatever the parser recovered, even on malformed
/// input — tree-sitter is error-tolerant by design and we propagate that.
/// `Err` is reserved for *runtime* failures: an ABI mismatch between the
/// pinned tree-sitter runtime and a grammar crate, or a malformed query
/// (which is a programmer bug, not a user one).
pub fn extract_symbols(source: &str, lang: Language) -> Result<Vec<Symbol>, TreesitterError> {
    let ts_lang = ts_language_for(lang);
    let query_src = query_for(lang);

    let mut parser = Parser::new();
    parser
        .set_language(&ts_lang)
        .map_err(|e| TreesitterError::ParseFailed { message: e.to_string() })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| TreesitterError::ParseFailed { message: "parser returned None".into() })?;

    let query = Query::new(&ts_lang, query_src)
        .map_err(|e| TreesitterError::QueryFailed { message: e.to_string() })?;

    // Resolve capture indices once. Anything missing in the query is just
    // unused — not fatal — so we keep this loose.
    let name_idx = query.capture_index_for_name("name");
    let kind_indices: Vec<(u32, SymbolKind)> =
        ["function", "method", "class", "interface", "module"]
            .iter()
            .filter_map(|tag| {
                let idx = query.capture_index_for_name(&format!("definition.{tag}"))?;
                Some((idx, capture_kind(tag)))
            })
            .collect();

    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(&query, tree.root_node(), source.as_bytes());

    let mut candidates: Vec<Symbol> = Vec::new();
    while let Some(m) = matches.next() {
        // Find the @name capture (if present) and the @definition.* capture.
        let mut name: Option<&str> = None;
        let mut def: Option<(SymbolKind, tree_sitter::Node)> = None;

        for cap in m.captures {
            if Some(cap.index) == name_idx {
                name = cap.node.utf8_text(source.as_bytes()).ok();
            } else if let Some((_, kind)) = kind_indices.iter().find(|(i, _)| *i == cap.index) {
                def = Some((*kind, cap.node));
            }
        }

        let (Some(name), Some((kind, node))) = (name, def) else {
            continue;
        };
        let byte_range = node.byte_range();
        let line_range =
            (node.start_position().row as u32 + 1)..(node.end_position().row as u32 + 2);
        candidates.push(Symbol { name: name.to_string(), kind, byte_range, line_range });
    }

    Ok(dedupe(candidates))
}

/// Dedupe symbols sharing the same `byte_range`, keeping the most
/// specific kind. Order: Method > Function > Class. Preserves first-seen
/// position for stable iteration.
fn dedupe(symbols: Vec<Symbol>) -> Vec<Symbol> {
    let mut out: Vec<Symbol> = Vec::with_capacity(symbols.len());
    for sym in symbols {
        if let Some(existing) = out.iter_mut().find(|s| s.byte_range == sym.byte_range) {
            if kind_rank(sym.kind) > kind_rank(existing.kind) {
                *existing = sym;
            }
        } else {
            out.push(sym);
        }
    }
    out
}

fn kind_rank(k: SymbolKind) -> u8 {
    match k {
        SymbolKind::Method => 3,
        SymbolKind::Function => 2,
        SymbolKind::Class => 1,
    }
}

fn capture_kind(tag: &str) -> SymbolKind {
    match tag {
        "function" => SymbolKind::Function,
        "method" => SymbolKind::Method,
        // class / interface / module collapse to Class for V1.
        _ => SymbolKind::Class,
    }
}

fn ts_language_for(lang: Language) -> TsLanguage {
    match lang {
        Language::Rust => tree_sitter_rust::LANGUAGE.into(),
        Language::Python => tree_sitter_python::LANGUAGE.into(),
        Language::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Language::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Language::Go => tree_sitter_go::LANGUAGE.into(),
        Language::Java => tree_sitter_java::LANGUAGE.into(),
    }
}

fn query_for(lang: Language) -> &'static str {
    match lang {
        Language::Rust => include_str!("queries/rust.scm"),
        Language::Python => include_str!("queries/python.scm"),
        Language::TypeScript => include_str!("queries/typescript.scm"),
        Language::JavaScript => include_str!("queries/javascript.scm"),
        Language::Go => include_str!("queries/go.scm"),
        Language::Java => include_str!("queries/java.scm"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names_of(symbols: &[Symbol]) -> Vec<&str> {
        symbols.iter().map(|s| s.name.as_str()).collect()
    }

    fn kinds_of(symbols: &[Symbol]) -> Vec<SymbolKind> {
        symbols.iter().map(|s| s.kind).collect()
    }

    // --- Rust ---

    #[test]
    fn rust_top_level_fn() {
        let src = "fn add(a: i32, b: i32) -> i32 { a + b }";
        let syms = extract_symbols(src, Language::Rust).unwrap();
        assert_eq!(names_of(&syms), vec!["add"]);
        assert_eq!(kinds_of(&syms), vec![SymbolKind::Function]);
    }

    #[test]
    fn rust_impl_method() {
        let src = "struct S; impl S { fn hello(&self) {} }";
        let syms = extract_symbols(src, Language::Rust).unwrap();
        let methods: Vec<_> =
            syms.iter().filter(|s| s.kind == SymbolKind::Method).map(|s| s.name.as_str()).collect();
        assert_eq!(methods, vec!["hello"]);
    }

    #[test]
    fn rust_struct_with_methods() {
        let src = "struct Foo { x: i32 } impl Foo { fn a(&self) {} fn b(&self) {} }";
        let syms = extract_symbols(src, Language::Rust).unwrap();
        let names: Vec<_> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Foo"));
        assert!(names.contains(&"a"));
        assert!(names.contains(&"b"));
        let foo = syms.iter().find(|s| s.name == "Foo").unwrap();
        assert_eq!(foo.kind, SymbolKind::Class);
        let a = syms.iter().find(|s| s.name == "a").unwrap();
        assert_eq!(a.kind, SymbolKind::Method);
    }

    // --- Python ---

    #[test]
    fn python_top_level_def() {
        let src = "def greet(name):\n    return name\n";
        let syms = extract_symbols(src, Language::Python).unwrap();
        assert_eq!(names_of(&syms), vec!["greet"]);
        assert_eq!(kinds_of(&syms), vec![SymbolKind::Function]);
    }

    #[test]
    fn python_class_method() {
        let src = "class Foo:\n    def bar(self):\n        return 1\n";
        let syms = extract_symbols(src, Language::Python).unwrap();
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
    }

    #[test]
    fn python_top_level_class() {
        let src = "class Widget:\n    pass\n";
        let syms = extract_symbols(src, Language::Python).unwrap();
        let widget = syms.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);
    }

    // --- TypeScript ---

    #[test]
    fn typescript_top_level_function() {
        let src = "function add(a: number, b: number): number { return a + b; }";
        let syms = extract_symbols(src, Language::TypeScript).unwrap();
        assert_eq!(names_of(&syms), vec!["add"]);
        assert_eq!(kinds_of(&syms), vec![SymbolKind::Function]);
    }

    #[test]
    fn typescript_class_method() {
        let src = "class Foo { bar(): void {} }";
        let syms = extract_symbols(src, Language::TypeScript).unwrap();
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
    }

    #[test]
    fn typescript_top_level_class() {
        let src = "class Widget { x: number = 0; }";
        let syms = extract_symbols(src, Language::TypeScript).unwrap();
        let widget = syms.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);
    }

    // --- JavaScript ---

    #[test]
    fn javascript_top_level_function() {
        let src = "function add(a, b) { return a + b; }";
        let syms = extract_symbols(src, Language::JavaScript).unwrap();
        assert_eq!(names_of(&syms), vec!["add"]);
        assert_eq!(kinds_of(&syms), vec![SymbolKind::Function]);
    }

    #[test]
    fn javascript_class_method() {
        let src = "class Foo { bar() { return 1; } }";
        let syms = extract_symbols(src, Language::JavaScript).unwrap();
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
    }

    #[test]
    fn javascript_top_level_class() {
        let src = "class Widget { constructor() {} }";
        let syms = extract_symbols(src, Language::JavaScript).unwrap();
        let widget = syms.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);
    }

    // --- Go ---

    #[test]
    fn go_top_level_func() {
        let src = "package main\nfunc Add(a, b int) int { return a + b }\n";
        let syms = extract_symbols(src, Language::Go).unwrap();
        let add = syms.iter().find(|s| s.name == "Add").unwrap();
        assert_eq!(add.kind, SymbolKind::Function);
    }

    #[test]
    fn go_method_on_struct() {
        let src = "package main\ntype T struct{}\nfunc (t T) Hello() {}\n";
        let syms = extract_symbols(src, Language::Go).unwrap();
        let hello = syms.iter().find(|s| s.name == "Hello").unwrap();
        assert_eq!(hello.kind, SymbolKind::Method);
    }

    #[test]
    fn go_struct_decl_as_class() {
        let src = "package main\ntype Widget struct { X int }\n";
        let syms = extract_symbols(src, Language::Go).unwrap();
        let widget = syms.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);
    }

    // --- Java ---

    #[test]
    fn java_top_level_method_static() {
        let src = "class Main { public static void main(String[] args) {} }";
        let syms = extract_symbols(src, Language::Java).unwrap();
        let main = syms.iter().find(|s| s.name == "main").unwrap();
        assert_eq!(main.kind, SymbolKind::Method);
    }

    #[test]
    fn java_instance_method() {
        let src = "class Foo { void bar() {} }";
        let syms = extract_symbols(src, Language::Java).unwrap();
        let bar = syms.iter().find(|s| s.name == "bar").unwrap();
        assert_eq!(bar.kind, SymbolKind::Method);
    }

    #[test]
    fn java_top_level_class() {
        let src = "class Widget {}";
        let syms = extract_symbols(src, Language::Java).unwrap();
        let widget = syms.iter().find(|s| s.name == "Widget").unwrap();
        assert_eq!(widget.kind, SymbolKind::Class);
    }

    // --- Error tolerance: malformed source returns Ok with partial results ---

    #[test]
    fn rust_malformed_returns_ok() {
        // Unbalanced brace — tree-sitter recovers what it can.
        let src = "fn ok() {}\nfn broken( {\n";
        let got = extract_symbols(src, Language::Rust);
        assert!(got.is_ok(), "malformed source must not error");
    }

    #[test]
    fn python_malformed_returns_ok() {
        let src = "def ok():\n    return 1\ndef broken(:\n";
        let got = extract_symbols(src, Language::Python);
        assert!(got.is_ok());
    }

    #[test]
    fn typescript_malformed_returns_ok() {
        let src = "function ok() {}\nfunction broken( {\n";
        let got = extract_symbols(src, Language::TypeScript);
        assert!(got.is_ok());
    }

    #[test]
    fn javascript_malformed_returns_ok() {
        let src = "function ok() {}\nfunction broken( {\n";
        let got = extract_symbols(src, Language::JavaScript);
        assert!(got.is_ok());
    }

    #[test]
    fn go_malformed_returns_ok() {
        let src = "package main\nfunc ok() {}\nfunc broken( {\n";
        let got = extract_symbols(src, Language::Go);
        assert!(got.is_ok());
    }

    #[test]
    fn java_malformed_returns_ok() {
        let src = "class Foo { void ok() {} void broken( { }";
        let got = extract_symbols(src, Language::Java);
        assert!(got.is_ok());
    }

    // --- detect_language: one happy path per extension family + unknown ---

    #[test]
    fn detect_language_rust_ext() {
        assert_eq!(detect_language(Path::new("a/b.rs")), Some(Language::Rust));
    }

    #[test]
    fn detect_language_python_ext() {
        assert_eq!(detect_language(Path::new("x.py")), Some(Language::Python));
        assert_eq!(detect_language(Path::new("stub.pyi")), Some(Language::Python));
    }

    #[test]
    fn detect_language_typescript_ext() {
        assert_eq!(detect_language(Path::new("x.ts")), Some(Language::TypeScript));
        assert_eq!(detect_language(Path::new("c.tsx")), Some(Language::TypeScript));
    }

    #[test]
    fn detect_language_javascript_ext() {
        assert_eq!(detect_language(Path::new("a.js")), Some(Language::JavaScript));
        assert_eq!(detect_language(Path::new("a.jsx")), Some(Language::JavaScript));
        assert_eq!(detect_language(Path::new("a.mjs")), Some(Language::JavaScript));
        assert_eq!(detect_language(Path::new("a.cjs")), Some(Language::JavaScript));
    }

    #[test]
    fn detect_language_go_and_java() {
        assert_eq!(detect_language(Path::new("x.go")), Some(Language::Go));
        assert_eq!(detect_language(Path::new("X.java")), Some(Language::Java));
    }

    #[test]
    fn detect_language_unknown_returns_none() {
        assert_eq!(detect_language(Path::new("README.md")), None);
        assert_eq!(detect_language(Path::new("noext")), None);
    }
}
