---
description: Remove a task and its subtasks from the graph.
---

Take the task id from the user. Run `hew task close <id> --reason "dropped"`. Recursively close children (use `hew task children <id>` to enumerate). If the task has dependents, surface them before proceeding.
