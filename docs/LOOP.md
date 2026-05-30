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
--jobs              = 1     parallel worker slots (1 = serial fast-path)
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

---

## Per-task model selection

By default every iter runs against the runtime's default model. When
one task is genuinely harder than the rest of the queue — a tricky
refactor, a thorny algorithm, an architectural call — you can route
just that task to a stronger model without touching the rest. The
loop resolves a `--model` / `-m` override per iter from this
precedence chain (highest wins):

1. Description tag: `<!-- hew:model=opus-4-7 -->` anywhere in the
   task description. Cheapest to add; travels with the task body.
2. Label: `bd label add hew-X model:opus-4-7`. Useful when you don't
   want to edit the description, or when many sibling tasks should
   share an override.
3. Config: `loop.model.by_priority.<P>` and `loop.model.by_type.<t>`,
   both maps. By-priority wins over by-type when both match.
4. Config: `loop.model.default`. The project-wide floor.
5. None — runtime picks its default.

### TOML config

```toml
[loop.model]
default = "sonnet-4-6"

[loop.model.by_priority]
P0 = "opus-4-7"
P1 = "opus-4-7"

[loop.model.by_type]
bug = "opus-4-7"
```

Read/write via `hew config get loop.model.by_priority` (comma-separated
`KEY=VAL` pairs) or `hew config set loop.model.by_priority.P0 opus-4-7`
(dotted single-entry form; empty value clears).

### Per-model spend in the summary

When at least one iter records a `model`, `hew loop summary` adds a
"by model" breakdown:

```text
by model
─────────────────────────────────────────────────────────────────
model         iters   tasks   input    cached   output   total
opus-4-7         3       3   12.4k    98.1k     4.2k    114.7k
sonnet-4-6       7       6    8.1k    71.2k     2.9k     82.2k
(default)        1       1    0.5k     3.4k     0.2k      4.1k
─────────────────────────────────────────────────────────────────
```

Iters without a resolved model collapse under `(default)`. The table
is hidden when no iter recorded one — no flag, no config to enable.

### Caveat: per-model cache pools

Anthropic's prompt cache is keyed per model. If half a run uses
`sonnet-4-6` and half uses `opus-4-7`, you pay the cache-creation
input cost twice — once per model — even if the prefix bytes are
identical. The compounding effect from "Memory-graph compounding" is
still real, but it's per-model. Reserve overrides for tasks that
genuinely benefit from the swap; sprinkling them across an otherwise
uniform queue costs cache hits.

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

## Parallel runs

`--jobs N` (range `1..=16`) launches `N` worker slots that drain the
ready queue concurrently. The default is `1`, which preserves today's
single-threaded loop byte-for-byte (no worktrees, no merge-back, no
manifest). `N >= 2` switches to the dispatcher path.

Per `DECISION:loop-parallel-overlap-policy` v1 is **trust-the-graph**:
any `bd ready` task is parallelizable. No "touches" predicate, no
overlap heuristic. The decomposer already encodes ordering as
dependency edges; if two ready tasks shouldn't run together, file a
dep instead of annotating overlap metadata. Merge conflicts surface
as `[merge-conflict]` bug tasks at run end.

### Layout

```text
~/.hew/wt/                                 ← out-of-tree per worker
  <run-id>/
    0/                                     ← worker 0 checkout
    1/                                     ← worker 1 checkout

<project>/.hew/loop/<run-id>/              ← run-dir (unchanged location)
  manifest.json                            ← cross-worker manifest (parallel only)
  worker-0/
    run.json
    iter-001.json
    iter-002.json
  worker-1/
    run.json
    iter-001.json
```

Per `DECISION:loop-worktree-location` worktrees live under `~/.hew/wt/`
rather than inside the project (the project's `.hew/loop/` directory
is tracked in git on this repo; an in-tree worktree would pollute
`git status` or force gitignore drift).

Each worker gets a fresh branch `loop/<run-id>/w<n>` cut from the
launch HEAD's sha. Iters commit there; the dispatcher merges every
branch back at shutdown.

### Branch naming

```text
loop/<run-id>/w<n>
```

Stable per worker, deterministic per run. A `branch_exists` pre-check
refuses to overwrite if you reuse a `run-id` (which can happen if the
previous run crashed before cleanup) — run `hew loop prune-worktrees`
first.

### Merge-back

At shutdown the dispatcher checks each `loop/<run-id>/w<n>` back onto
the launch HEAD with `git merge --no-ff --no-edit`. Each branch lands
as its own merge commit so worker history survives in `git log
--graph`. Behavior is sequential and short-circuit-free: one
conflicting merge does not stop later workers from being attempted.

- **Clean merge:** worker branch becomes a merge commit on HEAD; the
  worktree is pruned in the same shutdown pass (`hew-kt5q`). Branch
  reference stays.
- **Conflict:** the in-progress merge is aborted (`git merge --abort`),
  a `[merge-conflict]` bug task is filed via `bd q` with the unmerged
  file list, and the worker's worktree **stays on disk** so the
  operator can resolve by hand. Hint in the bug body points at
  `~/.hew/wt/<run-id>/<n>/`.

### `--jobs N` worked example (N=2)

```text
$ bd ready
hew-r1  Refactor TodoStore::insert path
hew-r2  Add  cli flag --json to todo list
hew-r3  Fix typo in README
hew-r4  Drop dead helper in cli.rs

$ hew loop run --jobs 2 --max-iter 4
hew loop loop-2026-05-30T12:30:00Z-deadbeef — jobs=2 \
  run-dir=.hew/loop/loop-2026-05-30T12:30:00Z-deadbeef
dispatcher: jobs=2 ready_seen=4 assigned=2 claim_failures=0

# Dispatcher claims hew-r1 → slot 0, hew-r2 → slot 1.
# Worktrees created at:
#   ~/.hew/wt/loop-2026-05-30T12:30:00Z-deadbeef/0  (branch loop/.../w0)
#   ~/.hew/wt/loop-2026-05-30T12:30:00Z-deadbeef/1  (branch loop/.../w1)
# Worker 0 drains its remaining ready queue (hew-r3); worker 1 hits
# the agent and closes hew-r2, then picks up hew-r4 next iter.

merge_back: merged=2 conflicts=0 bugs_filed=0
worktrees: pruned 2 cleanly-merged

  per-worker:
             wkr  iters  closed runtime       tokens stop
             0        2       2 claude         12,500 ready_empty
             1        2       2 claude         11,800 ready_empty
             all      4       4               24,300

  hew loop summary — total tokens 24,300 …
```

Read the manifest directly:

```sh
cat .hew/loop/<run-id>/manifest.json
```

### Concurrency caveats

- **Anthropic tier rate limits.** Each worker is a separate `claude -p`
  subprocess. Two workers running at the same tier double the
  per-minute rate request load. Start at `--jobs 2`; only increase
  once you've watched a run and confirmed you're not throttling.
- **Prompt-cache pool fragmentation.** Each worker assembles its own
  prompt. With identical `--skill` + identical bd primer the prefix
  hash will match across workers (verify in `iter-001.json::
  prompt_prefix_hash`), so cache lookups hit. Mixing models or skills
  across workers fragments the cache pool; expect higher per-iter
  input tokens.
- **Disk pressure.** Each worktree is a full checkout of the project.
  For a 500 MB repo at `--jobs 8` that's 4 GB of working trees under
  `~/.hew/wt/`. Prune after crashed runs.
- **Token budgets are global.** `--budget-tokens N` caps the
  cumulative spend across **all** workers, not per worker.

### Recommendation

Start at `--jobs 2`. The graph-trust assumption holds up best when the
two slots target genuinely disjoint task sets; if you see frequent
`[merge-conflict]` bug tasks, that's a signal your decomposer is
under-specifying deps — fix the graph, not the loop.

---

## Recovering from a crashed parallel run

If the loop process dies (panic, kill -9, host reboot) mid-run:

1. Worker worktrees under `~/.hew/wt/<run-id>/<n>/` stay on disk
   because no shutdown pass ran.
2. `<project>/.hew/loop/<run-id>/run.json` either is missing
   (crashed before first iter) or has `stop_reason = None`. Either
   way, `hew_core::loop_log::active_run_ids` flags the run as still
   "active" and won't auto-prune its worktrees.
3. Inspect the crashed worktrees by hand if you need to. Per-worker
   branches `loop/<run-id>/w<n>` are intact — you can `git checkout`
   any of them.
4. Once you've extracted what you need, mark the run completed by
   either deleting `<project>/.hew/loop/<run-id>/` or by hand-setting
   a stop reason in its `run.json`. Then run:

   ```sh
   hew loop prune-worktrees           # dry-run; lists orphans
   hew loop prune-worktrees --apply   # delete them
   ```

`hew loop prune-worktrees` walks `~/.hew/wt/` and removes worktrees
whose `<run-id>` has no matching active run-dir under the current
project's `.hew/loop/`. It does **not** delete the per-worker branches
— those live in the project's git history and remain rebaseable
material until you `git branch -D` them yourself.

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

## Scope

A run is *scoped* — `hew loop run` needs to know which set of bd tasks
counts as the queue. Two shapes ship today:

- **`--scope=ready`** — every `bd ready` task. Current behavior; the
  agent picks the top of the ready list every iter.
- **`--scope=epics --epics=<csv>`** — restricted to tasks transitively
  under the listed epics. The dispatcher walks `bd children` for each
  epic id at startup and filters every `dispatch_tick` against that
  set, so workers never see siblings of unrelated epics. Epic ids
  themselves are included so an "epic only ever closes when its
  children are done" graph still resolves.

**Resolution order:**

1. CLI args (`--scope=...`, `--epics=...`) win.
2. Interactive picker fires when stderr is a TTY and `--scope` was
   omitted. Operators get a list-of-checkboxes prompt for epic ids
   when they pick `epics`.
3. Non-interactive without `--scope` returns
   `HewError::MissingFlag { flag: "scope" }` (exit 2). Agents calling
   agents *must* pass `--scope` explicitly — there is no fallback to
   "everything is ready," because that's how a parallel `--jobs N`
   run accidentally consumes the rest of the graph.

Scope is persisted on the run's `run.json` as the `scope` field:

```json
{ "scope": { "kind": "ready" } }
{ "scope": { "kind": "epics", "epic_ids": ["hew-6az"] } }
```

Pre-scope `run.json` files (no field) load as `None` and `hew loop
summary` renders them as `scope: ready (legacy)` so post-mortems can
tell "scope defaulted before the field existed" apart from
`Some(Ready)` ("operator explicitly chose ready").

**v1 non-goals:** priority / label / branch filters, a persistent
config default knob, mid-run scope changes. `/hew:auto` is already
epic-scoped (per `hew-6n0v`) and stays out of this surface.

---

## Batch planner

Parallel runs (`--jobs N >= 2`) need to choose *which* of the bd-ready
tasks dispatch this iter. The dispatcher layers two informed signals on
top of `bd ready`, with `bd ready` itself as the safety floor:

1. **Iter agent's `next_iteration:` block.** The previous iter's close
   output can name task ids the dispatcher should consider next. Cheapest
   signal — already part of the iter's token budget; no extra subprocess.
2. **Planner subprocess.** Spawned between iters *only* when (1) is
   absent. Bounded by `loop.planner.budget_tokens` (default `10_000`).
   When the budget would be exceeded the planner skips entirely rather
   than truncating its context to fit.
3. **Floor: `bd ready`.** The dispatcher always intersects the chosen
   batch with the live `bd ready` set. Suggestions can only *narrow* the
   candidate set, never expand it — see
   `DECISION:loop-batch-planner-floor` and
   `DECISION:loop-parallel-overlap-policy`.

The cascade is **agent → planner → trust-the-graph**. If the agent
emits `next_iteration:`, that wins. Otherwise the planner runs (if
budgeted). If neither produces a usable batch (no agent block, planner
skipped or declined), the dispatcher falls through to `bd ready` order
exactly as a serial run would.

**Each iter persists a `batch-NNN.json` artifact** to the run dir:

```
.hew/loop/<run-id>/batch-001.json
.hew/loop/<run-id>/batch-002.json
...
```

Schema (`schema_version: 1`):

```json
{
  "schema_version": 1,
  "iter_number": 3,
  "task_ids": ["hew-aaa", "hew-bbb"],
  "source": "agent",          // "agent" | "planner" | "skipped"
  "reason": null,             // populated on "skipped" (e.g. "planner budget exhausted")
  "created_at": "2026-05-30T00:00:00Z",
  "planner_tokens": null      // {input,output,cache_read,cache_create} when source="planner"
}
```

A future `hew loop graph` (`hew-m7lq`) consumes these artifacts to
render the dispatch history.

**End-of-run summary** rolls the counts up into one line, right after
`scope:`:

```
planner:   agent=4, runtime=2, fallback=1
```

`agent` = iter-suggested batches, `runtime` = planner-subprocess
batches, `fallback` = skipped batches that fell through to bd-ready
order. The line is omitted entirely when no `batch-*.json` files exist
(serial run, or a parallel run that crashed before the first iter).

### Configuration

```toml
[loop.planner]
enabled = true              # master switch; false disables the planner subprocess layer
budget_tokens = 10_000      # hard cap; planner skips rather than truncates
```

CLI overrides on `hew loop run`:

| Flag                     | Effect                                              |
|--------------------------|-----------------------------------------------------|
| `--no-planner`           | Disable the planner-subprocess layer for this run. The iter agent's `next_iteration:` block still drives the batch when present; otherwise the dispatcher falls through to `bd ready`. |
| `--planner-budget N`     | Override `loop.planner.budget_tokens` for this run. |

**v1 wire-up:** Only triggers when `--jobs >= 2`. `--jobs=1` skips the
planner layer entirely — there's nothing for it to narrow.

**Non-goals (v1):** replacing trust-the-graph; static touches-overlap
analysis; cross-run batch memory; retroactive recovery of hung iters.

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
- `commands/auto.md` — the `/hew:auto` slash: in-conversation,
  epic-scoped walk (one session, one epic, mid-loop steering).
- `hew_core::runner` — pure precedence logic for stop signals.
- `hew_core::prompt` — cache-disciplined assembler.
- `hew_core::runtime` — Claude Code spawner.
- `hew_core::backpressure` — gate verdict logic.
- `hew_core::stop_signals` — I/O-side signal gathering.
- `hew_core::loop_log` — per-run atomic logging layout.
- Epic `hew-gr1` — the 10 design decisions, plus the task graph that
  built the loop.
