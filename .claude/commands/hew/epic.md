---
description: CRUD ops on epics: new, close, audit, summary, gaps, bond, tree.
---

Subcommand handler. Args:
- new <name>      -> bd create "<name>" -t epic
- close           -> close current/named epic (children must close first)
- audit           -> compare epic intent vs closed tasks
- summary         -> generate summary from closed task descriptions + memories
- gaps            -> find open tasks with no path to completion
- bond <A> <B>    -> bd mol bond B A (B after A)
- tree            -> bd dep tree <epic-id>
