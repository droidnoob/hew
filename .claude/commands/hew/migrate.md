---
description: Detect DB-schema drift between code models and migration files. Persists MIGRATION memories and flags mismatches.
---

Invoke the hew-migrate skill. Reads the project's ORM/schema
definitions (SQLAlchemy / Prisma / Diesel / etc.) and cross-checks
against the migration files in the configured migrations directory.
Drift surfaces as a flagged finding plus a `MIGRATION:<name>` memory
the executor reads before touching either side.

Use whenever model changes ship without matching migrations, or when
auditing a brownfield codebase for hidden schema debt.

ARGUMENTS: $ARGUMENTS
