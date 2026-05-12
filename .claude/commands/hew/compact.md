---
description: Compact a noisy memory prefix from N entries down to 1-2 canonical entries per logical sub-cluster. Dry-run by default.
---

Invoke the hew-compact skill. The argument names the prefix to
compact (`CONVENTION`, `RESEARCH`, `factual`, `DECISION`, etc.); if
no prefix is given, the skill surveys current per-prefix counts via
`hew compact list-prefixes` and asks the user to pick.

Default behavior is **dry-run** (per `compact.dry_run_default = true`
and `DECISION:compact-safety`): the agent renders a cluster-by-cluster
preview and waits for explicit approval before piping the
`CompactPlan` JSON to `hew compact apply`. Pass `--apply` to skip
the dry-run preview when you trust the cluster choice.

Other flags forwarded to the skill:

- `--granularity=broad|fine` — overrides `compact.granularity_default`.
  Broad (default) = fewer, larger clusters; fine = more, smaller.
- `--allow-recompact` — opt out of the drift-guard (refuses to
  re-compact entries already carrying a `[compacted-from:` suffix).
  Strongly discouraged — see `DECISION:compact-drift-guard`.

The skill never touches `STATUS:scan`, `STATUS:convention`,
`STATUS:plan`, or `STATUS:decompose` (hardcoded exempt); user-added
exempt keys live in `compact.exempt`.

ARGUMENTS: $ARGUMENTS
