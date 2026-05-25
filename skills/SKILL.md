<!-- hew:version=0.6.1 -->
---
name: hew
description: Index of installed hew skills. Loaded by the agent on session start.
---

# Hew

Methodology for AI coding agents. State lives in Beads (`bd`), not markdown files.

## How to use this index

On every session, run `hew prime <skill>` before invoking that skill. `hew prime`
returns one JSON blob with: project state, `STATUS:` flags, prerequisites,
unblocked tasks (`hew status` ready list), categorized memories, and the embedded skill body.

**Session resume.** In Claude Code, `hew init` installs a `SessionStart` hook
that runs `hew prime resume` automatically on every session entry (startup,
resume, clear). The first turn you take already has project state, the latest
`CHECKPOINT:`, and all memories in context — no manual prime needed. Use
`/hew:checkpoint` before `/clear` to save in-flight state for the next session.

If the user describes intent in plain English, route by intent:

| User says | Skill |
|-----------|-------|
| "new project from scratch" / "bootstrap a project" | `hew-new-project` |
| "let's build / plan X" | `hew-plan` (tail picker may route to `hew-research` first) |
| "break this down" / "create tasks" | `hew-decompose` |
| "start coding" / "what's next?" | `hew-execute` |
| "fix this one bug" / "tiny tweak" | `hew-quick` |
| "did we finish?" / "verify" | `hew-verify` |
| "new codebase / map this repo" | `hew-scan` → `hew-convention` → `hew-audit` → `hew-boundary` |

## Workflow

```
plan → decompose → (ready → claim → [branch?] → execute → guard → close) → verify
```

The agent does not manage phases. It manages the dependency graph. `hew status`
always says what to do next. `hew task close` marks it done. Nothing else is required.

**Auto-branching is opt-in.** `hew config set branching.strategy epic`
makes `hew-execute` create a `<prefix>/<epic-id>-<slug>` branch on the
first claim under each epic. Default is `none` — branches stay manual
via `hew branch new --prefix=<type> --slug=<text>`.

## Skills

### Core (always installed)

- **hew-new-project** — bootstrap a project from a 1–3 sentence outline (Socratic + research + roadmap + first-milestone decompose). Runs once at project start.
- **hew-plan** — strategic planning, goal-backward reasoning, tech choices
- **hew-decompose** — translate the plan into a Beads graph (epics, tasks, gates, bonds)
- **hew-execute** — the work loop
- **hew-verify** — end-to-end verification after a batch closes
- **hew-guard** — pre-close sanity gate (lint, secrets, conventions)
- **hew-checkpoint** — save in-flight session state before context reset

### Brownfield (for existing codebases)

- **hew-scan** — architecture mapping via `hew remember`
- **hew-convention** — extract `CONVENTION:` rules from existing code
- **hew-audit** — dependency health check
- **hew-boundary** — API + interface map (`BOUNDARY:` memories)
- **hew-migrate** — schema-drift detector

### Optional (opt-in)

- **hew-deps** — inspect a candidate new library
- **hew-research** — domain research with web search
- **hew-spec** — ambiguity gate before planning (Socratic, scored)
- **hew-review** — friendly second-pass review against CONVENTION/BOUNDARY/SECURITY
- **hew-adversarial-review** — red-team / steelman pass; pairs with hew-review
- **hew-quick** — fast mode (one task, no plan/decompose)
- **hew-security** — lightweight checks on auth/input/secrets

## Custom skills

Anything in `custom/` is auto-discovered and never touched by `hew update`. Teams
put their own deploy/review/onboard skills there.

## Memory prefixes

Every `hew remember` follows a prefix convention. The executor treats prefixes differently.

| Prefix | Meaning | Treatment |
|--------|---------|-----------|
| `STATUS:` | phase completion flag | routes the agent |
| `CONVENTION:` | prescriptive coding rule | **constraint** — do not violate |
| `CONVENTION:craft.<id>` | adaptive craft principle the project picked | hew-guard soft-warns; hew-review walks each |
| `BOUNDARY:` | API contract / public interface | check before changing |
| `AUDIT:` | dependency health finding | may open tasks |
| `SECURITY:` | security decision or pattern | check on auth/input code |
| `MIGRATION:` | DB schema change | match in code + migration file |
| `DEP:` | new dependency evaluation | informational |
| (none) | factual codebase knowledge | context |

## Craft principles

Hew ships a catalog of craft principles (SOLID, DRY, KISS, Clean
Architecture, Hexagonal, DDD, Idempotence, Fail Fast, Pure Functions,
…) at `skills/data/craft-principles.toml`. The catalog is embedded
into the binary and exposed as the `CraftTable` JSON schema via
`hew schema craft-principles`.

**Adaptive, not universal.** Principles are *picked per project*, not
applied globally. Three entry points populate the project's set:

- `hew-new-project` Phase C surfaces a multi-select picker; defaults
  come from each principle's `default_for_stacks` list. Each chosen
  principle persists as a `CONVENTION:craft.<id>` memory.
- `hew-convention` Step 11 (brownfield) extracts the principles the
  codebase *already* follows — function-length distribution → suggested
  `craft.max_function_lines`; layering style → architecture principle;
  test-to-source ratio → `testing.require`; opportunistic style
  fingerprints.
- `hew-plan`'s Craft refinement step records per-feature deviations
  as `DECISION:craft-feature:<plan-id>` memories. The executor and
  reviewer read these *in addition to* the project-wide set.

**Soft warnings, not hard blocks.** `hew-guard` reads the picked set
plus per-plan deviations and emits soft warnings via
`hew_core::guard::craft_warnings(memories, diff, cfg)`. Three
heuristics ship today: `missing-tests`, `function-length` (gated on
`craft.max_function_lines > 0`), and `duplication` (gated on
`CONVENTION:craft.dry`). Soft by default; `testing.require = true`
is the one current promotion from warn to fail.

**Brownfield deference.** `CONVENTION:craft.consistency-with-existing-code`
defaults on every seeded stack. When a picked principle conflicts
with an existing `CONVENTION:` memory describing how the code is
actually written, the existing convention wins. New work does not
refactor the codebase to satisfy a craft rule the rest of the project
ignores.

The full lifecycle:

```
new-project → CONVENTION:craft.* picked
  hew-convention (brownfield) → CONVENTION:craft.* extracted
hew-plan → DECISION:craft-feature:<plan-id> per-feature deviations
hew-decompose → Tests + Craft lines per task description
hew-execute Step 5a → inline craft check before writing code
hew-guard → soft warnings on the staged diff
hew-verify → batch-level Maintainability dimension
hew-review → Craft pillar walks picked principles
hew-adversarial-review → attacks gaps left by unpicked principles
```

## Anti-patterns

Do not create planning markdown files (`PLAN.md`, `TODO.md`, `ROADMAP.md`,
`MEMORY.md`). All state belongs in Beads or `hew remember`. The filesystem is for
code, not plans.

Do not use string-prefixed task titles (`"GATE: ..."`, `"PHASE: ..."`) to fake
structural roles. Beads has native types: `--type=gate`, `--type=epic`,
`bd mol bond`. Use them.

Do not skip the `hew-guard` step before `hew task close`. Drift compounds.
