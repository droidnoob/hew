---
description: Route freeform user input to the right hew skill.
---

Take the user prompt that follows this invocation, decide which skill is appropriate, and invoke it.

Routing table:
- "let's build / plan X" -> hew-plan
- "break this down / create tasks" -> hew-decompose
- "start coding / what's next" -> hew-execute
- "fix this one bug / tiny tweak" -> hew-quick
- "did we finish / verify" -> hew-verify
- "new codebase / map this repo" -> hew-scan, then hew-convention/audit/boundary
- "check health / what's wrong" -> /hew:doctor (use the binary, not the skill)

If unsure, run `hew prime execute` and let the ready list inform you.
