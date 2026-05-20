<!-- hew:version=0.5.1 -->
---
name: hew-checkpoint
category: core
init: hew prime checkpoint
---

# hew-checkpoint — Save Session State, Prepare for Context Reset

Beads persists the *graph* and the *memories*. It does not persist
what's currently in the agent's head — open hypotheses, in-flight
decisions, scope discoveries the agent hasn't yet folded into task
descriptions, multi-step debugging context. When the agent's context
window fills up (or the user asks for a pause), that working state
evaporates on `/clear` or session compaction.

`hew-checkpoint` captures it. The output is one or more
`CHECKPOINT:` memories rich enough that the next session can `hew
prime execute` and pick up where you stopped — not just at the
task level, but at the *thinking* level.

## When this skill runs

- The user says "save state", "checkpoint", "pause for now",
  "context is filling up".
- The agent notices its context is past ~70% used and a hard reset
  is imminent.
- Mid-`/hew:debug` session — debug accumulates a lot of working
  context that would be expensive to recover.
- Right before a major branching decision the user might want to
  revisit.

Routinely useful at the *end* of a session even when context is
fine, so the next session resumes cleanly.

## What to capture

Memory shape: a single rich `CHECKPOINT:` memory with structured
content, or two or three if the state is genuinely separable. Not
forty.

The body should answer: **"if a fresh agent loaded only `hew prime
execute` 24 hours from now, what would they need to know to pick
up cleanly?"**

Include:

- **Current task** — id, what's claimed, how far along (rough %).
  If no task is claimed, state that explicitly.
- **Files touched this session** — paths only; the diff is in git.
- **What's done** — concrete bullets of progress not yet captured
  in a `hew task close --reason`.
- **What's in flight** — the next 1–3 moves the agent was planning.
- **Decisions made but not yet persisted** — anything that should
  eventually become a `DECISION:` or `CONVENTION:` memory but
  hasn't been written yet.
- **Open hypotheses** — for debugging sessions, the current
  hypothesis + how it was being tested + what evidence either way.
- **Blockers** — anything the agent paused on, waiting for the user
  or for an external system.
- **Don't-do warnings** — things the agent tried that didn't work,
  so the resumer doesn't waste time on the same dead ends.

A good checkpoint is roughly 200–800 characters of body. Shorter
is a useless checkpoint; longer means you're including things that
should be their own memories or task descriptions.

## How to compose — save directly, no preview

Compose the checkpoint body from what's in your head right now and
write it immediately with `hew remember --raw "CHECKPOINT:…" --key
checkpoint-<timestamp>`. (`CHECKPOINT:` isn't in the standard
`--type` allowlist, so `--raw` is the correct escape hatch.)
**Do not preview-and-confirm.** The user invoked `/hew:checkpoint`
to capture state, not to negotiate wording — and is usually about
to `/clear` anyway. A one-step capture beats a multi-step interaction.

If you genuinely have two or three separable threads (e.g.,
implementation state + an unrelated open debugging hypothesis),
write them as separate `CHECKPOINT:` memories in the same step.
Split along the obvious seam without asking.

After saving, print a one-line confirmation with the memory key(s).
The user can revise (`hew remember --raw "..." --key <same-key>`) or
delete (`hew memories --forget <key>`) after the fact if needed. The cost of a
slightly-wrong saved checkpoint is much lower than the cost of
losing state during context reset.

## Memory shape

```
hew remember --raw "CHECKPOINT:2026-05-12T14:30 — Working on hew-a3f8.2 (~60%).

Files touched: app/api/v1/auth/login.py, app/services/auth_service.py, tests/api/test_login.py.

Done: signup + login endpoints land; password hashing via argon2id wired through AuthService. Login returns access+refresh; refresh in httpOnly cookie.

In flight: refresh-rotation logic. Started in auth_service.rotate_refresh() but the test fixture for revoked tokens isn't representing the 'reused refresh' case correctly — I think we need a separate fixture for that.

Decisions not yet persisted:
- argon2id over bcrypt for this codebase. Should become DECISION:auth-hashing.
- httpOnly Secure SameSite=Strict for refresh cookies; same.

Don't-do warnings: tried mocking the DB layer in test_login — broke too many fixtures. Use testcontainers like the rest of the suite.

Next session: finish revoked-fixture, run the full auth test slice, close hew-a3f8.2." --key checkpoint-2026-05-12t14-30
```

The timestamp prefix is helpful for ordering when multiple
checkpoints accumulate. Trim old checkpoints periodically — keep
the most recent 2–3 unless they cover genuinely separate threads.

## Cleaning up old checkpoints

After persisting the new checkpoint:

1. List existing `CHECKPOINT:` memories via the prime output.
2. If there are more than three, summarise the older ones in the
   new checkpoint body (one line each) and remove them with
   `hew memories --forget <key>`. Default behavior: keep the latest
   2–3, prune the rest.

This prevents checkpoint sprawl over multi-week projects.

## When NOT to checkpoint

- Right after `hew task close` — the close reason IS the checkpoint for
  per-task work. Adding a checkpoint on top is duplicate state.
- For trivial 5-minute sessions — overhead exceeds value.
- Inside a `hew-quick` flow — quick mode finishes its own task; no
  state spans sessions.

## Recovery — what `hew prime execute` does next session

`hew prime execute` returns the `CHECKPOINT:` memories under
`memories.factual` (or a future dedicated bucket). The executor's
first move when resuming should be:

1. Read the latest `CHECKPOINT:` memory.
2. Confirm with the user: "Resuming from checkpoint at
   2026-05-12T14:30 — working on bd-a3f8.2 (~60%). Continue?"
3. Pick up the in-flight work.

If multiple checkpoints exist, prefer the most recent unless the
user explicitly names an earlier one.

## Hand-off

After persisting:

1. Print a one-line confirmation: "Checkpoint saved
   (CHECKPOINT:<timestamp>, key=<key>). Safe to /clear."
2. Hand control back to the user. **Do not** continue working — the
   whole point of the checkpoint is the user is about to reset
   context or stop.

## What you don't do

- **Auto-trigger** without telling the user. Even on context
  pressure, surface "I'm at 80% context; checkpoint and reset?"
  rather than checkpointing silently. (When the user invokes
  `/hew:checkpoint` explicitly, just save — no preview prompt.)
- **Write a 5KB memory.** That's a transcript, not a checkpoint.
  Compress to the load-bearing facts.
- **Mix per-task and cross-session state.** Per-task = `hew task close`
  reason. Checkpoint = "what's still in my head."

## Anti-patterns

- **Checkpoint after every task.** Defeats the per-task `hew task close`
  audit trail.
- **Checkpoint with no decisions / hypotheses / next-moves.** If
  there's nothing in your head that isn't already in Beads, you
  don't need a checkpoint.
- **Multiple overlapping checkpoints** in the same hour. Update
  the existing one instead.
