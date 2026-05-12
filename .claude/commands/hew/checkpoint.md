---
description: Save rich session state to a CHECKPOINT memory before context reset.
---

Invoke the `hew-checkpoint` skill.

This captures what's in the agent's head right now — current task
progress, in-flight decisions, open hypotheses, scope discoveries
not yet folded into task descriptions — as a `CHECKPOINT:` memory.

The skill composes a checkpoint body from current session state and
saves it directly via `bd remember`. No preview, no confirmation —
the user explicitly invoked `/hew:checkpoint` to capture state, so
the skill captures and gets out of the way. Revise after the fact
with `bd remember --key <key>` or delete with `bd forget <key>`.

Use it before `/clear`, before a long pause, when context is past
~70% used, or at the end of a long debugging session. The next
session can `hew prime execute` and resume from the checkpoint
without re-discovering the working state.
