---
description: Add a new task to the existing graph.
---

Parse the task description from the user. Use `hew task new` with the right type + priority + parent. Wire dependencies via `hew dep add <child> --on <parent>` if obvious. Print `hew dep tree <root-id>` of the affected subtree before confirming.
