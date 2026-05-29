# Hew Slash Command Reference

Full reference for every `/hew:*` slash command shipped by `hew init`. 39 entries grouped by purpose.

> Descriptions below are pulled verbatim from each command's frontmatter at [`commands/<name>.md`](../commands/). If you add a new slash, update both the registry test counts (see `GOTCHA:test-counts-drift`) and this file.

---

## Daily loop

The slashes most users hit every session.

### `/hew:do`

Route freeform user input to the right hew skill.

- Skill: routes to whichever skill best matches the prompt
- Use when: you can't remember which slash applies

### `/hew:next`

Pick and start the highest-priority unblocked task.

- Skill: invokes `hew-execute` after `bd ready`
- Prereqs: at least one ready task in the graph

### `/hew:work`

Enter the work loop (alias for `/hew:next` when a graph exists).

- Skill: same as `/hew:next`
- Use when: you want explicit "go" semantics

### `/hew:quick`

Fast mode: one task, no plan/decompose overhead.

- Skill: `hew-quick`
- Use when: the change is < 30 minutes of work and behavior-free or trivially testable

### `/hew:auto`

In-conversation, epic-scoped driver. Walks the children of one active
epic in dependency order, tail-calling `/hew:next` per task, in a
single Claude session. For the subprocess loop that drains the global
ready queue with cache-warm prefixes and on-disk per-iter logs, use
`/hew:loop`.

- Use when: you want one session focused on one epic with mid-loop steering

### `/hew:loop`

Drive the autonomous outer loop at the process level. Each iter is a
fresh `claude -p` subprocess; cache-disciplined prompt prefix keeps
Anthropic's cache warm; per-iter test + lint runs as a backpressure
gate (failed iters → `git reset --hard`); Ctrl+C produces clean
`stop_reason: cancelled`; coloured summary at end.

- CLI: `hew loop run [--max-iter N] [--until-empty] [--unattended] [--budget-tokens N] [--budget-wall 30m] [--dry-run]`
- Inspect: `hew loop list`, `hew loop logs --tail 5`
- Cancel: `hew loop cancel` (or Ctrl+C in the running shell)
- Use when: draining many ready tasks across one long run that
  survives chat-session limits. Full guide: [`docs/LOOP.md`](./LOOP.md).

### `/hew:status`

Print human-readable project state (shells to `hew status`).

- CLI shell-out: `hew status`
- Shows phases, task counts, memory tallies

### `/hew:report`

Generate a session summary for stand-ups.

- Skill: `hew-report`
- Output: a one-screen recap of what closed since the last checkpoint

### `/hew:checkpoint`

Save rich session state to a CHECKPOINT memory before context reset.

- Skill: `hew-checkpoint`
- Use before: `/clear`, long break, end of day

---

## Planning + decompose

### `/hew:plan`

Strategic planning via hew-plan skill.

- Skill: `hew-plan`
- Ends with: research-or-decompose tail picker (honors `research.default`)
- Captures: `DECISION:` memories for load-bearing choices

### `/hew:decompose`

Translate an approved plan into a Beads task graph — epic + child tasks + dependency edges. Runs the hew-decompose skill.

- Skill: `hew-decompose`
- Prereqs: a plan in context (or an explicit `<epic-id>` to re-decompose)
- Writes: `STATUS:decompose:complete for <epic-id>`

### `/hew:spec`

Score the user's ask on goal-clarity + acceptance-clarity; loop Socratic questions until the ambiguity gate passes (or 4 rounds elapse).

- Skill: `hew-spec`
- Use before: `/hew:plan` when the ask is vague

### `/hew:research`

Ad-hoc topic research with web search + cited findings. Persists RESEARCH memories with `[VERIFIED]` / `[CITED]` / `[ASSUMED]` provenance tags.

- Skill: `hew-research`
- Opt-in: `hew config set optional-skills.research true`

### `/hew:new-project`

Bootstrap a new project from a 1–3 sentence outline.

- Skill: `hew-new-project`
- Refuses if `STATUS:new-project:complete` is set (pass `--re-bootstrap` to override)
- Produces: PROJECT / DECISION / CONVENTION / CONVENTION:craft / RESEARCH / ROADMAP / MILESTONE memories + first-milestone epic

---

## Execution + close

### `/hew:verify`

End-to-end verification after a batch closes.

- Skill: `hew-verify`
- Five dimensions: full tests, acceptance criteria, boundary regressions, golden path, maintainability (craft drift across the batch)

### `/hew:ship`

Create a PR and (optionally) a gate for the next epic.

- Skill: `hew-ship`
- Prereqs: `STATUS:verify:complete` (or `/hew:verify` passed in this session)

### `/hew:test`

Generate tests for the most recently completed work.

- Skill: `hew-test`
- Use when: code closed without a co-changed test and the gate caught it

### `/hew:review`

Friendly second-pass code review against CONVENTION/BOUNDARY/SECURITY memories. Files findings as bd bugs.

- Skill: `hew-review`
- Reads: `hew review bundle` (closed tasks in scope, diff, applicable memories, epic body)
- Writes: `STATUS:review:<ts>` marker only; findings go to `bd` as bug/chore tasks

### `/hew:adversarial-review`

Red-team / steelman pass — attacks undocumented gaps the friendly review can't see. Files findings as bd bugs.

- Skill: `hew-adversarial-review`
- Pair with: `/hew:review` for two-pass coverage

> Note: `hew-guard` (the pre-close gate) has no slash by design — it runs inline before every `bd close` via `hew-execute`. Invoking it speculatively is what the skill body forbids.

---

## Brownfield chain

Run in order on a new-to-you codebase before any feature planning. Each step's `STATUS:*` marker gates the next.

### `/hew:scan`

Architecture-mapping pass for a brownfield codebase — first link in the scan → convention → audit → boundary chain. Persists findings as factual + STATUS:scan memories.

- Skill: `hew-scan`
- Writes: `STATUS:scan:complete`

### `/hew:convention`

Extract prescriptive CONVENTION rules (and CONVENTION:craft.<id> picks) from an existing codebase. Second link in the brownfield chain.

- Skill: `hew-convention`
- Prereqs: `STATUS:scan:complete`
- Writes: `STATUS:convention:complete` + N × `CONVENTION:*` memories + ≥3 × `CONVENTION:craft.*`

### `/hew:audit`

Dependency health check + craft-drift audit. Third link in the brownfield chain. Files AUDIT memories and opens bug tasks for clear-cut findings.

- Skill: `hew-audit`
- Prereqs: `STATUS:scan:complete`
- Writes: `STATUS:audit:complete`

### `/hew:boundary`

Map the public API + interface boundaries of a brownfield codebase. Fourth link in the chain. Persists BOUNDARY contracts.

- Skill: `hew-boundary`
- Prereqs: `STATUS:scan:complete`
- Writes: `STATUS:boundary:complete`

### `/hew:migrate`

Detect DB-schema drift between code models and migration files. Persists MIGRATION memories and flags mismatches.

- Skill: `hew-migrate`
- Use when: model changes ship without matching migrations

---

## Memory + capture

### `/hew:note`

Zero-friction capture as a NOTE: memory.

- Shells to: `hew remember --raw "NOTE:<arg>"`
- Use for: ephemeral thoughts you want surfaced in the next `hew prime resume`

### `/hew:ingest`

Ingest external docs into hew remember entries.

- Skill: `hew-ingest`
- Use when: bringing prior planning docs (PRD, ADR, design notes) into the memory store

### `/hew:compact`

Compact a noisy memory prefix from N entries down to 1–2 canonical entries per logical sub-cluster. Dry-run by default.

- Skill: `hew-compact`
- Safety: dry-run default, `[compacted-from: …]` provenance suffix, drift-guard, exempt allowlist (per `DECISION:compact-*` memories)
- Refuses: `STATUS:scan/convention/plan/decompose` (hardcoded exempt)

---

## Task graph CRUD

### `/hew:add`

Add a new task to the existing graph.

- Shells to: `hew task new` with sensible defaults
- Use when: discovering work mid-session that doesn't belong in the current task

### `/hew:drop`

Remove a task and its subtasks from the graph.

- Shells to: `hew task close --reason "dropped: <reason>"` or `bd delete` depending on graph state
- Destructive — confirms before running

### `/hew:epic`

CRUD ops on epics: new, close, audit, summary, gaps, bond, tree.

- Shells to: `hew epic <op>`
- One-stop for epic-level inspection and management

---

## Lifecycle + diagnostic

### `/hew:debug`

Systematic debugging with persistent state.

- Skill: `hew-debug`
- Persists debug session across context resets via `DEBUG:` memories

### `/hew:forensic`

Investigate what went wrong in a failed build.

- Skill: `hew-forensic`
- Use after: a verify failure or a production incident

### `/hew:resume`

Reload the session-resume context (project state + STATUS flags + memories + latest CHECKPOINT). Manual re-run of the SessionStart hook.

- Shells to: `hew prime resume`
- Use after: `/clear`, mid-session context drift

### `/hew:prime`

Emit JSON context for a specific skill (consumed by the agent). Manual invocation of the hew prime <skill> primer.

- Shells to: `hew prime <skill>`
- Use when: an agent has lost track of a skill's prerequisites

---

## Optional skills

These slashes ship by default but their underlying skill is opt-in via `hew config set optional-skills.<name> true` to persist across `hew update`.

### `/hew:deps`

Evaluate a candidate new dependency before adding it. Persists a DEP memory with verdict (adopt / hold / reject) + rationale.

- Skill: `hew-deps`
- Config: `optional-skills.deps`

### `/hew:security`

Lightweight security check on auth + input + secret paths. Persists SECURITY memories; opens bug tasks for clear-cut findings.

- Skill: `hew-security`
- Config: `optional-skills.security`
- Distinct from: `hew-guard`'s per-task secret check (inline) and `hew-review`'s SECURITY pillar (batch-level)

---

## Admin

### `/hew:doctor`

Diagnose hew + beads + project health (shells to `hew doctor`).

- CLI shell-out: `hew doctor`
- Checks: `bd` on PATH, hooks wired, settings.json valid, etc.

### `/hew:config`

Read/write hew configuration (shells to `hew config`).

- CLI shell-out: `hew config get|set|keys|list`
- Config lives at `<XDG_CONFIG_HOME>/hew/config.toml`

### `/hew:help`

List hew commands with descriptions.

- Shells to: `hew commands`
- Equivalent to this document in terminal form

### `/hew:update`

Self-update binary + skill files (shells to `hew update`).

- CLI shell-out: `hew update`
- Updates: the `hew` binary, embedded skills, embedded slash commands

---

## CLI-only surfaces

Not slashes, but referenced by skill bodies and worth knowing:

| Command | What it does |
|---------|--------------|
| `hew prime <skill>` | Emit JSON context for a skill (consumed by agents) |
| `hew prime resume` | The SessionStart-hook payload — project state + memories + checkpoint |
| `hew status` | Human-readable project state |
| `hew memories [--prefix\|--grep\|--research\|--recall\|--forget]` | List, filter, recall, or forget memories |
| `hew memories --export [-o PATH] [--plaintext]` | Dump filtered memories to a file. Default format JSON; `--plaintext` for text. Default path `<projname>-memories-<iso-ts>.<ext>` when `-o` is omitted. |
| `hew remember --type=<allowlist> "<body>"` | Write a memory with type validation; `--raw` to bypass |
| `hew remember --from-file <path>` | Bulk insert from a JSON array of `{type, body, key?, raw?}` entries. All-or-nothing: every entry validated before any write. |
| `hew task {show,list,claim,close,new,reopen,children,note,search}` | Curated `bd` wrappers for task ops |
| `hew dep {add,remove,tree,blocked}` | Curated `bd` wrappers for dependency ops |
| `hew epic {list,show,tree,close,audit,summary}` | Epic-level operations |
| `hew compact {apply,list-prefixes}` | Memory compaction CLI (called by `/hew:compact` skill) |
| `hew review bundle` | Assemble the bundle the review skills consume |
| `hew schema <name>` | JSON Schema for prime / resume / config / review-bundle / task / epic / craft-principles / compact-plan / etc. |
| `hew commands` | List installed slash commands (same content as this doc) |
| `hew skills [--category]` | List installed skills |
| `hew check <skill>` | Exit 0 if the skill's prerequisites are met |
| `hew doctor [--fix]` | Project + tooling health diagnostics |
| `hew config {get,set,keys,list}` | Configuration |
| `hew init [--runtime=<r>[,<r>...]]` | Install hew into the current project. `--runtime` accepts CSV (`--runtime=claude,codex`) or repeated flags (`--runtime=claude --runtime=codex`); omit to auto-detect or pick interactively. |
| `hew uninstall [--runtime=<r>] [--purge]` | Reverse `hew init` |
| `hew update` | Self-update binary + embedded artifacts |
| `hew branch new --prefix=<type> --slug=<text>` | Create a conventional branch |
| `hew statusline [--compact\|--full] [--width=N]` | One-line agent statusline (scope · bar · pct · phase · epic-fraction · user). Auto-wired into Claude Code by `hew init`. |

---

## Adding a new slash

Per `GOTCHA:test-counts-drift`, every new slash bumps **three** counts:

1. `hew-core/src/slash.rs` — add to `ALL` + bump the `ships_N_commands` test
2. `hew-core/src/install.rs` — bump `install_claude_writes_every_skill_and_slash_command` total
3. This document — add an entry under the appropriate category

Forget any of the three and `cargo test` will fail (the first two) or your reference docs will rot (the third).
