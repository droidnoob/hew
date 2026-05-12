---
description: Translate an approved plan into a Beads task graph — epic + child tasks + dependency edges. Runs the hew-decompose skill.
---

Invoke the hew-decompose skill. Reads the plan in conversation
context (or the named epic if `<epic-id>` is passed), then builds
the task graph per the skill body: pick graph shape, decide vertical
slice vs horizontal layer, write each task with Why / What / Files /
Tests / Craft / Acceptance lines (the CR.7 task-description
template), wire dependencies, place gates for external blockers,
pick types + priorities, self-validate against the plan.

Typically invoked at the tail of `/hew:plan` after the
research-or-decompose picker resolves. Can also be invoked directly
when re-decomposing a partially-shipped epic that needs a different
graph shape.

Writes `STATUS:decompose:complete for <epic-id> — <ts>` on success.

ARGUMENTS: $ARGUMENTS
