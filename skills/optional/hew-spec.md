<!-- hew:version=0.4.0 -->
---
name: hew-spec
category: optional
init: hew prime spec
---

# hew-spec — Ambiguity Gate Before Planning

Lightweight pre-planning gate. Catches vague asks before they get planned
against. If the user says "build me an app for tracking stuff," you don't
want `hew-plan` heroically inventing a domain model — you want one Socratic
round that turns "tracking stuff" into a concrete acceptance criterion.

Not installed by default. Opt in via
`hew config set optional-skills.security true` *(typo guard: the key for
spec will be added when this skill is wired into config; for now invoke
directly via `/hew:spec` or via the `hew-plan` tail picker.)*

## When this skill runs

- `hew-plan` detected a vague ask: the user's request lacks an observable
  outcome ("build a thing," "add some logging," "make it better") AND
  `STATUS:spec:complete` is missing.
- The user explicitly invokes `/hew:spec <topic>`.
- A previous attempt at planning produced an architecture the user
  rejected as "not what I meant" — strong signal the spec was vague.

## Inputs from `hew prime spec`

- The **request** — the user's prompt verbatim.
- `memories.factual` — what's already known about the codebase, so you
  don't ask the user to restate things hew already knows.
- `memories.boundaries` — existing public interfaces that constrain what
  "build X" can mean here.

## The two scoring dimensions

You score the request on two dimensions, each in `[0.0, 1.0]`. Both
weighted at `0.5`. Ambiguity is `1.0 - (0.5 * goal + 0.5 * acceptance)`.

### Goal clarity (weight 0.5)

Does the request describe an **observable change in the world** when done?

| Clarity | Example |
|---------|---------|
| `1.0` | "When a user clicks 'export,' a CSV of their last 30 days of orders downloads." |
| `0.65` | "Let users export their order history." |
| `0.3` | "Add an export feature." |
| `0.0` | "Make it more useful." |

The bar for `0.65`: a reader who's never seen the codebase can describe
*what* the feature does, even if not *how*.

### Acceptance clarity (weight 0.5)

Does the request describe **how we'll know it's done**?

| Clarity | Example |
|---------|---------|
| `1.0` | "Done = clicking export emits a CSV with columns {id, date, total}; the existing test suite passes; a new e2e test covers the click→download path." |
| `0.65` | "Done when there's a working export button and a test for it." |
| `0.3` | "Done when it works." |
| `0.0` | (no acceptance signal at all) |

Acceptance must be **verifiable by inspection** — a test pass, a manual
check, a UI behavior. "Done when the team is happy" is not acceptance.

## The gate

```
ambiguity = 1.0 - (0.5 * goal_clarity + 0.5 * acceptance_clarity)

PASS = ambiguity <= 0.20  AND  goal_clarity >= 0.65  AND  acceptance_clarity >= 0.65
```

Both per-dimension floors *and* the overall ambiguity ceiling must hold.
A request with `goal=1.0, acceptance=0.4` averages well but still fails
because acceptance is below the floor.

## The Socratic loop

Up to **four rounds**. Each round asks the most leverage-y question for
the lowest-scored dimension. Stop early when the gate passes.

```
1. Score the current request (goal, acceptance, ambiguity)
2. If gate PASSES → persist + hand off
3. Pick the weakest dimension; ask ONE focused question targeting it
4. Re-score with the user's answer folded in
5. Loop until pass or round 4
```

Question templates per dimension:

**Goal clarity (low):**
- "When this is done, what does a user see / do differently?"
- "Walk me through the click-by-click path that exercises the new
  behavior."
- "Is this a new entry point, or does it extend an existing one?"

**Acceptance clarity (low):**
- "How will we know it's done? Manual check? Test pass? Both?"
- "What's the smallest change to the test suite that would catch a
  regression here?"
- "If I shipped it tomorrow and it was broken in production, what
  symptom would tell us?"

Keep questions to ONE per round. Stacking questions ("and also, and
also") destroys the signal.

## When max rounds hit and gate still fails

After 4 rounds, surface the unresolved dimensions as `[ASSUMED]`
memories the planner inherits. Don't loop forever; planning can proceed
with explicit assumptions.

```
hew remember --type=decision "spec:<topic> [ASSUMED] goal: <best-guess restatement>. Confirmed only at clarity 0.4 — planner should re-validate after first decompose pass."
hew remember --type=decision "spec:<topic> [ASSUMED] acceptance: <best-guess>. Clarity 0.3."
```

Tell the user: "Spec stays partial. The planner will proceed with
`[ASSUMED]` decisions tagged for re-validation."

## When the gate passes

Persist the spec body verbatim plus the completion marker:

```
hew remember --raw "SPEC:<topic> goal=<final-restated-goal>; acceptance=<final-acceptance>. Goal-clarity=<n>; Acceptance-clarity=<n>; Ambiguity=<n>."
hew remember --type=status "spec:complete — <ISO-8601 timestamp>"
```

(`SPEC:` isn't in the standard `--type` allowlist; use `--raw`.)

Then hand off: "Spec is solid. Calling `hew-plan` now."

## Output

End with a one-screen recap:

```
hew-spec: order export — pass (ambiguity 0.15)

Goal (0.85): User clicks Export on /orders → downloads CSV of last 30
  days with columns {id, date, total}.

Acceptance (0.85): Existing tests pass + new Playwright spec covers
  click→file-saved + CSV header matches schema.

Persisted:
  SPEC:order-export …
  STATUS:spec:complete — 2026-05-12T…
```

If the gate failed after 4 rounds, the recap lists which dimensions
ended `[ASSUMED]` and at what clarity.

## What you don't do

- **Ask 10 questions in one round.** One question per round; signal
  beats brute force.
- **Score and not ask.** A `<0.65` dimension is a question, not a
  verdict.
- **Plan during spec.** Don't propose architecture, libraries, or files
  — that's `hew-plan`'s job. Spec is about *what*, not *how*.
- **Refuse vague asks outright.** Score them, ask, persist what you got.
  Some projects start fuzzy and crystallize through Socratic rounds.
- **Skip persistence on partial pass.** Even `[ASSUMED]` outcomes get
  written — future sessions need the breadcrumbs.

## Anti-patterns

- **Single-round gate.** "Your ask is vague" is not a Socratic loop;
  it's a brush-off.
- **Question without anchor.** "What do you want?" is a worse signal
  than "Walk me through what a successful click on the Export button
  shows the user."
- **Scoring drift.** If you scored `goal=0.5` round 1 and `goal=0.5`
  round 2 with no new info, you're not making progress — ask a
  different angle, not the same one.
- **Skipping the persist on pass.** No `STATUS:spec:complete` ⇒ the
  next session re-runs the gate. Always write the marker.
