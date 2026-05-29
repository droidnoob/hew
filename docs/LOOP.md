# `hew loop` — the autonomous outer loop

`hew loop` is a process-level harness that drains the bd ready queue by
repeatedly spawning fresh Claude Code subprocesses. Each iter is one
`claude -p` invocation against a cache-stable prompt prefix; the loop
collects the result, runs the test/lint gate, logs to
`.hew/loop/<run-id>/`, then re-evaluates stop signals and decides
whether to iterate again.

This is the methodology's outer loop. Skills are the inner loop.

---

## When to use it

Reach for `hew loop` when:

- A long backlog of unblocked tasks is ready and you want them drained
  without holding a single chat session open.
- You want each task tackled in a fresh context window, with the
  skill body + memory primer cached across iters.
- You want hard caps (token budget, wall clock, max iter) instead of
  trusting a single session to stay coherent.

Use the in-conversation flow (`/hew:auto`, `/hew:work`) when:

- The work is exploratory and you want to see what the model proposes
  before committing iters of compute.
- You want to steer mid-loop rather than after each subprocess returns.

---

## Defaults

```text
--until-empty   = on    stop when ready queue drains
--strict        = on    craft warnings (testing, lint) promote to fail
--interactive   = off   no ask-file pauses
--runtime       = claude  one of {claude, codex}; see Runtimes below
--fallback-runtime  = unset  on primary RuntimeError, route iters to this runtime
--fallback-cooldown-iters = 3  iters to stick on fallback before retrying primary
```

Override via CLI flags; see `hew loop run --help` for the full list.

---

## Decision flow per iter

1. **Stop check.** Collector reads cancel flag, stop-file presence,
   wall clock, cumulative tokens. Precedence:
   `Cancelled > StopFile > BudgetTokens > BudgetWall > MaxIter >
   RuntimeError > GuardTrip > ReadyEmpty`.
2. **Pick task.** Top of `bd ready`. Empty + `--until-empty` →
   `ReadyEmpty`, exit.
3. **Assemble prompt.** Skill body + memory primer (cacheable prefix)
   + task brief (per-iter tail). `prefix_hash` is FNV-1a-64; logged
   so you can verify cache stability.
4. **Spawn.** `claude -p <prompt> --allowedTools <csv>
   --output-format json`. Tools come from the skill's
   `tools:` frontmatter (or the default allowlist).
5. **Parse outcome.** JSON `result` scanned for `closed <id>` marker;
   `usage` lifted into per-iter `TokenSpend`.
6. **Gate.** Test + lint commands run (or skipped per project config).
   Hard failure always fails; craft warnings fail under `--strict`.
7. **Log.** Atomic write of `iter-NNN.json`; `run.json` rewritten.
8. **Loop or break.** Back to step 1 unless a stop fires.

---

## Runtimes

`--runtime` selects which agent CLI the loop drives. Both runtimes
implement the same `RuntimeSpawner` trait; the loop is symmetric in
how it parses outcomes and accounts tokens.

| Runtime | Subprocess shape | Stream format | Bin override env |
|---------|-----------------|---------------|------------------|
| `claude` | `claude -p <prompt> --allowedTools <csv> --output-format json` | one JSON envelope on stdout | `HEW_LOOP_CLAUDE_BIN` |
| `codex` | `codex exec --json --skip-git-repo-check --sandbox <X> [-m <model>] [-C <wd>] -- <prompt>` | JSONL events (`turn.completed` / `turn.failed` / `item.completed`) | `HEW_LOOP_CODEX_BIN` |

**Sandbox mapping (codex only).** Codex has no per-tool allowlist;
hew's `allowed_tools` list maps to one of three sandbox values:

| `allowed_tools` contains | Codex `--sandbox` |
|--------------------------|-------------------|
| any of `Edit` / `Write` / `MultiEdit` / `NotebookEdit` | `workspace-write` |
| anything else (incl. `Read`, `Bash(...)`) | `read-only` |

Per `DECISION:codex-sandbox-mapping`. Bash subcommand restrictions like
`Bash(git:*)` are silently broadened to whatever bash the chosen sandbox
allows — codex's sandbox enum cannot express finer-grained gating. If a
future iter needs a tighter shell allowlist, the right fix is finer
codex gating, not a shell wrapper.

**HEW_LOOP_CODEX_BIN** overrides the `codex` binary location (mirrors
`HEW_LOOP_CLAUDE_BIN`). Use for pinning to a specific install or in
tests.

---

## Fallback runtime

`--fallback-runtime <claude|codex>` (alias of the
`loop.fallback_runtime` config key) wires a secondary runtime that the
loop routes to when the primary returns a `RuntimeError`. The fallback
is **primary-sticky with cooldown** (per `DECISION:loop-fallback-policy`):

1. Primary errors → enter cooldown for `--fallback-cooldown-iters` iters
   (default 3, config: `loop.fallback_cooldown_iters`).
2. While in cooldown, every iter routes to the fallback spawner.
3. A fallback error inside the cooldown window resets the counter back
   to quantum — primary stays parked.
4. A fallback success decrements the counter by 1.
5. When the counter reaches 0, the next iter retries the primary once.
6. Primary retry success → exit cooldown. Primary retry error →
   re-enter cooldown for another quantum.

`GuardTrip` and `BudgetExhausted` do *not* change the cooldown state —
the runtime is fine, the iter failed for other reasons.

### Worked example

```text
--fallback-runtime=codex --fallback-cooldown-iters=3

iter 1: claude fails (401, Auth)         → cooldown starts, remaining=3
iter 2: codex succeeds                   → remaining=2
iter 3: codex succeeds                   → remaining=1
iter 4: codex succeeds                   → remaining=0
iter 5: claude retry succeeds            → cooldown exits, back on primary
iter 6: claude succeeds                  → no cooldown
```

If iter 5's claude retry had errored, the loop would re-enter cooldown
for another 3 iters on codex, then retry claude once at iter 9.

### Failure classification

The loop categorizes each iter's outcome before deciding what to do:

| `SpawnFailureClass` | Triggered by | Fallback fires? |
|---------------------|-------------|-----------------|
| `Success` | `turn.completed` (codex) / clean `result` envelope (claude) and exit 0 | n/a |
| `RuntimeError(Auth)` | HTTP 401 / 403 | yes |
| `RuntimeError(RateLimit)` | HTTP 429 | yes |
| `RuntimeError(Server)` | HTTP 5xx | yes |
| `RuntimeError(BadRequest)` | HTTP 400 / 404 / 422 | yes (but unlikely to help — deterministic refusal) |
| `RuntimeError(Spawn)` | missing binary / ETXTBSY / truncated stream | yes |
| `RuntimeError(Unknown)` | error envelope with no status | yes |
| `GuardTrip` | gate failure under `--strict` | **no** (different runtime won't help) |
| `BudgetExhausted` | token / wall / iter cap hit | **no** (terminal stop) |

> ⚠ **Codex exit codes lie.** `codex exec` returns exit 0 even on API
> 400 errors — the stream's `turn.failed` event is the source of truth.
> Hew's parser reads the event, not the exit code, when classifying
> failures. Per `RESEARCH:codex-exec-exit-code`.

---

## Memory-graph compounding effect

The skill body + primer prefix is byte-identical across iters, so
Anthropic's prompt cache hits from iter 2 onwards. As iters file
`DECISION:` / `CONVENTION:` / `RESEARCH:` memories, those memories
land in the primer that future iters read — so each iter operates
against a richer context than the last without paying for additional
context tokens (cache reads are billed at ~1/10th of input rate).

Watch for it: across a long run, `cumulative_tokens` climbs roughly
linearly while *per-iter* cache_read tokens rise as a share of total
spend. That's the compounding loop doing its job.

If `prompt_prefix_hash` changes between iters of the same skill, the
cache isn't hitting. Common causes: primer pulled in a memory edit
mid-run (expected) or skill body got hot-edited (unexpected). The
hash in each iter log makes this easy to spot.

---

## Stop signals

- `hew loop cancel` — touches `.hew/loop/<run-id>/.stop`.
- `--budget-tokens N` — soft cumulative cap; the iter that pushes the
  total over `N` is the last one.
- `--budget-wall <s/m/h>` — wall-clock cap, e.g. `30m`, `2h`.
- `--max-iter N` — hard iteration cap.
- Runtime non-zero exit on the previous iter → `RuntimeError`.
- Gate failure under `--strict` on the previous iter → `GuardTrip`
  (after the iter's commits get reverted).

---

## Troubleshooting

**`bd discover` fails.** The loop expects `bd` on PATH and a
`.beads/` initialized project. Run `hew doctor`.

**`unknown skill` error.** `--skill` defaults to `hew-execute`. If
you pass a custom name, it must match a `skills::find()` entry — i.e.
a skill registered in `hew_core::skills::CORE/BROWNFIELD/OPTIONAL`.

**Iter outcome is `no_close` repeatedly.** The runtime returned
without emitting a `closed <id>` line. Either the prompt didn't land
the agent on a closeable task, or the agent hit its own internal
caps. Check `iter-NNN.json::stderr_tail` for clues; consider
narrowing `--skill` or chunking the task.

**Iter outcome is `runtime_error`.** Claude Code subprocess crashed
or exited non-zero. `stderr_tail` carries the last 16 lines. If
`claude` isn't on PATH, set `HEW_LOOP_CLAUDE_BIN=/path/to/claude`.

**`Verdict::Fail` with no commits made.** The gate detected a clean
tree was made dirty by the iter; under `--strict` even craft warnings
fail. Pass `--no-strict` to demote craft warnings back to warn-only.

**Prompt-cache misses across iters.** Check
`iter-001.json::prompt_prefix_hash` vs subsequent iters. If they
drift, the primer changed mid-run (a new `DECISION:` / `RESEARCH:`
memory landed that the primer surface includes). That's working as
designed — the next iter's prefix incorporates the new memory and
itself becomes cacheable for iter+1.

---

## First real run

The first end-to-end exercise of `hew loop run` against a live
`claude -p` (claude 2.1.150) drove a toy todo crate to three closes
on 2026-05-26. Captured under
`examples/loop-runs/2026-05-26/hew-loop-crud-runs.tar.gz` — contains
the `.hew/loop/<run-id>/` directories from three runs:

- **run 1** (1 iter): added `Todo` struct + tests. iter outcome
  `no_close` — pre-fix detect-marker miss; **hew-7tp** filed and
  fixed in the same session via out-of-band `bd.ready()` diff.
- **run 2** (2 iters): added `TodoStore` + CRUD tests, then the
  `todo add/list/done/rm` CLI. Both iters `outcome=closed` with the
  new detector. ~1.7M tokens combined; the project compiled clean and
  11 unit tests passed.
- **run 3** (1 iter): forced lint failure (deliberate
  `clippy::useless_vec` planted in `main.rs`); iter rolled back to
  the pre-iter sha, `outcome=backpressure_fail`, and a
  `STATUS:loop-iter-failed:` memory landed in bd.

Outstanding bugs discovered by the run:

- **hew-7tp** (closed) — agent-via-Bash closes weren't detected.
- **hew-2dx** (open) — the per-iter primer lives inside the cacheable
  prefix, so `prompt_prefix_hash` changes every iter and the
  Anthropic prompt cache misses. Fix is a structural move of the
  primer into the tail.

---

## Related

- `commands/loop.md` — the `/hew:loop` slash body.
- `commands/auto.md` — the `/hew:auto` slash, now a thin pointer at
  `hew loop run --until-empty`.
- `hew_core::runner` — pure precedence logic for stop signals.
- `hew_core::prompt` — cache-disciplined assembler.
- `hew_core::runtime` — Claude Code spawner.
- `hew_core::backpressure` — gate verdict logic.
- `hew_core::stop_signals` — I/O-side signal gathering.
- `hew_core::loop_log` — per-run atomic logging layout.
- Epic `hew-gr1` — the 10 design decisions, plus the task graph that
  built the loop.
