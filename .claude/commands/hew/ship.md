---
description: Create a PR and (optionally) a gate for the next epic.
---

After the current epic verifies clean:
1. Push the branch (if not already on main).
2. Open a PR with a summary auto-generated from closed task descriptions.
3. Optionally create an external-state gate that blocks the next epic on
   this PR's merge: `hew gate new --gh-pr=N --title="PR #N merged"`
   (returns a gate task id). Backed by `gh pr view N --json state,mergedAt`;
   the gate resolves when GitHub reports `state=MERGED`.
4. Wire the gate as a dependency of the next epic via
   `hew dep add <next-epic> <gate-id>` so the loop blocks on merge.
5. Run `hew gate poll` periodically (or wire into a loop hook) to flip
   resolved gates from open → done and unblock downstream work.
