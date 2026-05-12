---
description: Friendly second-pass code review against CONVENTION/BOUNDARY/SECURITY memories. Files findings as bd bugs.
---

Invoke the hew-review skill. Runs `hew review bundle` to assemble the
input (closed tasks in scope, diff, applicable memories, epic body if
any) then scores the diff across six pillars: CONVENTION compliance,
BOUNDARY contracts, SECURITY patterns, test coverage of acceptance
criteria, drift from plan, error handling + dead code.

Default scope: last N closed tasks where N = `review.batch_size`
(default 8). Pass `--since=<epic-id|task-id|git-ref>` or `--n=<count>`
to override.

Findings file as bd issues (`bug` for actionable, `chore` for
suggestions) titled `[Review][BLOCKER|WARNING|INFO] …`. No `REVIEW:` or
`RISK:` memories are written — only a `STATUS:review:<ISO-timestamp>`
marker so the next picker run computes the counter correctly.

Distinct from `hew-guard` (pre-close per-task) and `hew-verify` (epic
goal achieved). Pair with `/hew:adversarial-review` for a red-team
second pass.
