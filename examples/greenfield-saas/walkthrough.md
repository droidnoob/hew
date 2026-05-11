# Walkthrough — Greenfield SaaS

A complete end-to-end build using Hew. Empty repo to working auth +
billing slice in roughly one session.

## Goal

> "Build a small SaaS — users can sign up, log in, subscribe via Stripe,
> and access a paid dashboard. Stack is up to you."

## Step 0 — Initialize

```
$ cd ~/projects/notebook-saas
$ hew init
hew init: claude detected, writing .claude/skills/hew/ + .claude/commands/hew/
hew init: bd init (.beads/ added to .gitignore)
hew installed for claude (Local scope) → 38 files under /Users/.../notebook-saas
```

Open Claude Code in the project. Type:

> Plan and start building this. Use /hew:plan.

## Step 1 — Plan

The agent invokes `hew-plan`. Output:

```
Plan — Notebook SaaS

Goal: paid Notebook SaaS — sign up, log in, subscribe via Stripe, access
paid dashboard.

Acceptance criteria:
1. Anonymous user can sign up with email + password.
2. Returning user can log in; access token lasts 15 min, refresh 7 days.
3. Logged-in user can start a Stripe checkout session for a single plan.
4. After successful checkout, /dashboard renders user's plan name.
5. Logged-out user hitting /dashboard is redirected to /login.
6. All endpoints under /api/v1/dashboard require an active subscription.

Architecture: Next.js 14 (app router) frontend, FastAPI backend, Postgres
DB, Stripe Checkout for payments. JWT auth (jose), httpOnly refresh
cookies. Monorepo with backend/ and frontend/.

Order of work:
  1. Walking skeleton: signup -> login -> protected /me endpoint (slice).
  2. Stripe checkout integration (slice).
  3. Subscription gate on /dashboard (slice).

Graph shape: single epic "Notebook SaaS" with 3 vertical-slice child
tasks. Each slice closes when its acceptance bullet ships.

Open questions: none — proceed?
```

User says "proceed."

## Step 2 — Decompose

`hew-decompose` translates the plan into Beads.

```
bd create --type=epic --priority=1 --title="Notebook SaaS v1"
# → bd-saas

bd create --parent=bd-saas --title="Walking skeleton: signup + login + /me" \
  --acceptance="signup creates user, login returns tokens, /me returns user when authed, 401 when not"
# → bd-saas.1

bd create --parent=bd-saas --title="Stripe checkout integration" \
  --acceptance="POST /api/v1/billing/checkout creates a Stripe Checkout session, redirects back, webhook updates user.plan"
# → bd-saas.2

bd create --parent=bd-saas --title="Dashboard subscription gate" \
  --acceptance="GET /api/v1/dashboard returns 200 with plan name for active subscribers, 402 otherwise. Frontend redirects on 402."
# → bd-saas.3

bd dep add bd-saas.2 bd-saas.1
bd dep add bd-saas.3 bd-saas.2
```

`bd dep tree bd-saas`:

```
bd-saas Notebook SaaS v1 [epic] [P1] (open)
  ├── bd-saas.1 Walking skeleton (open)             ← READY
  ├── bd-saas.2 Stripe checkout (open)              ← blocked by .1
  └── bd-saas.3 Dashboard gate (open)               ← blocked by .2
```

## Step 3 — Execute the walking skeleton

User says "/hew:next."

The agent runs `hew prime execute`, picks `bd-saas.1`, claims it, and:

- Scaffolds the backend (FastAPI + SQLAlchemy + Alembic).
- Adds POST /api/v1/auth/signup, POST /api/v1/auth/login, GET /api/v1/auth/me.
- Writes pytest tests for each.
- Scaffolds the frontend (Next.js app router) with /signup, /login pages.
- Runs `hew-guard` — pass.
- `bd close bd-saas.1 --reason "Walking skeleton: signup creates row, login issues tokens, /me reads JWT. 12 tests pass."`
- Commits: `feat(auth): walking skeleton — signup + login + /me`.

Memories captured along the way:

```
bd remember "DECISION:auth — JWT access 15min + refresh 7d, httpOnly cookie via jose 5.x."
bd remember "CONVENTION:errors — All routes raise AppError(code, message, details); middleware translates to HTTP."
bd remember "CONVENTION:services — Constructor-DI service classes in app/services/; routes are thin."
bd remember "BOUNDARY: POST /api/v1/auth/signup — body {email, password}, returns 201 {id, email}."
bd remember "BOUNDARY: POST /api/v1/auth/login — body {email, password}, returns 200 {access, refresh_in_cookie}."
```

## Step 4 — Stripe integration

`bd-saas.2` becomes ready. The agent claims it.

Before adding `stripe` as a dependency it invokes `hew-deps stripe@latest`.
Verdict: ADOPT. Latest stable, MIT, active.

Implementation: checkout session endpoint, webhook handler, idempotency
key handling. `hew-guard` flags an issue — webhook handler doesn't verify
the Stripe signature. Agent fixes inline (Rule 2: missing critical
functionality), re-runs guard, closes.

```
bd remember "SECURITY: Stripe webhook /api/v1/billing/webhook MUST verify Stripe-Signature header before processing the body."
bd remember "MIGRATION:002_add_user_plan — added users.plan_id (FK -> plans), users.subscription_status."
```

## Step 5 — Dashboard gate

`bd-saas.3`. Subscription middleware, frontend redirect on 402. Tests
cover paid + unpaid + expired states. Close + commit.

## Step 6 — Verify

User says "/hew:verify."

`hew-verify` runs the four dimensions:

```
VERIFY: pass
[1] Tests: 47 pass, 0 fail (coverage 81%)
[2] Acceptance: 6/6 met
[3] Boundaries: 5 checked, all unchanged
[4] Golden path: signup → login → checkout → /dashboard end-to-end OK
```

Epic auto-closes. `STATUS:verify:bd-saas:complete` written.

## Done

Total session: ~90 minutes of agent work, 12 commits, 3 epic children
closed. The Beads graph is the source of truth; the agent can resume
into the next epic by running `hew prime execute` on the next session.
