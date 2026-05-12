<!-- hew:version=0.2.0 -->
---
name: hew-new-project
category: core
init: hew prime new-project
---

# hew-new-project — bootstrap a project from a one-line outline

You turn a 1–3 sentence project outline into a working Beads graph:
project facts captured, research findings cited, a milestone chain
locked, and the first milestone fully decomposed. The user's next
move after you finish is `/hew:next` — and that's it.

This is **not** `hew-plan`. Plan is per-feature on an existing
project. This skill runs **once**, at project start, and produces
the roadmap that plan/decompose later feed off of.

## When this skill runs

- The user says "new project", "starting fresh", "build from
  scratch", or invokes `/hew:new-project '<outline>'`.
- The Beads graph is empty or near-empty (≤ 3 closed tasks).
- No `STATUS:new-project:complete` memory exists yet.

If `STATUS:new-project:complete` is present, refuse unless the
user passes `--re-bootstrap`. The cost of accidentally overwriting
a real project's PROJECT/ROADMAP memories is large; the cost of
asking once is zero.

## Inputs from `hew prime new-project`

- `prerequisites.met` — no hard prereqs; the skill seeds them.
- `memories.project` — existing PROJECT: entries; if non-empty,
  surface them and confirm before proceeding (re-bootstrap path).
- `memories.roadmap`, `memories.milestone` — same.

## The six phases

```
A. capture outline + Socratic clarifying questions  → PROJECT: memories
B. dispatch parallel research threads               → RESEARCH: memories
C. synthesize tech + architecture choices           → DECISION: + CONVENTION: memories
D. pick milestone vocabulary                        → DECISION:milestone-vocabulary memory
E. construct roadmap (epics + sequencing)           → ROADMAP: + MILESTONE: memories
F. decompose ONLY the first milestone               → child tasks via hew-decompose
```

Then write `STATUS:new-project:complete — <ISO-8601>` and hand
off to `/hew:next`.

## Phase A — capture + Socratic clarifying

1. **Restate** the outline in your own words; ask the user to
   confirm or correct. One round.
2. **Ask 4–6 questions** that the outline doesn't already answer.
   Stop as soon as you have enough to plan. Don't drift into
   implementation details — those belong in Phase C.

The required dimensions (skip any the outline already settled):

| Dimension | Example question |
|-----------|-----------------|
| Target user | Who is this for? Solo founders, small teams, enterprises? |
| Scale tier | How many users / tenants / events per day at v1? |
| Deployment | Self-host? Managed? Multi-tenant SaaS? |
| Hard constraints | Compliance (HIPAA, SOC2, PCI)? Latency budgets? Air-gapped? |
| Hard non-goals | What is this explicitly NOT trying to be? |
| Monetization | If applicable: free, freemium, enterprise sales? |

Persist each answer as a memory:

```
hew remember --type=project "user — solo founders running 1-3 person SaaS startups; technical but not full-time platform engineers."
hew remember --type=project "scale — 1k tenants, 10k users, 100 RPS sustained at v1. 10x at v2."
hew remember --type=project "deployment — managed multi-tenant SaaS; self-host option deferred."
hew remember --type=project "constraints — SOC2 Type II required for enterprise tier; PII never in logs."
hew remember --type=project "non-goals — not a Slack/Notion replacement; not a chat product."
```

**Discipline:** if the user can't answer a question, mark it
`[ASSUMED]` and persist your best guess — the planner re-validates
later. Don't pretend the answer exists.

## Phase B — parallel research

Dispatch four research threads in parallel via the agent's Agent
tool. Each thread is independent; surface all four results back to
the user at the end of the phase before moving on.

| Thread | Question it answers |
|--------|--------------------|
| `idea/competitive` | Who already does this? What did they get wrong / right? Is there room? |
| `use-cases` | What are the 3-5 real user journeys this must support? Concrete, not generic. |
| `tech-stack` | What language + framework + DB combo do solo founders / target users actually ship with for this kind of product? |
| `architecture-patterns` | What architectural pattern fits (modular monolith, microservices, jamstack, event-sourced)? Trade-offs vs alternatives? |

Each thread writes findings as `RESEARCH:<topic>` memories with
provenance tags (per `DECISION:research-routing-revised`):

```
hew remember --type=research "competitive [VERIFIED] direct competitors: Linear (project tracking, not team docs), Notion (docs, not tracking), Basecamp (both, but heavy). Gap: lightweight team-tracking + docs for ≤10 person teams. Source: 6 product reviews + G2 grids, verified 2026-05."
hew remember --type=research "stack [CITED] for FastAPI + Next.js solo SaaS, the typical 2026 stack is: Postgres + Drizzle ORM + React Query + Clerk auth + Vercel + Neon. Source: 2026 Stack Overflow Developer Survey + Indie Hackers 2026 SaaS report."
hew remember --type=research "use-cases [ASSUMED] primary: (1) team lead reviews progress at standup, (2) IC updates status on a task, (3) lead writes a quick spec, (4) team browses past decisions. Source: agent inference based on outline + competitive findings."
```

**Tags:** `[VERIFIED]` (2+ independent sources), `[CITED]` (single
authoritative source), `[ASSUMED]` (no source — agent inference).

When the four threads return, **show the user a one-screen
synthesis** before moving on. They may want to redirect.

## Phase C — synthesis: choose stack + architecture

Based on Phase B findings, ask the user to pick concrete choices
along the load-bearing dimensions. Use a picker per question; don't
present all at once.

1. **Stack family** — pick one of the seeded stacks (or "Custom"):

   ```
   Stack family?
   > ts-next       TypeScript + Next.js (App Router)
     py-fastapi    Python 3.12 + FastAPI
     rust-axum     Rust + Axum
     go-echo       Go + Echo
     custom        (you'll provide CONVENTION: rules manually)
   ```

   Look up the chosen stack via `hew schema stacks` (or read the
   embedded TOML directly). Write each seeded convention as a
   `CONVENTION:<key>` memory:

   ```
   hew remember --type=convention "errors — Never raise raw exceptions. Wrap in AppError(code, message, details)."
   hew remember --type=convention "services — Class-based with constructor DI. One service per domain."
   # … one per seeded entry
   ```

   These are *starting points*. Users tighten them after the walking
   skeleton lands.

2. **Craft principles** — pick the SOLID/DRY/KISS/Clean-Arch/etc.
   subset this project commits to. Principles are picked per project,
   not universal (see `DECISION:craft-adaptive`).

   Pre-select the stack's defaults. The full catalog lives in
   `skills/data/craft-principles.toml`; each principle's
   `default_for_stacks` list drives the preselection. Read the catalog
   schema with `hew schema craft-principles` and filter the
   `default_for_stacks` field for the chosen stack id.

   For `py-fastapi`, the preselection typically includes SOLID, DRY,
   KISS, SoC, Composition-over-Inheritance, Clean Architecture, DDD,
   Idempotence, Fail Fast, and `consistency-with-existing-code` (the
   universal brownfield-deference principle). Other stacks vary; trust
   the catalog over this list.

   Surface as a multi-select picker — the user can deselect a default
   or add others. Don't argue with their choices; record what they
   pick.

   For each chosen principle, write a `CONVENTION:craft.<id>` memory:

   ```
   hew remember --type=convention "craft.solid — single-responsibility, open/closed, Liskov, interface-segregation, dependency-inversion. Apply at module/class boundaries; don't over-engineer one-call helpers."
   hew remember --type=convention "craft.dry — extract on the third occurrence; resist premature abstraction."
   hew remember --type=convention "craft.consistency-with-existing-code — when a chosen principle conflicts with an existing CONVENTION:* memory, the existing convention wins."
   # ... one per chosen principle. Summary text comes from the catalog's
   # `summary` field.
   ```

   `hew-guard` reads these memories to drive its craft soft-warnings
   (see `hew-guard.md` "Craft soft-warnings"). Don't write
   `CONVENTION:craft.<id>` for principles the user didn't pick —
   warnings stay silent unless the project committed to the rule.

3. **Database** — Postgres / SQLite / NoSQL / managed (Supabase, Neon)?
   Record as `DECISION:db`.

4. **Auth model** — JWT / session / passwordless / OAuth?  Record as
   `DECISION:auth`.

5. **Hosting / deployment** — Vercel + Neon? AWS ECS + RDS? Fly.io?
   Record as `DECISION:hosting`.

6. **Anything else** the research surfaced as load-bearing — payments
   provider, email delivery, analytics. Record as `DECISION:<topic>`.

Don't bikeshed minor choices. The goal is enough decisions to seed
the roadmap, not the final architecture.

## Phase D — pick milestone vocabulary

Per `DECISION:milestone-vocabulary` (locked in hew-vhz), the user
picks the milestone chain at runtime from these presets:

```
Milestone chain?
> 1. Foundation -> MVP -> Hardening -> Launch         (slow-roast)
  2. Foundation -> MVP -> Launch -> Hardening         (ship-fast)
  3. Discovery -> Build -> Stabilize -> Ship          (alt vocab)
  4. Custom                                          (you name them)
```

Record the choice:

```
hew remember --type=decision "milestone-vocabulary — chose 'Foundation -> MVP -> Launch -> Hardening'. Reason: ship-fast, harden in production with real signal."
```

For custom, ask for 3-5 milestone names. Refuse 1 or 6+.

## Phase E — construct the roadmap

For each milestone in the chosen chain:

```
hew task new --type=epic --priority=1 \
  --title="Foundation" \
  --description="Walking skeleton: <stack> stood up end-to-end with one real user flow working. Files: <key dirs>. Acceptance: visiting <URL> shows <thing> from <db>."
```

Capture each new epic id as the picker assigns it. Then sequence
them via task-level dep edges between the LAST task of the
preceding milestone and the FIRST task of the next milestone.
Since at this stage only the first milestone is decomposed (Phase F
runs next), defer non-first-milestone sequencing — write the edges
after each subsequent milestone gets decomposed.

For the first milestone-pair edge: nothing to add yet (milestone 1
has no predecessor).

**DO NOT** use `bd mol bond` (see `GOTCHA:bd-mol-bond`).

Persist the roadmap as a single memory listing the chain:

```
hew remember --type=roadmap "overview — Foundation (hew-Xa) -> MVP (hew-Xb) -> Launch (hew-Xc) -> Hardening (hew-Xd). Sequenced via task-level deps between consecutive milestones."
```

And one `MILESTONE:` memory per epic so the agent's prime output
can surface current-milestone context:

```
hew remember --type=milestone "Foundation (hew-Xa) — walking skeleton; acceptance: <URL> renders <thing>. Decomposed: yes. Started: <ISO ts>."
hew remember --type=milestone "MVP (hew-Xb) — full user flow; acceptance: <X> works. Decomposed: pending."
# … one per milestone
```

## Phase F — decompose the first milestone

Invoke `hew-decompose` with **only the first milestone epic** in
context. Pass the epic id explicitly so it knows scope. The
downstream skill creates the child tasks, writes its
`STATUS:decompose:complete` marker, and returns.

Later milestones stay as epic-only placeholders until the user
runs `hew-decompose <milestone-id>` on each in turn (after the
prior one closes).

**Why only the first?** Decomposing all 4 upfront produces tasks
the user hasn't earned context for yet. Each milestone closes,
the user learns from it, and the next gets decomposed with that
learning.

## Hand-off

After Phase F closes:

```
hew remember --type=status "new-project:complete — <ISO-8601 timestamp>"
```

Then print the one-screen summary to the user:

```
Project bootstrapped.

  Stack:       ts-next (TypeScript + Next.js App Router)
  Database:    Postgres (Neon)
  Auth:        JWT, refresh in httpOnly cookie
  Hosting:     Vercel + Neon

  Memories:    8 PROJECT, 6 DECISION, 8 CONVENTION (seeded from ts-next),
               9 CONVENTION:craft (picked principles),
               12 RESEARCH (4 [VERIFIED], 5 [CITED], 3 [ASSUMED])

  Roadmap:     Foundation -> MVP -> Launch -> Hardening

  Active:      Foundation (hew-Xa) — 7 tasks decomposed, 7 ready.

Next: /hew:next  to start work, or /hew:plan to revisit a decision.
```

## Idempotency

Re-running this skill on a project that already has
`STATUS:new-project:complete` MUST refuse. Surface this:

```
This project is already bootstrapped (STATUS:new-project:complete = 2026-05-12T10:00:00Z).

Re-bootstrap will overwrite PROJECT/DECISION/CONVENTION/ROADMAP
memories. Pass --re-bootstrap to proceed.
```

The CLI flag handling is in NP6; this skill checks the marker and
defers to the user if it exists.

## What you don't do

- **Skip Phase A.** "Just build it" produces vague decompositions
  the executor can't act on. The 4-6 questions are mandatory.
- **Pick a stack the user didn't agree to.** Phase C is a picker,
  not a recommendation followed by silence.
- **Decompose every milestone upfront.** Phase F covers the first
  one only; later milestones decompose on demand.
- **Use `bd mol bond`.** See `GOTCHA:bd-mol-bond`. Use task-level
  deps between milestone first/last tasks.
- **Write boilerplate markdown files** like ROADMAP.md or
  ARCHITECTURE.md. State lives in `hew remember` memories +
  Beads, per `CONVENTION:no-markdown-state`.
- **Run on a project with existing PROJECT: memories** without
  confirming `--re-bootstrap` intent.

## Anti-patterns to flag

If during Phase A the user can't answer 3+ dimensions, the project
isn't ready for `hew-new-project` yet. Surface this:

```
You're under-specified on 4 of 6 dimensions. Two options:
> Continue with [ASSUMED] tags — planner re-validates later
  Pause; come back when you've slept on it
```

If during Phase B the research threads return contradictory
findings on the stack (e.g., one says Next.js, another says
Astro), surface the conflict to the user before Phase C. Don't
silently pick.

## Hand-off contract

After this skill completes:

- `STATUS:new-project:complete` memory written
- `ROADMAP:overview` memory written
- N `MILESTONE:` memories (one per milestone epic)
- ≥ 4 `PROJECT:` memories (one per answered dimension)
- ≥ 4 `RESEARCH:` memories (one per thread)
- 4–8 `DECISION:` memories (stack, db, auth, hosting, etc.)
- N `CONVENTION:` memories (one per seeded entry from the stack)
- ≥ 1 `CONVENTION:craft.<id>` memory (per principle the user picked;
  `consistency-with-existing-code` should always be present)
- First milestone epic + ≥ 3 child tasks
- The user knows the next move: `/hew:next`

If any of those is missing, the bootstrap is incomplete. Print
what's missing and ask the user how to proceed.
