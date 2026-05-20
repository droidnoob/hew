<!-- hew:version=0.5.2 -->
---
name: hew-migrate
category: brownfield
init: hew prime migrate
---

# hew-migrate — Schema Migration Awareness

You catch the most common GSD-era failure mode: code changes that
modify a database model but never produce a matching migration file.
The result is "works in dev because someone hand-altered the DB; CI
blows up on a fresh database."

You run inside the work loop, not standalone. When the executor
finishes touching a model file, it invokes this skill to verify the
migration exists.

## When this skill runs

- Inline within `hew-execute` / `hew-guard` when the changed diff
  includes ORM model files.
- The user explicitly asks to "check migrations."
- Periodically (cron-like) before shipping, to catch drift.

## Inputs from `hew prime migrate`

- `memories.factual` — tells you the ORM (`SQLAlchemy + Alembic`,
  `Prisma`, etc.). If unknown, refuse with a useful error.
- The git diff (or staged files) — passed by the caller.

## Supported ORMs

| ORM | Migration tool | Model glob | Migration glob |
|-----|---------------|------------|----------------|
| SQLAlchemy | Alembic | `app/models/*.py`, `**/models.py` | `alembic/versions/*.py` |
| Django | Django migrations | `**/models.py` | `**/migrations/*.py` |
| Prisma | `prisma migrate` | `prisma/schema.prisma` | `prisma/migrations/*/migration.sql` |
| TypeORM | TypeORM migrations | `src/entity/*.ts`, `src/entities/*.ts` | `src/migration/*.ts` |
| Drizzle | drizzle-kit | `src/db/schema.ts` | `src/db/migrations/*.sql` |
| Diesel (Rust) | diesel-cli | `src/models/*.rs`, `src/schema.rs` | `migrations/*/up.sql` |

Read the factual memories to identify which one applies. If the
project uses something else, fail loudly with a request to add a
`MIGRATION:` memory describing the pattern.

## The check

1. **Inspect the diff** the caller passed in. Identify changed model
   files using the glob for the detected ORM.
2. **If no model files changed**, exit clean — no migration needed.
3. **If model files changed**, look for a new migration file in the
   same diff. Heuristics:
   - Alembic: a new file in `alembic/versions/` with a recent
     timestamp prefix.
   - Django: a new `<n>_<name>.py` in any app's `migrations/`.
   - Prisma: a new directory under `prisma/migrations/` + a non-empty
     `migration.sql`.
   - TypeORM: a new file in `src/migration/` matching the naming
     convention.
   - Drizzle: a new file in `src/db/migrations/`.
   - Diesel: a new directory under `migrations/` with `up.sql` and
     `down.sql`.
4. **If a migration file is staged**, verify:
   - The migration references the changed columns/tables (grep the
     migration content for the new field names).
   - It is at the latest version (no gap with the previous one in
     the chain).
5. **If no migration file is staged**, fail and tell the executor
   exactly what to run:

```
hew-migrate: FAIL — model files changed but no migration was added.

Changed models:
  app/models/user.py (added email_verified column)

Generate the migration:
  alembic revision --autogenerate -m "add user.email_verified"

Re-run hew-migrate after generating.
```

## Memory shape

After a migration is successfully added and applied, persist the link:

```
hew remember --raw "MIGRATION:003_add_email_verified — Added users.email_verified BOOLEAN NOT NULL DEFAULT false. Models touched: app/models/user.py. Applied 2026-05-12."
```

Future audits and `hew remember`-driven recall can answer "when did
this column appear?" without scanning git history.

## Verification — does the migration actually do what the model says?

A model change is more than adding a column. Check:

- **Column type matches** — `BOOLEAN` in model vs `INTEGER` in
  migration is a bug.
- **NOT NULL constraint matches** — model `required: true` vs migration
  `nullable: true` is a bug.
- **Index presence** — if the model declares an index, the migration
  must create it.
- **Default value matches** — model default vs migration default.
- **Drop direction handles existing data** — a NOT NULL column on a
  populated table needs a default or a data backfill.

For each mismatch, surface it before the migration is applied. Cheap
to fix on disk; expensive to recover from in production.

## Migration safety review

Even when the migration exists and matches the model, flag risky
operations:

- **Adding a NOT NULL column without a default** on a large table —
  will lock the table while backfilling.
- **Dropping a column** that's still referenced in API responses
  (cross-reference with `BOUNDARY:` memories).
- **Renaming a column** without an intermediate rename-and-keep step.
- **Foreign key changes** that may cascade to other tables.
- **Index creation on a large table** without `CONCURRENTLY` (Postgres).

These are warnings, not failures. Surface them; let the user decide.

## Output

```
hew-migrate: pass
  Models changed: app/models/user.py
  Migration added: alembic/versions/2026_05_12_1430_add_email_verified.py
  Column types match. Index declared on users.email_verified, created in migration.
  Safety: ⚠ NOT NULL column on a table with 50M rows — confirm default is acceptable.
```

or:

```
hew-migrate: FAIL
  Models changed: app/models/user.py
  Migration: missing
  Run `alembic revision --autogenerate -m "..."` and re-run hew-migrate.
```

## What you don't do

- **Generate migrations.** That's the ORM's job; you only verify.
- **Apply migrations.** Different operation — applied = deployed.
- **Block on warnings.** Failures block; warnings surface.
- **Re-check unchanged files.** Scope to the diff.
- **Speculate about run-time effects.** Static analysis only. Real
  load tests live in CI / staging.

## Anti-patterns

- **Generating a migration to "make the check pass"** without
  reading what it contains. The migration must match the model
  *intent*, not just the model file's hash.
- **Treating the absence of a migration as OK** because "we'll do it
  later." Drift compounds.
- **Skipping verification when the migration is auto-generated.**
  Autogen tools miss things (especially indexes, defaults, type
  precision).
- **Failing the check on docs-only changes** that touched a comment
  in a model file. Scope to *structural* model changes.

## Hand-off

If pass: the caller (`hew-guard` or `hew-execute`) proceeds.
If warnings: surface, caller decides.
If fail: caller does NOT close the task. The fix is generating the
migration; the executor then re-invokes `hew-migrate`.

This skill never closes Beads tasks directly. It only reports.
