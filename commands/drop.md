---
description: Remove a task and its subtasks from the graph.
---

Take the task id from the user. Run bd close <id> --reason "dropped". Recursively close children. If the task has dependents, surface them before proceeding.
