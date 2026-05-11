# Codebase Scan — Prompt Template

> This is the prompt the agent uses when running `hew-scan` on a
> brownfield project. It's a checklist, not a story. Each numbered
> item maps to one or more `bd remember` calls.

You are running `hew-scan`. The user has just initialized hew on an
existing codebase. Walk the project and persist what you find as
**discrete `bd remember` entries** — never a summary document.

For each section below, identify the relevant facts and write a
separate memory per fact. Use file paths in backticks where they
clarify. Don't invent details; if you cannot determine a fact from
the code, skip it.

## 1. Tech stack

- Language(s) and versions (read from `Cargo.toml` / `pyproject.toml` /
  `package.json` / `go.mod`).
- Web framework + version.
- ORM / data layer + version.
- Test runner + assertion library.
- Build / package manager.
- Notable dev tools (formatter, linter, type checker).

## 2. File layout

- The organizing principle (monorepo? feature-folders? layered?).
- Notable top-level directories and what each holds.
- Where new code of common kinds goes (route, model, service, test).

## 3. API surface

- Routing pattern (REST? GraphQL? RPC?).
- Path conventions (`/api/v1/{resource}`, plural vs singular).
- Request validation pattern.
- Response shape (envelope? plain? problem-details?).
- Error response shape.

## 4. Auth + security baseline

- Auth scheme (JWT? session? OAuth?).
- Token storage and TTLs.
- Password hashing library + params.
- Authorization decorators / middleware.
- Notable `SECURITY:` patterns (CSRF, CORS, rate limiting, input
  validation).

## 5. Data layer

- Database engine + version.
- ORM session lifecycle.
- Migration tool + location.
- Repository / data-access pattern.
- Schema base mixin (UUID PK? timestamps? soft delete?).

## 6. Testing setup

- Test discovery pattern.
- Fixture style (factories? conftest? mocks?).
- Test DB strategy (in-memory? testcontainers? mocked?).
- Coverage tooling and threshold (if enforced).

## 7. CI/CD + deployment

- CI system + workflow file paths.
- What runs on every PR (lint, type, test, security scans).
- Deploy target + tooling (Docker? Terraform? Serverless?).
- Environments (prod / staging / dev / preview).

## 8. Environment configuration

- Config library and loading pattern.
- Required env vars.
- `.env.example` presence (and whether it lists every var).
- Frontend env conventions (e.g., `NEXT_PUBLIC_` prefix discipline).

## 9. Non-obvious coupling and gotchas

The highest-value section. Capture things that would surprise a new
contributor:

- Implicit signals or hooks (e.g., post-save signals creating side
  effects across services).
- Patterns that look interchangeable but aren't (soft-delete vs hard
  delete; specific column required for X).
- External integration quirks (webhook idempotency, deploy ordering,
  feature flags).
- Performance traps (N+1 queries, expensive serializers).

## 10. Mark the phase complete

After everything above, finish with:

```
bd remember "STATUS:scan:complete — <ISO-8601 timestamp>"
```

This unblocks `hew-convention`, `hew-audit`, `hew-boundary`, and
`hew-migrate`.

## What not to do

- **No `CODEBASE.md` / summary doc.** Memories only.
- **No opinions or recommendations.** Those belong in `hew-audit`.
- **No `CONVENTION:` rules.** Those belong in `hew-convention`.
- **No exhaustive enumeration.** Pattern-match across a sample; you're
  recording the rules, not the lines.
