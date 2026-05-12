---
description: Save rich session state to a CHECKPOINT memory before context reset.
---

Invoke the `hew-checkpoint` skill.

This captures what's in the agent's head right now — current task
progress, in-flight decisions, open hypotheses, scope discoveries
not yet folded into task descriptions — as a `CHECKPOINT:` memory.

The skill is interactive: it composes a checkpoint body, shows it
to you, and asks whether to save as-is, edit, split into multiple
checkpoints, or cancel. Nothing lands without your approval.

Use it before `/clear`, before a long pause, when context is past
~70% used, or at the end of a long debugging session. The next
session can `hew prime execute` and resume from the checkpoint
without re-discovering the working state.
