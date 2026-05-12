---
description: Score the user's ask on goal-clarity + acceptance-clarity; loop Socratic questions until the ambiguity gate passes (or 4 rounds elapse).
---

Invoke the hew-spec skill on the user's request. Score on two dimensions
(goal-clarity, acceptance-clarity, each weighted 0.5). Loop up to four
Socratic rounds. Persist SPEC:<topic> + STATUS:spec:complete on pass; on
max-rounds-without-pass, persist the unresolved dimensions as [ASSUMED]
DECISION: memories and hand off to hew-plan anyway.

Lightweight pre-planning gate. Use when the ask is vague ("build a thing")
or when prior planning produced an architecture the user rejected as "not
what I meant."
