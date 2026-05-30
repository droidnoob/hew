<!-- hew:version=0.11.0 -->
---
name: hew-scan
category: brownfield
init: hew prime scan
---

# hew-scan — Architecture Mapper for Existing Codebases

You walk an existing codebase and turn what you find into **discrete,
retrievable memories** via `hew remember`. Not a summary document. Not a
big markdown file. Individual facts the agent can recall on demand.

Why this matters: a summary doc rots the moment the code changes. A
collection of small memories ages gracefully — wrong facts get corrected
one at a time, new facts get appended as the project grows. And `hew
prime` injects all memories automatically, so future sessions inherit
everything you discover without anyone reading a doc.

## When this skill runs

- Brownfield project initialization (`hew init` detected an existing
  codebase and the user said "scan it").
- `STATUS:scan` is not `complete` (`hew prime scan` reports it under
  `status.scan`).
- The user explicitly asks for a re-scan after a major refactor.

If the project is greenfield (no existing source code), skip this skill
entirely.

## Inputs from `hew prime scan`

- `project.beads_initialized` — confirm `hew remember` will land somewhere.
- `memories` — pre-existing memories from prior scans or executor
  discoveries. Don't duplicate; extend.
- The current working directory is the project root.

## Ask once: how detailed should the memories be?

Before starting the scan, ask the user — using the host's choice
picker (`AskUserQuestion` in Claude Code; equivalent elsewhere) —
which depth applies to this codebase:

- **Terse** — one-line facts only. Best for small projects, quick
  onboarding, or projects where the user already knows the
  architecture and just wants the agent to inherit it.
- **Balanced** (default) — short for simple facts (tech stack,
  layout) and detailed where the fact warrants it (gotchas,
  boundaries, complex coupling).
- **Detailed** — multi-paragraph for non-trivial facts, code-sample
  inclusion welcome. Best for projects the agent will work in
  heavily over multiple sessions.

The choice applies to the *whole brownfield chain* (this scan plus
the convention / audit / boundary skills that follow). Do not ask
again per-step.

## The scan loop

```
1. detect tech stack
2. map the file layout
3. extract API + data shapes
4. extract auth + security baseline
5. extract data layer (DB, ORM, models)
6. extract testing setup
7. extract CI/CD + deployment
8. extract environment configuration
9. find non-obvious coupling
10. write STATUS:scan:complete
```

Each step produces zero or more `hew remember` calls. Each memory is a
short, self-contained fact. **One fact per memory.** "FastAPI" is one
fact; "FastAPI + SQLAlchemy + Postgres" is three.

## Memory shape — one fact per memory, as long as the fact demands

Each memory is one self-contained fact. **Length is whatever that
fact needs.** A tech-stack line is one sentence; a non-obvious
gotcha with a code sample is several. No padding either way.

Good (terse — single facts):

```
hew remember --type=factual "Backend: FastAPI 0.115 on uvicorn; entry app/main.py."
hew remember --raw "ORM:SQLAlchemy 2.x async; session factory in app/db/session.py."
hew remember --type=factual "Auth: JWT (jose) with refresh rotation; middleware in app/auth/middleware.py."
```

Good (detailed — the fact warrants depth):

```
hew remember --type=boundary "webhook/stripe — POST /api/v1/webhooks/stripe accepts the raw body with Stripe-Signature header. Idempotency-key consumed via app/middleware/idempotency.py; replays return 200 + cached response. Verify signature BEFORE reading body. Five frontend consumers depend on the redirect-back behavior; do not change."

hew remember --type=gotcha "order.create — order_service.create() implicitly creates an Invoice via signals/post_save (app/signals/order.py:18). Not visible from the route; mocking the order service alone in tests leaves dangling Invoice rows. Use the factory in tests/factories/order_with_invoice.py instead."
```

Bad:

```
# Too vague (no information):
hew remember --type=factual "Backend uses Python."

# Too compound (multiple facts in one memory — split):
hew remember --type=factual "Uses FastAPI, SQLAlchemy, Postgres, Redis, structlog, pydantic, alembic, pytest, and Docker for deployment."

# Padding without information:
hew remember --type=factual "The backend is a FastAPI application that was originally
written as a Flask service and then migrated. It has gone through
several iterations of the ORM layer, currently using SQLAlchemy 2.x..."
```

Rule of thumb: one fact per memory; trim everything that doesn't add
information; otherwise use the length you need.

## Step 1 — Tech stack

For the languages and frameworks in use, record:

- Language + version (read from `Cargo.toml` / `pyproject.toml` /
  `package.json` / `go.mod`).
- Framework + version (web, ORM, queue, auth lib).
- Build / package manager (`uv`, `poetry`, `pnpm`, `cargo`).
- Notable dev tooling (formatter, linter, type checker, test runner).

```
hew remember --type=factual "Lang: Python 3.12 (pyproject.toml). Package manager: uv."
hew remember --type=factual "Web framework: FastAPI 0.115. Entry: app/main.py."
hew remember --type=factual "Test runner: pytest 8 with pytest-asyncio. Config: pyproject.toml [tool.pytest.ini_options]."
hew remember --type=factual "Formatter: ruff format. Linter: ruff check (rules in ruff.toml). Type: mypy --strict."
```

## Step 2 — File layout

Don't enumerate every file. Record the *organizing principle* and the
notable directories.

```
hew remember --type=factual "Layout: monorepo. backend/ (FastAPI), frontend/ (Next.js), packages/types/ (shared)."
hew remember --type=factual "Backend dirs: app/api/ (routes), app/services/ (business logic), app/db/ (ORM/session), app/auth/ (auth)."
hew remember --type=factual "Frontend dirs: src/app/ (routes, app router), src/components/ (UI), src/lib/ (utilities)."
```

This answers "where do I put X?" — usually the most-asked question in
brownfield work.

## Step 3 — API + data shapes

For the API surface:

- Routing pattern (`/api/v1/{resource}`, REST conventions, GraphQL).
- Auth requirements (which routes need it, which are public).
- Response shape conventions (envelope, error format, pagination).

```
hew remember --raw "API:routes under /api/v1/{resource}. Defined in app/api/v1/*.py and registered in app/api/__init__.py."
hew remember --type=factual "API errors: AppError(code, message, details) wrapped by error_middleware; routes never raise raw exceptions."
hew remember --type=factual "Pagination: cursor-based via {next_cursor, results} envelope; limit 50, max 200."
```

For shared data shapes (DTOs, schemas), point to the canonical file.

## Step 4 — Auth + security baseline

```
hew remember --type=factual "Auth: JWT access (15min) + refresh (7d, httpOnly cookie, rotates on use). app/auth/jwt.py."
hew remember --type=factual "Authz: per-route Depends(require_role) decorator; roles in app/auth/roles.py."
hew remember --type=factual "Password hashing: argon2id via passlib. Never bcrypt in this codebase."
```

If you discover security-relevant patterns (CSRF, CORS, rate limiting,
input validation), also write `SECURITY:` memories — `hew-execute` and
`hew-guard` route on the prefix:

```
hew remember --type=security "All endpoints accepting user input run through validate_input() (app/security/validate.py)."
hew remember --type=security "CORS allow-list in app/main.py — never use allow_origins=['*']."
```

## Step 5 — Data layer

```
hew remember --raw "DB:Postgres 16. Connection string from DATABASE_URL env."
hew remember --raw "ORM:SQLAlchemy 2.x async. Session factory async_session_maker() in app/db/session.py."
hew remember --type=factual "Models: app/models/*.py, all inherit from BaseModel mixin (UUID PK, created_at, updated_at)."
hew remember --type=factual "Migrations: Alembic under alembic/versions/. Auto-generate via `alembic revision --autogenerate -m '...'`."
hew remember --type=factual "Repository pattern: data access in app/repos/*.py; routes never query directly."
```

If migrations or schema-evolution discipline matters (it almost always
does), write a `MIGRATION:` baseline memory:

```
hew remember --raw "MIGRATION:Every model change requires an Alembic migration. Never edit migrations after they've been applied to a shared environment."
```

## Step 6 — Testing setup

```
hew remember --type=factual "Tests: pytest with async fixtures in tests/conftest.py."
hew remember --type=factual "Test DB: testcontainers/postgres. Fresh DB per test module via fixture."
hew remember --type=factual "Fixtures: factory_boy; one factory per model in tests/factories/."
hew remember --type=factual "Coverage: pytest-cov, threshold 80% enforced in CI (.github/workflows/test.yml)."
```

If TDD or specific testing styles are in use, note that as a convention
(handled in `hew-convention`).

## Step 7 — CI/CD + deployment

```
hew remember --raw "CI:GitHub Actions. Workflow in .github/workflows/ci.yml. Runs lint + type + test on every PR."
hew remember --type=factual "Deploy: Docker on AWS ECS Fargate. Terraform in infra/. Deploy via GitHub Actions on push to main."
hew remember --type=factual "Env: prod, staging, dev. Config in infra/{env}/terraform.tfvars."
```

If there are notable deploy gotchas (secrets management, blue/green, DB
migrations on deploy), record them.

## Step 8 — Environment configuration

```
hew remember --type=factual "Env config: pydantic-settings; loaded at startup in app/config.py. Required vars validated on boot."
hew remember --type=factual "Env vars list: DATABASE_URL, JWT_SECRET, STRIPE_KEY, SENDGRID_KEY. Documented in .env.example."
hew remember --type=factual "Frontend env: only NEXT_PUBLIC_* vars are inlined at build; secrets stay server-side."
```

## Step 9 — Non-obvious coupling and gotchas

These are the most valuable scan outputs. Things that would surprise a
new contributor or get an agent yelled at.

```
hew remember --type=gotcha "payment webhooks require idempotency_key check before processing — handler in app/api/v1/webhooks/stripe.py."
hew remember --type=gotcha "user deletion is SOFT-DELETE only (is_active=False). Never DELETE rows; downstream consumers break."
hew remember --type=gotcha "frontend build requires NEXT_PUBLIC_ prefix for ALL env vars used in the bundle. Missing = silent runtime error."
hew remember --raw "COUPLING:order_service.create() implicitly creates an invoice via signals/post_save; not visible from the order endpoint."
hew remember --type=gotcha "pytest must run with --forked when testing the cache layer (Redis connection state leaks otherwise)."
```

These come from reading the code carefully and looking for the things
that *would not be obvious from a 30-second skim*.

## Step 10 — Mark the phase complete

After the scan finishes:

```
hew remember --type=status "scan:complete — <ISO-8601 timestamp>"
```

This unblocks `hew-convention`, `hew-audit`, `hew-boundary`, and
`hew-migrate` (their prerequisites check for `STATUS:scan`).

## How to actually walk the codebase

You don't have to read every file. Sample intelligently:

1. **Read the manifests first** — `package.json`, `pyproject.toml`,
   `Cargo.toml`, `go.mod`, `requirements.txt`. They tell you the stack
   without reading any code.
2. **Read the entry points** — `main.py`, `index.ts`, `cmd/main.go`,
   `src/main.rs`. Confirm how the app actually boots.
3. **Read the layout** — `find . -maxdepth 3 -type d` (or just `ls`
   the top dirs). Pattern-match against common conventions.
4. **Read one of each notable kind of file** — one route, one model,
   one service, one test. Find the *patterns*, not the totals.
5. **Read the CI workflow** — that tells you what discipline the
   project actually enforces vs aspires to.
6. **Read the README and `*.md` docs** *last* — they're often stale.
   Use them to confirm or cross-check what you found in code, not as
   primary source.

Don't read all the routes, all the models, all the tests. Read enough
to pattern-match; let `hew-convention` handle the pattern extraction.

## When to stop

A complete scan on a medium codebase produces roughly 20–40 memories
across the eight content steps. Less than ~10 and you skipped something.
More than ~60 and you're recording trivia.

A good test: skim the memories you wrote. Could a new agent, primed
with just those memories, sensibly start working on this codebase? If
yes, stop. If they'd still hit "where do I put X?" or "how do I do
auth?" — add more memories targeting those gaps.

## What you don't do

- **Write a CODEBASE.md or summary doc.** Memories only. The plan is
  explicit on this.
- **Record opinions.** "This code is messy" is not useful. "Order
  service has 1200 lines and 4 unrelated responsibilities" is — and it
  belongs in `hew-audit`'s output, not `hew-scan`.
- **Record `CONVENTION:` rules.** That's `hew-convention`'s job. Scan
  captures *what exists*; convention captures *how to write matching
  code*. Different prefix, different skill.
- **Try to be exhaustive.** Hitting all eight content steps with ~3–5
  memories each is better than 100 memories on tech stack and zero on
  testing.
- **Skip the gotchas step.** Step 9 is the highest-value output.

## Hand-off

The brownfield onboarding chain is `hew-scan → hew-convention →
hew-audit → hew-boundary`. Run it through without pausing between
skills. Stopping after each step to ask the user "continue?" defeats
the chain — they already invoked the chain.

When the scan is complete:

1. Write `STATUS:scan:complete`.
2. Print a one-line summary: "Scan complete: N memories across X
   content steps."
3. **Continue directly into `hew-convention`.** Do not stop unless
   the user explicitly asked for "scan only" or you hit a Rule-4
   architectural surprise that needs human input.

Standalone `/hew:scan` invocations (when that command exists) are
the only case where scan stops here.

## Anti-patterns

- **Long compound memories** ("Uses FastAPI and SQLAlchemy and Postgres
  and Redis..."). Split them.
- **Vague memories** ("Uses some kind of auth"). If you can't pin it
  down, the memory has no value.
- **Memories without file paths** when a path would help. "Auth
  middleware exists" vs "Auth middleware: `app/auth/middleware.py`."
- **Recording duplicate memories** that overlap with existing ones —
  check `hew prime scan` first.
- **Recording opinions or recommendations** — wrong skill. Open a chore
  task or a `hew-audit` finding instead.
