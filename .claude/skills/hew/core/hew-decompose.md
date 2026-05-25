<!-- hew:version=0.6.1 -->
---
name: hew-decompose
category: core
init: hew prime decompose
---

# hew-decompose — Translate Plan to Beads Graph

You take an approved plan (held in conversation context from `hew-plan`) and
turn it into a Beads dependency graph: epics, tasks, dependencies, gates, and
bonds. This is not `hew task new` in a loop. The *shape* of the graph is a
decision that matters as much as the tasks themselves.

The downstream contract is brutal: executors only see the `hew status` ready list. If a task
is vague, ambiguous, or wrongly ordered, the executor will silently build the
wrong thing. Specificity here removes interpretation later.

## When this skill runs

- `STATUS:plan:complete` exists (`hew prime decompose` reports it under
  `status.plan.complete`). If missing, refuse — tell the user to run
  `hew-plan` first.
- The user approved the plan from `hew-plan`. You have it in context: goal,
  acceptance criteria, architecture, order of work, graph shape.

## Inputs from `hew prime decompose`

- `prerequisites.met` must be `true`. If not, stop.
- `tasks` — current graph state. If non-empty, you may be *extending* a
  graph; check before duplicating epics.
- `memories.conventions` and `memories.boundaries` — referenced inside task
  descriptions and acceptance criteria.
- `memories.dep` and `memories.audit` — flag library versions / deprecations
  to mention in tasks that touch those areas.

## Step 1 — pick the graph shape

This is the most consequential decision. Confirm or override the shape
`hew-plan` proposed, based on what is already in the graph.

```
Is the work a single bug fix or one-shot tweak?
  yes → FLAT TASKS. Skip epic wrapper. Usually 1–3 tasks total.
  no  → continue.

Does it deliver one coherent feature with multiple parts?
  yes → SINGLE EPIC + child tasks. Epic name = feature name.
  no  → continue.

Does it span multiple features that depend on each other?
  yes → MULTI-EPIC + task-level deps. One epic per feature.
        Sequence B-after-A via `hew dep add <first-B-task> --on <last-A-task>`.
        Parallel features = no deps.
        (Avoid `bd mol bond` — semantics are broken; see GOTCHA:bd-mol-bond.)
```

### Brownfield onboarding before features

Brownfield projects with any of `STATUS:scan` / `STATUS:convention` /
`STATUS:audit` / `STATUS:boundary` missing get an **onboarding epic first**.
Feature epics bond to it.

```
hew task new --type=epic --priority=0 --title="Codebase Onboarding"
  hew task new --parent=<epic> --title="Scan architecture"
  hew task new --parent=<epic> --title="Extract conventions"
  hew task new --parent=<epic> --title="Audit dependencies"
  hew task new --parent=<epic> --title="Map API boundaries"

# bd mol bond is intentionally NOT wrapped — semantics are broken
# (see GOTCHA:bd-mol-bond). Use task-level deps instead:
hew dep add <first-feature-task> --on <last-onboarding-task>
```

Feature tasks stay unready until onboarding closes.

## Step 2 — decide vertical slice vs horizontal layer

For everything bigger than a one-shot fix, structure tasks as **vertical
slices**, not horizontal layers.

**Vertical slice rule:** after each task closes, a real user can do something
they could not do before. If a task only "lays foundation" (all models, all
APIs, all UIs first), it is horizontal disguised as vertical — restructure.

| Pattern | Vertical | Horizontal |
|---------|----------|------------|
| Auth feature | login slice → signup slice → reset slice | all DB models → all routes → all UI |
| Add field to entity | model + API + UI in one task | model task, then API task, then UI task |
| New page | thin skeleton + one button works | full layout, then full logic, then wire |

Vertical = parallelizable, ships value continuously, reveals integration
issues early. Horizontal = serial, late integration, big-bang failure mode.

Use horizontal **only** when a shared foundation is genuinely required by
multiple slices (e.g., new auth middleware that ten endpoints depend on).
Even then, the foundation slice should be the thinnest viable version, not
the eventual complete version.

### Walking Skeleton — new projects

A brand-new project's first epic is a **walking skeleton**: the thinnest
end-to-end stack that proves the architecture works. One task that adds a
login button, one that has the API return 200 from a stub, one that puts
something in the DB. Real flow, fake fillings. Subsequent epics fatten the
slices.

Persist the skeleton's architectural commitments:

```
hew remember --type=decision "skeleton — Next.js 14 (app router) + FastAPI + Postgres. Set in walking skeleton; later phases inherit."
```

## Step 3 — build each task with discipline

For every unit of work in the plan, `hew task new` it. Each task gets four
things, all of which the executor will read on `hew task show <id>`:

### Title (verb phrase, ≤ 60 chars)

- Good: `Add password reset endpoint`, `Wire login button to /api/v1/auth/login`
- Bad: `Auth stuff`, `Backend work`, `Update files`

### Description — the *why*, *what*, and *which conventions apply*

The executor reads this *first* on `hew task show`. Write so the description
plus `hew prime execute` output is enough to do the work without asking.

Include:

- **Why this task exists** — one sentence linking to a `DECISION:` memory or
  the user's stated goal.
- **What "done" looks like** — concrete output. Files touched, behavior
  added, what to test.
- **Constraints from memory** — `CONVENTION:` rules that bind this work,
  `BOUNDARY:` interfaces not to break, `AUDIT:` warnings about libs.
- **Files the work should touch** — exact paths if known, glob if a known
  module. `src/auth/jwt.py` beats "the auth code."
- **Tests** — one line naming the test file plus the specific behavior the
  test will pin. Tasks that legitimately can't be unit-tested (config,
  pure glue, docs) must say so and explain why.
- **Craft** — one line listing which `CONVENTION:craft.<id>` bindings the
  executor should honor for this task, and any planned deviations the
  reviewer should expect. Pull from the project-wide
  `CONVENTION:craft.*` set plus any `DECISION:craft-feature:<plan-id>`
  refinement memory written by hew-plan.

When the description tells the executor to write a memory mid-task (e.g.
a `DECISION:` captured while implementing), have them attach explicit
links so future readers can navigate the graph: `hew remember
--type=decision "..." --key=decision-x --related=convention-y
--related-task=<this-task-id>`. The link sidecars surface in `hew
memories --links` and survive compaction (LINK: is exempt). Outbound
edges die with the source on `hew forget`; inbound edges intentionally
dangle so the next author notices.

```
hew task new --type=task --priority=1 \
  --title="Implement JWT issuance for /api/v1/auth/login" \
  --description="
  Why: D-04 specifies JWT auth (DECISION:auth memory).
  What: POST /api/v1/auth/login handler returns {access, refresh} tokens.
  Convention: wrap errors in AppError per CONVENTION:errors. Service layer per CONVENTION:services.
  Boundary: must not change the existing POST /api/v1/users contract (BOUNDARY:users-create).
  Files: app/api/v1/auth.py (new), app/services/auth_service.py (new), tests/api/test_auth.py (new).
  Tests: tests/api/test_auth.py pins (a) 200 + tokens on valid creds, (b) 401 + AppError on invalid, (c) refresh rotation. Each maps to one acceptance criterion.
  Craft: CONVENTION:craft.solid (service-layer SRP), CONVENTION:craft.fail-fast (reject malformed body before DB hit), CONVENTION:craft.dry (share token-encode helper with /refresh). No deviations from project set.
  "
```

### Acceptance criteria — `--acceptance="…"`

How `hew-guard` and `hew-verify` decide done. 1–4 verifiable statements.

```
--acceptance="Login returns 200 + access+refresh on valid creds; 401 + AppError on invalid;
             refresh rotates on use; access TTL is 15 min; refresh TTL is 7 days; pytest -k auth passes."
```

**Nyquist rule:** every acceptance criterion must be verifiable by a
deterministic check (test, grep, file existence, HTTP call), not a vibe.
"It works" is not acceptance. "`pytest -k login` passes and `curl POST
/login` with bad creds returns 401" is.

### Specificity test

Before saving the task: *could a different agent execute this without asking
clarifying questions, given the description plus `hew prime execute` output?*
If no, add specificity. Common gaps:

- Vague file scope → name the files.
- Vague behavior → name the inputs and outputs.
- Vague auth/security → reference the `DECISION:` memory.
- Vague tests → name the test file and one expected assertion.

### Right-sizing — one focused session

Each task should fit in ~30 minutes of agent work. Heuristics for "too big":

- Description names 3+ independent concerns ("login UI **and** JWT **and** tests").
- Acceptance has >4 verifiable points.
- The task touches >5 files.
- You want to add sub-bullets to the description.

Fix it by creating subtasks under the same parent and wiring deps inside the
parent. The parent closes when subtasks close.

Heuristics for "too small":

- Description is one sentence with no behavior detail.
- Acceptance is "the file exists."
- Combining with the next task wouldn't change anything.

Then merge it into the next task.

## Step 4 — wire dependencies

A task `hew dep add <child> --on <prerequisite>` if it cannot start until
the other is done. Be tight — chain only what truly must wait.

**Interface-first ordering:** when a plan creates new types/interfaces
consumed by later tasks, the first task defines the contract (types,
function signatures, route paths). Implementation tasks depend on the
contract task. This prevents executors from scavenger-hunting for context.

```
hew task new --title="Define auth contracts (types, route paths)"  # → hew-X.1
hew task new --title="Implement /login against contracts"          # → hew-X.2
hew task new --title="Implement /refresh against contracts"        # → hew-X.3
hew task new --title="Wire login button to /login"                 # → hew-X.4
hew dep add hew-X.2 --on hew-X.1
hew dep add hew-X.3 --on hew-X.1
hew dep add hew-X.4 --on hew-X.2
```

## Step 5 — place gates for external blockers

Gates are for anything outside the Beads graph. Never fake a gate with a
title prefix. Create the blocked task first, then attach a gate to it.

| Trigger | Command |
|---------|---------|
| Wait for PR merge | `bd gate create --type=gh:pr --blocks=<task-id> --await-id=42 --reason="PR #42 merge"` |
| Wait for CI | `bd gate create --type=gh:run --blocks=<task-id> --await-id=<run-id> --reason="CI green"` |
| Manual approval | `bd gate create --type=human --blocks=<task-id> --reason="Staging approved"` |
| Timer / cooldown | `bd gate create --type=timer --blocks=<task-id> --timeout=30m` |

Gate creation is a documented hold-out — `hew` has no `gate` wrapper yet
(`bd gate create` stays the path; `bd gate resolve <id>` closes manual
gates). Inspect with `bd gate list`. The `--blocks` flag does the
dependency wiring inline, so no separate `hew dep add` is needed.

## Step 6 — pick types and priorities

| `--type=` | Use for |
|-----------|---------|
| `task` (default) | a unit of work |
| `feature` | semantic alias — surfaces in feature filters |
| `bug` | semantic alias — surfaces in bug queries |
| `chore` | refactor, cleanup, non-feature |
| `epic` | container; only closes when all children close |
| `gate` | external blocker (above) |

| `--priority=` | Meaning |
|---------------|---------|
| `0` / `P0` | critical path for this milestone |
| `1` / `P1` | scoped in, should-have |
| `2` / `P2` | scoped in, nice-to-have |
| `3` / `P3` | backlog candidate |
| `4` / `P4` | deferred |

`"high"`, `"medium"`, `"low"` are **rejected** by Beads. Numeric only.

Priority inflation kills the signal. Reserve P0 for the critical path. If
everything is P0, you have not decomposed enough.

## Step 7 — concrete example, end to end

User asks for "auth on an existing FastAPI app." Plan is approved.

```
hew task new --type=epic --priority=1 --title="Auth System" \
  --description="JWT auth for /api/v1/*. See DECISION:auth, DECISION:db memories."
# → hew-a3f8

hew task new --parent=hew-a3f8 --type=task --priority=1 \
  --title="Define auth contracts" \
  --description="Why: D-04 + interface-first ordering. What: AuthResponse, RefreshRequest, route paths /login /refresh /logout. Files: app/api/v1/auth/types.py (new)."
# → hew-a3f8.1
# (set acceptance separately with `hew task update <id> --acceptance "…"` when needed.)

hew task new --parent=hew-a3f8 --type=task --priority=1 \
  --title="Implement POST /api/v1/auth/login" \
  --description="Why: D-04. What: validates {email,password}, returns AuthResponse. CONVENTION:errors (AppError), CONVENTION:services (DI). Files: app/services/auth_service.py, app/api/v1/auth/login.py, tests/api/test_login.py. Tests: test_login.py pins 200+tokens (valid), 401+AppError (invalid), brute-force throttle. Craft: CONVENTION:craft.solid + craft.fail-fast (validate before DB hit)."
# → hew-a3f8.2

hew task new --parent=hew-a3f8 --type=task --priority=1 \
  --title="Implement POST /api/v1/auth/refresh + rotation" \
  --description="Why: D-04 refresh rotation requirement. Files: app/api/v1/auth/refresh.py, tests/api/test_refresh.py. Tests: test_refresh.py pins rotation-on-use + revoke-old-token + reuse-detection. Craft: CONVENTION:craft.idempotence (rotate is replay-safe), craft.dry (reuse token-encode helper from .2)."
# → hew-a3f8.3

hew task new --parent=hew-a3f8 --type=task --priority=2 \
  --title="Wire frontend login button to /login" \
  --description="Files: frontend/src/auth/login.tsx, frontend/src/auth/auth-client.ts. CONVENTION:errors-frontend."
# → hew-a3f8.4

hew task new --parent=hew-a3f8 --type=task --priority=2 \
  --title="End-to-end auth integration tests" \
  --description="Full login → protected → refresh → logout cycle. Files: tests/e2e/test_auth.py."
# → hew-a3f8.5

hew dep add hew-a3f8.2 --on hew-a3f8.1
hew dep add hew-a3f8.3 --on hew-a3f8.1
hew dep add hew-a3f8.4 --on hew-a3f8.2
hew dep add hew-a3f8.5 --on hew-a3f8.2
hew dep add hew-a3f8.5 --on hew-a3f8.3
```

`hew dep tree hew-a3f8` should now show:

```
bd-a3f8 Auth System [epic] [P1] (open)
  ├── bd-a3f8.1 Define auth contracts (open)             ← READY
  ├── bd-a3f8.2 Implement /login (open)                   ← blocked by .1
  ├── bd-a3f8.3 Implement /refresh + rotation (open)      ← blocked by .1
  ├── bd-a3f8.4 Wire frontend login button (open)         ← blocked by .2
  └── bd-a3f8.5 E2E integration tests (open)              ← blocked by .2, .3
```

One task ready. Three blocked on the contract. Two blocked downstream. Clean
critical path.

## Step 8 — self-validate before declaring done

Run each:

1. **`hew prime execute`** — `ready_list` must be ≥1 task. If empty, you
   have a cycle or over-constrained the graph. Inspect with `hew dep
   blocked`.
2. **`bd orphans`** — must return nothing. Orphans = broken dependency
   refs. No `hew` wrapper yet; bd is fine.
3. **`hew epic tree <epic-id>`** for each epic — visualize the hierarchy
   and confirm it matches the plan's "order of work."
4. **`bd lint`** — flags tasks missing descriptions or acceptance
   criteria. No `hew` wrapper yet.
5. **Read three random tasks with `hew task show <id>`** — descriptions must be
   self-contained. If you cannot understand the task without conversation
   context, rewrite the description.

If validation fails, fix the graph before handing off. Cheap to fix now;
disruptive to fix mid-execution.

## Decision fidelity — never silently reduce scope

If the plan locks a decision (e.g., "cost from billing table in impulses"),
the graph MUST deliver that. Forbidden softeners in task descriptions:

- "v1 will be …", "static for now", "hardcoded for now"
- "wire later", "skip for now", "placeholder", "minimal version"
- Any phrasing that delivers less than the plan specified

If the work genuinely will not fit in one milestone:

1. **Surface this to the user** before creating tasks. State which item
   cannot fit and why.
2. Propose a split: which items form a self-contained earlier slice.
3. After user approval, decompose only the in-scope items.

The planner does not get to decide that something is too hard. If the user
locked it, plan it.

## Anti-patterns

- **Faking native types with title prefixes** (`"GATE: …"`, `"PHASE 1: …"`,
  `"EPIC: …"`). Use `--type=`.
- **Strings for priority** (`"high"`, `"medium"`, `"low"`). Numeric.
- **One giant flat task list** when the work is clearly multi-feature.
- **Cycle creation.** Verify with `hew prime execute` after every batch of `hew dep add`.
- **Priority inflation.** Everything P0 = no signal.
- **Empty descriptions** or **acceptance criteria.**
- **Horizontal-layer decomposition** unless a shared foundation truly demands it.
- **Forgetting to bond multi-epic projects** — agent will start dependent work
  before its dependency exists.
- **Inventing dates or owners** in descriptions — let Beads track those.

## Hand-off

When the graph is built and validation passes:

1. Print `hew epic tree <epic-id>` to the user.
2. Ask: "Graph looks like this. Approve and start work, or revise?"
3. On approval, invoke `hew-execute`.

Then write the phase marker so downstream skills know decomposition is done:

```
hew remember --type=status "decompose:complete — <ISO-8601 timestamp>"
```

Mid-flight decomposition (the executor finds a task is too big) is handled
inside `hew-execute` by spawning subtasks on the same parent. You don't get
called again for the same epic unless the user explicitly asks to re-plan.
