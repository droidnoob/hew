//! Render the project state as human-readable text.
//!
//! Reuses `prime::build` (with a synthetic skill of "plan", which has
//! no prerequisites) to gather state, then formats it.

use std::fmt::Write;

use crate::bd::BdClient;
use crate::error::Result;
use crate::prime;

pub struct StatusReport {
    pub bd_version: Option<String>,
    pub tasks_total: u64,
    pub tasks_done: u64,
    pub tasks_in_progress: u64,
    pub tasks_ready: u64,
    pub tasks_blocked: u64,
    pub ready_titles: Vec<String>,
    pub phases: Vec<PhaseLine>,
    pub memory_counts: MemoryCounts,
    pub conventions: Vec<String>,
}

pub struct PhaseLine {
    pub name: String,
    pub complete: bool,
    pub timestamp: Option<String>,
}

#[derive(Default)]
pub struct MemoryCounts {
    pub conventions: usize,
    pub boundaries: usize,
    pub audit: usize,
    pub security: usize,
    pub migration: usize,
    pub dep: usize,
    pub factual: usize,
}

const KNOWN_PHASES: &[&str] =
    &["scan", "convention", "audit", "boundary", "plan", "decompose", "verify"];

pub fn build(client: &dyn BdClient) -> Result<StatusReport> {
    // `plan` has no prerequisites — guaranteed to succeed.
    let out = prime::build(client, "plan")?;

    let phases = KNOWN_PHASES
        .iter()
        .map(|name| match out.status.get(*name) {
            Some(entry) => PhaseLine {
                name: (*name).to_string(),
                complete: entry.complete,
                timestamp: entry.timestamp.clone(),
            },
            None => PhaseLine { name: (*name).to_string(), complete: false, timestamp: None },
        })
        .collect();

    let memory_counts = MemoryCounts {
        conventions: out.memories.conventions.len(),
        boundaries: out.memories.boundaries.len(),
        audit: out.memories.audit.len(),
        security: out.memories.security.len(),
        migration: out.memories.migration.len(),
        dep: out.memories.dep.len(),
        factual: out.memories.factual.len(),
    };

    // Conventions can be long; reduce to short labels (after the `CONVENTION:` prefix, before the `—`).
    let conventions = out
        .memories
        .conventions
        .iter()
        .filter_map(|line| {
            let after_prefix = line.trim().strip_prefix("CONVENTION:")?;
            let label = after_prefix.split('—').next()?.trim();
            if label.is_empty() { None } else { Some(label.to_string()) }
        })
        .collect();

    Ok(StatusReport {
        bd_version: out.project.bd_version,
        tasks_total: out.tasks.total,
        tasks_done: out.tasks.done,
        tasks_in_progress: out.tasks.in_progress,
        tasks_ready: out.tasks.ready,
        tasks_blocked: out.tasks.blocked,
        ready_titles: out
            .tasks
            .ready_list
            .into_iter()
            .map(|t| format!("[P{}] {} {}", t.priority, t.id, t.title))
            .collect(),
        phases,
        memory_counts,
        conventions,
    })
}

pub fn render_text(r: &StatusReport) -> String {
    let mut s = String::new();
    let _ = writeln!(s, "hew status");
    let _ = writeln!(s, "──────────────────────────────────");
    if let Some(ref v) = r.bd_version {
        let _ = writeln!(s, "  bd:        v{v}");
    } else {
        let _ = writeln!(s, "  bd:        not detected");
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Phases");
    let _ = writeln!(s, "──────────────────────────────────");
    for p in &r.phases {
        let mark = if p.complete { "✓" } else { "○" };
        let ts = p.timestamp.as_deref().unwrap_or("");
        let _ = writeln!(s, "  {mark} {:<10}  {ts}", p.name);
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Tasks");
    let _ = writeln!(s, "──────────────────────────────────");
    let _ = writeln!(
        s,
        "  {} total │ {} done │ {} in progress │ {} ready │ {} blocked",
        r.tasks_total, r.tasks_done, r.tasks_in_progress, r.tasks_ready, r.tasks_blocked,
    );
    if !r.ready_titles.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "  Next up:");
        for line in r.ready_titles.iter().take(5) {
            let _ = writeln!(s, "    • {line}");
        }
    }
    let _ = writeln!(s);

    let _ = writeln!(s, "Memories");
    let _ = writeln!(s, "──────────────────────────────────");
    let _ = writeln!(
        s,
        "  {} CONVENTION │ {} BOUNDARY │ {} AUDIT │ {} SECURITY │ {} MIGRATION │ {} DEP │ {} factual",
        r.memory_counts.conventions,
        r.memory_counts.boundaries,
        r.memory_counts.audit,
        r.memory_counts.security,
        r.memory_counts.migration,
        r.memory_counts.dep,
        r.memory_counts.factual,
    );

    if !r.conventions.is_empty() {
        let _ = writeln!(s);
        let _ = writeln!(s, "  Conventions: {}", r.conventions.join(", "));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_report() -> StatusReport {
        StatusReport {
            bd_version: Some("1.0.0".into()),
            tasks_total: 0,
            tasks_done: 0,
            tasks_in_progress: 0,
            tasks_ready: 0,
            tasks_blocked: 0,
            ready_titles: vec![],
            phases: KNOWN_PHASES
                .iter()
                .map(|n| PhaseLine { name: (*n).into(), complete: false, timestamp: None })
                .collect(),
            memory_counts: MemoryCounts::default(),
            conventions: vec![],
        }
    }

    #[test]
    fn render_includes_required_sections() {
        let r = empty_report();
        let text = render_text(&r);
        assert!(text.contains("hew status"));
        assert!(text.contains("Phases"));
        assert!(text.contains("Tasks"));
        assert!(text.contains("Memories"));
        assert!(text.contains("bd:        v1.0.0"));
    }

    #[test]
    fn render_marks_complete_phases() {
        let mut r = empty_report();
        r.phases[0] = PhaseLine {
            name: "scan".into(),
            complete: true,
            timestamp: Some("2026-05-11T14:30:00".into()),
        };
        let text = render_text(&r);
        assert!(text.contains("✓ scan"));
        assert!(text.contains("2026-05-11T14:30:00"));
    }

    #[test]
    fn ready_titles_truncate_to_five() {
        let mut r = empty_report();
        for i in 0..10 {
            r.ready_titles.push(format!("[P1] id-{i} task"));
        }
        let text = render_text(&r);
        assert!(text.contains("id-0"));
        assert!(text.contains("id-4"));
        assert!(!text.contains("id-5"));
    }
}
