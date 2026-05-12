---
description: Strategic planning via hew-plan skill.
---

Invoke the hew-plan skill on the user's described goal. Do not produce tasks yet; planning ends with goal + acceptance criteria + architecture + order + graph shape. Hand off to hew-decompose only after the user approves.

An optional `<topic>` argument hints a research focus to the tail picker:

- `/hew:plan <free-form goal>` — runs the planner normally; tail picker
  defaults per `hew config get research.default`.
- `/hew:plan --research <topic>` — preselects "Research first" at the tail
  picker and passes `<topic>` to `/hew:research` if the user accepts.

The research detour is always optional. The picker honors --non-interactive
by using the configured default without prompting.
