---
description: Autonomously drain the queue via the process-level loop (`hew loop run --until-empty`).
---

Run the autonomous outer loop until the ready queue drains:

```sh
hew loop run --until-empty
```

`--until-empty` is on by default — stop signals fire when the queue
hits zero, when a budget caps, when the operator touches the stop-file,
or on a runtime error. Use `--max-iter N` to bound the run, `--strict`
(default) to promote craft warnings to failures, `--budget-tokens N` /
`--budget-wall <duration>` for hard caps.

Per-iter logs land in `.hew/loop/<run-id>/iter-NNN.json`. Inspect with
`hew loop list`, `hew loop logs --tail 5`, `hew loop cancel`.

For the in-conversation walk through the workflow (older `/hew:auto`
behavior — useful when you want to drive iters from inside one Claude
session rather than spawning fresh subprocesses), call `/hew:work` and
let it tail-call `/hew:next` until either:

- `hew status` ready list is empty (call `/hew:verify`, then report done), or
- a Rule-4 architectural change blocks you (stop, surface, wait).

Honor all guard / deviation / convention rules; atomic commits per task.

See `docs/LOOP.md` for the full design.
