<!-- hew:version=0.1.0 -->
---
name: hew-audit
category: brownfield
init: hew prime audit
---

# hew-audit — Existing Dependency Health Check

You inspect the libraries the project already depends on and surface
the ones that are deprecated, unmaintained, vulnerable, or duplicated.
Findings become `AUDIT:` memories, and the critical ones become tasks
in the graph.

Different from `hew-deps`: `hew-deps` evaluates a *candidate new*
dependency before the agent adds it. `hew-audit` looks at what's
already there.

## When this skill runs

- Brownfield onboarding, after `hew-scan` (`STATUS:scan:complete`
  required).
- Periodically (the user re-runs once a quarter or after a major
  upgrade).
- Triggered by a CVE notification or supply-chain incident.

## Inputs from `hew prime audit`

- `prerequisites.met` — refuses if `STATUS:scan` missing.
- `memories.factual` — tells you the stack so you know which package
  manager to interrogate.
- `memories.audit` — prior findings; don't duplicate, update where
  appropriate.

## What to actually run

Pick the commands matching the stack `hew-scan` recorded:

| Stack | Command |
|-------|---------|
| Node / npm | `npm outdated --json`, `npm audit --json` |
| Node / pnpm | `pnpm outdated --json`, `pnpm audit --json` |
| Python / poetry | `poetry show --outdated --tree`, `pip-audit -r requirements.txt --format=json` |
| Python / uv | `uv pip list --outdated --format=json`, `pip-audit` |
| Rust | `cargo outdated --root-deps-only --format json`, `cargo audit --json` |
| Go | `go list -u -m -json all`, `govulncheck ./...` |

Run them. Parse the JSON. For each package, decide whether it's a
finding.

## What counts as a finding

Record an `AUDIT:` memory when **any** of these apply:

1. **Known CVE** — anything with a severity ≥ Medium.
2. **Deprecated** — package marked deprecated on the registry, or
   marked end-of-life.
3. **Unmaintained** — last publish > 24 months and the project still
   has open issues. Pure 1.0-and-done libs are fine; abandoned ones
   are not.
4. **Major version behind** — current usage > 1 major behind latest
   stable, especially when the major bump fixes security issues or
   removes a dependency the team is trying to drop.
5. **Duplicate versions** — same package present at multiple
   incompatible versions in the dependency tree (bloats bundle,
   diverges behavior).
6. **License-incompatible** — added a GPL/AGPL dependency to a
   non-copyleft project, or a license the deny list rejects.

For everything else (mildly outdated, no security implication, still
maintained): don't open a finding. Audit is for action items, not
homework.

## Memory shape

```
bd remember "AUDIT: jsonwebtoken@8.5.1 — DEPRECATED, last publish 3yr ago. Migrate to jose@5.x. Used in app/auth/jwt.py."
bd remember "AUDIT: lodash@4.17.20 — CVE-2021-23337 (prototype pollution). Bump to 4.17.21+ or migrate to lodash-es."
bd remember "AUDIT: cryptography@38.0.4 — 7 majors behind. Bump path is 38→39→40→…→42 with type changes at 39, 41."
bd remember "AUDIT: moment — UNMAINTAINED, last publish 2y, project README recommends moving to dayjs/date-fns."
bd remember "AUDIT: duplicate uuid@8.3.2 + uuid@9.0.0 in tree. Unify to 9.x."
```

Be specific: package name + version + why + suggested action + where
it's used (file path if known).

## Open Beads tasks for critical findings

For severity ≥ High and clear-cut paths (deprecation, CVE):

```
bd create --type=bug --priority=1 \
  --title="Migrate from jsonwebtoken to jose (deprecated upstream)" \
  --description="
  AUDIT: jsonwebtoken@8.5.1 deprecated 2023; jose is the maintained replacement.
  Touch: app/auth/jwt.py, tests/auth/test_jwt.py.
  See https://github.com/auth0/node-jsonwebtoken#readme-deprecation.
  " \
  --acceptance="jsonwebtoken removed from package.json; pytest tests/auth -k jwt passes; auth flow works end-to-end."
```

For lower-severity findings (slight version drift, unmaintained but
not vulnerable): leave them as memories. The agent surfaces them when
relevant; the user prioritizes.

## When to ask vs auto-open

Auto-open tasks for:
- CVEs ≥ High severity.
- Deprecated packages with a clear replacement.
- License violations (these block legal compliance).

Surface for user decision when:
- The fix is a major bump with breaking changes.
- The replacement library involves an architectural shift.
- The project is intentionally on an old version (e.g., LTS commitment).

Print the findings list to the user at the end of the audit. They
decide what becomes a task vs what waits.

## Output to user

```
hew-audit findings
──────────────────────────────────
critical (3):
  jsonwebtoken@8.5.1     deprecated     → opens bd-X.7
  lodash@4.17.20          CVE-2021-23337 → opens bd-X.8
  GPL-3.0 ricecake@1.0   license clash  → opens bd-X.9

warnings (5):
  cryptography@38.0.4    7 majors behind
  moment                  unmaintained
  uuid 8.3.2 + 9.0.0      duplicate
  …

Tasks opened: 3. Memories written: 8. Run `hew status` to confirm.
```

## Step — mark phase complete

```
bd remember "STATUS:audit:complete — <ISO-8601 timestamp>"
```

## What you don't do

- **Auto-upgrade.** Never run `npm install <pkg>@latest` or equivalent.
  Audit reports; the executor (in a dedicated task) does the bump
  with proper testing.
- **Open tasks for every outdated package.** Most are fine; opening
  noise dilutes the real findings.
- **Trust transitive vulnerability tooling blindly.** Some "high"
  advisories don't apply to your usage pattern. Look at the actual
  function affected.
- **Skip the user surface.** Always print the findings summary, even
  if everything is clean ("Audit clean — no critical findings.").

## Anti-patterns

- **Recording "lodash is bad" as a finding.** Be specific: version,
  CVE / deprecation note, suggested replacement.
- **Auto-opening 30 tasks** because `npm outdated` listed 30 packages.
- **Treating warnings as criticals** — license violation = critical,
  "deprecation predicted" = warning.
- **Re-running without checking existing `AUDIT:` memories.** If a
  finding is already recorded and unchanged, don't duplicate it.
