# Hew

> Carve code, not chaos.

A methodology and CLI for AI coding agents (Claude Code, Cursor, Codex,
Windsurf) that replaces markdown-based planning with **Beads** — a
dependency-aware graph issue tracker backed by Dolt.

The agent doesn't read `PLAN.md`. It runs `bd ready` and gets the next
unblocked task. It doesn't lose state across sessions. The graph
remembers.

## The pitch

**GSD but the agent actually tracks what it's doing.**

GSD stores everything — plans, task status, decisions — in markdown
files inside `.planning/`. The agent must read those, parse YAML
frontmatter, and infer state from prose. Dependencies between tasks
live in English, not in a structure. The LLM may or may not honor
them. State drifts. Sessions die. Context bloats. Sometimes the agent
just makes things up.

Hew replaces that with a real graph. Six commands, one mental model:

| Command | What it does |
|---------|--------------|
| `bd create "Title" -p N` | Create a task with a priority |
| `bd dep add <child> <parent>` | Add a dependency |
| `bd ready` | List tasks with no open blockers (JSON) |
| `bd update <id> --claim` | Atomically claim a task |
| `bd close <id> "Summary"` | Mark task done |
| `bd remember "insight"` | Persistent project memory |

The agent loops through them. `hew prime <skill>` injects the project
state + the relevant skill instructions in one JSON blob. No 41-file
workflow system. No 22 specialized sub-agents. One agent, one
methodology, one CLI.

## What ships

- **`hew` CLI** (Rust binary, single-file install via cargo-dist) —
  `hew init`, `hew prime`, `hew status`, `hew doctor`, `hew config`,
  `hew schema`, `hew update`.
- **14 skills + index** — markdown instruction files installed into
  your agent runtime's skill directory.
  - core (5): `hew-plan`, `hew-decompose`, `hew-execute`, `hew-verify`,
    `hew-guard`
  - brownfield (5): `hew-scan`, `hew-convention`, `hew-audit`,
    `hew-boundary`, `hew-migrate`
  - optional (4): `hew-deps`, `hew-research`, `hew-quick`, `hew-security`

## Quick start

### Install Beads

```sh
brew install beads
# or
curl -sSL https://beads.sh/install | sh
```

### Install hew

```sh
brew install droidnoob/hew/hew
# or
curl -sSL https://hew.sh/install | sh
```

### Wire it into your project

```sh
cd your-project
hew init
```

`hew init` detects your agent runtime (Claude Code, Cursor, Codex,
Windsurf), runs `bd init` if needed, adds `.beads/` to `.gitignore`,
and writes the skill files into the right place. It is non-interactive
by default; pass `--runtime=...` and `--scope=...` to override.

### Open a session and tell your agent what to do

The skills route on intent. Your first sentence might be:

> "Plan and start building a feature: passwordless email-link auth for
> the API. Use `hew-plan`, then `hew-decompose`, then `hew-execute`."

Or just:

> "What's next?"

…and the agent runs `hew prime execute`, sees the ready queue, claims
the top item, does the work, runs `hew-guard`, closes the task,
commits, loops. You watch.

## The workflow

```
plan → decompose → (ready → claim → execute → guard → close) → verify
```

- **plan** — strategic, goal-backward. Hold the plan in conversation;
  never write `PLAN.md`.
- **decompose** — translate the plan into a Beads graph (epics, tasks,
  gates, bonds). Pick the right shape: flat for tiny work, single epic
  for one feature, multi-epic + `bd mol bond` for big builds.
- **execute** — the work loop. Claim, code, test, run `hew-guard`,
  close, commit. Four deviation rules cover the messy reality:
  auto-fix bugs (R1), auto-add critical correctness (R2), auto-fix
  blocking issues (R3), ask about architectural changes (R4).
- **guard** — pre-close sanity gate. Seven checks (debug statements,
  secrets, TODOs, lint, types, tests, conventions). Blocks `bd close`
  on any fail.
- **verify** — post-batch, end-to-end. Four dimensions: tests,
  acceptance criteria, boundaries, golden path.

Each skill has its own markdown file. The agent loads the one it
needs via `hew prime <skill>`.

## How it compares

| | GSD v1 | GSD v2 | Hew |
|--|--------|--------|-----|
| State | markdown files | TypeScript + DB | Beads (Dolt) |
| Crash recovery | none | yes (migration heavy) | free |
| Brownfield support | poor | better | first-class |
| Token overhead | 4:1 typical | 4:1 typical | < 5% target |
| Commands | 42 | 42 | 22 |
| Skills/agents | 22+ | 22+ | 14 |
| Workflow files | 41 | 41 | 0 |
| Installs into project | yes (heavy) | yes (heavy) | one binary |

## What this is not

- **Not a framework.** No runtime in your project. The `hew` binary
  runs locally; the skill files are plain markdown.
- **Not a TypeScript application.** No build step. No node_modules.
- **Not multi-agent orchestration.** This is a methodology for a
  single agent working through a graph. Multi-agent is a separate
  problem.
- **Not a replacement for Beads.** It's a methodology that uses
  Beads. Beads is the tool; this is the workflow.

## Memory prefix taxonomy

Every `bd remember` follows a convention. The executor treats prefixes
differently:

| Prefix | Meaning | Treatment |
|--------|---------|-----------|
| `STATUS:` | phase completion flag | routes the agent |
| `CONVENTION:` | prescriptive coding rule | constraint — do not violate |
| `BOUNDARY:` | API contract / public interface | check before changing |
| `AUDIT:` | dependency health finding | may open tasks |
| `SECURITY:` | security decision or pattern | check on auth/input code |
| `MIGRATION:` | DB schema change | match in code + migration file |
| `DEP:` | new dependency evaluation | informational |
| (none) | factual codebase knowledge | context |

## Roadmap

The first release ships the methodology + CLI together. After that:

- **Cursor / Codex / Windsurf adapters** for `hew init` (Claude works today).
- **`hew schema`** integration with agents (validating prime output).
- **Slash commands** for Claude Code (`/hew:do`, `/hew:next`, `/hew:auto`).
- **Examples** — greenfield, brownfield, bug fix walkthroughs.
- **Multi-agent support** — once Beads' multi-agent story lands.

Track progress with `hew status` against the repo's own Beads graph.

## License

MIT. See [LICENSE](LICENSE).

## Acknowledgments

Built on [Beads](https://gastownhall.github.io/beads/) by the Gastown
Hall team. Distilled from observing what works (and what doesn't) in
[GSD](https://github.com/gsd-build/get-shit-done) and other AI-agent
methodologies.
