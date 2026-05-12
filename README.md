# Hew

> Carve code, not chaos.

A methodology and CLI for AI coding agents, backed by
[Beads](https://gastownhall.github.io/beads/) — a dependency-aware
graph issue tracker on Dolt.

## Why

GSD and similar frameworks ask the LLM to be the source of truth for
project state. State lives in `PLAN.md` / `TODO.md` / `STATE.md`, the
agent has to read and re-parse it every session, and dependencies live
in English prose. State drifts. Context bloats. Crash recovery requires
migration scripts. The agent makes things up.

Hew inverts that. Beads is the graph. The agent queries it.

- **Real dependency resolution.** `bd ready` returns JSON of unblocked
  tasks. The agent doesn't reason about whether task 7 can start —
  the graph does.
- **Crash recovery is free.** Sessions die mid-task; the Dolt-backed
  graph keeps state. Next session: `hew prime execute` shows exactly
  where you stopped.
- **Brownfield first-class.** A separate skill chain (`hew-scan`,
  `hew-convention`, `hew-audit`, `hew-boundary`) maps an existing
  codebase into discrete `bd remember` facts before any planning
  starts.
- **One binary, no runtime in your repo.** 15 skill markdown files
  installed into your agent runtime's skill directory. No 41-file
  workflow system, no 22 specialised sub-agents.

## Install

macOS / Linux:

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/droidnoob/hew/releases/latest/download/hew-installer.sh | sh
```

Windows:

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/droidnoob/hew/releases/latest/download/hew-installer.ps1 | iex"
```

Homebrew:

```sh
brew install droidnoob/hew/hew
```

From source (any platform with `rustup`):

```sh
cargo install --git https://github.com/droidnoob/hew --bin hew
```

## Set up a project

```sh
cd your-project
hew init
```

`hew init` detects your agent runtime (Claude Code, Cursor, Codex,
Windsurf), runs `bd init`, installs the 15 skills + 24 slash commands,
and gitignores `.beads/`. It's non-interactive by default — override
with `--runtime=...`, `--scope=...`, `--git-track`, `--prefix=...`.

Re-running is idempotent. Run it again any time to re-sync skills.

## Run inside the coding agent

Once `hew init` has run, open your agent (or just keep it open) and
talk to it. The skills route on intent.

### New project from scratch

> Starting a new project — &lt;1-3 sentence outline&gt;. Use `/hew:new-project`.

For genuinely greenfield projects (empty Beads graph, no PROJECT:
memories yet), route through `/hew:new-project '<outline>'` first.
The `hew-new-project` skill runs once and produces the project
foundation:

1. **Capture + Socratic clarifying** — 4–6 questions on target
   user, scale tier, deployment, constraints, non-goals,
   monetization. Each answer persists as a `PROJECT:` memory.
2. **Parallel research** — four threads in parallel
   (idea/competitive, use-cases, tech-stack, architecture-patterns)
   via the agent's Agent tool. Findings persist as `RESEARCH:`
   memories with provenance tags `[VERIFIED]` / `[CITED]` /
   `[ASSUMED]`.
3. **Synthesis pickers** — stack family (one of `ts-next`,
   `py-fastapi`, `rust-axum`, `go-echo`, or `custom`), database,
   auth model, hosting. Each choice persists as a `DECISION:`
   memory; the chosen stack seeds 6–8 `CONVENTION:` memories from
   the embedded `skills/data/stack-conventions.toml` table.
4. **Milestone vocabulary** — pick from four presets per the
   locked `DECISION:milestone-vocabulary`:
   - Foundation → MVP → Hardening → Launch (slow-roast)
   - Foundation → MVP → Launch → Hardening (ship-fast)
   - Discovery → Build → Stabilize → Ship (alt vocab)
   - Custom (you name 3–5 milestones)
5. **Roadmap construction** — one `hew task new --type=epic` per
   milestone, sequenced via `hew dep add <next> --on <prev>`. The
   chain persists as a `ROADMAP:` memory plus one `MILESTONE:`
   memory per epic.
6. **First-milestone decompose** — invokes `hew-decompose` on the
   first milestone only. Later milestones decompose on demand as
   prior ones close.

Hand-off writes `STATUS:new-project:complete` and the user moves to
`/hew:next` to start work. Idempotency: re-running the skill on a
project that already has the marker refuses unless
`--re-bootstrap` is passed.

### Existing project

> Plan and start building &lt;thing&gt;. Use `/hew:plan`.

For projects with an existing graph (or after `/hew:new-project`
completes), the daily loop is plan → decompose → execute.

The agent walks goal-backward through `hew-plan`, decomposes into a
Beads graph via `hew-decompose`, then enters the work loop:
`/hew:next` claims the highest-priority unblocked task, codes it,
runs `hew-guard`, closes it, commits. Repeat until `hew prime
execute` shows no ready tasks, then `/hew:verify`.

### Existing codebase

> Map this codebase, then plan a feature: &lt;description&gt;.

The agent runs the brownfield chain — `hew-scan` (architecture
mapping), `hew-convention` (extract coding rules), `hew-audit`
(dependency health), `hew-boundary` (API contracts) — before any
feature planning starts. Subsequent work respects existing patterns
because the `CONVENTION:` memories are mandatory constraints to the
executor.

See [`examples/brownfield-feature/walkthrough.md`](./examples/brownfield-feature/walkthrough.md)
for the full flow.

### One-off fix

> Fix &lt;bug&gt; — small change, no planning needed.

Routes to `/hew:quick` — one task, one commit, no plan/decompose
overhead. Escalates back to `/hew:plan` if the fix turns out to be
bigger than expected.

## Session resume

Agent context dies on `/clear`, on session compaction, or when you
start a new shell. Hew restores it automatically so you don't have
to brief the agent twice.

**Claude Code.** `hew init --runtime=claude` writes a `SessionStart`
hook into `.claude/settings.json`. On every session entry (startup,
resume, clear) the hook runs `hew prime resume`, which emits a JSON
document with project state, `STATUS:` flags, categorized memories,
and the most recent `CHECKPOINT:`. The agent reads that on first
turn — no manual `/hew:prime` step needed. The hook entry carries a
`hew_managed: true` flag so re-running `hew init` replaces it in
place rather than duplicating.

**Cursor, Codex, Windsurf, Generic.** No `SessionStart` equivalent
yet. The adapter file (`.cursorrules`, `.windsurfrules`, `AGENTS.md`,
`CLAUDE.md`) carries a top-of-section instruction telling the agent
to run `hew prime resume` as its first action in any new session.
Same effect, one extra read.

**Saving state before `/clear`.** Use `/hew:checkpoint` to dump
in-flight session state (current task, files touched, open
hypotheses, next moves) into a `CHECKPOINT:` memory. The next
session's `hew prime resume` surfaces it under `latest_checkpoint`.

## Removing hew

Easy to walk away from. `hew uninstall` reverses everything `hew init`
wrote, runtime-by-runtime, while leaving your Beads graph and
`.gitignore` intact:

```sh
hew uninstall                  # remove skills + slash commands
hew uninstall --runtime=claude # specific runtime
hew uninstall --purge --yes    # also drop .beads/ (destructive)
```

Single-file adapters (`.cursorrules`, `.windsurfrules`, `AGENTS.md`)
keep any user content outside the `HEW:BEGIN/HEW:END` markers — only
the hew section is removed.

## What ships

- **`hew` CLI** — top-level commands fall into two families:
  - **Workspace + agent contract:** `init`, `prime`, `status`,
    `doctor`, `config`, `schema`, `update`, `completions`, `manpage`,
    `check`, `skills`, `commands`, `uninstall`, `branch`, `review`.
  - **Curated `bd` wrappers** (agent-facing; stable JSON via
    `--json`, schemas exposed via `hew schema <name>`):
    - `hew task {show,list,claim,close,new,reopen,children,note,search}`
    - `hew dep {add,remove,tree,blocked}`
    - `hew epic {show,tree,close,audit,summary}`
    - `hew remember --type=<allowlist> "<body>"` (or `--raw` to
      bypass)
    - `hew memories [--prefix|--grep|--research|--recall|--forget]`

  Every command supports `--json` and `--non-interactive`.
- **19 skills + 1 index** under [`skills/`](./skills) — installed
  verbatim into the agent runtime. Skill bodies invoke the
  curated `hew` wrappers above, not raw `bd`.
- **27 slash commands** under [`commands/`](./commands) — `/hew:plan`,
  `/hew:next`, `/hew:auto`, `/hew:new-project`, `/hew:quick`,
  `/hew:verify`, `/hew:ship`, `/hew:checkpoint`, `/hew:review`,
  `/hew:adversarial-review`, `/hew:doctor`, …
- **Schemas** — `hew schema {prime,resume,config,review-bundle,
  task,epic,task-list-filter,new-task}` emits JSON-Schema for every
  agent-facing output and arg shape.

## Where to go next

- **See it in action** — three example walkthroughs in
  [`examples/`](./examples): greenfield SaaS, brownfield feature add,
  single-bug fix.
- **Understand the design** — [ARCHITECTURE.md](./ARCHITECTURE.md)
  covers the workspace split, the `BdClient` seam, the `prime` JSON
  contract, and the memory prefix taxonomy.
- **Read the methodology directly** — the skill bodies in
  [`skills/`](./skills) are the docs. The LLM loads them; you can read
  them too.
- **Contribute** — [CONTRIBUTING.md](./CONTRIBUTING.md) for dev
  setup, MSRV, hooks, release process.

## License

MIT. See [LICENSE](./LICENSE).

Built on [Beads](https://gastownhall.github.io/beads/) by the Gastown
Hall team. Distilled from observing what works (and what doesn't) in
[GSD](https://github.com/gsd-build/get-shit-done) and other AI-agent
methodologies.
