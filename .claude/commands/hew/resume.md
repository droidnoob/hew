---
description: Reload the session-resume context (project state + STATUS flags + memories + latest CHECKPOINT). Manual re-run of the SessionStart hook.
---

Run the binary: hew prime resume.

The output is the same JSON payload Claude Code's SessionStart hook
emits automatically on session startup. Use this slash when:

- You ran `/clear` and need the project context back.
- A long agent session drifted from the original priming and needs a
  fresh read of the state of the world.
- You want to confirm the `STATUS:*` flags and the latest
  `CHECKPOINT:` memory before deciding what to claim next.

The JSON contract is stable per
`factual:agent-contract-hew-prime-skill-always-emits-json` — the
shape will not break between minor versions.
