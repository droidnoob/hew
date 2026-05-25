<!-- hew:version=0.7.1 -->
---
name: hew-blast
category: optional
init: hew blast --json
---

# hew-blast — Symbol-level diff over the current branch

You produce a symbol-level changelog of the current branch (or an
arbitrary set of files). Different from `git diff` — it answers
*which functions / classes / methods actually changed*, not *which
lines moved*. Backed by tree-sitter parsers for Rust, Python,
TypeScript, JavaScript, Go, and Java.

Use this when the work loop needs to know which named definitions a
branch touched — for scoped reviews, change summaries, or attaching a
semantic delta to a task close note.

## When this skill runs

- The user invokes `/hew:blast` (with or without arguments).
- `hew-review` calls it under the hood to scope its bundle to the
  changed symbols (after BL.4 lands).
- `hew-execute` optionally calls it on close to attach a symbol
  list to the task notes (after BL.3 lands, gated by
  `craft.symbol-trace`).

## Inputs from `hew blast --json`

```json
{
  "base": "main",
  "files": [
    {
      "path": "hew-core/src/treesitter/grammars.rs",
      "language": "Rust",
      "symbols": [
        { "name": "extract_symbols", "kind": "Method",
          "byte_range": {"start": 1700, "end": 4200},
          "line_range": {"start": 57, "end": 113} }
      ]
    }
  ]
}
```

## Three input modes

| Mode | Trigger | Behavior |
|------|---------|----------|
| **diff** (default) | `hew blast` | Walks `git diff <base>...HEAD`, intersects each touched file with extracted symbols. |
| **scoped diff** | `hew blast file1.rs file2.py` | Same as diff mode but restricted to the given files. |
| **no-diff** | `hew blast --no-diff <files>...` | Skip git entirely; emit every symbol from each file. Combines with `--stdin`. |

## What you DO

- When a user asks "what changed?" semantically (not line-wise), call
  `hew blast --json` and summarize the per-file symbol lists.
- When `hew-review` runs against a large diff, prefer the per-symbol
  source slices over re-reading whole files.
- When in doubt about whether a refactor touched a function's
  *behavior* (vs just moved it), check `hew blast` — if the symbol
  appears, the diff overlaps its body.

## What you DO NOT

- Treat the absence of a symbol from blast as proof nothing changed.
  Comments, imports, top-level statements, and macros may not surface
  in the V1 capture set.
- Use blast as a substitute for code review. It tells you *which*
  symbols moved, not *whether* the move was correct.

## Opt-in

Off by default. Enable with `hew config set optional-skills.blast true`
(or via `hew init` with `--features treesitter`). Requires the binary
to be built with `--features treesitter`; otherwise `hew blast` errors
out with a rebuild hint.

## Persistence

- No memories of its own. `hew-execute` and `hew-review` may persist
  symbol references as part of their own STATUS / GOTCHA writes.
