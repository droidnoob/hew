---
description: Dependency health check + craft-drift audit. Third link in the brownfield chain. Files AUDIT memories and opens bug tasks for clear-cut findings.
---

Invoke the hew-audit skill. Reads the project's dependency manifest
(Cargo.toml / package.json / pyproject.toml / go.mod / etc.) and
web-searches every notable dep for CVEs, deprecations, unmaintained
status, major-version drift, and license conflicts. Plus the
brownfield craft-drift pass: walks each `CONVENTION:craft.<id>`
memory and greps for codebase regions that already contradict the
picked principle (e.g. domain importing infrastructure when
craft.clean-architecture is in force).

Persists findings as `AUDIT:<finding>` memories with location + fix
direction. Auto-opens `bug` tasks for severity ≥ High and clear-cut
local fixes; leaves the rest as memories for user triage.

Requires `STATUS:scan:complete`. Writes `STATUS:audit:complete`.

ARGUMENTS: $ARGUMENTS
