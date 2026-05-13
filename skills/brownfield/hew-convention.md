<!-- hew:version=0.2.1 -->
---
name: hew-convention
category: brownfield
init: hew prime convention
---

# hew-convention — Extract Prescriptive Coding Rules

You read existing code and turn the patterns into **prescriptive rules**
stored as `CONVENTION:` prefixed memories. The executor and guard treat
these as mandatory constraints.

`hew-scan` captures **what** exists. `hew-convention` captures **how to
write code that matches**. Different prefix, different skill, different
treatment by the executor.

The single biggest cause of "AI agent fights the codebase" is missing
conventions. The executor extrapolates from training data instead of the
project's actual patterns and the codebase splits. Your job is to
prevent that.

## When this skill runs

- After `hew-scan` finishes (`STATUS:scan:complete` is required).
- The user invoked `hew init` on a brownfield project and is going
  through the onboarding chain.
- After a major refactor, when the agent should re-derive conventions
  from the post-refactor state.

## Inputs from `hew prime convention`

- `prerequisites.met` — must be `true`. Refuse if `STATUS:scan` is
  missing.
- `memories.factual` — the scan output. Use these to know where to look.
- `memories.conventions` — any existing conventions. Extending; do not
  duplicate.

If `STATUS:convention:complete` already exists, ask the user before
re-running: "Conventions were extracted on `<date>`. Re-run to update,
or skip?"

## The convention areas

Walk these areas, and for each, derive 1–4 `CONVENTION:` memories.

### 1. Service layer / business logic

Read 3–5 service files. Look for:

- Class vs module-level functions?
- Dependency injection? Constructor, parameter, or globals?
- Public method signatures: keyword-only? typed returns? async?
- Where do services live? (`app/services/`, `src/lib/services/`)
- How are they registered/wired into routes?

```
hew remember --type=convention "services — Class-based with constructor DI. One service per domain. Public methods async, keyword-only args. See app/services/user_service.py for the template."
hew remember --type=convention "services — Services never raise; they return Result[T, AppError]. Routes unwrap and translate to HTTP."
```

### 2. Error handling

Read the error middleware, a route, a service. Look for:

- Custom exception classes vs raw raises?
- How are errors translated to HTTP responses?
- Are errors structured (code/message/details)?
- Sentinel values vs exceptions vs Result types?

```
hew remember --type=convention "errors — Never raise raw exceptions. Wrap in AppError(code: str, message: str, details: dict). Defined in app/exceptions.py."
hew remember --type=convention "errors — AppError caught by error_middleware (app/middleware/errors.py); never call jsonify/Response directly from a route."
```

### 3. API / routing

Read 3–5 route files. Look for:

- Where do route handlers live?
- Are they thin (delegate to services) or fat (logic inline)?
- Naming pattern for routes? (`/api/v1/{resource}`)
- Request validation pattern? (pydantic, zod, manual)
- Response shape? (envelope, plain object, problem-detail)

```
hew remember --type=convention "api — Route handlers are thin: parse + validate + delegate to service + map result to response. Never put business logic in route handlers."
hew remember --type=convention "api — Validate request bodies with pydantic models declared in app/api/v1/{resource}/schemas.py. Reuse the same models for OpenAPI generation."
hew remember --type=convention "api — Successful responses use the envelope {data: T} for items, {results: T[], next_cursor: str} for lists."
```

### 4. Database / data access

Read 2–3 repository/query files. Look for:

- Repository pattern vs queries-in-services?
- Raw SQL allowed?
- Transaction handling pattern?
- Eager vs lazy loading strategy?
- Migration pattern?

```
hew remember --type=convention "db — All queries go through repository classes in app/repos/. No raw SQL outside app/repos/."
hew remember --type=convention "db — All DB models use UUID primary keys (BaseModel mixin in app/models/base.py adds id, created_at, updated_at)."
hew remember --type=convention "db — Transactions managed via `async with db.transaction():` in service layer, never in routes."
```

### 5. Tests

Read 3–5 test files spanning route, service, and unit levels. Look for:

- One test file per source module? (`tests/test_X.py` for `src/X.py`)
- Fixture pattern? (`conftest.py`, factory_boy, manual)
- Naming pattern for tests?
- Use of mocks vs real dependencies?
- Integration / e2e test layer?

```
hew remember --type=convention "tests — One test file per source module. Pattern: tests/{path}/test_{module}.py mirrors app/{path}/{module}.py."
hew remember --type=convention "tests — Fixtures in tests/conftest.py and tests/factories/ (factory_boy). Never instantiate models directly in tests."
hew remember --type=convention "tests — Test names: test_<what>_<expected>. Example: test_create_user_returns_id."
hew remember --type=convention "tests — Real Postgres via testcontainers. No mocking of the DB layer."
```

### 6. Imports + module organization

Read the top of 5–10 random source files. Look for:

- Import ordering (stdlib / third-party / local)?
- Are there separator blank lines?
- Absolute vs relative imports?
- Re-export patterns in `__init__.py` / `index.ts`?

```
hew remember --type=convention "imports — Stdlib first, third-party second, local third. Blank line between groups. Enforced by ruff isort rule."
hew remember --type=convention "imports — Always absolute imports from `app.*` root, never relative (no `from .foo import bar`)."
```

### 7. Naming

Look across files and check for:

- Function naming (`snake_case` / `camelCase` / `kebab-case`)?
- Class naming (`PascalCase` is near-universal, but confirm)?
- Constants (`UPPER_SNAKE_CASE`)?
- Test functions?
- Variable conventions (Hungarian, prefixes)?

```
hew remember --type=convention "naming — Functions and variables snake_case. Classes PascalCase. Constants UPPER_SNAKE_CASE. Private fields prefix with single underscore."
```

### 8. Logging + observability

Read where logging is set up + a few places it's used. Look for:

- Library (stdlib logging, structlog, pino, slog)?
- Structured vs unstructured?
- Context propagation? (request_id, user_id)
- PII handling rule?

```
hew remember --type=convention "logging — structlog. Always include request_id in context (added by app/middleware/request_context.py). Never log PII (emails, tokens, body contents)."
```

### 9. Type annotations / type discipline

For typed languages: read a few files of each layer.

- Are all public functions annotated?
- Optional / Union conventions?
- Type aliases (`type Alias = ...`)?
- Strict mode? Generics?

```
hew remember --type=convention "types — All public functions annotated. Return type explicit even when None. mypy --strict in CI. No Any without an annotated reason comment."
```

### 10. Frontend-specific (if applicable)

If there's a frontend, repeat the exercise on its conventions:
component structure, state management, routing, styling, accessibility,
forms.

```
hew remember --type=convention "components — Functional components only. Props typed via TypeScript interface, not type alias. Default exports only for pages; named exports for components."
hew remember --type=convention "state — Zustand for global state (one store per feature). React Query for server state. Never useState for shared state."
hew remember --type=convention "styling — Tailwind classes only; no CSS modules, no inline styles, no css-in-js. Shared classes via cn() from src/lib/cn.ts."
```

### 11. Craft principles already in force

After the ten descriptive areas above, surface the *craft principles*
the codebase is implicitly following. Brownfield projects almost
always follow some subset of SOLID/DRY/Clean-Arch/etc. — the goal is
to make it explicit so `hew-guard`'s craft soft-warnings and
`hew-review`'s craft pillar bind to the project's actual character,
not a generic checklist.

This step matters because `CONVENTION:craft.consistency-with-existing-code`
(the meta-principle that defaults on every seeded stack) says
*existing conventions beat picked principles*. To honor it, the
existing principles have to be persisted.

Three heuristics on the live code:

**A. Function-length distribution → `craft.max_function_lines`.**

Sample function lengths across `src/` (or your project root). A quick
shell pass:

```
# Python
grep -nE '^[[:space:]]*(async )?def ' src/**/*.py | wc -l
# then sample 20 random functions and eyeball the p95 length
```

If most functions sit under ~20 lines and outliers are rare, the
project follows `craft.small-functions`. Persist the practical
threshold:

```
hew remember --type=convention "craft.small-functions — Functions stay under 25 lines (p95 observed). Set craft.max_function_lines=25 so hew-guard warns on outliers."
```

If lengths are wild (50+ line handlers common, no clear pattern),
*don't* fabricate a principle — the project hasn't committed to it.

**B. Layering style → architecture principle.**

Walk the top-level directory layout:

| What you see                                       | Suggested principle                    |
|----------------------------------------------------|----------------------------------------|
| `domain/` + `application/` + `infrastructure/` + `interfaces/` | `craft.clean-architecture` |
| `core/` + `adapters/` + `ports/`                   | `craft.hexagonal-architecture`         |
| `api/` + `services/` + `repos/` + `models/`        | `craft.layered-monolith` (or `clean-architecture` if dependency direction is enforced) |
| Single `src/` flat layout                          | `craft.kiss` / no architectural principle — don't impose one |
| Per-feature `features/<x>/{api,service,db}.py`     | `craft.feature-folder` / `craft.cohesion` |

```
hew remember --type=convention "craft.clean-architecture — domain depends on nothing; application depends on domain; infrastructure depends on application via interfaces. Verified by the existing src/{domain,application,infrastructure,interfaces} split."
```

If the layering is inconsistent, skip — don't legislate retroactively.

**C. Testing density → `testing.require`.**

Count the test-to-source ratio:

```
# Python
test_files=$(find tests/ -name '*.py' | wc -l)
src_files=$(find src/ -name '*.py' ! -path '*/tests/*' | wc -l)
echo "ratio: $test_files / $src_files"
```

Plus: does each behavior-bearing module have a sibling test?

- **High density** (ratio ≥ 0.6, most modules tested): the project
  treats tests as load-bearing. Suggest `testing.require=true` so
  hew-guard escalates missing-tests from warn to fail.

  ```
  hew remember --type=convention "craft.test-first — Every behavior-bearing module has tests. Run `hew config set testing.require true` so hew-guard enforces this on close."
  ```

- **Medium density** (0.2 ≤ ratio < 0.6): tests exist but aren't
  universal. Leave `testing.require=false`; persist as a soft norm.

- **Low density** (< 0.2): the project hasn't picked test-first.
  Don't fabricate a principle — surface to the user instead.

**D. Style fingerprints — opportunistic.**

While walking the code, watch for these signals and persist matching
craft principles when they're clearly in force:

- Result/Either return type prevalent → `craft.errors-as-values` /
  `craft.fail-fast` (depending on shape).
- Most domain calls go through interfaces / protocols, not concrete
  classes → `craft.dependency-inversion` (part of SOLID).
- Pure functions for transformations, side effects ringfenced →
  `craft.pure-functions`.
- Idempotency keys on POSTs, retries everywhere → `craft.idempotence`.
- Heavy use of dataclasses / immutable record types →
  `craft.immutability`.

For each clear signal, persist a `CONVENTION:craft.<id>` memory
sourced from the catalog (`hew schema craft-principles`) so the id and
summary stay consistent with new-project picks.

**Acceptance for this step:** on a typical Python+FastAPI repo you
should leave with ≥ 3 `CONVENTION:craft.<id>` memories (e.g.
`small-functions`, `clean-architecture`/`layered-monolith`, plus one
style fingerprint). Less than 3 means the codebase genuinely doesn't
commit to much — surface that to the user; don't pad.

Brownfield deference is non-negotiable: always also persist the meta
principle, so any later `hew-new-project --re-bootstrap` or craft
picker honors it:

```
hew remember --type=convention "craft.consistency-with-existing-code — When a craft principle conflicts with an existing CONVENTION:* memory, the existing convention wins. Brownfield deference is the default."
```

## Writing good conventions — prescriptive, not descriptive

A `CONVENTION:` is a rule the executor must follow. Phrase it as a rule.

**Prescriptive (good):**
```
CONVENTION:errors — Wrap exceptions in AppError(code, message, details). Defined in app/exceptions.py. Never raise raw exceptions.
```

**Descriptive (less useful):**
```
Most modules use AppError for errors.
```

Include:
- **What to do** (the rule).
- **Where to see it** (the canonical file).
- **What not to do** (the anti-pattern, when it exists in the wild).

If a convention has known exceptions, include them in the rule — the
executor will follow the rule literally otherwise.

## Group by domain — one memory per domain, not one per rule

When a domain has multiple related rules (e.g. `errors`, `cli-output`,
`testing`), **issue one `hew remember` per domain** with the rules
folded into a single structured body. Atomic per-rule memories are
how a project ends up with a 30-entry `CONVENTION:*` set that has to
be compacted back down to 9 anyway — write them grouped from the
start.

**Atomic per rule (avoid for multi-rule domains):**

```
hew remember --type=convention "errors — never raise raw exceptions"
hew remember --type=convention "errors — wrap in AppError"
hew remember --type=convention "errors — middleware catches AppError; never call jsonify"
```

**Grouped per domain (preferred):**

```
hew remember --type=convention "errors — Never raise raw exceptions. Wrap in AppError(code, message, details) defined in app/exceptions.py. AppError is caught by error_middleware (app/middleware/errors.py); never call jsonify/Response directly from a route handler."
```

Rule of thumb: if you're about to issue ≥3 `hew remember` calls with
the same `<domain> —` lead, fold them into one body with sub-paragraphs.

### Bulk emission via `--from-file`

For a whole pass that produces 5+ memories across several domains,
prefer `hew remember --from-file <path>` over a sequence of CLI
calls. Each entry is one *domain-grouped* memory:

```json
[
  {
    "type": "convention",
    "body": "errors — Never raise raw exceptions. Wrap in AppError(code, message, details). Caught by error_middleware; never jsonify from a route."
  },
  {
    "type": "convention",
    "body": "tests — One test file per source module. Fixtures live in tests/conftest.py + tests/factories/ (factory_boy). Real Postgres via testcontainers; no mocking of the DB layer."
  },
  {
    "type": "convention",
    "body": "naming — snake_case for functions/variables. PascalCase for classes. UPPER_SNAKE for constants. Private fields prefix with single underscore."
  }
]
```

```sh
hew remember --from-file conventions.json
# → remembered 3
```

The whole file is validated up-front; a single malformed entry rejects
the batch with `entry[N]: <reason>` and zero side effects.

## Resolution when the codebase contradicts itself

Real codebases have drift. You'll find services that follow the
pattern and services that don't. Decide:

1. **Majority wins.** If 8 services use class-based DI and 2 use
   module-level functions, the convention is class-based DI. Add a
   note: "Some legacy services still use module-level functions; new
   work follows the class-based pattern."
2. **If split is roughly 50/50,** ask the user which is canonical.
   Don't guess.
3. **If the new pattern is clearly emerging** (recent files use it,
   older files don't), call it out:

```
hew remember --type=convention "errors — Use Result[T, AppError] pattern (added in PR #47). Older modules still raise AppError directly; migrating opportunistically. New code uses Result."
```

## Decompose into scan subtasks

If the codebase is large, you don't have to extract every convention in
one session. Create scan subtasks per area:

```
hew task new --parent=<onboarding-epic> --title="Extract service-layer conventions"
hew task new --parent=<onboarding-epic> --title="Extract API conventions"
hew task new --parent=<onboarding-epic> --title="Extract test conventions"
...
```

Claim them one at a time, finish each (`hew remember --type=convention "..."`
entries), close, move on. Each subtask is ~30 min of agent work.

## When to stop

A complete convention extraction on a medium codebase produces ~10–25
`CONVENTION:` memories across the areas above. Less than 5 means you
missed something. More than 40 means you over-fitted to specific files
instead of generalizing patterns.

A useful exit check: pick a file in the codebase you haven't read, and
predict what it looks like *based purely on your conventions*. Then read
it. If you're surprised, your conventions are incomplete.

## Step — Mark phase complete

After extraction:

```
hew remember --type=status "convention:complete — <ISO-8601 timestamp>"
```

This signals downstream skills that conventions are usable.

## What you don't do

- **Write a CONVENTIONS.md doc.** Memories only. The plan is explicit.
- **Invent rules the codebase doesn't follow.** If you think a pattern
  *should* exist but doesn't, that's a `hew-audit` finding (open a
  chore task), not a convention.
- **Record opinions.** "Should use dependency injection more" is not a
  convention. "Services use constructor DI" is.
- **Record one-off cases** as conventions. If a pattern appears in one
  file, it's a fact (for `hew-scan` or `BOUNDARY:`), not a rule.
- **Re-record memories that `hew-scan` already wrote.** Convention is
  prescriptive; scan is factual. Different prefix, different content.

## Hand-off

When extraction is complete:

1. Write `STATUS:convention:complete`.
2. Print a one-line summary: "Conventions extracted: N rules across
   X areas."
3. **Continue directly into `hew-audit`.** The brownfield onboarding
   chain is `scan → convention → audit → boundary`; stop only at the
   end of the chain or on a Rule-4 surprise.

## Anti-patterns

- **Descriptive memories** instead of prescriptive rules. "Some code
  uses X" doesn't constrain the executor; "Use X. Don't use Y." does.
- **Memories without examples.** "Services use DI." Where? The
  executor needs the canonical file to copy from.
- **Over-specific rules** that hard-code one file's choices instead of
  the pattern across files.
- **Recording the same rule under multiple prefixes** (`CONVENTION:` +
  `SECURITY:` for the same thing). Pick the one that fits the
  executor's routing best — usually `CONVENTION:` for coding rules,
  `SECURITY:` for security baselines specifically.
- **Treating "the README says..." as authoritative.** READMEs lie. Read
  the code.
