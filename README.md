# Hew

**Carve code, not chaos.**

A methodology and CLI for AI coding agents, backed by [Beads](https://gastownhall.github.io/beads/) — a dependency-aware graph issue tracker on Dolt.

Hew puts project state in the graph (not in markdown files) so the agent queries, not narrates, what to do next.

---

## Why I Built This

GSD and similar frameworks ask the LLM to be the source of truth for project state. State lives in `PLAN.md` / `TODO.md` / `STATE.md`; the agent re-parses it every session; dependencies live in English prose.

State drifts. Context bloats. Crash recovery requires migration scripts. The agent makes things up.

Hew inverts that. **Beads is the graph. The agent queries it.**

- **Real dependency resolution.**
  `bd ready` returns JSON of unblocked tasks. The agent doesn't reason about whether task 7 can start — the graph does.

- **Crash recovery is free.**
  Sessions die mid-task; the Dolt-backed graph keeps state. Next session: `hew prime execute` shows exactly where you stopped.

- **Brownfield first-class.**
  A separate skill chain (`hew-scan` → `hew-convention` → `hew-audit` → `hew-boundary`) maps an existing codebase into discrete `bd remember` facts before any planning starts.

- **One binary, no runtime in your repo.**
  20 skill markdown files installed into your agent runtime's skill directory. No 41-file workflow system, no 22 specialised sub-agents.

---

## How It Works

The daily loop is `plan → decompose → execute → verify`, repeating until the milestone closes.

```text
plan        →   write the goal-backward plan; capture DECISION: memories
decompose   →   translate plan into Beads tasks with deps + acceptance
execute     →   for each ready task: claim → code → guard → commit → close
verify      →   batch-level check that the epic actually delivers
```

Three things keep the loop coherent across sessions:

1. **The graph is the state.**
   `bd ready` knows what's next. `bd close` marks it done. There is no `PLAN.md` to forget about.

2. **Memories are constraints, not context.**
   Every `CONVENTION:`, `BOUNDARY:`, `SECURITY:`, `DECISION:` memory is a load-bearing rule the executor reads before writing code. The taxonomy keeps prefix lookup cheap.

3. **Skills are stateless contracts.**
   Each skill body is a markdown file with deterministic inputs (`hew prime <skill>` JSON) and outputs. The same skill behaves the same way in session 1 and session 50.

<br>

### New project from scratch

For genuinely greenfield projects (empty Beads graph, no `PROJECT:` memories yet), route through `/hew:new-project` first.

```text
/hew:new-project "Bootstrap a SaaS that <one-to-three-sentence outline>"
```

The `hew-new-project` skill runs once and produces the project foundation:

1. **Capture + Socratic clarifying** — 4–6 questions on target user, scale, deployment, constraints, non-goals, monetization.

   Each answer persists as a `PROJECT:` memory.

2. **Parallel research** — four threads (idea/competitive, use-cases, tech-stack, architecture-patterns) via the agent's Agent tool.

   Findings persist as `RESEARCH:` memories with `[VERIFIED]` / `[CITED]` / `[ASSUMED]` provenance tags.

3. **Synthesis pickers** — stack family (one of `ts-next`, `py-fastapi`, `rust-axum`, `go-echo`, or `custom`), craft principles (SOLID / DRY / Clean Architecture / etc., per `DECISION:craft-adaptive`), database, auth model, hosting.

4. **Milestone vocabulary** — pick from `Foundation → MVP → Hardening → Launch` (slow-roast), `Foundation → MVP → Launch → Hardening` (ship-fast), `Discovery → Build → Stabilize → Ship`, or custom 3–5 names.

5. **Roadmap construction** — one `hew task new --type=epic` per milestone, sequenced via `hew dep add`.

6. **First-milestone decompose** — invokes `hew-decompose` on the first milestone only.

   Later milestones decompose on demand.

Writes `STATUS:new-project:complete`; user moves to `/hew:next` to start work.

<br>

### Existing project

For projects with an existing graph, the daily loop is `plan → decompose → execute`.

```text
/hew:plan "Add passwordless email-link auth for B2B users"
```

The agent walks goal-backward through `hew-plan`, decomposes into a Beads graph via `hew-decompose`, then enters the work loop: `/hew:next` claims the highest-priority unblocked task, codes it, runs `hew-guard`, closes it, commits.

Repeat until `hew prime execute` shows no ready tasks, then `/hew:verify`.

<br>

### Existing codebase

For brownfield projects, the agent runs the onboarding chain before any feature work:

```text
/hew:scan          # architecture mapping → STATUS:scan:complete
/hew:convention    # extract CONVENTION:* rules → STATUS:convention:complete
/hew:audit         # dep health + craft drift → STATUS:audit:complete
/hew:boundary      # public API contracts → STATUS:boundary:complete
```

Subsequent planning respects existing patterns because the `CONVENTION:` memories are mandatory constraints for the executor.

See [`examples/brownfield-feature/walkthrough.md`](./examples/brownfield-feature/walkthrough.md) for the full flow.

<br>

### One-off fix

For small, scoped changes:

```text
/hew:quick "Fix off-by-one in the pagination cursor"
```

Routes to the `hew-quick` skill — one task, one commit, no plan/decompose overhead.

Escalates back to `/hew:plan` if the fix turns out bigger than expected.

<br>

### Craft principles

Hew ships a catalogue of 28 craft principles (SOLID, DRY, KISS, Clean Architecture, Hexagonal, DDD, Idempotence, Fail Fast, Pure Functions, …) at `skills/data/craft-principles.toml`, exposed as `hew schema craft-principles`.

The catalogue is **adaptive, not universal**. Projects pick their subset at bootstrap:

- Greenfield → via `hew-new-project` Phase C
- Brownfield → via `hew-convention` Step 11

Each chosen principle persists as `CONVENTION:craft.<id>`. `hew-guard` reads them and surfaces soft warnings (`missing-tests`, `function-length`, `duplication`) on close.

The `consistency-with-existing-code` meta-principle defaults on every stack — brownfield convention always beats a freshly-picked rule.

> Per `DECISION:craft-enforcement`: warnings never block close on their own. `testing.require=true` is the one current promotion path from warn to fail.

---

## Getting Started

### Install

**macOS / Linux:**

```sh
curl --proto '=https' --tlsv1.2 -LsSf \
  https://github.com/droidnoob/hew/releases/latest/download/hew-installer.sh | sh
```

**Windows:**

```powershell
powershell -ExecutionPolicy ByPass -c "irm https://github.com/droidnoob/hew/releases/latest/download/hew-installer.ps1 | iex"
```

**Homebrew:**

```sh
brew install droidnoob/tap/hew
```

**From source** (Rust toolchain required):

```sh
cargo install --git https://github.com/droidnoob/hew hew
```

<br>

### Initialize in your project

```sh
cd <your-project>
hew init
```

`hew init` detects your agent runtime (Claude Code / Cursor / Codex / Windsurf / Generic) and installs the right skill + slash command layout.

It also wires a `SessionStart` hook on Claude Code so `hew prime resume` runs automatically on every session entry.

<br>

### Daily flow

Open your agent (Claude Code, Cursor, etc.) and route on intent — skills auto-route via `/hew:do` or direct slashes:

```text
/hew:plan       Plan a feature; ends with the research-or-decompose picker
/hew:decompose  Translate the plan into Beads tasks
/hew:next       Claim the top unblocked task and start work
/hew:verify     End-to-end verification after a batch closes
/hew:quick      Fast mode — one task, no plan overhead
/hew:auto       Run plan → decompose → execute → verify autonomously
```

---

## Commands

A brief table of the most-used slashes. Full reference: [`docs/COMMANDS.md`](./docs/COMMANDS.md).

| Slash | What it does |
|-------|--------------|
| `/hew:do` | Free-form router — pick the right skill from your prompt |
| `/hew:plan` | Strategic planning via hew-plan (goal-backward, decisions captured) |
| `/hew:decompose` | Translate an approved plan into a Beads task graph |
| `/hew:next` | Claim the highest-priority unblocked task and start coding |
| `/hew:quick` | One task, one commit — no plan/decompose overhead |
| `/hew:verify` | Batch-level end-to-end verification |
| `/hew:ship` | Create a PR and prepare for merge after verify passes |
| `/hew:auto` | Run plan → decompose → execute → verify autonomously |
| `/hew:checkpoint` | Save rich session state before `/clear` |
| `/hew:status` | Human-readable project state |
| `/hew:compact` | Reduce a noisy memory prefix from N entries to ~K canonical ones |

**39 total slashes** covering:

- Brownfield onboarding — `/hew:scan`, `/hew:convention`, `/hew:audit`, `/hew:boundary`, `/hew:migrate`
- Reviews — `/hew:review`, `/hew:adversarial-review`
- Capture — `/hew:note`, `/hew:remember`, `/hew:ingest`
- Lifecycle — `/hew:debug`, `/hew:forensic`, `/hew:resume`, `/hew:prime`

---

## Why It Works

Three design choices make the loop survive scale.

**State lives in the graph.**

Beads is a dependency-aware issue tracker backed by Dolt — every task carries `closed_at`, dependency edges, and acceptance criteria.

`bd ready` is a query, not a parse. The agent does not maintain a TODO list in its context; it asks the graph.

<br>

**Memories are typed and prefixed.**

`CONVENTION:`, `BOUNDARY:`, `SECURITY:`, `MIGRATION:`, `AUDIT:`, `DEP:`, `DECISION:`, `RESEARCH:`, `STATUS:`, `CHECKPOINT:` — each prefix tells the executor *how* to treat the body.

`CONVENTION:errors` is a constraint; `RESEARCH:auth` is context. The taxonomy makes prefix lookup cheap and routing deterministic.

<br>

**Skill bodies are deterministic contracts.**

Every skill receives the same shape of input (`hew prime <skill>` JSON: prerequisites, project state, categorized memories, skill body) and produces the same shape of output (task changes, memory writes, status flags).

The same skill behaves the same way in session 1 and session 50.

---

## Session resume

Agent context dies on `/clear`, on session compaction, or when you start a new shell. Hew restores it automatically.

**Claude Code.**
`hew init --runtime=claude` writes a `SessionStart` hook into `.claude/settings.json`. On every session entry the hook runs `hew prime resume`, which emits a JSON document with project state, `STATUS:` flags, categorized memories, and the most recent `CHECKPOINT:`.

The agent reads that on first turn — no manual `/hew:prime` step needed.

**Cursor / Codex / Windsurf / Generic.**
No `SessionStart` equivalent yet. The adapter file (`.cursorrules`, `AGENTS.md`, etc.) carries a top-of-section instruction telling the agent to run `hew prime resume` as its first action.

**Saving state before `/clear`.**
Use `/hew:checkpoint` to dump in-flight session state (current task, files touched, open hypotheses, next moves) into a `CHECKPOINT:` memory. The next session's `hew prime resume` surfaces it under `latest_checkpoint`.

---

## Configuration

Configuration lives at `<XDG_CONFIG_HOME>/hew/config.toml` (typically `~/.config/hew/config.toml`).

Inspect or change keys via `hew config get|set <key> [value]`.

Selected knobs:

| Key | Default | What it does |
|-----|---------|--------------|
| `branching.strategy` | `none` | `none` / `epic` / `always` — when to auto-create a branch on first claim |
| `research.default` | `ask` | `ask` / `auto-skip` / `auto-run` — default at `/hew:plan` tail picker |
| `testing.require` | `false` | When `true`, hew-guard fails close on missing-tests instead of warning |
| `craft.max_function_lines` | `0` | Soft-warn when a changed function exceeds this many lines (0 = disabled) |
| `compact.dry_run_default` | `true` | `/hew:compact` starts in dry-run mode unless `--apply` passed |
| `review.after_n_tasks` | `0` | Fire the review picker after this many closed tasks (0 = disabled) |

Run `hew config keys` for the full list.

---

## Removing hew

Easy to walk away from. `hew uninstall` reverses everything `hew init` wrote, runtime-by-runtime, while leaving your Beads graph and `.gitignore` intact:

```sh
hew uninstall                  # remove skills + slash commands
hew uninstall --runtime=claude # specific runtime
hew uninstall --purge --yes    # also drop .beads/ (destructive)
```

Single-file adapters (`.cursorrules`, `.windsurfrules`, `AGENTS.md`) keep any user content outside the `HEW:BEGIN/HEW:END` markers — only the hew section is removed.

---

## Documentation

| Document | What it covers |
|----------|----------------|
| [docs/COMMANDS.md](./docs/COMMANDS.md) | Full slash-command reference (39 entries, grouped by category) |
| [ARCHITECTURE.md](./ARCHITECTURE.md) | Workspace split, `BdClient` seam, prime JSON contract, memory taxonomy, craft system |
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Dev setup, MSRV, hooks, release process |
| [CHANGELOG.md](./CHANGELOG.md) | Release notes |
| [`skills/`](./skills) | The methodology itself — skill bodies the LLM loads. Read them directly. |
| [`examples/`](./examples) | Three walkthroughs: greenfield SaaS, brownfield feature add, single-bug fix |

---

## Troubleshooting

**`hew prime <skill>` is silent / hangs.**

Likely a `bd` invocation got stuck on a pipe-buffer deadlock for a very large memory store. `hew` internally routes big-output `bd` queries through temp files via `read_via_temp`; if you see hangs, run `hew doctor` to confirm `bd` is on PATH and responsive.

<br>

**Skill registry test counts mismatch on `cargo test`.**

You probably added a new skill or slash command without bumping the three hardcoded counts (`ship_one_index_plus_N_skills`, `install_claude_writes_every_skill`, `install_codex_writes_per_skill_toml`).

See `GOTCHA:test-counts-drift` for the canonical list.

<br>

**`/hew:compact` refuses to compact a memory.**

That memory is either in the hardcoded exempt list (`STATUS:scan`, `STATUS:convention`, `STATUS:plan`, `STATUS:decompose`) or already carries a `[compacted-from:` provenance suffix and the drift-guard kicked in.

Pass `--allow-recompact` only if you intentionally want to re-compact; see `DECISION:compact-drift-guard`.

<br>

**Agent doesn't see the latest project state on session start.**

On Claude Code, confirm the `SessionStart` hook is wired:

```sh
cat .claude/settings.json | grep hew
```

On other runtimes, the adapter file's top-of-section instruction should tell the agent to run `hew prime resume` as its first action.

---

## License

MIT. See [LICENSE](./LICENSE).

Built on [Beads](https://gastownhall.github.io/beads/) by the Gastown Hall team.

Distilled from observing what works (and what doesn't) in [GSD](https://github.com/gsd-build/get-shit-done) and other AI-agent methodologies.
