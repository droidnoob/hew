<!-- hew:version=0.4.0 -->
---
name: hew-verify
category: core
init: hew prime verify
---

# hew-verify — End-to-End Verification

You run after a batch of work closes — typically an epic, sometimes a
chain of related tasks. Your job is to answer one question: **does the
thing the user asked for actually work, end to end?**

Per-task correctness is `hew-guard`'s domain. Verify operates one level
up: integration, regression, user-facing behavior. If you find gaps, you
create new tasks in the graph with proper deps and tell the user. You
don't fix them inline — fixing belongs to `hew-execute`.

## When this skill runs

- An epic's tasks are all closed and the epic itself is eligible for
  closure (`tasks.epics_eligible_for_closure > 0` in stats).
- The user asks "is it done?" / "did we finish?" / "verify."
- A milestone wraps and we're about to ship.

Don't run verify after every single task. It's batch-scope, not
per-task — running too often produces noise and burns context.

## Inputs from `hew prime verify`

- `tasks.done` history — the closed tasks since the last verification.
  Use `hew task list --status=closed --since=<last-verify-timestamp>` if needed.
- `memories.boundaries` — every public interface that must still work.
- `memories.security` — security baselines that must still hold.
- The epic's acceptance criteria (in the epic's own description) — the
  contract this verification is checking against.

You may also need to consult the conversation context from `hew-plan` if
the epic predates persistent memories on this project. Acceptance
criteria captured at plan time are what verify checks against.

## The five verification dimensions

### 1. Full test suite passes

Not the targeted slice that `hew-guard` runs — the whole thing.

```
pytest                     # or
npm test                   # or
cargo test                 # or
go test ./...
```

**Fail** on any test failure. Specifically:

- New failures introduced by this batch → regression, create a task.
- Pre-existing failures still failing → flag as deferred work.
- Tests pass but coverage dropped sharply (>5%) → flag as a quality
  concern (track if the project cares about coverage).

### 2. Acceptance criteria, one by one

For each criterion in the epic's description:

1. State the criterion verbatim.
2. State how to verify it (the exact command, URL, or check).
3. Run the verification.
4. Mark each as **met**, **partial**, or **missing**.

Example for an auth epic:

```
[met]     "Valid creds return 200 + access+refresh tokens"
            pytest tests/api/test_login.py::test_valid_creds → pass
[met]     "Invalid creds return 401 + AppError"
            pytest tests/api/test_login.py::test_invalid_creds → pass
[partial] "Refresh rotates on use; revoked tokens 401"
            rotation works (test_refresh_rotates pass), but revocation
            after reuse is not implemented → opens a task
[missing] "Logout invalidates session server-side"
            no /logout endpoint exists → opens a task
[met]     "Frontend login button works end-to-end"
            manual: http://localhost:3000/login → submit → redirect to /app
```

Partial and missing items each become a new task in the graph (see
below).

### 3. Boundary regressions

For every `BOUNDARY:` memory that names an interface this batch could
have touched:

- Probe the boundary directly. Hit the endpoint, call the function,
  import the module. Confirm signature + behavior unchanged.
- If the boundary's `BOUNDARY:` memory lists known downstream consumers
  ("4 dependents"), spot-check one or two.

**Fail** on any change to a documented boundary that wasn't intentional.
Surface to user: "Boundary X changed, was that intended?"

If the change was intentional and the boundary should be updated:

```
hew remember --type=boundary "POST /api/v1/users now expects {email, password, name, accept_tos}. Migration deadline: <date>. Old shape returns 400."
```

### 4. End-to-end golden path

Whatever the user described in plain English at plan time — run through
it manually or with an integration test:

- "User can sign up" → actually create a user via the UI / API.
- "Payment flow works" → drive a test card all the way through.
- "Dashboard loads" → open it; assert the right data appears.

If the project has Playwright / Cypress / equivalent: run the relevant
suite. If not, this is a checklist you walk manually (or ask the user
to walk and report back).

The full suite (dimension 1) often misses integration breaks because
units pass in isolation. The golden path is the last sanity check.

### 5. Maintainability

Walk the project's `CONVENTION:craft.*` set and confirm each picked
principle was honored across the batch that just closed. The hard
checks in `hew-guard` ran per-task; this dimension is the *batch-level*
re-read with the whole epic in view, where cross-task drift surfaces
that a per-task review can't see.

For each `CONVENTION:craft.<id>`:

- **SRP / SOLID** — read each new module's public surface; does it
  still have one reason to change after the full batch landed?
- **DRY** — diff the closed tasks' added code as one unit. If the
  same logic appears in two files because two different tasks both
  needed it, that's the kind of duplication per-task guard misses.
- **Small functions / Single Level of Abstraction** — any function
  that grew across multiple closing commits? Flag it.
- **Idempotence / Fail Fast / Pure Functions** — check the new
  boundaries (handlers, retries, computation cores).

Also surface any unresolved `hew-guard` craft soft-warnings from the
batch (the executor may have legitimately deferred them with a
`DECISION:craft-feature:<plan-id>` justification — note those as
*documented* deviations, not drift).

If you find drift, either:

1. Open a chore task to fix (preferred when the fix is small and the
   batch hasn't shipped).
2. Document the deviation as a `DECISION:` memory if the team
   consciously accepted it.

Don't silently let craft drift through verify — that's how a picked
principle quietly stops binding.

## Output

### Pass — every dimension green

```
VERIFY: pass

[1] Tests:  248 pass, 0 fail (coverage 84%, was 82%)
[2] Acceptance:  6/6 met
[3] Boundaries:  3 checked (POST /users, GET /users/{id}, POST /login) — unchanged
[4] Golden path:  signup → login → fetch /me → logout end-to-end OK
[5] Maintainability:  craft set honored
                      - CONVENTION:craft.solid: AuthService SRP intact
                      - CONVENTION:craft.dry: token encode/decode shared via _token_codec.py
                      - CONVENTION:craft.fail-fast: input validation pre-DB on all 3 endpoints
                      - 0 unresolved craft soft-warnings from this batch
```

Then:

```
hew epic close <epic-id> --reason "verified end-to-end. all 6 acceptance criteria met. 248 tests pass. boundaries unchanged."
hew remember --type=status "verify:<epic-id>:complete — <ISO-8601 timestamp>"
```

The epic closes; if this was the milestone's last epic, the milestone is
ready to ship.

### Fail — at least one dimension red

```
VERIFY: fail (2/5 dimensions)

[1] Tests:  248 pass, 0 fail ✓
[2] Acceptance:  4/6 met, 1 partial, 1 missing
    [partial] "Refresh rotates; revoked tokens 401"
              → opening bd-a3f8.6: implement refresh-token revocation
    [missing] "Logout invalidates session server-side"
              → opening bd-a3f8.7: implement POST /logout
[3] Boundaries:  POST /api/v1/users now returns 422 instead of 400 ✗
                 → opening bd-a3f8.8: restore 400 contract or update BOUNDARY memory
[4] Golden path:  fail — logout button on frontend does nothing
                  → covered by bd-a3f8.7
[5] Maintainability:  craft drift detected
    CONVENTION:craft.dry — token encode logic duplicated across login.py + refresh.py
                 → opening bd-a3f8.9: extract _token_codec.py helper
    CONVENTION:craft.small-functions — login_handler grew to 84 lines after rebase
                 → opening bd-a3f8.10: split validation out of login_handler

Epic stays open. Re-run hew-verify after the new tasks close.
```

Then actually create the tasks:

```
hew task new --parent=hew-a3f8 --type=task --priority=1 \
  --title="Implement refresh-token revocation on reuse" \
  --description="Verified missing in hew-verify. Per epic acceptance: revoked tokens must 401."

hew task new --parent=hew-a3f8 --type=task --priority=1 \
  --title="Implement POST /api/v1/auth/logout" \
  --description="Verified missing in hew-verify. Per epic acceptance: logout invalidates server-side session."

hew task new --parent=hew-a3f8 --type=bug --priority=0 \
  --title="POST /users contract regression: 422 vs 400" \
  --description="Verified in hew-verify boundary check. Decide: restore 400 or update BOUNDARY:users-create memory."
```

The epic now blocks on these new children. The executor picks them up
on the next `hew prime execute`.

## Decide whether to fix or surface

Some verify failures are obvious bugs (test failure, missing handler) →
open a task, the executor will fix.

Some are decisions:
- "Did we change this boundary on purpose?"
- "Is this missing feature in-scope or descoped?"
- "Is the regression actually a fix?"

For decisions, **surface to the user** with the specific question. Don't
auto-create tasks for things that might not be issues.

## What you don't do

- **Fix things.** Verify reports; execute fixes. Mixing them muddies the
  audit trail.
- **Re-run hew-guard.** Guard is per-task; verify is per-epic. They
  check different things.
- **Run on a single closed task.** Wait for the batch.
- **Approve a verify pass when boundaries silently changed.** Always
  surface unintended contract changes, even if tests still pass.
- **Skip dimensions because "it obviously works."** Run all four.
  Confidence comes from coverage, not vibes.

## What "good enough" looks like

A verify pass on a non-trivial epic typically takes 5–15 minutes of
agent work. If you finish in 30 seconds, you skipped something. If you
spend an hour, the epic was probably too big and should have been
multiple smaller epics.

Acceptable: tests run, every acceptance criterion individually checked,
boundaries probed, golden path walked.

## Anti-patterns

- **"Tests pass, ship it"** — tests passing is one of four dimensions.
  Boundary regressions slip through unit tests routinely.
- **Marking a partial as met** to close the epic faster. The task graph
  is forgiving — adding two more tasks costs nothing. Lying in the
  audit trail costs trust.
- **Verifying as you go, per-task.** Burn-out for the agent, noise for
  the user, and you'd just be re-running guard.
- **Skipping the golden path because "integration tests cover it."**
  Walk it once. Find what units missed.
- **Closing the epic with verify pending.** Verify gates the epic close,
  not the other way around.

## Hand-off

When verify passes and the epic closes:

1. Tell the user: "Epic <name> verified. <N> tests pass, <M>/<M>
   acceptance criteria met, boundaries unchanged."
2. If this completes the user's stated goal: ask whether to wrap the
   session (suggest `/hew:ship`).
3. If more epics remain: continue with `hew-execute` on the next ready
   batch.

If this was the final epic of a milestone: surface that, and recommend
the user run `/hew:epic summary` to generate the milestone wrap-up from
closed task descriptions + memories.
