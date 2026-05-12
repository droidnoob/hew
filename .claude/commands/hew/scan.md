---
description: Architecture-mapping pass for a brownfield codebase — first link in the scan → convention → audit → boundary chain. Persists findings as factual + STATUS:scan memories.
---

Invoke the hew-scan skill. Walks the top-level layout, identifies
the stack(s), maps key modules and their relationships, and
persists the architectural fingerprint as factual memories so the
rest of the loop (convention extraction, dep audit, boundary
mapping) has shared context.

Writes `STATUS:scan:complete — <ts>` on success and surfaces the
suggested next step (`/hew:convention`) to continue the brownfield
onboarding chain.

Run once per new codebase the agent enters. Re-run after a major
refactor that reshapes the top-level layout.

ARGUMENTS: $ARGUMENTS
