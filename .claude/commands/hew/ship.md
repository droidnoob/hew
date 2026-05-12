---
description: Create a PR and (optionally) a gate for the next epic.
---

After the current epic verifies clean:
1. Push the branch (if not already on main).
2. Open a PR with a summary auto-generated from closed task descriptions.
3. Optionally create a Beads gate (gates aren't wrapped by `hew task new`; use bd directly): `bd create --type=gate --title="PR #N merged" --await-type=gh:pr --await-id=N`
4. Add the gate as a dependency of the next epic via `hew dep add <next-epic> --on <gate-id>` so the agent waits for merge.
