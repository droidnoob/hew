use hew_core::Ctx;
use hew_core::bd::RealBd;
use hew_core::status;

pub fn run(ctx: &Ctx, _: ()) -> miette::Result<()> {
    let client = RealBd::discover()?;
    let report = status::build(&client)?;

    if matches!(ctx.output, hew_core::OutputMode::Json) {
        let payload = serde_json::json!({
            "bd_version": report.bd_version,
            "tasks": {
                "total": report.tasks_total,
                "done": report.tasks_done,
                "in_progress": report.tasks_in_progress,
                "ready": report.tasks_ready,
                "blocked": report.tasks_blocked,
                "ready_list": report.ready_titles,
            },
            "phases": report.phases.iter().map(|p| serde_json::json!({
                "name": p.name, "complete": p.complete, "timestamp": p.timestamp,
            })).collect::<Vec<_>>(),
            "memories": {
                "conventions": report.memory_counts.conventions,
                "boundaries": report.memory_counts.boundaries,
                "audit": report.memory_counts.audit,
                "security": report.memory_counts.security,
                "migration": report.memory_counts.migration,
                "dep": report.memory_counts.dep,
                "factual": report.memory_counts.factual,
            },
            "conventions": report.conventions,
        });
        println!("{}", serde_json::to_string_pretty(&payload).unwrap());
    } else {
        print!("{}", status::render_text(&report));
    }
    Ok(())
}
