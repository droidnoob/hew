<!-- hew:version=0.7.0 -->
---
name: hew-quick
category: optional
init: hew prime quick
---

# hew-quick — Fast Mode

For work that is small enough that going through `hew-plan` and
`hew-decompose` is more ceremony than it's worth. One task in, one
task out, one commit.

This is the escape hatch. Use it for bug fixes, tiny tweaks, one-line
config changes — anything where the plan fits in the task description
and decomposition would just produce one task anyway.

## When this skill runs

- The user types `/hew:quick <description>` or any synonym ("fix
  this," "just add X," "one-liner to do Y").
- The task is obviously single-shot: one file, one behavior, one
  test, ≤ 30 min of agent work.
- No architectural decisions needed.
- No new dependencies introduced.

If any of those don't hold, **escalate to `hew-plan`**. Quick mode is
not "skip the discipline" — it's "the discipline collapses to one
step because the work is genuinely small."

## What quick mode actually does

```
1. hew task new (one task, no epic)
2. hew task claim
3. do the work
4. invoke hew-guard
5. hew task close
6. git commit
```

Same as the regular loop, with steps 1 collapsed into the user's
prompt and steps 2–6 unchanged.

The skipped steps are:
- `hew-plan` — the user already stated the goal in plain English.
- `hew-decompose` — there's nothing to decompose; it's one task.
- The acceptance-criteria conversation — quick-mode tasks self-verify
  via tests + manual confirmation.

## What quick mode does NOT skip

- `hew-guard` — still runs before close. The seven checks are fast
  and the cost of skipping them on "small" work is exactly the kind
  of drift quick mode shouldn't cause.
- Tests — if the change has any behavior, a test covers it.
- Commit discipline — atomic commit with conventional message.
- `hew remember --type=gotcha "..."` for any gotcha you discovered.

## Craft minimum

Quick mode is the most common place craft discipline silently lapses
("it's small, I'll skip the test"). Two rules survive every quick
task:

1. **A test, unless the change is provably behavior-free.** Config
   tweaks, comment edits, type-only renames, and dead-code deletions
   are the only exemptions. If you exempt, name the reason in the
   close: `--reason "comment-only: clarified pagination boundary"`.
   `hew-guard`'s `missing-tests` warning fires if you skip a test on
   a behavior-changing file; `testing.require=true` promotes that to
   fail.
2. **Don't violate the project's existing `CONVENTION:craft.*` set.**
   You don't need to enumerate them — they're picked once at
   `hew-new-project` time and carry through every task. Just don't
   ship code that contradicts a principle the project actively chose
   (e.g., DRY-violating copy-paste in a project that picked
   `CONVENTION:craft.dry`).

That's the floor. The rest of quick mode is unchanged.

## Sizing rule

If, halfway through, the work turns out to be bigger than expected:

1. **Stop.** Don't keep going under quick-mode discipline.
2. **Sub-decompose inline**: create sub-tasks under the current task
   for the unexpected pieces, wire deps.
3. **Surface to the user**: "This is bigger than a quick — three
   subtasks now. Continue or pause to plan?"

Quick mode that turns into a saga without acknowledgment is how
quick-mode discipline rots.

## Example

User: "fix the off-by-one in the pagination cursor."

```
hew task new --type=bug --priority=1 \
  --title="Fix off-by-one in pagination cursor" \
  --description="Pagination skips the last item per page. See app/repos/users.py:list_users — cursor is exclusive but consumer expects inclusive. Fix + test."

hew task claim <id>

# read app/repos/users.py:list_users
# read tests/repos/test_users.py
# notice the bug; fix cursor handling
# add a regression test covering the edge

# invoke hew-guard → pass
hew task close <id> --reason "Cursor handling fixed; test_list_users_returns_last_item added; existing pagination tests still pass."

# commit
git commit -m "fix(repos): pagination cursor includes last item

- correct exclusive vs inclusive boundary in list_users
- regression test for the edge case
"
```

Done. No plan, no decompose. One file touched, one test added, one
commit. Total: ~10 minutes.

## When the user thinks it's quick but it isn't

The most common quick-mode failure is "this is a one-liner" turning
into "we need to change three modules and add a new abstraction." If
you notice any of these, push back:

- Multiple files need touching.
- A new dependency is involved.
- An interface in a `BOUNDARY:` memory changes.
- A new test fixture / harness piece is needed.
- The fix has subtle correctness implications (e.g., concurrency, money).

For each, say so and propose escalation to `hew-plan`. The user can
override (and you proceed under quick discipline anyway), but the
default is to escalate.

## What you don't do

- **Skip tests** because "it's tiny." Tiny changes break tiny things,
  and the test catches it next time you forget.
- **Skip `hew-guard`.** The whole point of guard is preventing the
  drift quick mode invites.
- **Open epics from quick mode.** If you need an epic, you needed
  `hew-plan`.
- **Persist the work without `hew task close`.** Even quick-mode tasks
  appear in `hew task show` history — that's the audit trail.

## Anti-patterns

- **Quick mode that touches > 3 files.** Stop, escalate.
- **Quick mode introducing a new dep.** Stop, run `hew-deps`.
- **Quick mode without a test.** If the change is truly behavior-free
  (config-only, comment-only, type-only), say so in the close
  reason. Otherwise: write a test.
- **Quick mode skipping the commit.** No commit = no audit trail.
- **Saga creep** — what started quick is now five files in.
  Surface to the user before it becomes a half-baked epic.
- **Skipping the test silently.** "Quick mode means no tests" is the
  myth that produces the worst regressions. Either write the test or
  spell out the behavior-free exemption in the close reason — silence
  isn't an exemption.
