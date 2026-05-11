<!-- hew:version=0.1.0 -->
---
name: hew-scan
category: brownfield
init: hew prime scan
---

# hew-scan — Architecture Mapper for Existing Codebases

You walk an existing codebase and turn what you find into **discrete,
retrievable memories** via `bd remember`. Not a summary document. Not a
big markdown file. Individual facts the agent can recall on demand.

Why this matters: a summary doc rots the moment the code changes. A
collection of small memories ages gracefully — wrong facts get corrected
one at a time, new facts get appended as the project grows. And `bd
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

- `project.beads_initialized` — confirm `bd remember` will land somewhere.
- `memories` — pre-existing memories from prior scans or executor
  discoveries. Don't duplicate; extend.
- The current working directory is the project root.

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

Each step produces zero or more `bd remember` calls. Each memory is a
short, self-contained fact. **One fact per memory.** "FastAPI" is one
fact; "FastAPI + SQLAlchemy + Postgres" is three.

## Memory shape — short, scannable, self-contained

Good memories:

```
bd remember "Backend: FastAPI 0.115 on uvicorn; entry point app/main.py."
bd remember "ORM: SQLAlchemy 2.x async; session factory in app/db/session.py."
bd remember "DB: Postgres 16, Alembic for migrations under alembic/versions/."
bd remember "Auth: JWT (jose) with refresh rotation; middleware in app/auth/middleware.py."
bd remember "Frontend: Next.js 14 app router; Tailwind + shadcn/ui; pnpm."
```

Bad memories:

```
# Too vague:
bd remember "Backend uses Python."

# Too compound (should be 4 memories):
bd remember "Uses FastAPI, SQLAlchemy, Postgres, Redis, structlog, pydantic, alembic, pytest, and Docker for deployment."

# Too narrative:
bd remember "The backend is a FastAPI application that was originally written as a Flask service and then migrated. It has gone through several iterations of the ORM layer, currently using SQLAlchemy 2.x..."
```

Length target: 1–3 sentences, plus optional file path in backticks.

## Step 1 — Tech stack

For the languages and frameworks in use, record:

- Language + version (read from `Cargo.toml` / `pyproject.toml` /
  `package.json` / `go.mod`).
- Framework + version (web, ORM, queue, auth lib).
- Build / package manager (`uv`, `poetry`, `pnpm`, `cargo`).
- Notable dev tooling (formatter, linter, type checker, test runner).

```
bd remember "Lang: Python 3.12 (pyproject.toml). Package manager: uv."
bd remember "Web framework: FastAPI 0.115. Entry: app/main.py."
bd remember "Test runner: pytest 8 with pytest-asyncio. Config: pyproject.toml [tool.pytest.ini_options]."
bd remember "Formatter: ruff format. Linter: ruff check (rules in ruff.toml). Type: mypy --strict."
```

## Step 2 — File layout

Don't enumerate every file. Record the *organizing principle* and the
notable directories.

```
bd remember "Layout: monorepo. backend/ (FastAPI), frontend/ (Next.js), packages/types/ (shared)."
bd remember "Backend dirs: app/api/ (routes), app/services/ (business logic), app/db/ (ORM/session), app/auth/ (auth)."
bd remember "Frontend dirs: src/app/ (routes, app router), src/components/ (UI), src/lib/ (utilities)."
```

This answers "where do I put X?" — usually the most-asked question in
brownfield work.

## Step 3 — API + data shapes

For the API surface:

- Routing pattern (`/api/v1/{resource}`, REST conventions, GraphQL).
- Auth requirements (which routes need it, which are public).
- Response shape conventions (envelope, error format, pagination).

```
bd remember "API: routes under /api/v1/{resource}. Defined in app/api/v1/*.py and registered in app/api/__init__.py."
bd remember "API errors: AppError(code, message, details) wrapped by error_middleware; routes never raise raw exceptions."
bd remember "Pagination: cursor-based via {next_cursor, results} envelope; limit 50, max 200."
```

For shared data shapes (DTOs, schemas), point to the canonical file.

## Step 4 — Auth + security baseline

```
bd remember "Auth: JWT access (15min) + refresh (7d, httpOnly cookie, rotates on use). app/auth/jwt.py."
bd remember "Authz: per-route Depends(require_role) decorator; roles in app/auth/roles.py."
bd remember "Password hashing: argon2id via passlib. Never bcrypt in this codebase."
```

If you discover security-relevant patterns (CSRF, CORS, rate limiting,
input validation), also write `SECURITY:` memories — `hew-execute` and
`hew-guard` route on the prefix:

```
bd remember "SECURITY: All endpoints accepting user input run through validate_input() (app/security/validate.py)."
bd remember "SECURITY: CORS allow-list in app/main.py — never use allow_origins=['*']."
```

## Step 5 — Data layer

```
bd remember "DB: Postgres 16. Connection string from DATABASE_URL env."
bd remember "ORM: SQLAlchemy 2.x async. Session factory async_session_maker() in app/db/session.py."
bd remember "Models: app/models/*.py, all inherit from BaseModel mixin (UUID PK, created_at, updated_at)."
bd remember "Migrations: Alembic under alembic/versions/. Auto-generate via `alembic revision --autogenerate -m '...'`."
bd remember "Repository pattern: data access in app/repos/*.py; routes never query directly."
```

If migrations or schema-evolution discipline matters (it almost always
does), write a `MIGRATION:` baseline memory:

```
bd remember "MIGRATION: Every model change requires an Alembic migration. Never edit migrations after they've been applied to a shared environment."
```

## Step 6 — Testing setup

```
bd remember "Tests: pytest with async fixtures in tests/conftest.py."
bd remember "Test DB: testcontainers/postgres. Fresh DB per test module via fixture."
bd remember "Fixtures: factory_boy; one factory per model in tests/factories/."
bd remember "Coverage: pytest-cov, threshold 80% enforced in CI (.github/workflows/test.yml)."
```

If TDD or specific testing styles are in use, note that as a convention
(handled in `hew-convention`).

## Step 7 — CI/CD + deployment

```
bd remember "CI: GitHub Actions. Workflow in .github/workflows/ci.yml. Runs lint + type + test on every PR."
bd remember "Deploy: Docker on AWS ECS Fargate. Terraform in infra/. Deploy via GitHub Actions on push to main."
bd remember "Env: prod, staging, dev. Config in infra/{env}/terraform.tfvars."
```

If there are notable deploy gotchas (secrets management, blue/green, DB
migrations on deploy), record them.

## Step 8 — Environment configuration

```
bd remember "Env config: pydantic-settings; loaded at startup in app/config.py. Required vars validated on boot."
bd remember "Env vars list: DATABASE_URL, JWT_SECRET, STRIPE_KEY, SENDGRID_KEY. Documented in .env.example."
bd remember "Frontend env: only NEXT_PUBLIC_* vars are inlined at build; secrets stay server-side."
```

## Step 9 — Non-obvious coupling and gotchas

These are the most valuable scan outputs. Things that would surprise a
new contributor or get an agent yelled at.

```
bd remember "GOTCHA: payment webhooks require idempotency_key check before processing — handler in app/api/v1/webhooks/stripe.py."
bd remember "GOTCHA: user deletion is SOFT-DELETE only (is_active=False). Never DELETE rows; downstream consumers break."
bd remember "GOTCHA: frontend build requires NEXT_PUBLIC_ prefix for ALL env vars used in the bundle. Missing = silent runtime error."
bd remember "COUPLING: order_service.create() implicitly creates an invoice via signals/post_save; not visible from the order endpoint."
bd remember "GOTCHA: pytest must run with --forked when testing the cache layer (Redis connection state leaks otherwise)."
```

These come from reading the code carefully and looking for the things
that *would not be obvious from a 30-second skim*.

## Step 10 — Mark the phase complete

After the scan finishes:

```
bd remember "STATUS:scan:complete — <ISO-8601 timestamp>"
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

When the scan is complete:

1. Print the count of memories created, grouped by content step.
2. Tell the user: "Scan complete. Run `hew-convention` next to extract
   `CONVENTION:` rules from existing code."
3. Write `STATUS:scan:complete`.
4. Stop. Don't continue into `hew-convention` automatically — it's a
   distinct skill the user invokes explicitly.

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
