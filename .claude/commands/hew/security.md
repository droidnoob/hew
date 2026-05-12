---
description: Lightweight security check on auth + input + secret paths. Persists SECURITY memories; opens bug tasks for clear-cut findings.
---

Invoke the hew-security skill. A focused pass on the three highest-
yield areas: authentication flows (token TTL, rotation, revocation),
input handling (validation before side-effect, escape boundaries,
deserialization), and secrets (no inline keys, env-var sourcing,
.env.example coverage).

Findings persist as `SECURITY:<area> — <pattern>` memories. Clear-
cut local fixes auto-open `bug` tasks tagged `[Security][BLOCKER]`
or `[Security][WARNING]`.

Distinct from `hew-guard` (per-task pre-close, the seven hard checks)
and the security pillar of `hew-review` (batch-level diff review).
This skill is the dedicated security pass for the auth/input/secrets
surface area.

Opt-in skill — set `hew config set optional-skills.security true`
to keep this slash installed across `hew update` runs.

ARGUMENTS: $ARGUMENTS
