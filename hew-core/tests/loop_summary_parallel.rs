//! Fixture-driven render tests for the parallel `hew loop summary`
//! per-worker breakdown. Reads `tests/fixtures/parallel-run-2workers/`
//! end-to-end: manifest.json + worker-<n>/iter-*.json.
//!
//! Task: hew-h0tu.

use std::path::PathBuf;

use hew_core::loop_log::{IterLog, Manifest};
use hew_core::loop_summary::{WorkerSlice, render_parallel_breakdown, worker_slice};

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parallel-run-2workers")
}

fn read_manifest() -> Manifest {
    let body = std::fs::read_to_string(fixture_root().join("manifest.json")).unwrap();
    serde_json::from_str(&body).unwrap()
}

fn read_worker_iter_logs(n: u32) -> Vec<IterLog> {
    let dir = fixture_root().join(format!("worker-{n}"));
    let mut out: Vec<IterLog> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .filter_map(|p| std::fs::read_to_string(&p).ok())
        .filter_map(|body| serde_json::from_str::<IterLog>(&body).ok())
        .collect();
    out.sort_by_key(|l| l.number);
    out
}

#[test]
fn summary_renders_per_worker_table_for_parallel_run() {
    let manifest = read_manifest();
    assert_eq!(manifest.workers.len(), 2);

    let slices: Vec<WorkerSlice> = manifest
        .workers
        .iter()
        .map(|row| worker_slice(row, &read_worker_iter_logs(row.id)))
        .collect();

    // Per-worker counts. Worker 0: 3 iters, 3 closed, runtime=claude, 12_000 tokens.
    assert_eq!(slices[0].worker_n, 0);
    assert_eq!(slices[0].iter_count, 3);
    assert_eq!(slices[0].tasks_closed, 3);
    assert_eq!(slices[0].runtime_used.as_deref(), Some("claude"));
    assert_eq!(slices[0].total_tokens, 12_000);
    // Worker 1: 2 iters, 1 closed (one no_close), runtime=codex, 8_000 tokens.
    assert_eq!(slices[1].worker_n, 1);
    assert_eq!(slices[1].iter_count, 2);
    assert_eq!(slices[1].tasks_closed, 1);
    assert_eq!(slices[1].runtime_used.as_deref(), Some("codex"));
    assert_eq!(slices[1].total_tokens, 8_000);

    let rendered = render_parallel_breakdown(&slices, false);
    assert!(rendered.contains("per-worker"), "missing section header:\n{rendered}");
    // Row header columns appear.
    for col in ["wkr", "iters", "closed", "runtime", "tokens", "stop"] {
        assert!(rendered.contains(col), "missing column header `{col}`:\n{rendered}");
    }
    // Both runtimes show up in the rows.
    assert!(rendered.contains("claude"));
    assert!(rendered.contains("codex"));
    // Totals row sums iters (3+2=5) and tokens (12k + 8k = 20k).
    assert!(rendered.contains("all"), "expected totals row:\n{rendered}");
    assert!(rendered.contains("20,000"), "expected total tokens 20,000:\n{rendered}");
    // Totals iter count.
    assert!(rendered.lines().any(|l| l.trim_start().starts_with("all") && l.contains(" 5 ")));
}

#[test]
fn summary_falls_back_to_serial_view_when_no_manifest() {
    // No manifest → render_parallel_breakdown on an empty slice list
    // returns the empty string. Callers (the CLI) gate on this to skip
    // the per-worker section entirely; the regular `render` block then
    // stays the only output, identical to the pre-parallel path.
    let rendered = render_parallel_breakdown(&[], false);
    assert!(rendered.is_empty());
}
