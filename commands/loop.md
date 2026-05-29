---
description: Drive the autonomous outer loop — `hew loop run` until the queue drains, with cancel / logs / list subcommands for inspection.
---

Shell to `hew loop`. The default is the `run` subcommand, which loops
`prompt → spawn → gate → log` until a stop signal fires (budget,
ready-queue empty, stop-file, max-iter, runtime error).

Common invocations:

```sh
hew loop run --dry-run --max-iter 1   # smoke: prompt-assemble, no spawn
hew loop run --until-empty            # drain the ready queue (default on)
hew loop run --max-iter 5 --strict    # bounded run, craft warnings = fail
hew loop run --budget-tokens 200000   # cap cumulative tokens
hew loop run --budget-wall 30m        # cap wall clock
hew loop run --runtime=codex          # drive codex-cli instead of claude
hew loop run --fallback-runtime=codex # swap to codex on primary RuntimeError

hew loop list                         # recent runs + state
hew loop logs --tail 5                # last 5 iters of latest run
hew loop logs --iter 3 --json         # raw payload for one iter
hew loop cancel                       # touch stop-file on latest run
```

Per-iter artifacts land in `.hew/loop/<run-id>/`:

- `iter-NNN.json` — atomically-written per-iter record
  (task id, outcome, prompt prefix hash, token spend, stderr tail)
- `run.json` — running summary, rewritten after each iter
- `.stop` — sentinel the operator (or `hew loop cancel`) drops to ask
  the loop to halt at the next iter boundary

The loop runner is process-level: each iter is one fresh `claude -p`
(or `codex exec`) invocation. Prompt prefix (skill body + memory primer)
is byte-stable across iters of the same skill so the agent's prompt
cache hits; `prompt_prefix_hash` in the iter log lets you verify that.
`--fallback-runtime` is primary-sticky with a configurable cooldown
(`--fallback-cooldown-iters`, default 3); see `docs/LOOP.md` for the
full state machine.

For the full design + the 10 locked decisions behind the loop, see
`docs/LOOP.md` and the `hew-gr1` epic.
