<!-- hew:version=0.6.0 -->
---
name: hew-deps
category: optional
init: hew prime deps
---

# hew-deps — Inspect a Candidate New Dependency

You vet a library *before* the executor adds it. You — the agent —
do the inspection: look up the package on its registry, check
maintenance signals, look for CVEs, read the license. Tools are
optional. The verdict is yours.

This exists because the most predictable AI agent failure is picking
a library version from training data that has been deprecated,
vulnerable, or replaced for two years.

You return a recommendation — **adopt**, **adopt with caveats**, or
**reject** — and persist a `DEP:` memory.

## When this skill runs

- The executor wants to add a new dependency (`npm install`,
  `cargo add`, `pip install`, etc.) and the convention says check
  first.
- The user types `/hew:deps <package>` ad-hoc.
- Inside `hew-plan` when an architectural decision names a library.

## Inputs from `hew prime deps`

- `memories.dep` — prior verdicts on this package. If already
  evaluated and the requested version is the same, return the
  cached verdict.
- `memories.audit` — if this package already appears as an `AUDIT:`
  finding (deprecated / CVE / etc.), treat as a strong reject
  signal.

You also get the **package name** and an **optional version**.

## What to do

For each candidate, the agent investigates seven dimensions:

### 1. Existence and authenticity

- **Resolve the package on its registry**:
  - `crates.io/crates/<name>`
  - `npmjs.com/package/<name>`
  - `pypi.org/project/<name>`
  - `pkg.go.dev/<module>`
- **Never assume a name exists.** Hallucinated package names are how
  supply-chain attacks land.
- If the name doesn't resolve, **STOP** and tell the user. Do not
  suggest "did you mean ..." substitutions — that's exactly the
  attack vector typo-squatters rely on.

### 2. Latest stable

- Identify the latest stable version (not pre-release, not RC).
- If the user requested an old version, note the drift and the
  reason latest is preferred (usually: yes).

### 3. Maintenance signal

- Last publish date — > 24 months without releases is concerning
  unless the lib is genuinely "done" (e.g., small, focused, no
  ecosystem churn).
- Open issues count + age of oldest unresponded.
- Recent commit activity in the linked repo.
- Number of maintainers.

### 4. Vulnerabilities

- Check the language's advisory channel (registry advisories tab,
  RustSec, npm advisories, PyPI advisories, govulncheck data).
- Any CVE ≥ Medium against the requested version = reject (or bump
  to a patched version).

### 5. License

- MIT / Apache-2.0 / BSD-3 / ISC = adopt freely.
- MPL-2.0 / LGPL = adopt with attribution.
- GPL / AGPL = surface as caveat; copyleft conflicts with most
  commercial codebases.
- Unknown / no license = reject pending clarification.

### 6. Breaking-change history

If migrating from an older major to the latest stable, skim the
changelog for the relevant breaking changes. Surface them; the
executor will adapt the code.

### 7. Bundle size / install weight (frontend only)

If the lib adds > 100KB gzipped and is only used in one place,
suggest a lighter alternative or vendor the small piece you need.

## How to gather this information — web first, always

Your training data is stale by months to years. For dependency
evaluation that's not acceptable; versions move and advisories
accumulate. The canonical mechanism is **web search**, every time.

The agent's order of operations:

1. **Fetch the registry page** for the package:
   - `crates.io/crates/<name>` (Rust)
   - `npmjs.com/package/<name>` (Node)
   - `pypi.org/project/<name>` (Python)
   - `pkg.go.dev/<module>` (Go)
   - `rubygems.org/gems/<name>` (Ruby)

   Read: latest stable version, last publish date, license,
   deprecation banner (if any), download counts.

2. **Open the linked source repo** (usually GitHub). Read:
   - README header — explicit "use X instead" / "deprecated"
     pointers
   - Pinned issues — security notices, sponsorship status
   - Recent commit activity — is the repo alive?
   - Number of contributors / maintainers

3. **Search the advisory database** for known CVEs against the
   requested version (RustSec, GHSA, PyPI advisories, npm
   advisories).

4. **Search the web** for "<package> deprecated", "<package>
   alternative", "<package> CVE" to catch anything the registry
   buries.

Only after that synthesis do you commit to a verdict. Anything
shorter is guessing.

Don't install scanner tools just to evaluate one dep. If the project
already has `cargo-deny`, `snyk`, or `npm audit` configured, their
output is corroboration; the agent's web-driven analysis is primary.

## Memory shape

```
hew remember --type=dep "Using jose@5.2.0 (latest stable, 2026-04). Actively maintained. MIT. Replaces deprecated jsonwebtoken. No known CVEs."
hew remember --type=dep "Adopted dayjs@1.11.10. MIT. 2KB gzipped. Replaces moment (unmaintained)."
hew remember --type=dep "REJECTED ricecake@1.0 (GPL-3.0 — incompatible with this project's MIT license). Use cake-mix@2.x (MIT) instead."
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
- **Install audit tooling** just to evaluate a single dep. The
  agent has direct registry access; use it.
- **Approve "for now, we'll switch later."** If you adopt with
  caveats, the caveats become tasks.

## Anti-patterns

- **Approving a dep with a 3-year-old last publish** without
  checking whether it's a finished library or an abandoned one.
- **Skipping the license check** because "it's open source." GPL on
  a commercial codebase is a real problem.
- **Recommending alternatives without verifying them.** Run the
  full check on the suggested replacement too.
