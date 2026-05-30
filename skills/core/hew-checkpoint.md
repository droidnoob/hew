<!-- hew:version=0.12.0 -->
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

`hew-checkpoint` captures it. The output is a single `CHECKPOINT:`
memory rich enough that the next session can `hew prime resume` and
pick up where you stopped — not just at the task level, but at the
*thinking* level.

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

The body should answer: **"if a fresh agent loaded only `hew prime
resume` 24 hours from now, what would they need to know to pick up
cleanly?"**

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

## How to save — one command, no preview

**Use `hew checkpoint "<body>"`.** That's the entire interface. It:

- Auto-prepends `CHECKPOINT:<ISO-8601-now> — ` to your body.
- Auto-generates the key (`checkpoint-<sanitised-iso>`).
- Writes via the same code path as `hew remember`.

**Do not** roll the shape by hand with `hew remember --raw
"CHECKPOINT:…" --key checkpoint-…`. That path was a foot-gun — a
body without an ISO timestamp directly after `CHECKPOINT:` silently
shadowed newer good checkpoints in `hew prime resume` (GitHub
issue #40). The `hew checkpoint` subcommand exists specifically so
this can't happen.

**Do not preview-and-confirm.** The user invoked `/hew:checkpoint`
to capture state, not to negotiate wording — and is usually about
to `/clear` anyway. A one-step capture beats a multi-step interaction.

### Example

```sh
hew checkpoint "Working on hew-a3f8.2 (~60%).

Files touched: app/api/v1/auth/login.py, app/services/auth_service.py, tests/api/test_login.py.

Done: signup + login endpoints land; password hashing via argon2id wired through AuthService. Login returns access+refresh; refresh in httpOnly cookie.

In flight: refresh-rotation logic. Started in auth_service.rotate_refresh() but the test fixture for revoked tokens isn't representing the 'reused refresh' case correctly.

Decisions not yet persisted:
- argon2id over bcrypt for this codebase. Should become DECISION:auth-hashing.
- httpOnly Secure SameSite=Strict for refresh cookies.

Don't-do warnings: tried mocking the DB layer in test_login — broke too many fixtures. Use testcontainers like the rest of the suite.

Next session: finish revoked-fixture, run the full auth test slice, close hew-a3f8.2."
```

The subcommand prints `checkpoint saved (CHECKPOINT:<iso>, key=<k>).
Safe to /clear.` after the write.

If you genuinely have two or three separable threads (e.g.,
implementation state + an unrelated open debugging hypothesis),
run `hew checkpoint` twice — once per thread. Split along the
obvious seam without asking.

### Linking the checkpoint to its task

If the checkpoint is about a specific in-flight task, link it:

```sh
hew checkpoint "..." --related-task hew-a3f8.2
```

This emits a `LINK:` sidecar so the task and the checkpoint are
discoverable from either side.

### After-the-fact revisions

Revise an existing checkpoint by calling `hew checkpoint` again with
the same `--key`, or delete with `hew memories --forget <key>`. The
cost of a slightly-wrong saved checkpoint is much lower than the
cost of losing state during context reset.

## Cleaning up old checkpoints

After persisting the new checkpoint:

1. List existing `CHECKPOINT:` memories: `hew memories --prefix=CHECKPOINT`.
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

## Recovery — what `hew prime resume` does next session

`hew prime resume` returns the most recent `CHECKPOINT:` memory
(ranked by the ISO-8601 timestamp embedded in the body). The
resumer's first move is:

1. Read the latest `CHECKPOINT:` memory.
2. Confirm with the user: "Resuming from checkpoint at
   <iso> — working on <task> (~N%). Continue?"
3. Pick up the in-flight work.

If multiple checkpoints exist, prefer the most recent unless the
user explicitly names an earlier one.

## Hand-off

After persisting:

1. The subcommand already prints the confirmation line.
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
- **Hand-roll `hew remember --raw "CHECKPOINT:…"`.** Always use the
  `hew checkpoint` subcommand. It guarantees the shape the resume
  primer needs.

## Anti-patterns

- **Checkpoint after every task.** Defeats the per-task `hew task close`
  audit trail.
- **Checkpoint with no decisions / hypotheses / next-moves.** If
  there's nothing in your head that isn't already in Beads, you
  don't need a checkpoint.
- **Multiple overlapping checkpoints** in the same hour. Update
  the existing one with the same `--key` instead.
