---
description: Pick and start the highest-priority unblocked task.
---

Run `hew next`. That claims the top ready task and prints its id+title.

Then follow `hew-execute`: `hew task show <id>`, work, guard, close, commit.

Variants:

- `hew next --no-claim` — peek at the top of the ready queue without claiming.
- `hew next --branch` — also create a feature branch (prefix inferred from `issue_type`, slug from title). Override with `--prefix=<feat|fix|...>` and `--slug=<short-kebab>`.
- `hew ready` — list all unblocked tasks (mirrors `bd ready`).
