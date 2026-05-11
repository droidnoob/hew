<!-- hew:version=0.1.0 -->
---
name: hew
description: Index of installed hew skills. Loaded by the agent on session start.
---

# Hew Skills

The work loop: `plan → decompose → (ready → claim → execute → guard → close) → verify`.

## Core (always installed)

- `hew-plan` — strategic planning, goal-backward reasoning
- `hew-decompose` — translate plan into a Beads graph (epics, tasks, gates, bonds)
- `hew-execute` — the work loop
- `hew-verify` — post-completion verification
- `hew-guard` — pre-close sanity gate

## Brownfield (existing codebases)

- `hew-scan` — architecture mapping via `bd remember`
- `hew-convention` — pattern extraction and enforcement
- `hew-audit` — existing dependency health check
- `hew-boundary` — API contract and interface mapping
- `hew-migrate` — schema migration awareness

## Optional (user opts in)

- `hew-deps` — new dependency inspector
- `hew-research` — domain research
- `hew-quick` — fast mode for trivial tasks
- `hew-security` — lightweight security patterns

Custom skills in `custom/` are auto-discovered and never overwritten by `hew update`.
