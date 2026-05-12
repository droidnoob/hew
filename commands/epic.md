---
description: CRUD ops on epics: new, close, audit, summary, gaps, bond, tree.
---

Subcommand handler. Args:
- new <name>      -> `hew task new --type=epic --title="<name>"`
- close <id>      -> `hew epic close <id>` (refuses if children still open; use `--force` to override)
- audit <id>      -> `hew epic audit <id>` (flags children with thin close reasons)
- summary <id>    -> `hew epic summary <id>` (one-line-per-child readout)
- show <id>       -> `hew epic show <id>` (body + first-level children)
- tree <id>       -> `hew epic tree <id>` (recursive parent-child walk)
- gaps <id>       -> find open tasks with no path to completion (no wrapper yet; consult `hew dep blocked`)
- bond <A> <B>    -> intentionally NOT wrapped — `bd mol bond` semantics are broken (see GOTCHA:bd-mol-bond). Use `hew dep add <first-B-task> --on <last-A-task>` for cross-epic sequencing.
