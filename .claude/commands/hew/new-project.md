---
description: Bootstrap a new project from a 1-3 sentence outline.
---

Invoke the `hew-new-project` skill (`hew prime new-project`).

Pass the user's outline verbatim as the first input. If the outline
is empty, ask the user for one before starting.

Takes optional `--re-bootstrap` flag (passed as `$ARGUMENTS` contains
the literal `--re-bootstrap`): allows the skill to proceed even when
`STATUS:new-project:complete` already exists in memory. The skill
will overwrite the existing PROJECT / DECISION / CONVENTION /
ROADMAP / MILESTONE memories.

Default behavior (no `--re-bootstrap`): if `STATUS:new-project:complete`
exists, refuse and surface the existing bootstrap timestamp to the
user.

When done, the skill writes `STATUS:new-project:complete` and the
user can move to `/hew:next` to start work on the first milestone's
ready task.
