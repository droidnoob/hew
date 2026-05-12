<!-- hew:version=0.2.0 -->
---
name: hew-boundary
category: brownfield
init: hew prime boundary
---

# hew-boundary — API + Public Interface Scanner

You map every interface the codebase exposes — HTTP routes, public
functions, exported modules — and record them as `BOUNDARY:` memories.
The executor checks these before modifying shared code, so a refactor
doesn't silently break twelve downstream callers.

Boundaries are *contracts*. Inside a module, you can refactor freely.
At a boundary, change is breaking.

## When this skill runs

- Brownfield onboarding after `hew-scan` (`STATUS:scan:complete`
  required).
- After a major refactor where boundaries may have shifted.
- The user explicitly asks for a boundary re-map.

## Inputs from `hew prime boundary`

- `prerequisites.met` — refuse if `STATUS:scan:complete` is missing.
- `memories.factual` — tells you the stack and entry points.
- `memories.boundaries` — existing entries; extend, don't duplicate.

## Boundary categories

### 1. HTTP API endpoints

For every route handler in the codebase:

- **Method + path** — `POST /api/v1/users`, `GET /api/v1/users/{id}`.
- **Request shape** — body schema, query params, headers.
- **Response shape** — status codes, success body, error body.
- **Auth requirement** — public, requires JWT, requires role.

```
hew remember --type=boundary "POST /api/v1/users — body {email, password, name}, returns 201 {id, token}. Auth: public. Handler app/api/v1/users.py:create_user."
hew remember --type=boundary "GET /api/v1/users/{id} — returns {id, email, created_at}. Auth: JWT. 7 frontend components consume."
hew remember --type=boundary "POST /api/v1/webhooks/stripe — Stripe-Signature header required, body raw. Idempotency-key consumed. Never break the existing signature."
```

For REST APIs: find routes by reading the route registry / decorator
sites. For GraphQL: enumerate the schema's queries and mutations as
boundaries.

### 2. Public functions and module exports

For library code (or any code that other modules import):

- **Function signature** — name, args, return type.
- **Side effects** — does it mutate state, write to DB, call external?
- **Consumers** — quick count of internal callers (grep helps).

```
hew remember --type=boundary "create_user(db, email, password, name) -> User. app/services/user_service.py. Side effects: writes users + audit_log."
hew remember --type=boundary "validate_input(payload, schema) -> Result[dict, AppError]. app/security/validate.py. Called by 18 routes."
```

For TypeScript: `index.ts` re-exports are usually the public surface.
For Rust: `pub` items in `lib.rs` and the crate's module tree.

### 3. Shared types / contracts

Any type that crosses module boundaries (DTOs, response shapes, ORM
models exposed externally):

```
hew remember --type=boundary "User type — {id: UUID, email: str, name: str, created_at: datetime}. Serialized by /users/{id}. app/models/user.py."
hew remember --type=boundary "AuthResponse type — {access: str, refresh: str, expires_in: int}. Returned by /auth/login + /auth/refresh."
```

### 4. CLI commands (if the project ships one)

Every documented subcommand + flags is a boundary; users script
against them.

```
hew remember --type=boundary "CLI `hew prime <skill>` — always emits JSON to stdout. Schema versioned via schema_version field."
```

### 5. Event / message contracts

Pub/sub, queue messages, webhook payloads:

```
hew remember --type=boundary "event order.created — {order_id, user_id, items: [{sku, qty}], total_cents}. Published to SNS topic orders. Consumed by billing-service and warehouse-service."
```

## How to find them efficiently

You don't have to enumerate every line of code. Look for:

1. **Route registries** — Flask blueprints, FastAPI routers, Express
   route files, Rust `axum::Router` builders.
2. **OpenAPI / GraphQL schemas** — if generated, the schema file is
   the canonical boundary list. Compare against the live routes to
   catch drift.
3. **`pub fn` / `export function` / `def public_*`** — explicit
   markers.
4. **Re-export files** — `index.ts`, `__init__.py`, `lib.rs`,
   `mod.rs`.
5. **Consumer side** — grep for imports of the module. If something
   is imported widely, its export surface is the boundary.

## When the boundary has known consumers

Always record consumer count if you can — it's the strongest signal
for impact assessment. `grep -r "from app.auth import"` for Python,
`rg "import.*auth-client"` for TS.

```
hew remember --type=boundary "POST /api/v1/users expects {email, password, name}. 4 frontend components + 1 mobile app consume."
```

When the executor considers changing this boundary, it now knows
exactly how many downstream sites need updating.

## What's NOT a boundary

- **Module-internal functions.** If nothing outside the file imports
  it, refactor freely. No memory needed.
- **Private DB columns** that aren't part of the API response.
- **Implementation details inside services** — only the service's
  *public method signatures* are boundaries.

If everything is a boundary, nothing is. Be ruthless about scope.

## When a boundary intentionally changes

`hew-verify` will catch unintended boundary regressions. When the
boundary should change (deliberate API evolution):

1. The executor opens a task explicitly titled "Change boundary X."
2. The task description names the migration deadline + downstream
   updates required.
3. After implementation, update the `BOUNDARY:` memory:

```
hew remember --type=boundary "POST /api/v1/users now expects {email, password, name, accept_tos}. Migration deadline 2026-07-01. Old shape returns 400."
```

The memory key (computed by Beads from the prefix) means the new
entry coexists with the old until you explicitly remove it.

## Output

Print the count by category at the end:

```
hew-boundary scan complete
──────────────────────────────────
  18 HTTP endpoints
  12 public functions
   6 shared types
   4 CLI commands
   3 event contracts
   ─────
  43 boundaries recorded
```

## Step — mark phase complete + end of chain

```
hew remember --type=status "boundary:complete — <ISO-8601 timestamp>"
```

`hew-boundary` is the last step of the brownfield onboarding chain.
Stop here and hand back to the user (or to whatever skill invoked
the chain — usually `hew-plan` for "scan and plan a feature"
intents). Print a one-line chain summary listing the four
`STATUS:` markers now complete.

## What you don't do

- **Record private internals as boundaries.** Internal != public.
- **Skip the consumer count.** Even an approximate count ("~6 consumers")
  is more useful than nothing.
- **Write a BOUNDARIES.md doc.** Memories, like every other skill.
- **Record every function in the codebase.** Only the public surface.
- **Update boundaries casually.** Any change is breaking by default;
  always tie a memory update to a deliberate migration plan.

## Anti-patterns

- **Boundaries with no signature** — "POST /users does something."
  Useless. Include the request + response shapes.
- **Boundaries that are actually private code.** Failing this test
  means modules will refuse legitimate refactors.
- **Recording the same boundary twice** under different keys after
  the API shifted. Update in place.
- **Skipping events and CLIs.** These are boundaries too; external
  consumers script against them.
