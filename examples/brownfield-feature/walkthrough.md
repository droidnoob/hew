# Walkthrough — Brownfield: Add a Feature to an Existing Repo

Scenario: an existing FastAPI + Next.js codebase. User wants to add
team workspaces (multi-tenant) to a previously single-tenant app.

## Step 0 — Initialize

```
$ cd ~/projects/notebook-saas
$ hew init
hew init: claude detected, writing .claude/skills/hew/ + .claude/commands/hew/
hew init: bd already initialized, skipping
hew init: .beads/ already in .gitignore
hew installed for claude (Local scope) → 38 files
```

The user says:

> Add team workspaces — multiple users per team, shared notebooks. Use
> /hew:plan but start with /hew:scan first since you don't know this
> codebase yet.

## Step 1 — Brownfield onboarding

The agent recognizes the brownfield trigger and runs the onboarding
chain *before* planning. From `hew-decompose`:

```
bd create --type=epic --priority=0 --title="Codebase Onboarding"
# → bd-onb

bd create --parent=bd-onb --title="Scan architecture"        # → bd-onb.1
bd create --parent=bd-onb --title="Extract conventions"      # → bd-onb.2
bd create --parent=bd-onb --title="Audit dependencies"       # → bd-onb.3
bd create --parent=bd-onb --title="Map API boundaries"       # → bd-onb.4

bd dep add bd-onb.2 bd-onb.1   # convention extraction needs scan first
```

The user has the agent run them in sequence.

### bd-onb.1 — hew-scan

```
bd remember "Backend: FastAPI 0.114 + SQLAlchemy 2 + Postgres 16. Entry: app/main.py."
bd remember "Frontend: Next.js 14 app router, Tailwind, pnpm. Path: frontend/."
bd remember "Auth: JWT via jose@5.2, httpOnly refresh cookie, 15min/7d TTLs. app/auth/middleware.py."
bd remember "Tests: pytest async, testcontainers/postgres. tests/conftest.py."
bd remember "Migrations: Alembic, alembic/versions/."
bd remember "CI: GitHub Actions .github/workflows/ci.yml — lint + type + test on every PR."
bd remember "Layout: monorepo. backend/ + frontend/. Shared types in packages/types/ (Pydantic + Zod codegen)."
bd remember "GOTCHA: notebook.delete is soft-delete (is_deleted=True). Never DELETE rows; UI filters on is_deleted=False."
bd remember "GOTCHA: subscription middleware in app/middleware/billing.py runs AFTER auth — order matters in app/main.py."
bd remember "STATUS:scan:complete — 2026-05-12T10:14:22"
```

20 memories total. Close.

### bd-onb.2 — hew-convention

```
bd remember "CONVENTION:services — class-based with constructor DI. See app/services/notebook_service.py for the template."
bd remember "CONVENTION:errors — Never raise raw exceptions. Wrap in AppError(code, message, details). Middleware translates."
bd remember "CONVENTION:api — Route handlers thin; logic in services. Routes under app/api/v1/, registered in app/api/__init__.py."
bd remember "CONVENTION:db — Repository pattern. All queries through app/repos/*.py. No raw SQL outside repos."
bd remember "CONVENTION:tests — One test file per source. Pattern tests/{path}/test_{module}.py. Factory_boy in tests/factories/."
bd remember "CONVENTION:imports — Stdlib / third-party / local. Absolute imports from app.* root. Enforced by ruff."
bd remember "STATUS:convention:complete — 2026-05-12T10:21:08"
```

### bd-onb.3 — hew-audit

```
bd remember "AUDIT: stripe@8.5 — 3 majors behind (latest 12.x). No CVEs but breaking changes in 9.0, 11.0. Bump path planned."
bd remember "AUDIT: pyjwt@2.7 — only used by an old script, not the auth path. Candidate for removal."
bd remember "STATUS:audit:complete — 2026-05-12T10:24:11"
```

### bd-onb.4 — hew-boundary

```
bd remember "BOUNDARY: POST /api/v1/auth/login — body {email, password}, returns {access, refresh_in_cookie}. ~6 frontend consumers."
bd remember "BOUNDARY: GET /api/v1/notebooks — returns paginated {results, next_cursor}. Filters: is_deleted=False auto-applied."
bd remember "BOUNDARY: User type — {id, email, plan_id, created_at}. Used by /me, /users/{id}, frontend session."
bd remember "STATUS:boundary:complete — 2026-05-12T10:28:30"
```

Onboarding epic auto-closes.

## Step 2 — Plan the feature

Now `hew-plan` runs with full context from memories:

```
Plan — Team Workspaces

Goal: Multi-tenant teams. Each user belongs to >= 1 team; notebooks are
team-scoped; access is enforced server-side.

Acceptance criteria:
1. User can create a team.
2. User can invite another user to a team via email.
3. Notebooks have a team_id; queries filter by current user's teams.
4. Cross-team access returns 403.
5. Existing notebooks migrate into a "Personal" team per user.

Architecture: add Team + Membership + Invite tables. Add team_id FK on
Notebook. Subscription policy: each team has one plan (extend
DECISION:auth and DECISION:billing memories accordingly). CONVENTION:db
repository pattern means /repos/team_repo.py + /repos/membership_repo.py.

Order of work (vertical slices):
  1. Schema + migration + Team creation + listing.
  2. Membership + invite flow.
  3. Notebook scoping (the gnarly one — touches every notebook query).
  4. Backfill migration: each existing user gets a "Personal" team
     containing their existing notebooks.

Graph shape: one epic "Team Workspaces" with 4 child tasks.

Open questions:
  Q: For Acceptance #5 — should backfill run in the schema migration
     itself (atomic) or as a separate Alembic data-only migration?
     Default plan: separate data migration, idempotent.
```

User confirms.

## Step 3 — Decompose

```
bd create --type=epic --priority=1 --title="Team Workspaces"   # → bd-tm
bd create --parent=bd-tm --title="Team schema + creation + listing"
bd create --parent=bd-tm --title="Membership + invite flow"
bd create --parent=bd-tm --title="Notebook scoping by team"
bd create --parent=bd-tm --title="Backfill: Personal team for existing users"

bd dep add bd-tm.2 bd-tm.1
bd dep add bd-tm.3 bd-tm.2
bd dep add bd-tm.4 bd-tm.3
```

## Step 4 — Execute

The agent works through each task. Two notable events:

### Mid-flight `hew-migrate` catch

On bd-tm.3 (notebook scoping), the agent adds `team_id` to the Notebook
model but forgets the migration. `hew-guard` invokes `hew-migrate` and
fails with:

```
hew-migrate: FAIL — model files changed but no migration was added.
Changed models: app/models/notebook.py (added team_id column)
Generate the migration:
  alembic revision --autogenerate -m "add notebook.team_id"
```

Agent runs the command, verifies the migration matches the model, re-runs
guard, passes, closes.

### Convention drift surfaced

On bd-tm.2 (invite flow), the agent adds a new service with module-level
functions instead of a class. `hew-guard` step 7 (convention compliance)
fails:

```
GUARD: fail — convention drift
src/services/invite_service.py uses module-level functions but
CONVENTION:services specifies class with constructor DI.
```

Agent rewrites as a class, re-runs, passes.

## Step 5 — Verify

```
VERIFY: pass
[1] Tests: 71 pass (was 47 before the feature; coverage 82%)
[2] Acceptance: 5/5 met
[3] Boundaries: GET /notebooks now includes team scoping — updated
    BOUNDARY:notebooks-list memory accordingly (intentional)
[4] Golden path: create team → invite user → user accepts → both see
    shared notebooks. Cross-team 403. Backfill applied via alembic.
```

Epic auto-closes. Done.

## What the onboarding bought us

Without `hew-scan` / `hew-convention`, the agent would have:

- Picked a fresh tech stack instead of matching the existing one.
- Written routes with logic inside (violating CONVENTION:api).
- Used raw SQL instead of the repo pattern.
- Forgotten the soft-delete filter and broken notebook listings.

With them, every line of new code matched the codebase's existing
shape. The user reviewed and merged without rewrites.
