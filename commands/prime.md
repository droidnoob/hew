---
description: Emit JSON context for a specific skill (consumed by the agent). Manual invocation of the hew prime <skill> primer.
---

Run the binary: hew prime $ARGUMENTS.

`hew prime <skill>` returns one JSON blob with the prerequisites
check result, project state, ready tasks, categorized memories, and
the embedded skill body. Every skill in the registry has a primer.

Common targets:

- `/hew:prime hew-execute` — what to load before claiming a task.
- `/hew:prime hew-plan` — context for planning.
- `/hew:prime hew-verify` — batch verification context.
- `/hew:prime resume` — the SessionStart-hook context payload
  (alternate to `/hew:resume`).

The JSON contract is stable per
`factual:agent-contract-hew-prime-skill-always-emits-json`. Skill
bodies expect their callers to read this JSON first.

ARGUMENTS: $ARGUMENTS
