<!-- hew:version=0.6.1 -->
---
name: hew-adversarial-review
category: optional
init: hew review bundle --json
---

# hew-adversarial-review — Red-Team / Steelman Pass

You become the hostile reviewer. Assume the code is wrong until proven
right. Steelman the alternative approach that wasn't taken. Find the
worst input the code accepts. Surface load-bearing assumptions that
aren't documented. Ask: what did we *not* test?

This is **not** a re-run of `hew-review`. That skill checks the code
against documented constraints. This skill attacks the *undocumented
gaps* — the things the friendly review can't see because they're not
in any memory.

Pair: `/hew:review` first (catches convention drift), then
`/hew:adversarial-review` (catches what we forgot to write rules for).

Not installed by default. Opt in via:

```
hew config set optional-skills.review true   # same flag as hew-review
```

## When this skill runs

- The user invokes `/hew:adversarial-review`.
- `hew-execute` Step 10 picker fires and the user chooses "Adversarial
  review" or "Both."
- A pre-launch / pre-merge check before the work goes live.
- After `/hew:review` filed minor findings but you suspect there's
  something deeper.

## Inputs from `hew review bundle`

Same JSON bundle as `hew-review` (invoke `hew review bundle --json` —
the text default is a short summary, not the full payload): `scope`,
`closed_tasks`, `diff`, `diff_base`, `memories`, `epic`,
`last_review_at`. The shape is identical; what differs is the stance
you take reading it.

## Stance — be the antagonist

Stop reviewing as the author. Start reviewing as:

- **The hostile user** who's actively trying to break this.
- **The on-call engineer** who'll be paged at 3am when this fails.
- **The reviewer who already shipped the v2 approach** and thinks v1
  was the wrong shape.
- **The security researcher** who'll write the disclosure when this
  ships.

You don't have to be right. You have to be *uncomfortable to dismiss*.
A finding the team can wave away easily ("that won't happen in
production") is a finding worth filing — either it's a real risk
they'll regret dismissing, or the dismissal is documented and now
auditable.

## Six adversarial axes

### 1. Input fuzz — the worst inputs

For every new input surface (route handler, parser, CLI arg, queue
consumer), enumerate the inputs that *would* break it:

- Empty string, single character, max-length string.
- Unicode (RTL marks, zero-width joiners, NFC vs NFD collisions).
- Negative numbers where positive expected; `0`; `INT_MAX`.
- Null bytes mid-string; ASCII control characters.
- Trailing whitespace; missing trailing newline.
- The same input twice (idempotency); the same input racing itself.
- Inputs that pass validation but fail the next stage.

If the tests in the diff don't exercise these and the input isn't
constrained by the protocol, that's a finding.

### 2. Threat-model gaps

Even if `memories.security` covers the obvious case, look for
*adjacent* attack surfaces:

- A new endpoint added beside a protected one — does it inherit the
  middleware, or did the agent forget?
- A new background job — what trust level does it run at?
- A new external dependency — does it get user input passed to it
  unsanitized? Does it have known CVEs?
- Logs that *almost* contain PII — error messages with the user's
  payload echoed back.
- An auth check that compares strings — is it timing-safe?

`memories.security` may be silent on these because they're new
surface. The silence isn't safety; it's a gap.

### 3. Race conditions + ordering

- Two requests arriving at the same time on the same row. Last-write
  wins? First-write wins? Does it matter?
- A retry on the client side double-executing a side-effect.
- An async job firing before the transaction that scheduled it
  commits.
- A migration running while writes are in flight.
- Tests that pass serially but would fail under `cargo test --jobs=N`
  with shared state.

### 4. Performance cliffs

- N+1 queries (loops with DB calls inside).
- Loading an unbounded list into memory.
- Operations that are O(n) per request but n grows with users.
- Cold-start cost of a new dependency.
- Lock contention introduced by a new exclusive critical section.

If the diff adds a loop touching the DB, count the queries per
iteration. Often the author didn't.

### 5. Abandoned error paths

- A `match` arm that swallows an error with `_` or `let _ =`.
- A `?` that bubbles up to a caller that ignores it.
- A panic in code that's reached by user input.
- Retries with unbounded count or no backoff.
- "Best-effort" cleanup that silently fails and leaks state.

For each one, ask: *who notices when this path fires in production?*
If the answer is "nobody until someone complains," it's a finding.

### 6. Hidden coupling + undocumented invariants

- Function A's correctness depends on something Function B does
  earlier in the same request, but that order isn't enforced.
- A struct field is "optional" in the type but required in 80% of
  call sites — file a finding to make it explicit.
- A test passes because of fixture data that happens to satisfy an
  unstated invariant.
- A config flag is documented as "experimental" but a code path
  unconditionally reads it.

These are the bugs that survive `/hew:review` because no convention
exists for them yet. The adversarial pass exists to surface them.

### 7. Craft pillar — attack the gaps the project didn't pick

Friendly review walks the principles the project **chose**. Adversarial
review attacks the principles the project **left out**. Read the
`CONVENTION:craft.*` set, then list every catalog principle *not*
present and ask: what failure mode does this absence enable?

Examples:

- Project didn't pick `craft.fail-fast`. → Where in the diff does an
  endpoint persist or send a side-effect before validating input?
  What happens when validation fails after the write?
- Project didn't pick `craft.idempotence`. → Which new handler is
  retried by the client / queue / load balancer? What happens on a
  duplicate?
- Project didn't pick `craft.pure-functions`. → Which "computation"
  reaches out to a clock, a DB, an env var, making it untestable?
- Project didn't pick `craft.dry`. → Where will the next person copy
  this code block, and what will go wrong when only one copy gets
  fixed?
- Project didn't pick `craft.tell-dont-ask`. → Where does the diff
  reach into another object's state and branch on it, creating an
  invariant that lives in the caller?

A *picked* craft principle whose violation `/hew:review` already
flagged is not adversarial-scope — friendly review owns those. Look
for what friendly review *couldn't* see because no `CONVENTION:craft.<id>`
authorized it.

File findings under the `[CRAFT]` tag with severity calibrated to the
realized risk:

```
[Adversarial][WARNING][CRAFT] app/api/billing.py:34 — handler is non-idempotent
  but POST /charge is retried by Stripe on 5xx. Project didn't pick
  craft.idempotence so this isn't a CONVENTION violation, but a duplicate
  retry will double-charge. Either pick craft.idempotence and fix, or
  document the retry-unsafe boundary in a SECURITY:/DECISION: memory.
```

The output of this pillar is sometimes "the project should pick
principle X" — that's a legitimate finding, filed as a chore so the
team can revisit the picker.

## Steelman the alternative

For the **biggest** chunk of changed code, write a one-paragraph
steelman of the approach that *wasn't* taken. Examples:

> Steelman of the not-taken path: instead of growing the BdClient
> trait with `run_to_file`, the same goal could be reached by making
> `run_raw` always use file-redirection internally — one path,
> simpler mental model, no caller decisions about "is this output
> big?". Trade-off: every small command pays a temp-file cost.

You don't have to recommend the alternative. You have to make it
*plausibly competitive*. If you can't, the current approach is
genuinely the right one — record that as a finding tagged INFO:

```
[Review][INFO] Steelman of alternative considered (file-redirection in
run_raw vs explicit run_to_file) — rejected because per-call temp file
overhead would dominate for `bd version` / `bd remember` calls. No
action needed; logged for the audit trail.
```

That single audit-trail INFO is more valuable than ten "looks good"
comments.

## Severity → filing

Identical to `hew-review`:

| Severity | bd type | Title prefix |
|----------|---------|--------------|
| BLOCKER  | `bug`   | `[Adversarial][BLOCKER] …` |
| WARNING  | `bug`   | `[Adversarial][WARNING] …` |
| INFO     | `chore` | `[Adversarial][INFO] …` |

Craft-gap findings append `[CRAFT]`:
`[Adversarial][WARNING][CRAFT] …`. Severity reflects the realized
failure mode, not the principle's prestige — an unenforced
`craft.idempotence` on a payment endpoint is BLOCKER; the same gap on
a cached read is INFO.

Filing template:

```
hew task new --type=bug --priority=<1|2|3> \
  --title='[Adversarial][BLOCKER] auth/jwt.rs:42 — token compare is non-constant-time, enables timing attack' \
  --description='Found during /hew:adversarial-review scope=Epic(hew-auth).
Originating tasks: hew-auth.3.
Attack: a remote attacker times millisecond differences in 401
responses to recover the JWT signing-key byte-by-byte (classic
non-constant-time comparison).
Diff line 42: `actual_token == expected_token` (PartialEq on bytes).
Fix: use `subtle::ConstantTimeEq::ct_eq` (already a workspace dep) or
equivalent. The original Review pass missed this because no
SECURITY: memory mentions timing-safety, but the project ships an
auth surface — file the missing convention as part of the fix.'
```

Title prefix is `[Adversarial]`, not `[Review]`, so the two passes
are distinguishable in `hew task list` / `hew task search`.

## After filing

Write the SAME `STATUS:review:<ts>` marker as `hew-review`. Both
skills satisfy the "last review" position — running adversarial after
friendly doesn't double-mark; running friendly after adversarial
overwrites cleanly. The `tasks_since_last_review` counter resets on
either.

```
hew remember --type=status "review:$(date -u +%Y-%m-%dT%H:%M:%SZ)"
```

## Output to the user

```
hew-adversarial-review — scope=Epic(hew-auth), 4 tasks, 380 LOC diff

Adversarial axes:
  Input fuzz             2 findings filed
  Threat-model gaps      1 BLOCKER filed
  Race conditions        0 findings
  Performance cliffs     1 WARNING filed
  Abandoned errors       1 WARNING filed
  Hidden coupling        1 INFO filed

Steelman:
  Alternative considered: middleware-based CSRF vs per-route. Rejected
  on this codebase because the route table is already explicit; the
  middleware would have a 2-line carve-out per public route. Logged.

Filed:
  hew-xxx.1  [BLOCKER] non-constant-time JWT compare in auth/jwt.rs:42
  hew-xxx.2  [WARNING] N+1 query in /api/orders/<id>/items
  hew-xxx.3  [WARNING] retry loop without backoff in queue/dispatch.rs
  hew-xxx.4  [INFO] document the email-token-TTL invariant explicitly
  hew-xxx.5  [INFO] consider middleware-based CSRF (steelman, audit)

Marker:
  STATUS:review:2026-05-12T14:45:00Z

Next: the JWT compare is exploitable today — patch before merge.
```

## What you don't do

- **Repeat hew-review findings.** Before filing, check whether the
  finding overlaps with an already-open `[Review]` bug. If yes, add a
  comment to the existing one rather than re-filing.
- **File "consider rewriting in Rust" musings.** Findings must name a
  concrete attack, regression, or invariant. "This could be cleaner"
  is not adversarial; it's bikeshedding.
- **Auto-fix.** Same rule as hew-review — file and stop. The fix path
  is `hew-execute` claiming the bd issue.
- **Skip the steelman.** Even on small diffs, one paragraph forces
  you to articulate *why* the chosen path beats the alternative. A
  steelman that can't be written is a finding.
- **Write memories besides the marker.** Same DECISION:review-filing
  rule — bd issues, not REVIEW:/RISK: memories.

## Anti-patterns

- **Performative hostility.** Filing 20 findings for the sake of
  filing isn't adversarial; it's noise. A real adversarial pass
  produces fewer findings than a friendly pass, but they're sharper.
- **Asking "what if?" without an attack path.** "What if the network
  fails?" — does the code handle it, or not? Answer the question
  before filing.
- **Steelman that's a strawman.** If your one-paragraph alternative
  is obviously worse, you're not steelmanning — you're justifying.
  Find a genuinely competitive alternative or admit the chosen path
  is correct.
- **Refusing to file INFO.** A surfaced-and-dismissed risk is more
  valuable than a hidden one. File it as INFO; the audit trail wins.
