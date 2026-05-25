---
description: Symbol-level changelog of the current branch (or arbitrary files). Wraps `hew blast`.
---

Invoke the hew-blast skill / surface. Walks the current branch's diff
against `main` (or a `--base <ref>` you pass) and prints, per file,
the symbols whose definitions overlap a hunk. Different from
`git diff` — answers *which functions / classes actually changed*, not
*which lines moved*.

Three input modes:

- No args → scan everything in `git diff <base>...HEAD`.
- Positional files → intersect with the diff set, only report those.
- `--no-diff <files>...` → skip git; print every symbol in each file
  (equivalent to a quick "tell me the shape of this file"). Combines
  with `--stdin` to pipe in `git ls-files | /hew:blast --no-diff --stdin`.

Other flags: `--path <substr>` (repeatable substring filter applied
after file resolution), `--base <ref>`, `--json`.

Requires the binary to be built with `--features treesitter`; otherwise
the call errors out with a rebuild hint.

ARGUMENTS: $ARGUMENTS
