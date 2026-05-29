---
description: Drive the loop in-conversation, walking the children of one active epic.
---

In-conversation, epic-scoped driver. `/hew:auto` stays inside the
current Claude session and walks the children of one active epic in
dependency order — one continuous transcript, full mid-loop steering.

This is **not** the subprocess loop. For fresh-context-per-iter with
prompt-cache hits, hard caps, and per-iter logs on disk, use
[`/hew:loop`](./loop.md) (`hew loop run`).

## Pick the active epic

If the user named one in the prompt, use it. Otherwise:

```sh
hew epic list                    # open epics
hew epic tree <epic-id>          # children + dep order for the chosen epic
```

If exactly one open epic exists, pick it. If several, ask the user once
which to drive. Never default to the full `bd ready` set — that's the
subprocess loop's job.

## The walk

Until the active epic has no open children:

1. From `hew epic tree <epic-id>`, pick the next unblocked child
   (lowest-id ties break by creation order). Skip closed/in_progress
   children belonging to another assignee.
2. Tail-call `/hew:next` for that task. `/hew:next` claims, runs the
   full `hew-execute` loop (read → code → tests → `hew-guard` → close
   → commit → optional `hew remember`), and returns.
3. Loop. The session stays focused on one epic at a time; cross-epic
   work needs an explicit re-invocation.

Stop when:

- All children of the active epic are closed → run `/hew:verify`,
  then offer to close the epic and report done.
- A Rule-4 architectural change blocks progress → surface to the
  user and wait.
- The user interrupts.

Honor every `hew-execute` rule along the way (guard before close,
deviation tags in close reasons, atomic commits, branching contract).

## When to reach for `/hew:loop` instead

- You want each task in a fresh context window with a cache-warm
  prefix.
- You want hard caps (token / wall-clock / max-iter) and per-iter
  logs to disk.
- You're draining the global ready queue, not focusing one epic.

See [`docs/LOOP.md`](../docs/LOOP.md) for the full design.
