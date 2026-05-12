---
description: Extract prescriptive CONVENTION rules (and CONVENTION:craft.<id> picks) from an existing codebase. Second link in the brownfield chain.
---

Invoke the hew-convention skill. Walks the codebase area by area
(services, errors, API, DB, tests, imports, naming, logging, types,
frontend) and persists each load-bearing pattern as a prescriptive
`CONVENTION:<key>` memory. The Step 11 craft pass surfaces the
SOLID/DRY/Clean-Arch/etc. principles the code already follows and
persists them as `CONVENTION:craft.<id>` memories (the brownfield
counterpart to hew-new-project's picker).

Requires `STATUS:scan:complete`. Writes `STATUS:convention:complete`
on completion.

A useful exit check: pick a file you haven't read, predict what it
looks like from your conventions, then read it. If you're surprised,
your conventions are incomplete.

ARGUMENTS: $ARGUMENTS
