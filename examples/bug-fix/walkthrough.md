# Walkthrough — Bug Fix with `/hew:quick`

Scenario: a user reports a paginate-by-cursor bug. Single file, single
test, single commit. Plan + decompose would be overkill.

## The trigger

User says:

> Fix this: pagination skips the last item on every page. It's the
> notebooks list endpoint.

## /hew:quick takes over

The agent recognizes a single-shot scope:

- One bug, one symptom.
- Suspected location identifiable from the description.
- No new dependencies.
- No architectural change.

It invokes `hew-quick`:

```
$ hew prime quick
# (compact JSON; agent reads it)

# pick the one source likely involved
bd create --type=bug --priority=1 \
  --title="Fix off-by-one in notebooks pagination cursor" \
  --description="
  User report: /api/v1/notebooks skips the last item per page.
  Suspected: app/repos/notebook_repo.py:list — cursor handled as exclusive
  but consumer expects inclusive.
  Files: app/repos/notebook_repo.py, tests/repos/test_notebook_repo.py.
  " \
  --acceptance="pytest tests/repos/test_notebook_repo.py::test_list_includes_last_item passes; existing pagination tests still pass."
# → bd-fix-N

bd update bd-fix-N --claim
```

## The work

The agent reads `app/repos/notebook_repo.py:list`. It sees:

```python
def list(self, cursor: Optional[str], limit: int) -> Page[Notebook]:
    q = self.session.query(Notebook).filter_by(is_deleted=False)
    if cursor:
        q = q.filter(Notebook.id > cursor)   # <-- BUG: strict >, should be >=
    rows = q.order_by(Notebook.id).limit(limit + 1).all()
    ...
```

Wait — the bug is more subtle. The cursor refers to the *last seen id*,
so `>` is actually correct for the "next page" semantics. The actual
bug: `limit(limit + 1)` is fetching `limit + 1` rows for has-next
detection but slicing wrong:

```python
    has_next = len(rows) > limit
    rows = rows[:limit]      # <-- always slices, even when len(rows) == limit
    return Page(results=rows, next_cursor=rows[-1].id if has_next else None)
```

That's wrong: when `len(rows) == limit` (full page, no overflow),
`has_next` is false and the slice keeps all `limit` rows — but
`rows[-1].id` is the last item *we just returned*. Setting
`next_cursor` to that would be correct for the next-page request.
When `len(rows) > limit`, slice drops the overflow row and uses the
last-included as cursor — also correct.

The actual bug: the test that exposed the user's report uses cursor
equal to the *previous-page's `next_cursor`*. The repo handles the
boundary as exclusive (`>`), which is correct under the
"cursor = id of the last item returned" convention. But the frontend
was treating `next_cursor` as inclusive ("next item starts at this
id"), so the frontend skipped one item.

Two ways to fix:

1. **Change the API contract** — but that's a BOUNDARY change. `hew-quick`
   detects this and *escalates*:

   ```
   This isn't a quick fix — the bug is a boundary mismatch between
   backend (exclusive cursor) and frontend (inclusive cursor). Either
   change is breaking. Recommend /hew:plan to choose the canonical
   semantics + migrate the other side.
   ```

User asks the agent to proceed under `quick` because both sides are
shipping at the same time. The agent flags it in the close-reason
and proceeds: change the repo to use `>=` and exclude the
last-item-on-previous-page differently.

## Test first

```python
def test_list_paginates_inclusive_cursor(repo, factory):
    items = [factory(Notebook) for _ in range(5)]
    page1 = repo.list(cursor=None, limit=2)
    assert [n.id for n in page1.results] == [items[0].id, items[1].id]
    page2 = repo.list(cursor=page1.next_cursor, limit=2)
    assert [n.id for n in page2.results] == [items[2].id, items[3].id]
    # The bug: items[2] was being skipped.
```

Test fails (red). The agent fixes:

```python
    if cursor:
        q = q.filter(Notebook.id >= cursor)
    # ...
    if has_next:
        rows = rows[:limit]
        next_cursor = rows[-1].id
    else:
        next_cursor = None
    # next page request must EXCLUDE the cursor since we returned it this page
    # — change to: store next_cursor = rows[-1].id + 1 conceptually
```

Wait, that's wrong too. The agent realizes the cleanest fix is to set
`next_cursor = (last_included_id + 1)` and keep the `>=` filter, so the
frontend semantics ("next page starts at next_cursor") hold.

Refactor + test passes (green).

## Guard

```
GUARD: pass (7/7)
- no debug statements
- no secrets
- no stray TODOs
- ruff clean
- mypy clean
- pytest tests/repos/test_notebook_repo.py -x → 7 passed
- conventions honored: CONVENTION:db (repository), CONVENTION:tests (one file per source)
```

## Close + commit

```
bd close bd-fix-N --reason "Pagination cursor now inclusive (next_cursor = last_returned_id + 1; filter uses >=). 1 new test (test_list_paginates_inclusive_cursor) covers the bug. Existing 4 pagination tests still pass. Flagged: this changed the cursor semantics — frontend uses the new inclusive convention; documented BOUNDARY update."

bd remember "BOUNDARY: GET /api/v1/notebooks cursor semantics — next_cursor is INCLUSIVE (next request returns items where id >= cursor). Frontend updated 2026-05-12."

git commit -m "fix(repos): notebooks pagination skipped the boundary item

- switched cursor filter to >= and next_cursor to last_id + 1
- new test_list_paginates_inclusive_cursor regression test
- updated BOUNDARY memory; frontend already aligned
"
```

## Total time

~12 minutes of agent work. Single commit. Single task in Beads. No
plan, no decompose. The one place quick-mode *almost* escalated was
caught and surfaced cleanly to the user before the agent silently
made a boundary-breaking change.
