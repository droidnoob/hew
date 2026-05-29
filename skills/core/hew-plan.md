<!-- hew:version=0.10.0 -->
---
name: hew-plan
category: core
init: hew prime plan
---

# hew-plan — Strategic Planning

You are turning a user's request into an executable plan. The plan is the
*thinking*, not the *graph*. `hew-decompose` turns this thinking into Beads
tasks. Your job ends at "here is the architecture, the order, and what good
looks like."

## When this skill runs

- User describes a goal in plain English ("build me X", "add Y to this repo").
- `STATUS:plan` is not `complete`, or the user explicitly asks to re-plan.
- The Beads graph for this work does not yet exist (no epic, no tasks). If one
  exists, you are probably looking for `hew-decompose` (to extend it) or
  `hew-execute` (to start working).

## Decide the branch shape (don't create it)

Planning **decides** which branch the work belongs on; `hew-execute`
**creates** it on first claim. Plan never runs `git` — it just names
the branch as part of the plan output.

Pick a **prefix** from the conventional-commit set, matching the
work's intent:

| Prefix | When |
|--------|------|
| `feat` | new feature or capability |
| `fix` | bug fix |
| `chore` | tooling, deps, infra, version bumps |
| `docs` | docs-only changes |
| `refactor` | restructure without behavior change |
| `perf` | performance improvement |
| `test` | tests only |
| `style` | formatting / linting only |

Pick a **slug** that describes the work in 2–5 kebab-case words —
not the file paths touched. Examples: `passwordless-email-auth`,
`pipe-deadlock-fix`, `craft-warnings-soft-mode`.

Record the branch decision in the plan output (next section). One
plan = one branch. Sub-epics within a milestone may earn their own
branches; let `hew-decompose` flag that if it surfaces during graph
construction.

Skip this step only when:

- The work is a single trivial commit that legitimately belongs on
  `main` (release tag commit, hotfix the team explicitly authorized).
  In that case, name the override path: `HEW_ALLOW_MAIN_COMMIT=1`.
- The project doesn't use protected-branch enforcement (no
  `.githooks/pre-commit`, no `.github/protection/main-ruleset.json`).
  Detect by checking `git config core.hooksPath` (should be
  `.githooks`) or the protection-config file's presence.

## Inputs you get from `hew prime plan`

- `project.bd_version` and `beads_initialized` — confirms `bd` is wired up.
- `memories.factual` — what the codebase already is (only meaningful on brownfield).
- `memories.conventions` — `CONVENTION:` rules to respect in your choices.
- `memories.boundaries` — public interfaces that must not break.
- `tasks.total` / `tasks.done` — context on what has already been built.
- `prerequisites` — `met: true` always for `hew-plan` (it is the entry point).

If `factual` and `conventions` are empty and this is a brownfield project, stop
and tell the user: run `hew-scan` and `hew-convention` first. Planning without
knowing the codebase produces fiction.

## How to plan — goal-backward, never goal-forward

1. **Restate the goal in one sentence.** If you cannot, ask the user. Do not
   plan against ambiguity.
2. **Ask "what must be TRUE when this is done?"** — three to seven verifiable
   acceptance criteria. These become the verification contract for `hew-verify`
   and feed into task acceptance criteria.
3. **Work backwards from each criterion to the slices that produce it.**
   Resist the urge to start at the bottom (file structure, dependencies). Start
   at the user-observable outcome and decompose only as far as needed to make
   the next slice obvious.
4. **Pick the architecture.** State the major components, their boundaries, and
   the data that flows between them. One paragraph or one diagram — not more.
   If you are reaching for a framework, name the version (and check
   `memories.dep` / `AUDIT:` entries before committing).
5. **Order the work.** Identify the critical path: what blocks what? Pull
   parallelizable slices to the side; mark them as such for `hew-decompose`.
6. **Sketch the graph shape.** You are not creating tasks yet, but you are
   telling `hew-decompose` what shape to build:
   - Single feature / small fix → flat task list, no epic wrapper.
   - One coherent feature → one epic + child tasks.
   - Multi-feature build → multiple epics, with bonding for sequencing.
   - Brownfield onboarding work first? → a separate epic that the feature
     epics depend on.

## Output (what you give back to the user, before invoking `hew-decompose`)

Hold the plan in conversation context — do **not** write a `PLAN.md` file.
The user sees a short summary; the next skill consumes the details directly
from the conversation. The summary contains:

- **Goal** (one sentence).
- **Acceptance criteria** (3–7 bullets, each independently verifiable).
- **Architecture** (one paragraph, naming components and key choices).
- **Order of work** (numbered list of slices, critical path called out).
- **Graph shape** (one of: flat / single epic / multi-epic + bonds).
- **Branch** (prefix + slug picked above — `hew-execute` Step 3a will
  run `hew branch new` from this on first claim. Format the recap
  line as `Branch: <prefix>/<slug>`).
- **Open questions** (anything you need from the user before decomposition).

Ask the user to confirm the plan, or to push back on any of it, before handing
off to `hew-decompose`. Cheap to revise here; expensive to revise once tasks
are in the graph.

## Decisions you make here, captured as memories

When you commit to non-obvious choices, persist them so future sessions inherit
the reasoning:

```
hew remember --type=decision "auth — JWT with 15min access + 7d refresh in httpOnly cookies. Reason: SPA + mobile share the API."
hew remember --type=decision "db — Postgres over SQLite because we need RLS for the multi-tenant story."
hew remember --type=decision "framework — FastAPI over Flask. Async-first, OpenAPI generation, pydantic models reused for DB."
```

These are factual decision memories (no special prefix beyond `DECISION:`),
not `CONVENTION:` rules. They explain *why* the codebase looks the way it
does. Future work can revisit them; current work treats them as settled.

If a decision is anchored to another memory or to the task that produced
it, tag the relationship with `--related` / `--related-task` so the link
surfaces in `hew memories --links <key>`:

```
hew remember --type=decision "auth — JWT with 15min access + 7d refresh; refresh rotates on every use." \
  --key=decision-auth \
  --related=convention-jwt-shape \
  --related-task=hew-a3f8
```

Each related target becomes a sidecar `LINK:` memory the reader stitches
into outbound/inbound edges. `--related` requires `--key` so the link's
`<from>` side is deterministic.

### Group related decisions under one domain key

When a single plan locks several related calls — e.g. three choices
that together define the auth boundary — fold them into one
domain-grouped `DECISION:` memory rather than three atomic ones.
Atomic per-choice writes fragment the decision log and lose the
*relationship* between the calls (which was the load-bearing
context).

**Atomic (avoid when choices are related):**

```
hew remember --type=decision "auth — JWT, 15min access tokens"
hew remember --type=decision "auth — refresh tokens 7d, httpOnly cookies"
hew remember --type=decision "auth — refresh rotation on every use"
```

**Grouped per domain (preferred):**

```
hew remember --type=decision "auth — JWT with 15min access + 7d refresh in httpOnly cookies; refresh rotates on every use. Reason: SPA + mobile share the API and we want revocation without a session store. Reviewed alternatives: opaque tokens with Redis (rejected — operational cost), session cookies (rejected — mobile)."
```

For a plan that locks 5+ decisions across several domains, prefer
`hew remember --from-file <path>` over a sequence of CLI calls — JSON
shape documented in the `hew-convention` skill body.

## Vague-ask gate (optional)

If the user's request lacks an observable outcome or a verifiable
acceptance signal — "build a thing," "make it better," "add some
logging" — AND `STATUS:spec:complete` is missing, surface
`/hew:spec` first via a non-blocking picker:

```
Your ask looks underspecified. Run /hew:spec first?
> Yes — score and Socratic-clarify (max 4 rounds)
  No — proceed; I'll plan against best-effort assumptions
```

`/hew:spec` writes `SPEC:<topic>` + `STATUS:spec:complete` on pass; on
4-round-without-pass, it writes `[ASSUMED]` `DECISION:` memories that
flow into this plan automatically. Either way, planning resumes here.

## Research-or-decompose — tail picker

Before handing off, ask once: should we research first, or go straight to
decompose? Don't score this with a heuristic — the user owns the call.

The default selection comes from `hew config get research.default`:

| Value | Default selection | Meaning |
|-------|-------------------|---------|
| `ask` *(default)* | no preselect | always prompt the user |
| `auto-skip` | "Skip — go to decompose" | for projects in well-understood territory |
| `auto-run` | "Research first" | for greenfield / unfamiliar domains |

Picker:

```
Research first?
> Yes — run /hew:research, then come back to decompose
  Skip — hand off to /hew:decompose now
```

When `auto-skip` or `auto-run`, the picker still shows but with the
recommended choice preselected — the user can override. Honor
`--non-interactive` / CI by using the configured default without
prompting.

If a major decision genuinely needs investigation (unfamiliar framework,
novel domain, unknown library landscape) and the user picked "Skip,"
record the uncertainty in a `DECISION:` memory tagged `[ASSUMED]` so it
can be revisited.

## Craft refinement — feature-level deviations

The project picked its craft set in `hew-new-project` Phase C; those
choices live as `CONVENTION:craft.<id>` memories and apply to every
plan by default. Sometimes a single feature legitimately needs a
narrower or wider set — an event-sourced slice in an otherwise CRUD
codebase, a perf-critical hot path that locally relaxes
`single-level-of-abstraction`, a transaction-script in a generally
DDD project.

Read the project's `CONVENTION:craft.*` set, then ask once:

```
Does this plan need to deviate from the project's craft set?
> No — project defaults apply unchanged
  Add: this slice needs <principle> in addition (e.g. event-sourcing)
  Relax: <principle> doesn't fit here (state the reason)
```

Record any deviation as a `DECISION:craft-feature:<plan-id>` memory.
The executor and reviewer pick these up alongside the project-wide
`CONVENTION:craft.*` set:

```
hew remember --type=decision "craft-feature:auth-mvp — ADD event-sourcing for the audit log slice (auditable token issuance). Project default: CRUD. Reason: regulatory replay requirement."
hew remember --type=decision "craft-feature:hot-render — RELAX single-level-of-abstraction in src/render/inner_loop.rs (perf). Justified by benchmark notes."
```

If the user picks "No," skip the memory write — silence means the
project-wide set applies.

Both `hew-execute` (Step 5 craft check) and `hew-review` (Craft pillar)
read these per-plan deviations *in addition to* the project's
`CONVENTION:craft.*` memories. Deviations are scoped to the plan id;
they do not bleed across features.

## What you don't do

- **No tasks.** That is `hew-decompose`. Do not run `hew task new` here.
- **No code.** Planning produces words and decisions, not files.
- **No markdown plan files.** State lives in conversation + `hew remember`.
- **No premature decomposition into 50 tiny steps.** Plans should fit on one
  screen. If yours is sprawling, your acceptance criteria are too vague.
- **No assumption-loading without confirmation.** If the user said "build a
  thing," do not silently assume tech stack, hosting, auth model. Ask.

## Anti-patterns to flag

If the user asks you to plan something where you notice:

- They are reinventing something the codebase already has — point at it
  (`BOUNDARY:` or factual memories) and propose extending instead.
- They are about to break a `CONVENTION:` rule — call it out and ask whether
  the rule should change first.
- They are working on a brownfield project with empty memories — refuse, run
  `hew-scan` first. Don't pretend to know the code.

## Hand-off

When the plan is confirmed, run the research-or-decompose picker above.
On "Skip" or after `/hew:research` completes:
"Plan is approved. Calling `hew-decompose` to build the Beads graph."
Then invoke `hew-decompose` with the plan in context.
`hew-decompose` will read the same memories, plus the conversation it inherits
from you, and produce `bd create --graph` for the task batch (or `hew task
new` for one-offs), `hew dep add` for cross-task ordering, and
`hew gate new --gh-pr=N` for external blockers (e.g. a downstream epic
that waits on a PR merge).

After `hew-decompose` finishes, write the phase marker:

```
hew remember --type=status "plan:complete — <ISO-8601 timestamp>"
```

This unblocks every downstream skill's prerequisite check.
