# Hew

> Carve code, not chaos.

A methodology and CLI for AI coding agents that replaces markdown-based
planning with [Beads](https://gastownhall.github.io/beads/) — a
dependency-aware graph issue tracker backed by Dolt.

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
- **One binary, no runtime in your repo.** 14 skill markdown files
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

## Getting started

Three paths in. Pick the one that matches your starting point.

### A. New project, no Beads yet

```sh
cd your-project
hew init --install-bd=brew      # also installs Beads if missing
```

`hew init` detects your agent runtime (Claude Code, Cursor, Codex,
Windsurf), runs `bd init`, installs the 14 skills + 23 slash commands,
and gitignores `.beads/`. Open the agent and say:

> Plan and start building &lt;thing&gt;. Use `/hew:plan`.

The agent runs `hew prime plan`, walks goal-backward, decomposes into
a Beads graph, then enters the work loop.

### B. Existing codebase

```sh
cd existing-project
hew init --install-bd=brew
```

Then in the agent:

> This is an existing codebase. Run `/hew:scan` first, then plan a
> feature: &lt;description&gt;.

The agent walks the codebase, persists architecture as discrete
`bd remember` facts (no `CODEBASE.md` summary file), then extracts
`CONVENTION:` rules, audits dependencies, and maps API boundaries
before planning the feature. Brownfield work that respects existing
patterns, every session.

See [`examples/brownfield-feature/walkthrough.md`](./examples/brownfield-feature/walkthrough.md)
for the full flow.

### C. Hew is already installed (globally), new repo

If you installed hew globally (`hew init --scope=global` in any prior
project) the skills are already loaded in your agent runtime. You don't
need to run `hew init` again — just talk to the agent:

> Open my Claude Code in `~/repos/new-thing` and run `/hew:do`. It's a
> new repo; set up Beads, scan if there's code, and plan.

The agent runs `hew init` itself (it's on PATH), then follows the same
flow as A or B depending on what it finds.

## What ships

- **`hew` CLI** — `init`, `prime`, `status`, `doctor`, `config`,
  `schema`, `update`, `completions`, `manpage`. Every command supports
  `--json` and `--non-interactive`.
- **14 skills + 1 index** under [`skills/`](./skills) — installed
  verbatim into the agent runtime.
- **23 slash commands** under [`commands/`](./commands) — `/hew:plan`,
  `/hew:next`, `/hew:auto`, `/hew:quick`, `/hew:verify`, `/hew:ship`,
  `/hew:doctor`, `/hew:config`, …

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
