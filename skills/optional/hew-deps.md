<!-- hew:version=0.1.0 -->
---
name: hew-deps
category: optional
init: hew prime deps
---

# hew-deps — Inspect a Candidate New Dependency

You vet a library *before* the executor adds it to the project. This
exists because the most predictable AI agent failure is picking a
library version from training data that's been deprecated, vulnerable,
or replaced for two years.

You run on a specific package name + version (or "latest"). You return
a recommendation: **adopt**, **adopt with caveats**, or **reject** —
plus a `DEP:` memory persisting the verdict.

## When this skill runs

- The executor wants to add a new dependency (`npm install`,
  `cargo add`, `pip install`, etc.) and the convention says check
  first.
- The user types `/hew:deps <package>` ad-hoc.
- Inside `hew-plan` when an architectural decision names a library.

## Inputs from `hew prime deps`

- `memories.dep` — prior verdicts on this package. If already evaluated
  and the version is the same, return the cached verdict.
- `memories.audit` — if this package already appears as an `AUDIT:`
  finding, treat as a rejection signal.

You also get the **package name** and an **optional version**.

## The check

For each candidate, look at:

### 1. Existence and authenticity

- Resolve the package on its registry. **Never assume a name exists.**
  Hallucinated package names are how supply-chain attacks land.
- If the name doesn't resolve, **STOP** and tell the user. Do not
  suggest "did you mean ..." substitutions — that's exactly the
  attack vector typo-squatters rely on.

### 2. Latest stable

- Identify the latest stable version (not pre-release, not RC).
- If the user requested an old version, note the drift and the reason
  to use latest (usually: yes).

### 3. Maintenance signal

- Last publish date — > 24 months = concerning.
- Open issues count + age of oldest unresponded.
- Recent commit activity in the repo (if linked).
- Number of maintainers.

A library with 50M downloads/month and one publish 3 years ago is
still probably fine (it's "done"). A library with 200 downloads and no
publishes in 18 months is dead.

### 4. Vulnerabilities

- Query the language's advisory database:
  - npm: `npm audit <package>@<version>` or registry advisories page
  - Python: `pip-audit` / Safety DB
  - Rust: `cargo audit` / RustSec
  - Go: `govulncheck`
- Any CVE ≥ Medium that affects the requested version = reject (or
  bump to a patched version).

### 5. License

- MIT / Apache-2.0 / BSD-3 / ISC = adopt freely.
- MPL-2.0 / LGPL = adopt with attribution.
- GPL / AGPL = surface as caveat; copyleft conflicts with most
  commercial codebases.
- Unknown / no license = reject pending clarification.

### 6. Breaking-change history

If migrating from an older major to the latest stable, skim the
changelog for the breaking changes. Surface the relevant ones; the
executor will adapt the code.

### 7. Bundle size / install weight

Optional but useful for frontend deps. If the lib adds > 100KB
gzipped and is only used in one place, suggest a lighter alternative
or vendor the small piece you need.

## Memory shape

```
bd remember "DEP: Using jose@5.2.0 (latest stable, 2026-04). Actively maintained. MIT. Replaces deprecated jsonwebtoken. No known CVEs."
bd remember "DEP: Adopted dayjs@1.11.10. MIT. 2KB gzipped. Replaces moment (unmaintained)."
bd remember "DEP: REJECTED ricecake@1.0 (GPL-3.0 — incompatible with this project's MIT license). Use cake-mix@2.x (MIT) instead."
```

Record the verdict + why + the chosen alternative if rejected.

## Output

```
hew-deps: jose@5.2.0 — ADOPT
  Latest stable: 5.2.0 (2026-04-15)
  Maintenance: active, 12 maintainers, last commit 3 days ago
  Vulns: none
  License: MIT
  Bundle: ~18KB gzipped (server-side OK)
  Memory: DEP: Using jose@5.2.0 ...
```

or:

```
hew-deps: somelib@2.1.0 — REJECT
  Reason: GPL-3.0 license. This project is MIT.
  Suggested alternative: cake-mix@2.x (MIT, similar API).
  Memory: DEP: REJECTED somelib@2.1.0 ...
```

or:

```
hew-deps: fancy-utils@9.99.0 — DOES NOT EXIST
  No package found on the registry.
  STOP — do NOT attempt to install. The name may be hallucinated.
  Action: confirm the exact spelling with the user; if no plausible
  package matches, abort.
```

## What you don't do

- **Install the package.** That's the executor's job, after your
  verdict.
- **Auto-substitute** a similarly-named package when the requested
  one doesn't exist. The risk of installing malware is too high.
- **Trust training-data version numbers** without checking the
  registry. Versions move; advisories accumulate.
- **Approve "for now, we'll switch later."** If you adopt with
  caveats, the caveats become tasks.

## Anti-patterns

- **Approving a dep with a 3-year-old last publish** without checking
  whether it's a finished library or an abandoned one.
- **Skipping the license check** because "it's open source." GPL on
  a commercial codebase is a real problem.
- **Recommending alternatives without verifying them too.** Run the
  full check on the suggested replacement.
