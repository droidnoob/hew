//! End-to-end check that the on-disk skill tree matches the registry.
//!
//! Reads the same files that `include_str!` pulls in via the workspace
//! manifest, then asserts every file is registered and vice versa. This
//! catches "new skill file added but forgotten in registry" drift.

use std::collections::BTreeSet;
use std::path::PathBuf;

use hew_core::skills;

fn skills_dir() -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    PathBuf::from(manifest).join("..").join("skills")
}

fn collect_md(dir: &std::path::Path, into: &mut BTreeSet<String>, prefix: &str) {
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("entry");
        let ft = entry.file_type().expect("file_type");
        let name = entry.file_name().to_string_lossy().to_string();
        if ft.is_dir() {
            // `skills/data/` holds embedded resource files (prompts,
            // TOML catalogs) — not skill bodies. They ship via
            // `include_str!` from the consuming module and aren't
            // registered in `skills::CORE/BROWNFIELD/OPTIONAL`.
            if prefix.is_empty() && name == "data" {
                continue;
            }
            let nested_prefix =
                if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            collect_md(&entry.path(), into, &nested_prefix);
        } else if name.ends_with(".md") {
            let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
            into.insert(rel);
        }
    }
}

#[test]
fn every_disk_skill_is_registered() {
    let mut disk = BTreeSet::new();
    collect_md(&skills_dir(), &mut disk, "");

    let registered: BTreeSet<String> =
        skills::all().into_iter().map(|s| s.relative_path.to_string()).collect();

    let missing: Vec<_> = disk.difference(&registered).collect();
    assert!(missing.is_empty(), "skill files on disk but not in registry: {missing:?}");

    let stale: Vec<_> = registered.difference(&disk).collect();
    assert!(stale.is_empty(), "registry references missing files: {stale:?}");
}

#[test]
fn version_markers_match_pkg_version() {
    let expected = env!("CARGO_PKG_VERSION");
    for s in skills::all() {
        let v = s.version().unwrap_or("MISSING");
        assert_eq!(v, expected, "{} has version {v}, expected {expected}", s.name);
    }
}
