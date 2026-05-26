<!-- hew:version=0.9.0 -->
---
name: hew-compact
category: optional
init: hew prime compact
---

# hew-compact — Memory Compaction

A project's memory store grows over time. `CONVENTION:` rules
accumulate from `hew-convention`, `RESEARCH:` findings from
`hew-research`, factual snippets from every executor cycle. Eventually
the prefix gets noisy — 28 CONVENTION memories where 6 would do, 70
factual entries that span three abandoned investigations.

`hew-compact` reduces a noisy prefix from N entries to 1–2 canonical
entries per logical sub-cluster, preserving the prescriptive shape
the executor relies on. Clustering happens here (in-context, by you);
the apply path is in `hew_core::compact::apply`.

## When this skill runs

- The user invokes `/hew:compact <PREFIX>` directly.
- `/hew:do` routes a "consolidate / compact / clean up memories"
  ask here.
- The user explicitly asks to "tidy" or "merge" a specific prefix.

Don't fire speculatively. Compaction is lossy — only run when the
user asked. The `compact.dry_run_default` flag (default `true`) is
the second line of defense.

## Inputs from `hew prime compact`

- `tasks` — current graph state (informational; compact does not
  read or modify tasks).
- `memories.<bucket>` — categorized by prefix. The bucket matching
  the user's chosen prefix is your input.

The skill argument is the **prefix** to compact: `CONVENTION`,
`RESEARCH`, `factual`, etc. If the user didn't name one, run
`hew compact list-prefixes` first and ask.

## The compaction loop

```
1. Survey: hew compact list-prefixes
2. Pick the prefix (user-supplied or chosen from survey)
3. Read memories: hew memories --prefix <PREFIX>
4. Cluster by topic (in-context, K ≈ ceil(√N) capped at 6)
5. Draft 1–2 replacement bodies per cluster
6. Render the diff preview to the user
7. Wait for approval
8. Emit CompactPlan JSON and pipe to `hew compact apply`
9. Show the ApplyReport
```

### Step 1 — survey

Always start with:

```
hew compact list-prefixes
```

The output is a per-prefix histogram. Compaction is worth running
when a single prefix exceeds ~10 entries — below that, the cost of
losing nuance outweighs the gain.

### Step 2 — pick the prefix

If the user passed one, use it. Otherwise show the survey and ask:

```
Which prefix should I compact?
> CONVENTION (28)
  factual (70)
  RESEARCH (9)
  DECISION (24)
  cancel
```

### Step 3 — read

```
hew memories --prefix <PREFIX>
```

This is the text-default surface. **Do not pipe through `python` or
`jq`** (per `FEEDBACK:no-json-piping`). The text shape is what you
cluster against.

### Step 4 — cluster

You are the clustering algorithm. Group memories by topic, not by
filename or memory key. For a typical mid-sized project, aim for
`K = ceil(√N)` capped at `compact.target_clusters_cap` (default 6).
N=28 → K=5; N=70 → K=6 (capped).

The granularity knob from `compact.granularity_default` shapes the
prompt you give yourself:

- **broad** (default, per `DECISION:compact-granularity-default`) —
  prefer fewer, larger clusters. Strict topic boundaries; merge
  borderline cases up the tree.
- **fine** — prefer more, smaller clusters. Looser boundaries;
  preserve nuance even when topics overlap.

The dual-prompt-granularity pattern is from LLM-MemCluster research
(`RESEARCH:memory-compaction [CITED]`).

### Step 5 — draft replacement bodies

For each cluster, draft **1–2** replacement bodies. Each body must
preserve the prescriptive shape `hew-convention` calls "Writing
good conventions":

- **What to do** (the rule).
- **Where to see it** (canonical file / module if applicable).
- **What not to do** (the anti-pattern, when present in the
  originals).

Don't summarize away the load-bearing details. If three CONVENTION
memories all named the same file path, the merged body should still
name the file. The point of compaction is removing redundancy, not
information.

The provenance suffix (`[compacted-from: k1, k2, ...]`) is appended
automatically by `apply` — don't write it yourself.

### Step 6 — render the diff preview

Show the user every cluster and its proposed replacement, formatted
so they can spot bad merges quickly:

```
Cluster 1/5: rust-style (4 sources → 1 replacement)
  Sources:
    - convention-rust-formatting:    CONVENTION:rust-formatting — use rustfmt …
    - convention-rust-layout:        CONVENTION:rust-layout — workspace split …
    - convention-imports:            CONVENTION:imports — std then external …
    - convention-naming:             CONVENTION:naming — Rust snake_case …
  Replacement:
    CONVENTION:rust-style — Code is formatted by `cargo fmt`; the
    workspace splits hew-core (lib) + hew (binary) per CONVENTION:rust-layout.
    Imports order std → external crates → crate-internal. Names are
    snake_case per Rust convention.
```

Repeat for every cluster. End with a one-line summary:

```
Total: 28 sources → 8 replacements across 5 clusters.
3 keys will be skipped (exempt) and 0 by the drift-guard.
```

### Step 7 — approval

Ask explicitly:

```
Apply this plan?
> Apply — write replacements + forget sources
  Refine — let me adjust a cluster
  Cancel — no changes
```

Honor `--non-interactive` by **refusing to apply** unless the user
also passed `--yes`. Compaction is lossy; silent agreement is not
a contract.

If the user picks "Refine," loop back to step 4 with their feedback.

### Step 8 — emit + apply

Write the CompactPlan JSON to a temp file and pipe it through:

```
hew compact apply < /tmp/compact-plan.json
```

`hew compact apply` enforces:

- Validation (rejects malformed plans before any bd contact).
- The `compact.dry_run_default` flag — pass `--apply` to force
  execution when the global default is dry-run.
- The four safety invariants from `hew_core::compact::apply`:
  adds-before-forgets, provenance suffix, drift-guard, exempt
  allowlist.

### Step 9 — show the report

The CLI prints an `ApplyReport`:

```
COMPACT applied:
  added:               8
  forgotten:           25
  exempt skipped:      3
  drift-guard skipped: 0
  marker:              STATUS:compact:CONVENTION:2026-05-12T20:30:00Z
```

If `drift_guard_skipped > 0`, that's the project rejecting a
re-compaction of already-compacted entries (per
`DECISION:compact-drift-guard`). Tell the user; don't suggest
`--allow-recompact` unless they ask.

## Config knobs (MC.3)

Read at run-time from the user's hew config:

| Key                              | Default  | What it does                                                            |
|----------------------------------|----------|--------------------------------------------------------------------------|
| `compact.dry_run_default`        | `true`   | `hew compact apply` starts in dry-run mode unless `--apply` passed.      |
| `compact.granularity_default`    | `"broad"`| Strict vs relaxed clustering prompt.                                     |
| `compact.target_clusters_cap`    | `6`      | Upper bound on `default_k(N) = ceil(√N).clamp(1, cap)`.                  |
| `compact.allow_recompact_default`| `false`  | Strongly discouraged. Keeps the drift-guard active by default.           |
| `compact.exempt`                 | `[]`     | Literal keys never forgotten. Hardcoded `STATUS:scan/convention/plan/decompose` always exempt regardless. |

Set via `hew config set compact.<key> <value>`.

## Anti-patterns

- **Compacting `DECISION:` / `STATUS:` prefixes.** Locked design
  decisions and phase markers are load-bearing for the executor.
  Refuse and surface the conflict.
- **Compacting `BOUNDARY:`.** These are interface contracts. They
  belong in the codebase, not in a synthesized summary. Refuse.
- **Compacting after a recent decompose / before review.** Either
  the work-in-progress will lose its hard-won context, or the
  reviewer will lose the per-decision audit trail. Wait until a
  natural pause.
- **Silently widening the compaction.** If the user asked for
  CONVENTION and you find a tangentially-related factual memory,
  do NOT pull it into the plan. One prefix per run.
- **Summary-style replacement bodies.** "CONVENTION:rust — various
  rules" is useless to the executor. Replacement bodies must be
  prescriptive (Where / What-to-do / What-not-to-do).
- **Repeated re-compaction.** The drift-guard catches this; if you
  find yourself wanting `--allow-recompact`, you're probably
  fighting the prior cluster choices, not refining them. Cancel
  and accept the previous output.

## What you don't do

- **Run without user consent.** Compaction is irreversible (the
  provenance suffix is recovery breadcrumbs, not a real history).
- **Read or modify tasks.** Compaction is memory-only.
- **Touch git.** No commits, no branches. Memories live in bd.
- **Cross-pollinate prefixes.** A CONVENTION memory and a factual
  memory describing the same code do not merge. Prefix is taxonomy.
- **Pipe `hew memories --json` through `python` / `jq`** — use the
  text default per `FEEDBACK:no-json-piping`.

## Hand-off

Compaction is terminal — there's no downstream skill in the loop.
On success, print a one-screen recap to the user:

```
Compacted CONVENTION: 28 → 8 (5 clusters).
3 keys preserved (exempt: STATUS:scan, STATUS:convention, STATUS:plan).
Marker: STATUS:compact:CONVENTION:2026-05-12T20:30:00Z

Next: continue your work, or compact another prefix.
```

## Hand-off contract

After this skill completes:

- N old `<PREFIX>:` memories forgotten.
- M ≪ N new `<PREFIX>:` memories written, each carrying a
  `[compacted-from: ...]` provenance suffix.
- 1 `STATUS:compact:<PREFIX>:<iso-ts>` marker written.
- 0 other side effects (no task changes, no git operations, no
  configuration changes).
