<!-- hew:version=0.11.0 -->
---
name: hew-review
category: optional
init: hew review bundle --json
---

# hew-review — Friendly Second-Pass Code Review

You re-read the work of recent closed tasks against the codebase's
constraint memories. This is a friendly review: assume the author
(usually past-you) was acting in good faith; surface drift, missing
tests, broken patterns, and forgotten edges so they get filed instead
of forgotten.

Distinct from `hew-guard` (which is pre-close per-task sanity). This
runs after a batch of work — typically an epic close, or N tasks since
the last review marker.

Not installed by default. Opt in via:

```
hew config set optional-skills.review true
```

For automatic triggering at epic close or after N closed tasks, also:

```
hew config set review.after_epic true
hew config set review.after_n_tasks 8   # default batch_size
```

The manual entry point `/hew:review` is always available regardless of
config.

## When this skill runs

- The user invokes `/hew:review` (optionally with `--since=<ref>` or
  `--n=<count>`).
- `hew-execute` Step 10 picker fires per the trigger rule and the user
  chooses "Review batch."
- After a `hew-verify` flags drift the user wants double-checked.

## Inputs from `hew review bundle`

`hew review bundle --json [--since=<epic-id|task-id|git-ref>] [--n=<count>]`
emits a JSON `ReviewBundle` (use `--json` — the text default is a
short summary, not the full payload) with:

- `scope` — what the caller asked for (LastN / Epic / Task / GitRef).
- `closed_tasks` — oldest-first list of tasks in scope, each with id,
  title, issue_type, priority, closed_at, close_reason, parent.
- `diff` — unified git diff covering the scope.
- `diff_base` — the SHA the diff is against (None when no commit
  predates the anchor).
- `memories.conventions` / `boundaries` / `security` — the
  constraint-bearing memories you must check the diff against.
- `epic` — populated when scope is `Epic`: id, title, body, child_count.
- `last_review_at` — prior `STATUS:review:<ts>` marker if any.
- `changed_symbols` — when the binary was built with `--features
  treesitter`, this is a per-symbol slice of the diff:
  `{file, language, name, kind, line_start, line_end, source_slice}`.
  `source_slice` is the literal bytes of the symbol's definition.
  **Read these slices first.** They give you the function bodies that
  actually changed without re-reading whole files; widen to the full
  file via the `diff` field only when the slice's context is
  insufficient (call-site analysis, surrounding fields, etc.).
  Absent under default builds — fall back to scanning the diff
  directly.

The bundle is the entire input. Don't grep the codebase for more
context until you've used what's in the bundle — you'll only catch
drift you can see in the diff.

## The review rubric

Six pillars. Score each finding under the pillar it primarily fits.

### 1. CONVENTION compliance

Cross-reference every changed file against `memories.conventions`.
The convention prefix often names the area it applies to
(`CONVENTION:naming`, `CONVENTION:errors-lib`,
`CONVENTION:rust-subprocess`). Look for:

- New code that violates an existing rule.
- Code that *would have* benefited from a convention but the
  convention is missing (file a `chore` to capture it).
- Tests that mock layers a convention says shouldn't be mocked.

### 2. BOUNDARY contracts

Every modified function/route/module listed in `memories.boundaries`
is a contract. Look for:

- Signature changes without callers updated.
- Removed fields that downstream code still reads.
- Added required fields that downstream code doesn't pass.
- Status-code changes (200 → 204, 200 → 200-with-different-shape).

Backwards-compatible extensions (new optional fields) are fine —
flag them only if the convention is "explicit version bumps required."

### 3. SECURITY patterns

For changes touching auth, input handling, secrets, or the network
surface, cross-reference `memories.security`. Look for:

- Missing input validation on a route flagged as user-facing.
- Hardcoded secrets, API keys, tokens, default credentials.
- Logging that captures PII or auth tokens.
- New endpoints lacking the project's auth pattern.

If `memories.security` is empty, the project hasn't documented its
security stance — file a chore noting that gap.

### 4. Test coverage of acceptance criteria

For each task in `closed_tasks`, the `close_reason` should mention the
tests that landed. Skim the diff for the test files it claims to
include. Look for:

- Acceptance criterion implied by the close reason but no test in the
  diff.
- Tests that exist but don't exercise the failing-input path.
- Tests that share state across cases (parallel-test surprise).
- Code paths added but not exercised at all.

### 5. Drift from the epic plan

When `epic` is populated, re-read the epic body. Does the cumulative
diff actually deliver what the epic said? Or did the agent drift —
shipping adjacent features, missing one of the Success Criteria, or
sneaking in an unrelated refactor?

Drift isn't always bad (Rule 1/2 deviations in hew-execute are
legitimate); the question is whether it was *acknowledged*. The close
reasons should mention any deviation rules applied.

### 6. Error handling + dead code

- Unwraps / expects in code paths that can legitimately fail at runtime.
- Newly-introduced functions or modules that aren't called anywhere.
- Public APIs added without inline tests for the error variants.
- TODOs / FIXMEs left in the diff (`hew-guard` should catch these but
  sometimes lets them through).

### 7. Craft pillar — picked principles

Walk every `CONVENTION:craft.<id>` memory in the bundle (the bundle
already collects them under `memories.conventions`). For each picked
principle, inspect the diff for violations:

| Principle (id)               | What to look for in the diff                                                      |
|------------------------------|-----------------------------------------------------------------------------------|
| `craft.solid` / SRP          | A class/module grew a second unrelated reason to change (e.g. service handles routing AND persistence). |
| `craft.dry`                  | Identical 5+ line blocks across two files / two functions; missed helper extraction. |
| `craft.kiss` / `craft.yagni` | A new abstraction introduced for a single caller; speculative interfaces.         |
| `craft.small-functions`      | Functions exceeding `craft.max_function_lines`; obvious split points ignored.     |
| `craft.fail-fast`            | Endpoint persists/sends side-effects before validating input.                     |
| `craft.idempotence`          | Retry-unsafe handler in a flow advertised as retryable.                           |
| `craft.tell-dont-ask`        | New code reads a getter then branches on the result, where a method on the owner would be cleaner. |
| `craft.consistency-with-existing-code` | The diff adopts a pattern that contradicts an existing `CONVENTION:` memory — always wins, file as drift. |

Skip principles **not** present in the project's set — applying SOLID
universally is exactly what `DECISION:craft-adaptive` rejected.

Cross-reference any unresolved soft-warnings `hew-guard` surfaced for
the closing tasks (`missing-tests`, `function-length`, `duplication`).
A warning that the executor silenced without a `DECISION:` justifying
it is a finding here.

If a `DECISION:craft-feature:<plan-id>` memory documents a deliberate
deviation for this plan's scope, don't flag the deviation — note it as
*acknowledged* in the review output.

## Severity → filing

Every finding lands in bd. **No memory pollution.** Two types:

| Severity | bd type | Title prefix |
|----------|---------|--------------|
| BLOCKER — actively broken, security risk, contract break | `bug` | `[Review][BLOCKER] …` |
| WARNING — convention drift, missing test, dead code, doc gap | `bug` | `[Review][WARNING] …` |
| INFO — suggestion, future improvement, "consider X" | `chore` | `[Review][INFO] …` |

Craft findings use the same severities with a `[CRAFT]` tag appended:
`[Review][WARNING][CRAFT] services/billing.py — DRY violation…`. Pick
severity by the project's existing rules, not the principle itself —
a missed extraction is usually WARNING; a SRP violation that hides a
race is BLOCKER.

Filing template:

```
hew task new --type=bug --priority=<1|2|3> \
  --title='[Review][BLOCKER] auth/jwt.rs:42 — missing CSRF check on POST /login' \
  --description='Found during /hew:review scope=LastN(8).
Originating tasks: hew-abc.3, hew-abc.4.
Diff line 42 of auth/jwt.rs: new POST handler skips the project'\''s
CSRF middleware (see SECURITY:csrf — required on POST).
Fix: route through validate_csrf() before issuing the JWT.'
```

Always reference:
1. The originating closed task IDs (so the audit trail links back).
2. The convention/boundary/security memory the finding ties to.
3. A concrete fix direction — not "needs work" but "do X to Y."

If the finding is a clear-cut breach of one specific `CONVENTION:` /
`BOUNDARY:` / `SECURITY:` memory, also drop a sidecar LINK row so the
bug task surfaces in that memory's `hew memories --links` view:

```
hew remember --type=link --raw "LINK:security-csrf->relates_to:task:hew-xxx.1"
```

This keeps the graph navigable for a future reviewer who searches the
memory side first ("what bugs have we filed against `SECURITY:csrf`?")
without bloating the bug description with cross-references.

## After filing

Write the review marker so the next run computes
`tasks_since_last_review` correctly:

```
hew remember --type=status "review:$(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

The `STATUS:review:<ts>` memory is the ONLY memory this skill writes.
Findings are bd issues, never memories. This keeps the memory store
clean — a project with five reviews accumulates five tiny `STATUS:`
markers, not five hundred `REVIEW:` findings.

## Output to the user

Short summary, in this shape:

```
hew-review — scope=LastN(8), 8 tasks, 1.2k LOC diff

Pillars checked:
  CONVENTION (22 rules)   2 findings filed
  BOUNDARY (4 contracts)  0 findings
  SECURITY (3 patterns)   1 BLOCKER filed
  Test coverage           1 WARNING filed
  Drift from plan         clean
  Error handling          2 INFO filed

Filed:
  hew-xxx.1  [BLOCKER] missing CSRF on POST /login
  hew-xxx.2  [WARNING] convention:naming — service_name in auth/handler.rs
  hew-xxx.3  [WARNING] no test for the rate-limit branch in auth/middleware.rs
  hew-xxx.4  [INFO] consider extracting shared retry logic to util/retry.rs
  hew-xxx.5  [INFO] add a #[doc] to public TokenStore trait

Marker:
  STATUS:review:2026-05-12T14:30:00Z

Next: triage the BLOCKER first (hew-xxx.1).
```

If nothing was found, say so plainly — and write the marker anyway
(the run still happened; the counter still resets).

## What you don't do

- **Auto-fix.** This skill *files* findings. `hew-execute` fixes them.
  Mixing review with fix corrupts the audit trail.
- **Re-review your own most recent commits in the same session
  without scope.** If the agent that wrote the code is the same agent
  reviewing it, lean adversarial — or invoke `/hew:adversarial-review`
  next.
- **Write `REVIEW:` or `RISK:` memories.** Per DECISION:review-filing,
  findings go to bd issues only.
- **Grep beyond the bundle without a reason.** The bundle was assembled
  to bound the review; wandering through the codebase makes the review
  unbounded and the findings vague.
- **Re-file an existing open `[Review]` bug.** Before creating, check
  with `hew task search '[Review]'`; update the
  existing one if it's still open.

## Anti-patterns

- **One-line findings.** "Looks bad" is not a finding. Name the file,
  line, convention violated, and proposed fix.
- **Vague severities.** If you can't decide between BLOCKER and
  WARNING, default WARNING — BLOCKER is for "ship this and something
  breaks."
- **Reviewing the bundle's diff in isolation without reading the
  memories.** The pillars are *what to check against*, not optional
  context.
- **Skipping the marker write.** Without `STATUS:review:<ts>` the next
  run double-counts and the picker fires twice.
- **Sending findings to memory.** Per the design contract, memories
  stay clean. Bd issues are the channel.
