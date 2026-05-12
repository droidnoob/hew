---
description: Red-team / steelman pass — attacks undocumented gaps the friendly review can't see. Files findings as bd bugs.
---

Invoke the hew-adversarial-review skill. Same JSON input as
`/hew:review` (via `hew review-bundle`), opposite stance: assume the
code is wrong until proven right, steelman the alternative that wasn't
taken, find the worst input the code accepts.

Six adversarial axes: input fuzz, threat-model gaps, race conditions,
performance cliffs, abandoned error paths, hidden coupling +
undocumented invariants.

Findings file as bd issues titled `[Adversarial][BLOCKER|WARNING|INFO]
…` (distinct from `/hew:review`'s `[Review]` prefix). Writes the same
`STATUS:review:<ISO-timestamp>` marker on completion — running either
review skill resets the picker counter.

Pair with `/hew:review`: friendly catches convention drift; adversarial
catches what we forgot to write conventions for.
