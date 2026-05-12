---
description: Map the public API + interface boundaries of a brownfield codebase. Fourth link in the chain. Persists BOUNDARY contracts.
---

Invoke the hew-boundary skill. Identifies the project's public
surface — HTTP routes, exported types, library entry points,
schema-bearing files — and persists each one as a `BOUNDARY:<name>`
memory describing the contract that downstream callers depend on.

The executor reads these before refactors. Breaking a `BOUNDARY:`
contract triggers `hew-execute`'s Rule 4 (architectural change —
surface to the user before proceeding).

Requires `STATUS:scan:complete`. Writes `STATUS:boundary:complete`
and closes the brownfield onboarding chain.

ARGUMENTS: $ARGUMENTS
