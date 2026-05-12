---
description: Evaluate a candidate new dependency before adding it. Persists a DEP memory with verdict (adopt / hold / reject) + rationale.
---

Invoke the hew-deps skill. Before pulling in a new crate / package /
gem, the skill checks: registry health (active maintenance, license,
last publish), CVE history, supply-chain footprint (transitive
count, build-from-source vs prebuilt binaries), and whether the
project already ships something that covers the use case.

Persists a `DEP:<package>@<version> — adopt|hold|reject — <rationale>`
memory. Adopted deps get a follow-up integration plan via `/hew:plan`.

Opt-in skill — set `hew config set optional-skills.deps true` to
keep this slash installed across `hew update` runs.

ARGUMENTS: $ARGUMENTS
