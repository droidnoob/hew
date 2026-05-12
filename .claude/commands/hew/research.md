---
description: Ad-hoc topic research with web search + cited findings. Persists RESEARCH memories with [VERIFIED]/[CITED]/[ASSUMED] provenance tags.
---

Invoke the hew-research skill directly on a named topic. Distinct
from `/hew:plan --research <topic>`, which loops research into the
planner; this slash runs research standalone (re-audit a framework,
investigate a new domain constraint, gather citations before a
spec).

Findings persist as `RESEARCH:<topic> [TAG] <claim> — <source>`
memories. Tags: `[VERIFIED]` = 2+ authoritative sources; `[CITED]` =
1 authoritative source; `[ASSUMED]` = agent inference, flagged for
revisit.

Writes `STATUS:research:complete — <ts>` and surfaces unresolved
contradictions to the user as open questions before exiting.

Opt-in skill — set `hew config set optional-skills.research true`
to keep this slash installed across `hew update` runs.

ARGUMENTS: $ARGUMENTS
