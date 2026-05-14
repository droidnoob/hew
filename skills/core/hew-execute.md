<!-- hew:version=0.3.1 -->
---
name: hew-execute
category: core
init: hew prime execute
---

# hew-execute — The Work Loop

You execute the Beads graph. The `hew status` ready list says what to do next, you claim it,
code it, test it, run `hew-guard`, close it, commit. Repeat until the graph
is empty or the user stops you.

This is the core loop. Most of the agent's runtime is spent here. Other
skills exist to set up the graph (`hew-plan`, `hew-decompose`), verify it
(`hew-verify`), or gate it (`hew-guard`). Execute is where work happens.

## When this skill runs

- The user says "start," "what's next," "keep going," or describes any task
  that has a corresponding open Beads issue.
- `STATUS:plan:complete` exists (`prerequisites.met` is `true` for execute).
  If not, refuse — run `hew-plan` / `hew-decompose` first.
- The `hew status` ready list is non-empty.

## Inputs from `hew prime execute`

- `prerequisites.met` — if `false`, stop and tell the user what's missing.
- `tasks.ready_list` — pre-sorted by priority. Pick the top one.
- `memories.conventions` — **constraints** you must honor while coding.
- `memories.boundaries` — public interfaces that must not break.
- `memories.audit` / `memories.security` — flagged libs and security
  decisions relevant to whatever you touch.
- `skill_instructions` — this file, embedded in the prime output.

## The loop

```
1. hew prime execute              → curated ready list
2. pick highest-priority unblocked task
3. hew task claim <id>            → atomic claim (sets in_progress + assignee)
4. hew task show <id>             → read the full description + acceptance
5. do the work                    → code, tests, verifying inline
6. invoke hew-guard               → pre-close sanity gate
7. hew task close <id> --reason "…"  → mark done with one-line summary
8. git commit                     → atomic, conventional message
9. if discovered something non-obvious: hew remember --type=<x> "…"
10. back to 1
```

This is the entire methodology in 10 steps. Everything below explains *how*
to do each step well.

## Step 1–2 — picking work

`hew prime execute` already pre-sorted `tasks.ready_list` by priority. Take
the top item. Ties on priority break by creation order — older first.

If the user asked for a specific task ID, prefer that over priority order.
Verify it's in `ready_list` first; if not, tell the user it's blocked and
show the blockers (`hew task show <id>` lists them).

If `ready_list` is empty:
- Check `tasks.blocked` — if non-zero, the graph has unreachable work.
  Surface this and stop.
- Otherwise, the work is done. Suggest `hew-verify`.

## Step 3 — claim atomically

`hew task claim <id>` is atomic: it sets `status=in_progress` and
`assignee` in one operation. This prevents two agents from racing on the
same task in shared-graph setups.

Never start work without claiming. The audit trail (`hew task show <id>`) shows
who picked up what when — important when sessions fail and a new agent
needs to resume.

### Step 3a — create the branch on first claim

`hew-plan` already decided the branch name and surfaced it in the plan
output (`Branch: <prefix>/<slug>`). This step **creates** it. If
planning didn't happen (entered the loop cold via `/hew:next` /
`/hew:work`), the protected-branch guard below catches it.

#### The branch source-of-truth

In priority order:

1. The plan's `Branch:` recap line, if `hew-plan` ran in this
   conversation. The prefix/slug there is authoritative.
2. Otherwise, the epic body — re-read `hew epic show <epic-id>` for
   a `Branch:` line.
3. Otherwise, ask the user once: "No branch decision found. What
   prefix + slug should this work go on?" Cache the answer for the
   session; don't re-ask for sibling tasks under the same epic.

Once you have a `<prefix>/<slug>`, run:

```
hew branch new --prefix=<prefix> --slug='<slug>'
```

Skip silently when you're already on the right branch — check via
`git symbolic-ref --short HEAD`.

#### Protected-branch guard

Run `git symbolic-ref --short HEAD` first. If HEAD is on `main` /
`master` **and** the project uses protected-branch enforcement
(detect via `git config core.hooksPath` returning `.githooks`, or
the presence of `.github/protection/main-ruleset.json`), **refuse to
proceed without a branch decision**. The pre-commit hook will refuse
the final commit otherwise; catching it here saves a back-out.

If the plan named a branch, just create it (per "The branch
source-of-truth" above). If no branch decision exists, ask the user
for one — don't invent it. Naming the branch is `hew-plan`'s job
(per `CONVENTION:skill-boundaries-plan-vs-execute`); doing it here
without a plan recap means the agent is making architectural calls
the planner skipped.

Skip the guard only when:

- The project doesn't have `.githooks/pre-commit` and doesn't have
  `.github/protection/main-ruleset.json` (no protection in force).
- You're already on a non-protected branch.
- The user explicitly authorized a main-commit (set
  `HEW_ALLOW_MAIN_COMMIT=1` in the shell). Honor it; do not branch.

#### Opt-in: auto-branch strategy

`hew config get branching.strategy` adds **per-claim** branching on
top of the plan's per-feature decision:

| Value | Behavior |
|-------|----------|
| `none` *(default)* | one branch per plan (the plan's `Branch:` decision). Sibling tasks under the same epic stay on it. |
| `epic` | same as `none` — one branch per epic. (The plan's branch decision typically maps 1:1 to an epic.) |
| `always` | create a fresh branch every claim. Rare; for review-per-task workflows. Each task ships under its own PR. |

When `always` fires, derive the per-task slug from the task title
(`hew task show` → slug the title) and append it to the plan's
branch: `<plan-branch>/<task-slug>`.

Skip the strategy silently — never block the claim — when:

- `branching.strategy=none` and you're already on the plan's branch.
- `git` is not on PATH (`hew branch new` will return
  `hew::git::not_found`; treat as soft-skip).
- The user passed `--no-branch` to the loop (or said "no branch" in
  conversation).

## Step 4 — read the task properly

Run `hew task show <id>` even though some of the fields appeared in `ready_list`.
`hew task show` also surfaces:

- Full description (truncated in ready listings).
- Acceptance criteria — this is your done contract.
- Dependencies — confirm they're closed.
- Comments and prior audit trail (if the task was previously claimed and
  unclaimed).

Re-read `CONVENTION:` and `BOUNDARY:` memories that the description
references. The plan put them there for a reason; ignoring them produces
drift.

If the description is vague, **do not guess**. Ask the user, or — if
mid-flight — sub-decompose: create subtasks under this one with clearer
descriptions and `hew dep add <subtask> --on <this-task>` to block this
one until they close.

## Step 5 — do the work

### Convention-first coding

Before writing new code in any area, check `CONVENTION:` memories that
apply. The categorized prefix in `hew prime execute` output makes this
easy. If you are creating a new service, you must follow
`CONVENTION:services`. If you don't see a convention for the pattern you
are implementing but you notice an obvious existing pattern, capture it
before continuing:

```
hew remember --type=convention "routing — All API routes live under app/api/v1/ and are registered via include_router in app/api/__init__.py."
```

This is how the codebase's convention library grows.

### Boundary-aware refactors

Before modifying a function/route/module listed in `BOUNDARY:` memories,
note which downstream code depends on it. The boundary line tells you. If
your change would alter the contract:

- **Backwards-compatible extension** (new optional field, new opt-in path):
  go ahead.
- **Breaking change** (changed signature, changed status code, removed
  field): stop. This is **Rule 4 below** (architectural change) — surface
  to user.

### Analysis-paralysis guard

If you make 5+ consecutive `Read` / `Grep` / `Glob` calls without an
`Edit` / `Write` / `Bash` action, **stop**. Either you have enough context
and are stalling, or you genuinely don't and should ask. Continuing to
read is a stuck signal, not productive research.

State in one sentence why you haven't written anything yet, then either:

1. Write code (you have enough context), or
2. Report blocked with the specific missing information.

### Tests as part of the task, not after

The task's acceptance criteria almost always include a test contract. Write
tests *with* the code, not as a bolt-on. If acceptance says "pytest -k
login passes," then the deliverable includes that test, the test must
actually fail before your implementation and pass after.

Quick TDD when it fits:
1. Write the failing test first.
2. Make it pass with the minimum code.
3. Refactor only if needed.

Test-first is the default when behavior is new. The cost is a handful
of extra tokens; the win is a hard pin on the acceptance criterion
that survives every later refactor. If `hew config get testing.require`
is `true`, hew-guard escalates "missing tests" from warn to fail at
close-time — see `DECISION:craft-testing` and `hew-guard`'s
soft-warning table.

If TDD doesn't fit (UI, config, glue), still write tests covering the
acceptance criteria before claiming done.

### Step 5a — Craft check

Before writing the first line of new code, read the project's craft
set:

```
hew memories --prefix CONVENTION   # filter for CONVENTION:craft.* entries
```

Plus any feature-level deviations from `hew-plan`'s craft refinement
step:

```
hew memories --prefix DECISION     # look for DECISION:craft-feature:<plan-id>
```

The task description's `Craft:` line (added by `hew-decompose` Step 3)
tells you which bindings are load-bearing for this task. Honor them
inline as you code:

- **SOLID / SRP** — if a class or function grows two unrelated reasons
  to change, split it before closing.
- **DRY** — when you copy-paste a block that you already wrote in
  another file this session, extract a helper. `hew-guard`'s
  `duplication` soft-warning fires when a 5+ line block repeats; treat
  the warning as a nudge, not noise.
- **Small functions / Single Level of Abstraction** — if a function
  spans more than `craft.max_function_lines` (config), `hew-guard`
  will flag it. Split or justify in the close reason.
- **Fail Fast / Idempotence / Pure Functions** — pick the right one
  for the boundary (input validation, retries, computation cores).

These are watchpoints, not universal rules. If a `CONVENTION:craft.<id>`
isn't in the project set, don't apply it. If a
`DECISION:craft-feature:<plan-id>` relaxes one, follow the deviation
and note it in the close reason so the reviewer doesn't flag it.

Brownfield deference: when a chosen craft principle conflicts with a
pre-existing `CONVENTION:` memory describing how the code is actually
written today, the existing convention wins (see
`CONVENTION:craft.consistency-with-existing-code` — it defaults on
every seeded stack). Don't refactor to satisfy a craft rule the rest
of the codebase ignores; surface the conflict instead.

## The four deviation rules

While executing, you will discover work the task description didn't
mention. Apply these rules automatically. Track them in your close
reason so the audit trail shows what you did beyond scope.

### Rule 1 — auto-fix bugs

**Trigger:** code doesn't work as intended. Wrong query, logic error, null
pointer, broken validation, race condition.

**Action:** fix inline, update tests, verify, continue the task. Track as
`[Rule 1 — Bug] description`.

No user permission needed.

### Rule 2 — auto-add missing critical functionality

**Trigger:** code missing essentials for correctness, security, or basic
operation. Missing error handling, no input validation, no auth on
protected route, missing CSRF, no error logging, missing DB index that
makes a query O(n) on every request.

**Action:** add it, update tests, verify, continue. Track as `[Rule 2 —
Critical] description`.

"Critical" means: required for correct/secure/performant operation. Not a
new feature.

Cross-reference `SECURITY:` memories — they often define what counts as
"critical" for this project (e.g., `SECURITY: All endpoints accepting user
input must run through validate_input(); see app/security/validate.py`).

### Rule 3 — auto-fix blocking issues

**Trigger:** something prevents completing this task. Wrong type, broken
import, missing env var, DB connection error, build config error.

**Action:** fix it and continue. Track as `[Rule 3 — Blocking] description`.

**Exclusion — package manager installs.** If `npm install <pkg>` /
`pip install <pkg>` / `cargo add <pkg>` fails or the package isn't found:
**do not** try a similarly-named alternative. Failed installs may indicate
slop-squatted or hallucinated package names. Stop and tell the user:

> Package `<name>` cannot be installed. Verify the package exists and is
> legitimate (check the registry directly). If the name is wrong, update
> the task description or `DEP:` memory and re-run.

Auto-substitution here can install something more dangerous than the
missing legit package.

### Rule 4 — ask about architectural changes

**Trigger:** the fix requires significant structural modification. New DB
table (not column), major schema change, new service layer, switching
libraries/frameworks, changing auth approach, breaking API contracts.

**Action:** stop. Surface to the user with: what you found, proposed
change, why needed, impact, alternatives. **User decision required.**

When unsure between Rule 1–3 and Rule 4: prefer Rule 4. Ask. Cheap.

### Scope boundary

Only auto-fix issues directly caused by the current task's changes. If you
notice a pre-existing bug in an unrelated file, **don't fix it**. Instead:

```
hew task new --type=bug --priority=2 --title="<one-line>" \
  --description="Discovered while working on hew-X.2: …"
```

Then continue your current task. The discovered work is now in the graph;
the user can prioritize it.

### Fix attempt limit

Three auto-fix attempts on a single task is the cap. After three:
- Stop fixing.
- Document remaining issues in your `hew task close --reason` and (optionally)
  open subtasks for them.
- Move on or report blocked.

Don't loop on the same problem hoping the next attempt works.

## Step 6 — hew-guard before close

Never `hew task close` without invoking `hew-guard` first. The guard runs the
pre-close sanity checks:

- Leftover `console.log` / `print` debug statements
- Hardcoded secrets, API keys, tokens
- Stray `TODO:` / `FIXME:` you left
- Unused imports
- Type errors
- Tests pass
- New code obeys `CONVENTION:` memories for this area

If guard fails, **fix and re-run guard** before close. The task stays open
until guard is clean.

## Step 7 — close with a useful reason

```
hew task close <id> --reason "Implemented POST /api/v1/auth/login with JWT issuance. Tests cover 200, 401, malformed body, missing fields. Added validation per CONVENTION:errors. Followed [Rule 2 — Critical]: added rate limit (10/min/IP) on the endpoint." --type=2
```

`--type=<1|2|3>` tags the reason with `[Rule N]` when a deviation rule applied.

A good close reason has:
- One-sentence summary of what was actually delivered.
- Tests run (and that they pass).
- Conventions / boundaries respected.
- Deviation rules applied (with rule number).

Future-you on `hew task show <closed-id>` reads this when debugging or auditing.

## Step 8 — commit

One commit per task. Conventional commit format:

```
feat(<scope>): <one-line summary>

- bullet on what changed
- bullet on what's tested
```

`<scope>` is the area touched (`auth`, `api`, `frontend`). `<type>` follows
the standard:

| Type | When |
|------|------|
| `feat` | new feature, endpoint, component |
| `fix` | bug fix, error correction |
| `test` | test-only changes (e.g., TDD RED phase) |
| `refactor` | code cleanup, no behavior change |
| `perf` | performance improvement, no behavior change |
| `docs` | documentation only |
| `style` | formatting, whitespace |
| `chore` | config, tooling, deps |

Reference the Beads task ID in the commit body when useful: `Closes
bd-a3f8.2`. The Beads audit trail already links commit→task via the close
reason; this is for the git side.

**Stage files individually.** Never `git add .` or `git add -A` —
accidentally including secrets, build artifacts, or `.env` is the #1
wrapper-CLI bug. Name each file.

## Step 9 — remember what you learned

While doing the work, if you discovered something non-obvious about the
codebase, persist it:

```
hew remember --type=gotcha "SQLAlchemy session is request-scoped via app/db/session.py — never instantiate Session() directly."
hew remember --type=convention "logging — Use structlog, always pass request_id in context. Never log PII."
hew remember --type=boundary "GET /api/v1/users/{id} returns {id,email,created_at} only — 7 frontend components consume."
```

The right time to write a memory is right after you needed it and figured
it out. Future-you saves the lookup time.

## Step 10 — loop

Run `hew prime execute` again. Pick the next ready task. Continue.

If the user said "do this one task," stop after closing it. If "keep going"
or `/hew:auto`, loop until `hew prime execute` shows no ready tasks (then call `hew-verify`) or
a Rule-4 architectural decision blocks you (then surface and stop).

### Step 10a — review picker (optional, config-gated)

Before continuing to the next task, run `hew review check --json`.
The output:

```json
{
  "tasks_since_last_review": 5,
  "config": { "after_n_tasks": 8, "after_epic": true, "batch_size": 8 },
  "epic_just_closed": false,
  "picker_should_fire": false,
  "reason": "..."
}
```

If `picker_should_fire` is `true`, surface this picker to the user:

```
The batch is ready for review. <reason>.
  Review batch         — friendly second-pass against conventions
  Adversarial review   — red-team / steelman pass
  Both                 — friendly first, then adversarial
  Skip                 — continue without reviewing (counter does NOT reset)
```

On selection:
- "Review batch" → invoke `/hew:review`.
- "Adversarial review" → invoke `/hew:adversarial-review`.
- "Both" → invoke `/hew:review`, then `/hew:adversarial-review`.
- "Skip" → continue to next task without writing a marker. The counter
  stays as-is so the next loop tick re-asks.

The review skills write `STATUS:review:<ts>` on success, which resets
the counter automatically.

When `picker_should_fire` is `false`, skip 10a silently and continue.
Don't run it for every task close — only consult when the loop tick
crosses a threshold, which `hew review check` reports.

Defaults: `review.after_n_tasks = 0` and `review.after_epic = false`,
so this step is invisible until the user opts in via
`hew config set review.after_n_tasks <n>` and/or
`hew config set review.after_epic true`.

## Auth gates and external blockers

If a tool/service returns an auth error mid-task ("Not authenticated,"
"401," "please run X login"):

1. Recognize this is a gate, not a failure.
2. Stop the current task — **don't close it**.
3. Tell the user the exact auth steps needed.
4. The user re-authenticates, then says "continue."
5. Re-claim the task and proceed.

Same pattern for any external blocker that needs the user (a webhook
secret, a deploy approval). Don't invent fake credentials, don't skip
the work. Just stop and ask.

## What you don't do

- **Work on multiple tasks at once.** Claim one, finish it, close it, then
  next. Beads tracks `in_progress`; multiple at once corrupts that signal.
- **Skip `hew-guard`.** Drift compounds.
- **Close on test failure.** Tests failed = task isn't done. Either fix
  them inside this task, or create a subtask blocker, link with `hew dep
  add <subtask> --on <this>`, fix the blocker first, then re-attempt close.
- **Create planning markdown files** (`TODO.md`, `NOTES.md`, `PLAN.md`).
  Use Beads + `hew remember`.
- **Add `console.log` / `print` "for debugging."** If you need to
  understand state, use the test or a debugger. Debug statements are a
  guard failure.
- **Hand-edit `.beads/`.** Use `bd` commands.

## Session handoff

Sessions die. Context runs out. The user closes the laptop. Nothing
special needs to happen:

- The currently-claimed task stays `in_progress` in Beads.
- Memories you wrote persist in Dolt.
- Commits are in git.
- Next session: `hew prime execute` shows your in-progress task at the top
  of the ready list (Beads still considers it claimed by you). You either
  finish it or release the claim via `bd update <id> --unclaim` (no hew
  wrapper for unclaim yet — see the raw bd escape hatch).

There is no "save state" step. The graph IS the state.

## Anti-patterns

- **Reading the same files repeatedly.** If you've seen them twice this
  session, the third read is procrastination. Write.
- **"Let me just refactor this real quick."** Out of scope. Open a chore
  task and move on.
- **Writing tests after the code "to save time."** Tests written after
  pass by construction; they don't catch bugs they were meant to.
- **Closing tasks with `--reason "done"`.** Useless. Write the actual
  summary.
- **Forgetting `hew remember` for hard-won discoveries.** Future-you pays
  for every gotcha relearned.
- **Closing while guard fails.** The whole point of guard is to prevent
  this.
- **Mega-function.** A 200-line handler with five distinct concerns
  inside it. Split before close; lean on
  `CONVENTION:craft.small-functions` / `single-level-of-abstraction`
  when the project picked them.
- **Duplicated logic across files.** If `hew-guard`'s `duplication`
  soft-warning fires (5+ identical lines across two locations),
  extract a helper. Silencing the warning by removing the DRY memory
  is a smell, not a fix.
- **Applying craft rules the project didn't pick.** Universal SOLID /
  DRY enforcement isn't the contract. Only honor
  `CONVENTION:craft.<id>` memories that actually exist in this
  project's set.
