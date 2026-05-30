# Hew configuration

Hew settings live in two TOML files, layered at load time. This
document is the operator-facing contract for that layering.

> **TL;DR:** personal preferences → `~/.config/hew/config.toml`
> (managed via `hew config set --global …`); team-shared settings →
> `<repo>/.hew.toml` (managed via `hew config set --project …`).
> `hew config show` prints the merged view with per-key attribution.

## Two files, one schema

| File | Scope | Tracked in git? | Written by |
|------|-------|-----------------|------------|
| `~/.config/hew/config.toml` | user-global | no | `hew config set --global` |
| `<repo>/.hew.toml` | project-local | **yes** | `hew config set --project`, `hew init` |
| `<repo>/hew.toml` | project-local (legacy / no-dotfile) | yes | rename to `.hew.toml` |

Both files share the same TOML schema (see `hew config list` for the
full key list, or [`hew_core::config::Config`](../hew-core/src/config.rs)
for the source of truth). Write project configs **sparsely** — fields
that aren't set inherit from user-global or the built-in default.

The user path follows the XDG base directory convention; on macOS
`etcetera::choose_base_strategy` resolves the platform-native location
(typically still `~/.config/hew/`). Override via `HEW_CONFIG` (single
file, bypasses layering) or `HEW_USER_CONFIG` (overrides the user path
but keeps layered project discovery).

## Discovery order

Highest precedence wins:

1. **Environment** — `HEW_CONFIG=<path>` short-circuits layering and
   treats the named file as the sole source.
2. **Project file** — closest ancestor of the current directory
   containing `.beads/` or `.git`. Inside that root, `.hew.toml` wins
   over `hew.toml`; both present logs a warning.
3. **User-global file** — `HEW_USER_CONFIG` if set, else the
   XDG-resolved path.
4. **Defaults** — hardcoded in `hew_core::config`.

Project discovery is **root-only** in v1 — hew anchors on the first
`.beads/` or `.git` ancestor, then looks for `.hew.toml` /
`hew.toml` only at that root. No ancestor walk. For git worktrees the
main repo's working tree is resolved via `git rev-parse
--git-common-dir`.

## Where each setting belongs

This is opinionated — the line is fuzzy, so use judgement.

| Setting | Belongs in | Why |
|---------|------------|-----|
| `loop.model.default`, `loop.model.by_priority`, `loop.model.by_type` | project | team picks the model per workload |
| `loop.fallback_runtime`, `loop.fallback_cooldown_iters` | project | shared runtime fallback policy |
| `loop.planner.enabled`, `loop.planner.budget_tokens`, `loop.planner.runtime` | project | planner tuning is workload-specific |
| `loop.end_of_run.verify_tests`, `loop.end_of_run.verify_command` | project | the project decides what "green" means |
| `branching.strategy`, `testing.require` | project | crew agrees on the workflow |
| `compact.exempt`, `compact.target_clusters_cap` | project | memory hygiene rules per project |
| `update-check`, `default-runtime`, `default-scope` | user-global | per-person preference |
| `optional-skills.*` | user-global | per-person skill picker prefs |
| `craft.symbol_trace`, `craft.max_function_lines` | project | team-shared coding-craft thresholds |
| `review.after_n_tasks`, `review.after_epic`, `review.batch_size` | project | per-project review cadence |

## `hew config set` write rules

When you run `hew config set <key> <value>`, the target file is
resolved as follows:

| `--global` | `--project` | project file present? | result |
|------------|-------------|-----------------------|--------|
| ❌ | ❌ | no | write to user-global (back-compat) |
| ❌ | ❌ | yes | **refuse** with the dual-flag error |
| ✅ | ❌ | either | write to user-global |
| ❌ | ✅ | yes | write to existing project file |
| ❌ | ✅ | no | create `.hew.toml` at project root with starter header + write |
| ✅ | ✅ | either | clap rejects (mutually exclusive) |

The refusal error format when neither flag is passed and a project file
exists:

```text
refusing to write to user-global config when `.hew.toml` exists at /Users/me/repo
       team-shared config lives in `.hew.toml`. Use one of:
         hew config set --project loop.model.default opus-4-7   # commit-shared
         hew config set --global  loop.model.default opus-4-7   # personal override
```

This is the loudest option from the design picker — it forces the
operator to make an explicit per-write choice instead of silently
mis-targeting team-shared config.

## `hew config show`

Inspect the merged effective config with per-key source attribution:

```text
$ hew config show
sources (in precedence order):
  [user-global] /Users/me/.config/hew/config.toml
  [project] /Users/me/code/myproj/.hew.toml

effective config:
  update-check                = true                         (default)
  default-runtime             = claude                       (user-global)
  loop.fallback_runtime       = codex                        (project)
  compact.exempt              = STATUS:user-a,STATUS:proj-b  (merged)
  ...
```

Sources are listed in precedence order (user-global → project → env).
Each key shows its winning value and the source that contributed it:

- `(default)` — neither file set the key
- `(user-global)` — only user-global set it
- `(project)` — project file set it (overriding user-global on collision)
- `(merged)` — both files contributed to an array / map (concatenated)
- `(env)` — `HEW_CONFIG` is active; this file is the sole source

`hew config show --json` emits a stable shape:

```json
{
  "sources": [
    { "label": "user-global", "path": "/.../config.toml" },
    { "label": "project", "path": "/.../.hew.toml" }
  ],
  "keys": {
    "default-runtime": { "value": "claude", "source": "user-global" },
    "loop.fallback_runtime": { "value": "codex", "source": "project" }
  }
}
```

## Merge semantics

When both files set the same key:

- **Scalars** (strings, bools, ints) — project wins.
- **`Option<T>`** — user value survives if project omits the key (the
  project-side `None` falls back via `Option::or`).
- **Arrays** (`compact.exempt`) — user entries first, then new project
  entries; duplicates dropped.
- **Maps** (`loop.model.by_priority`, `loop.model.by_type`) — extend;
  project wins on key collision.
- **Tables** — recurse field-by-field.

Worked example. User-global:

```toml
default_runtime = "codex"

[compact]
exempt = ["STATUS:user-a", "STATUS:shared"]

[loop.model.by_priority]
P0 = "opus-user"
P3 = "haiku-user"
```

Project `.hew.toml`:

```toml
[compact]
exempt = ["STATUS:shared", "STATUS:project-b"]

[loop.model.by_priority]
P0 = "opus-project"
P1 = "sonnet-project"
```

Effective config:

- `default_runtime = "codex"` — only user set it.
- `compact.exempt = ["STATUS:user-a", "STATUS:shared", "STATUS:project-b"]`
  — concat + dedupe.
- `loop.model.by_priority.P0 = "opus-project"` — project overrode.
- `loop.model.by_priority.P1 = "sonnet-project"` — project-only entry.
- `loop.model.by_priority.P3 = "haiku-user"` — user-only entry survived.

## Migration: moving settings from user-global to project

To promote a personal setting to a team-shared one without re-typing
the value:

```sh
# 1. Read the existing user-global value.
hew config get loop.model.default
# → claude-opus-4-7

# 2. Write it to the project file.
hew config set --project loop.model.default claude-opus-4-7

# 3. (Optional) clear the user-global override so it stays sparse.
hew config set --global loop.model.default ""
```

The project file gets the value committed to git; everyone on the team
inherits it on next pull.

## Future

These are explicitly **not** in v1; called out so expectations stay
calibrated.

- **`.hew.local.toml`** — gitignored sibling for per-developer
  project-scoped overrides. Will sit between project and user-global
  in precedence.
- **Ancestor walk** — today project discovery anchors on `.beads/` /
  `.git` only at the first match. A multi-repo monorepo with a parent
  `.hew.toml` won't be discovered.
- **Schema migration** — `version = 1` in the project file is a
  forward-compat marker; a future schema bump will read it.
- **Non-TOML formats** — out of scope.

## Reference

- Source: [`hew-core/src/config.rs`](../hew-core/src/config.rs)
- Key list: `hew config list`
- Set path: `hew config path`
- Effective view: `hew config show`
- Agent guidance: [`CLAUDE.md`](../CLAUDE.md) §Locked behavioral preferences
