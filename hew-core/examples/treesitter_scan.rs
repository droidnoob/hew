//! Tree-sitter smoke runner.
//!
//! Walks each path given on argv, detects the language by extension,
//! runs `extract_symbols`, and prints the recovered symbols. Built for
//! eyeballing real repo files against the queries.
//!
//! Usage:
//!   cargo run -p hew-core --features treesitter --example treesitter_scan -- <path>...

use std::path::Path;

use hew_core::treesitter::{detect_language, extract_symbols};

fn main() {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.is_empty() {
        eprintln!("usage: treesitter_scan <file>...");
        std::process::exit(2);
    }

    let mut total = 0usize;
    let mut skipped = 0usize;
    for arg in &paths {
        let path = Path::new(arg);
        let Some(lang) = detect_language(path) else {
            eprintln!("skip: {arg} (unknown extension)");
            skipped += 1;
            continue;
        };
        let source = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("read failed: {arg}: {e}");
                continue;
            }
        };
        match extract_symbols(&source, lang) {
            Ok(syms) => {
                println!("\n== {arg} ({lang:?}) — {n} symbols", n = syms.len());
                for s in &syms {
                    println!(
                        "  {kind:<9} {name:<32} lines {a}-{b}",
                        kind = format!("{:?}", s.kind),
                        name = s.name,
                        a = s.line_range.start,
                        b = s.line_range.end - 1,
                    );
                }
                total += syms.len();
            }
            Err(e) => eprintln!("extract failed: {arg}: {e}"),
        }
    }
    eprintln!(
        "\n-- {n} files scanned, {total} symbols, {skipped} skipped",
        n = paths.len() - skipped
    );
}
