//! `hew gate` — create and resolve external-state gates.
//!
//! A gate is a bd task whose closure waits on an external condition
//! (currently: a GitHub PR being merged). This module is the CLI half;
//! the typed spec, JSON serialization, and outcome classification live
//! in [`hew_core::external_gate`].

use std::ffi::{OsStr, OsString};

use clap::{Args as ClapArgs, Subcommand};
use hew_core::Ctx;
use hew_core::bd::{BdClient, RealBd};
use hew_core::external_gate::{
    GATE_LABEL, GateKind, GateSpec, PollOutcome, classify_gh_pr_view_json,
};

#[derive(Debug, ClapArgs)]
pub struct Args {
    #[command(subcommand)]
    pub op: Op,
}

#[derive(Debug, Subcommand)]
pub enum Op {
    /// Create a gate task that waits on an external condition.
    New(NewArgs),
    /// Poll open gate tasks; close those whose external state has resolved.
    Poll(PollArgs),
    /// List currently-open gate tasks.
    List,
}

#[derive(Debug, ClapArgs)]
pub struct NewArgs {
    /// Wait on this GitHub PR being merged. Resolved when
    /// `gh pr view <N> --json state` reports `state = MERGED`.
    #[arg(long, value_name = "N")]
    pub gh_pr: u64,

    /// Title for the gate task (the bd task title).
    #[arg(long)]
    pub title: String,
}

#[derive(Debug, ClapArgs)]
pub struct PollArgs {
    /// Specific gate task id (e.g. `hew-abcd`). If omitted, polls
    /// every open task labelled `hew-gate`.
    pub id: Option<String>,
}

pub fn run(ctx: &Ctx, args: Args) -> miette::Result<()> {
    let bd = RealBd::discover().map_err(|e| miette::miette!("bd discover: {e}"))?;
    match args.op {
        Op::New(a) => new_gate(ctx, &bd, a),
        Op::Poll(a) => poll_gates(ctx, &bd, a),
        Op::List => list_gates(ctx, &bd),
    }
}

fn new_gate(ctx: &Ctx, bd: &impl BdClient, args: NewArgs) -> miette::Result<()> {
    let spec = GateSpec { kind: GateKind::GhPr { id: args.gh_pr } };
    let metadata = spec.to_metadata_json();

    let argv: Vec<OsString> = vec![
        "create".into(),
        "--type=task".into(),
        OsString::from(format!("--labels={GATE_LABEL}")),
        "--metadata".into(),
        metadata.into(),
        "--json".into(),
        (&args.title).into(),
    ];
    let refs: Vec<&OsStr> = argv.iter().map(|s| s.as_os_str()).collect();
    let out = bd.run_raw(&refs).map_err(|e| miette::miette!("bd create: {e}"))?;
    let id = parse_created_id_json(&out.stdout).ok_or_else(|| {
        miette::miette!("could not parse new gate id from bd --json output:\n{}", out.stdout)
    })?;

    if ctx.quiet {
        println!("{id}");
    } else {
        println!("created gate {id} — waits on {}", spec.kind.short_label());
        println!("  next: hew dep add <next-epic> {id}");
    }
    Ok(())
}

fn poll_gates(ctx: &Ctx, bd: &impl BdClient, args: PollArgs) -> miette::Result<()> {
    let targets: Vec<String> = match args.id {
        Some(id) => vec![id],
        None => list_open_gate_ids(bd)?,
    };

    if targets.is_empty() {
        if !ctx.quiet {
            println!("no open gate tasks");
        }
        return Ok(());
    }

    let mut closed = 0usize;
    for id in &targets {
        let spec = match read_gate_spec(bd, id) {
            Ok(s) => s,
            Err(e) => {
                if !ctx.quiet {
                    eprintln!("  {id}  ✗ {e}");
                }
                continue;
            }
        };
        let outcome = poll_one(&spec)?;
        match outcome {
            PollOutcome::Resolved { reason } => {
                close_gate(bd, id, &reason)?;
                closed += 1;
                if !ctx.quiet {
                    println!("  {id}  {} → ✓ {reason}", spec.kind.short_label());
                }
            }
            PollOutcome::StillOpen { detail } => {
                if !ctx.quiet {
                    println!("  {id}  {} → {detail}", spec.kind.short_label());
                }
            }
            PollOutcome::Indeterminate { detail } => {
                if !ctx.quiet {
                    eprintln!("  {id}  {} → ? {detail}", spec.kind.short_label());
                }
            }
        }
    }
    if !ctx.quiet {
        println!("polled {} gate{} — {closed} closed", targets.len(), plural(targets.len()));
    }
    Ok(())
}

fn list_gates(ctx: &Ctx, bd: &impl BdClient) -> miette::Result<()> {
    let argv = [
        OsStr::new("list"),
        OsStr::new("--label"),
        OsStr::new(GATE_LABEL),
        OsStr::new("--status=open"),
        OsStr::new("--flat"),
        OsStr::new("--no-pager"),
    ];
    let out = bd.run_raw(&argv).map_err(|e| miette::miette!("bd list: {e}"))?;
    if out.stdout.trim().is_empty() {
        if !ctx.quiet {
            println!("no open gate tasks");
        }
    } else {
        print!("{}", out.stdout);
    }
    Ok(())
}

/// Extract `.id` from bd's `--json` output for `create`. bd emits a
/// single JSON object whose `id` field is the canonical task id
/// (e.g. `hew-y4sf`). Returns `None` if the payload is malformed or
/// `id` is missing — the caller surfaces the raw stdout in the error
/// so the operator can see what bd actually produced.
fn parse_created_id_json(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout).ok()?;
    v.get("id").and_then(|x| x.as_str()).map(|s| s.to_string())
}

fn list_open_gate_ids(bd: &impl BdClient) -> miette::Result<Vec<String>> {
    let argv = [
        OsStr::new("list"),
        OsStr::new("--label"),
        OsStr::new(GATE_LABEL),
        OsStr::new("--status=open"),
        OsStr::new("--json"),
        OsStr::new("--no-pager"),
    ];
    let out = bd.run_raw(&argv).map_err(|e| miette::miette!("bd list: {e}"))?;
    parse_listed_ids_json(&out.stdout)
        .ok_or_else(|| miette::miette!("could not parse bd list --json output:\n{}", out.stdout))
}

/// Pull `id` out of every element in bd's `list --json` array. Accepts
/// either a top-level array or an object with an `issues` field (bd's
/// list format has varied across versions — be permissive on read).
fn parse_listed_ids_json(stdout: &str) -> Option<Vec<String>> {
    let v: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let array = v.as_array().or_else(|| v.get("issues").and_then(|x| x.as_array()))?;
    Some(array.iter().filter_map(|item| item.get("id")?.as_str().map(|s| s.to_string())).collect())
}

fn read_gate_spec(bd: &impl BdClient, id: &str) -> miette::Result<GateSpec> {
    // `bd show <id> --json` carries the metadata blob; parse our wrapper out of it.
    let argv = [OsStr::new("show"), OsStr::new(id), OsStr::new("--json")];
    let out = bd.run_raw(&argv).map_err(|e| miette::miette!("bd show {id}: {e}"))?;
    let metadata = extract_metadata_from_show_json(&out.stdout)
        .ok_or_else(|| miette::miette!("task {id} has no metadata block in bd show output"))?;
    GateSpec::from_metadata_json(&metadata)
        .map_err(|e| miette::miette!("task {id} metadata is not a hew_gate spec: {e}"))
}

/// Pull the metadata blob out of bd's `show --json` payload. bd 1.0.3 wraps
/// the issue in a one-element array; older versions returned a bare object.
/// Be permissive on read so both shapes work.
fn extract_metadata_from_show_json(stdout: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(stdout).ok()?;
    let issue = match &v {
        serde_json::Value::Array(arr) => arr.first()?,
        serde_json::Value::Object(_) => &v,
        _ => return None,
    };
    let metadata = issue.get("metadata")?;
    serde_json::to_string(metadata).ok()
}

fn poll_one(spec: &GateSpec) -> miette::Result<PollOutcome> {
    match &spec.kind {
        GateKind::GhPr { id } => {
            let out = std::process::Command::new("gh")
                .args(["pr", "view", &id.to_string(), "--json", "state,mergedAt"])
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| miette::miette!("spawn gh: {e}"))?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                return Ok(PollOutcome::Indeterminate {
                    detail: format!(
                        "gh pr view {id} exited {:?}: {}",
                        out.status.code(),
                        stderr.trim()
                    ),
                });
            }
            let raw = String::from_utf8_lossy(&out.stdout);
            Ok(classify_gh_pr_view_json(&raw))
        }
    }
}

fn close_gate(bd: &impl BdClient, id: &str, reason: &str) -> miette::Result<()> {
    // Keep this in sync with `hew task close --reason ...`: bd's `close` accepts
    // `--reason` and flips status to closed in one shot.
    let argv: Vec<OsString> = vec!["close".into(), id.into(), "--reason".into(), reason.into()];
    let refs: Vec<&OsStr> = argv.iter().map(|s| s.as_os_str()).collect();
    bd.run_raw(&refs).map_err(|e| miette::miette!("bd close {id}: {e}"))?;
    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_id_from_bd_create_json() {
        // Verbatim shape from `bd create --type=task --json` (bd 1.0.3).
        let stdout = r#"{
          "id": "test-ify",
          "issue_type": "task",
          "metadata": {"hew_gate": {"id": 49, "kind": "gh:pr"}},
          "priority": 2,
          "status": "open",
          "title": "smoke3"
        }"#;
        assert_eq!(parse_created_id_json(stdout), Some("test-ify".into()));
    }

    #[test]
    fn parses_id_from_real_hew_format() {
        let stdout = r#"{"id":"hew-y4sf","title":"gate primitive"}"#;
        assert_eq!(parse_created_id_json(stdout), Some("hew-y4sf".into()));
    }

    #[test]
    fn parse_returns_none_on_unrelated_output() {
        assert_eq!(parse_created_id_json("not json"), None);
        assert_eq!(parse_created_id_json(""), None);
        assert_eq!(parse_created_id_json(r#"{"no_id": "here"}"#), None);
    }

    #[test]
    fn list_ids_parses_top_level_array() {
        let stdout = r#"[{"id":"hew-a"},{"id":"hew-b"},{"id":"hew-c"}]"#;
        assert_eq!(
            parse_listed_ids_json(stdout),
            Some(vec!["hew-a".into(), "hew-b".into(), "hew-c".into()])
        );
    }

    #[test]
    fn list_ids_parses_issues_envelope() {
        // Older bd versions wrapped the list in {"issues": [...]}.
        let stdout = r#"{"issues":[{"id":"hew-a"},{"id":"hew-b"}]}"#;
        assert_eq!(parse_listed_ids_json(stdout), Some(vec!["hew-a".into(), "hew-b".into()]));
    }

    #[test]
    fn list_ids_handles_empty() {
        assert_eq!(parse_listed_ids_json("[]"), Some(vec![]));
    }

    #[test]
    fn extracts_metadata_from_bd_show_array_form() {
        // Real bd 1.0.3 shape: `bd show <id> --json` returns [{...}].
        let stdout = r#"[{
            "id": "hew-4fp8",
            "metadata": {"hew_gate": {"kind": {"kind": "gh:pr", "id": 49}}}
        }]"#;
        let md = extract_metadata_from_show_json(stdout).expect("metadata present");
        assert!(md.contains("hew_gate"));
        // Round-trip into a GateSpec so the test fails if bd ever changes the shape
        // we expect under the hood.
        let spec = GateSpec::from_metadata_json(&md).expect("spec parse");
        assert_eq!(spec.kind, GateKind::GhPr { id: 49 });
    }

    #[test]
    fn extracts_metadata_from_bd_show_bare_object_form() {
        // Older / alternate bd output: a single object, not wrapped in array.
        let stdout = r#"{"id":"hew-x","metadata":{"hew_gate":{"kind":{"kind":"gh:pr","id":7}}}}"#;
        let md = extract_metadata_from_show_json(stdout).expect("metadata present");
        let spec = GateSpec::from_metadata_json(&md).expect("spec parse");
        assert_eq!(spec.kind, GateKind::GhPr { id: 7 });
    }

    #[test]
    fn extract_metadata_returns_none_when_absent() {
        assert!(extract_metadata_from_show_json(r#"[{"id":"hew-x"}]"#).is_none());
        assert!(extract_metadata_from_show_json("not json").is_none());
        assert!(extract_metadata_from_show_json("[]").is_none());
    }
}
