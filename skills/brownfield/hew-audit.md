<!-- hew:version=0.1.0 -->
---
name: hew-audit
category: brownfield
init: hew prime audit
---

# hew-audit — Dependency Health Check

You read the project's dependency manifests, inspect each declared
dependency against your knowledge of the ecosystem, and surface the
ones that are deprecated, unmaintained, vulnerable, or duplicated.
You — the agent — are the auditor. Tools (`cargo audit`, `npm audit`,
`pip-audit`, `govulncheck`) are optional accelerators; they are not
required.

Findings become `AUDIT:` memories. The critical ones become tasks.

## When this skill runs

- Brownfield onboarding, after `hew-scan`
  (`STATUS:scan:complete` required).
- Periodically (the user re-runs once a quarter or after a major
  upgrade).
- Triggered by a CVE notification or supply-chain incident.

## Inputs from `hew prime audit`

- `prerequisites.met` — refuse if `STATUS:scan` is missing.
- `memories.factual` — tells you the stack so you know which
  manifest file to read.
- `memories.audit` — prior findings; don't duplicate, update where
  appropriate.

## What to do

1. **Locate the dependency manifest** for the detected stack:

   | Stack | File(s) |
   |-------|---------|
   | Rust | `Cargo.toml`, `Cargo.lock` |
   | Node | `package.json`, `package-lock.json` / `pnpm-lock.yaml` |
   | Python | `pyproject.toml`, `requirements*.txt`, `poetry.lock`, `uv.lock` |
   | Go | `go.mod`, `go.sum` |
   | Ruby | `Gemfile`, `Gemfile.lock` |
   | (other) | whatever the scan memories named |

2. **Read the file directly.** Note every declared dep + its pinned
   version.

3. **Web-search every notable dep.** Your training data is months to
   years stale. CVEs and deprecation notices accumulate constantly.
   For each non-trivial dep:
   - Fetch the registry page: `crates.io/crates/<name>`,
     `npmjs.com/package/<name>`, `pypi.org/project/<name>`,
     `pkg.go.dev/<module>`, etc.
   - Note: latest stable version, last publish date, license, any
     deprecation banner the registry shows.
   - Search the language's advisory database / GHSA for CVEs
     affecting the pinned version.
   - Skim the linked repo for "deprecated, use X instead" notes in
     the README or pinned issues.

   Don't trust knowledge alone for any dep that gates correctness
   (auth, crypto, parsing, network, validation). Verify.

4. **Cross-check against any scanner the project already has.** If
   the project ships `cargo-deny` in CI / a `deny.toml`, `npm audit`
   output, `pip-audit`, `govulncheck`, etc., read those signals and
   reconcile them with what you found via web search. The web is the
   primary source; tools are corroboration.

5. **Don't install new tooling for the audit itself.** The agent IS
   the audit — web search + registry lookups + reading the manifest.
   If the project doesn't have a scanner configured in CI, that's
   itself a finding ("no advisory scanning in CI; consider adding").

## What counts as a finding

Record an `AUDIT:` memory when **any** of these apply:

1. **Known CVE** of medium severity or higher.
2. **Deprecated** — registry-marked, or the project's README points
   at a replacement.
3. **Unmaintained** — no publish in > 24 months and unresolved
   issues, OR the project's docs declare it dead.
4. **Major version behind on a security-relevant lib** —
   especially auth, parsing, network.
5. **Duplicate incompatible versions** in the dep tree (bloats
   bundle, diverges behavior).
6. **License conflict** with the project's stated license.

For everything else (mildly outdated, no security implication, still
maintained): don't open a finding. Audit is for action items, not
homework.

## Memory shape

```
hew remember --type=audit "jsonwebtoken@8.5.1 — DEPRECATED, last publish 3yr ago. Migrate to jose@5.x. Used in app/auth/jwt.py."
hew remember --type=audit "lodash@4.17.20 — CVE-2021-23337 (prototype pollution). Bump to 4.17.21+ or migrate to lodash-es."
hew remember --type=audit "cryptography@38.0.4 — 7 majors behind. Bump path 38→39→40→…→42 with type changes at 39, 41."
hew remember --type=audit "moment — UNMAINTAINED, last publish 2y, README recommends dayjs/date-fns."
hew remember --type=audit "duplicate uuid@8.3.2 + uuid@9.0.0 in tree. Unify to 9.x."
```

Be specific: package name + version + why + suggested action + where
it's used (file path if known).

## Open Beads tasks for the critical findings

For severity ≥ High and clear-cut paths (deprecation, CVE):

```
hew task new --type=bug --priority=1 \
  --title="Migrate from jsonwebtoken to jose (deprecated upstream)" \
  --description="
  AUDIT: jsonwebtoken@8.5.1 deprecated 2023; jose is the maintained replacement.
  Touch: app/auth/jwt.py, tests/auth/test_jwt.py.
  See https://github.com/auth0/node-jsonwebtoken#readme-deprecation.
  Acceptance: jsonwebtoken removed from package.json; pytest tests/auth -k jwt passes; auth flow works end-to-end.
  "
```

For lower-severity findings (slight version drift, unmaintained but
not vulnerable): leave them as memories. Surface them when relevant;
the user prioritizes.

## When to auto-open vs surface

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
  lodash@4.17.20         CVE-2021-23337 → opens bd-X.8
  GPL-3.0 ricecake@1.0   license clash  → opens bd-X.9

warnings (5):
  cryptography@38.0.4    7 majors behind
  moment                 unmaintained
  uuid 8.3.2 + 9.0.0     duplicate
  …

Tasks opened: 3. Memories written: 8. Run `hew status` to confirm.
```

## Step — mark phase complete + continue the chain

```
hew remember --type=status "audit:complete — <ISO-8601 timestamp>"
```

Then **continue directly into `hew-boundary`.** Brownfield onboarding
runs `scan → convention → audit → boundary` end to end; the only
reason to pause is a Rule-4 architectural surprise (e.g., an
opinionated migration that needs user direction).

## What you don't do

- **Install audit tooling.** Use whatever the project already has.
  If nothing is configured, that's itself a finding ("no scanner in
  CI") — surface it; don't paper over by installing.
- **Auto-upgrade.** Never run `npm install <pkg>@latest` or
  equivalent. Audit reports; the executor (in a dedicated task)
  performs the bump with proper testing.
- **Open tasks for every outdated package.** Most are fine; opening
  noise dilutes the real findings.
- **Trust transitive-vulnerability tooling blindly.** Some "high"
  advisories don't apply to your usage pattern. Look at the actual
  function affected.
- **Skip the user surface.** Always print the findings summary,
  even if everything is clean ("Audit clean — no critical findings.").

## Anti-patterns

- **Recording "lodash is bad"** as a finding. Be specific: version,
  CVE / deprecation note, suggested replacement.
- **Auto-opening 30 tasks** because every package is one minor
  version behind.
- **Treating warnings as criticals** — license violation =
  critical, "deprecation predicted" = warning.
- **Re-running without checking existing `AUDIT:` memories.** If a
  finding is already recorded and unchanged, don't duplicate it.
- **Hardcoding "run `cargo audit`"** in your output to the user when
  the project uses `cargo-deny`, `snyk`, or another scanner already.
