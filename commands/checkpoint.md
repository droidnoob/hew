---
description: Save rich session state to a CHECKPOINT memory before context reset.
---

Invoke the `hew-checkpoint` skill.

Captures what's in the agent's head right now — current task
progress, in-flight decisions, open hypotheses, scope discoveries
not yet folded into task descriptions — as a `CHECKPOINT:` memory.

Compose a 200–800-char body from current session state and save it
in **one** call:

```sh
hew checkpoint "<body>"
```

The subcommand auto-generates the ISO-8601 timestamp + key and
writes a canonical `CHECKPOINT:<ISO> — <body>` row. **Do not** roll
the shape by hand with `hew remember --raw "CHECKPOINT:…"` — that
path was a foot-gun (issue #40: a body without an ISO timestamp
directly after the prefix silently shadowed newer good checkpoints
in `hew prime resume`).

No preview, no confirmation — the user explicitly invoked
`/hew:checkpoint` to capture state, so the skill captures and gets
out of the way. Revise after the fact with `hew checkpoint
"<new-body>" --key <same-key>` or delete with `hew memories
--forget <key>`.

Use it before `/clear`, before a long pause, when context is past
~70% used, or at the end of a long debugging session. The next
session can `hew prime resume` and resume from the checkpoint
without re-discovering the working state.
