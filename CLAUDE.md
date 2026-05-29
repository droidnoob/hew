# Hew — agent guidance

Project-specific instructions for AI coding agents (Claude Code, Cursor, Codex, Windsurf) working in this repo. Read this **before** making changes.

For the public README aimed at humans, see [README.md](./README.md). For the methodology itself, see [`skills/`](./skills) and [ARCHITECTURE.md](./ARCHITECTURE.md).

---

## What this project is

Hew is a methodology + Rust CLI for AI coding agents. The CLI is a Cargo workspace:

```
hew-core/   pure logic, no clap/inquire (lib)
hew/        thin binary — arg parsing + presentation (bin)
skills/     20 skill markdown files + SKILL.md index (the methodology)
commands/   39 slash command files for /hew:* invocations
.claude/    self-installed agent runtime artifacts (dogfooded in-tree)
.beads/     this project's own Beads graph (state for hew-on-hew work)
```

The repo dogfoods itself: when you run `hew init --runtime=claude` here, it writes the skill bodies into `.claude/skills/hew/`, the slashes into `.claude/commands/hew/`, and a `SessionStart` hook into `.claude/settings.json`. All of that is tracked in git.

---

## Branching contract

`main` is protected on both ends.

**Local pre-commit hook** refuses commits while HEAD is on `main` / `master`. Always create a feature branch:

```sh
hew branch new --prefix=<feat|fix|chore|docs|refactor|perf|test|style> \
               --slug="short-kebab-case-summary"
# → <prefix>/<slug>
```

**GitHub branch protection** refuses direct pushes to `main`. Changes land via PR. CI must pass (rustfmt + clippy + 4-way test matrix + cargo-audit + cargo-deny) before merge. (Currently configured but not yet active server-side — see [`.github/protection/README.md`](./.github/protection/README.md).)

Override the local hook only for legitimate edge cases:

```sh
HEW_ALLOW_MAIN_COMMIT=1 git commit ...
```

---

## Build / test / lint

```sh
cargo build                                 # ~30s cold, ~1s incremental
cargo test                                  # ~2s after warm cache
cargo clippy --all-targets -- -D warnings   # strict
cargo fmt --all                             # rustfmt; --check in CI
```

The pre-commit hook runs `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, and `cargo test --quiet` on every commit touching Rust files. Skip the test step with `HEW_HOOK_NO_TESTS=1`; bypass everything with `HEW_SKIP_HOOKS=1` (use sparingly).

Install hooks once per clone:

```sh
git config core.hooksPath .githooks
```

---

## How to work in this repo

### Use the methodology on itself

This project uses its own loop:

```sh
hew status                # see ready tasks + STATUS flags + memory counts
hew prime <skill>         # load context for a specific skill
hew task list             # task graph
hew memories --prefix=X   # filter memories by prefix
```

If you're picking up where someone left off, start with `hew prime resume` (which the Claude SessionStart hook runs automatically). Latest `CHECKPOINT:` memory tells you the in-flight state.

### Memory prefixes that matter here

| Prefix | What it means in this repo |
|--------|----------------------------|
| `CONVENTION:` | Hard rule. Do not violate without an explicit DECISION updating it. |
| `CONVENTION:craft.<id>` | Adaptive craft principle this project picked. hew-guard soft-warns. |
| `DECISION:` | Locked architectural choice. Cite when relevant. |
| `GOTCHA:` | Hard-won discovery — read these before debugging anything weird. |
| `FEEDBACK:` | User-stated preference. Honor every time. |
| `STATUS:` | Phase markers (scan / convention / plan / decompose / verify / review / compact). Never compact-forget these. |
| `RESEARCH:` | Cited findings with `[VERIFIED]` / `[CITED]` / `[ASSUMED]` provenance. |

### Add a new skill or slash command

1. Drop the markdown in `skills/<category>/<name>.md` or `commands/<name>.md`. Use an existing file as template — they all start with `<!-- hew:version=X.Y.Z -->` and a frontmatter block.
2. Register in `hew_core::skills::CORE/BROWNFIELD/OPTIONAL` or `hew_core::slash::ALL`.
3. **Bump three hardcoded test counts** (see `GOTCHA:test-counts-drift` below). The drift tests will fail otherwise.

### Running the loop

For long autonomous runs, drive `hew loop` instead of staying in one chat session:

```sh
hew loop run --until-empty            # drain the ready queue
hew loop run --max-iter 5 --strict    # bounded run, craft = fail
hew loop run --budget-tokens 200000   # cap cumulative tokens
hew loop run --dry-run --max-iter 1   # prompt-assemble smoke (no spawn)
hew loop run --runtime=codex          # drive codex-cli instead of claude
hew loop run --fallback-runtime=codex # primary=claude, swap to codex on RuntimeError

hew loop list                         # recent runs + state
hew loop logs --tail 5                # last 5 iters of latest run
hew loop cancel                       # touch stop-file on latest run
```

Each iter is a fresh `claude -p` (or `codex exec`) subprocess; the skill body + memory primer prefix is byte-stable across iters so the prompt cache hits (check `prompt_prefix_hash` in `.hew/loop/<run-id>/iter-NNN.json`). `--fallback-runtime` is primary-sticky with a configurable cooldown (`--fallback-cooldown-iters`, default 3) — see [docs/LOOP.md](./docs/LOOP.md#fallback-runtime) for the worked example.

`/hew:auto` now points at `hew loop run --until-empty`. The in-conversation walk is still reachable via `/hew:work`. Full guide: [docs/LOOP.md](./docs/LOOP.md).

### Statusline

`hew statusline` emits a one-line agent statusline (scope label, progress bar, percent, phase, epic-fraction, optional user). It's auto-wired into Claude Code by `hew init --runtime=claude`, which upserts a top-level `statusLine` block into `.claude/settings.json` with the same `hew_managed: true` discriminator the SessionStart hook uses — re-install is idempotent, uninstall is symmetric, and a user can opt out by removing the flag.

Formats:

- `hew statusline --compact` — `<label> <bar> <pct>%`
- `hew statusline` — adds phase + epic-fraction (default)
- `hew statusline --full` — adds `<user> N/M`

Width is clamp `[1, 80]`; stdin from Claude Code is drained tolerantly; bd-not-initialized exits 0 with empty stdout so the host falls back gracefully. The pure render lives in `hew_core::statusline`; the CLI side-effects in `hew/src/commands/statusline.rs`.

Other runtimes (Cursor / Codex / Windsurf) wire the command through their own statusline configs — point them at `hew statusline` directly.

Existing Claude installs that predate the statusLine block self-heal on the next session: `hew prime resume` (the SessionStart hook) calls `install::auto_migrate_claude_statusline`, which adds the block iff the file shows a hew-managed SessionStart hook and no `statusLine` key yet. Silent / best-effort by design — the hook must never break because of a self-heal misfire.

---

## Hard-won gotchas

The single most useful thing to read before debugging. These are stored as `GOTCHA:` memories — run `hew memories --prefix=GOTCHA` for the live list. Highlights:

**`GOTCHA:test-counts-drift`** — adding a new skill or slash bumps **three** counts. Forget one and `cargo test` fails:

- `hew-core/src/skills.rs::tests::ship_one_index_plus_N_skills` (1 + N)
- `hew-core/src/install.rs::tests::install_claude_writes_every_skill_and_slash_command` (1 SKILL + N skills + M slashes + 1 settings.json)
- `hew-core/src/install.rs::tests::install_codex_writes_per_skill_toml_and_agents_md` (N skills + 1 AGENTS.md)

**`GOTCHA:pipe-deadlock`** — large `bd` JSON output deadlocks on the OS pipe buffer (~16KB on macOS, ~64KB on Linux) if you read after `wait_timeout`. Use `BdClient::run_to_file` (the `Stdio::from(File)` path) for any command that could exceed those. Helper: `RealBd::read_via_temp(args, label, ext)`.

**`GOTCHA:zsh-cmd-substitution`** — multi-line `hew task new --description "$(cat <<EOF ... EOF)"` hits zsh parse errors on embedded apostrophes/quotes. Use `bd q` + `bd update --body-file <path>` from a temp file instead.

**`GOTCHA:clippy-derivable-impls`** — clippy fires on hand-written `Default { field: false }`. Use `#[derive(Default)]` when the impl is trivial.

**`GOTCHA:clippy-field-reassign`** — `let mut x = Default::default(); x.foo = ...` is rejected. Use `Struct { foo: ..., ..Default::default() }`.

**`GOTCHA:bd-mol-bond`** — `bd mol bond` is documented broken. Don't wrap it in hew; use task-level deps via `hew dep add <child> --on <prerequisite>` instead.

**`GOTCHA:flaky-pre-commit`** — rarely the pre-commit hook reports `1 failed` then a re-run is clean with no reproducer. If it happens once, retry. If it happens twice, investigate.

---

## Locked behavioral preferences

These are `FEEDBACK:` memories. Honor every time.

**`FEEDBACK:no-json-piping`** — never pipe `hew ... --json` through `python` / `jq` / `sed`. Use the text-default surfaces: `hew memories --prefix`, `hew task show`, `hew status`, `hew compact list-prefixes`, etc. The text shape is the agent-facing contract.

**`FEEDBACK:prefer-hew-over-bd`** — use the curated `hew task` / `hew dep` / `hew epic` / `hew remember` / `hew memories` wrappers instead of raw `bd`. Documented hold-outs (where raw `bd` is still the path): gates, `--unclaim`, `--acceptance`, history, orphans, lint. Skill bodies follow this contract; don't regress.

**`CONVENTION:commit-messages`** — never reference "GSD" by name in commit messages or commit bodies. Compare to internal alternatives, prior art, or just describe the goal. Hew is positioned in the same space, but the messaging stays self-contained.

---

## Locked decisions worth knowing

These are `DECISION:` memories — the architectural choices this project has committed to. Run `hew memories --prefix=DECISION` for the live list. Most-load-bearing ones:

- **`DECISION:craft-enforcement`** — craft soft-warnings never block close. `testing.require=true` is the one current promotion path from warn to fail.
- **`DECISION:craft-adaptive`** — craft principles are picked per project, not universal. The catalog (`skills/data/craft-principles.toml`, 28 entries) is exposed; projects pick a subset that persists as `CONVENTION:craft.<id>`.
- **`DECISION:compact-safety`** — memory compaction writes additions FIRST, then forgets sources. A mid-apply crash leaves more memory, not less. `STATUS:scan/convention/plan/decompose` are hardcoded exempt.
- **`DECISION:review-filing`** — review findings go to bd as `bug` / `chore` tasks with `[Review][BLOCKER|WARNING|INFO]` titles. No `REVIEW:` / `RISK:` memory pollution. Only memory write is the `STATUS:review:<ts>` marker.
- **`DECISION:hew-remember-type-allowlist`** — `hew remember --type=<x>` validates against a 13-prefix allowlist. The 5 non-allowlisted prefixes (CHECKPOINT, SPEC, MIGRATION, DEBUG, NOTE) require `--raw`.

---

## Releases

Tagged via `git tag vX.Y.Z` on `main`. cargo-dist builds the binaries + installers and creates the GitHub release.

Before a release:

1. Bump workspace version in root `Cargo.toml`.
2. Bump every skill body's `<!-- hew:version=X.Y.Z -->` marker (the `version_markers_match_pkg_version` test enforces this — `find skills commands -name "*.md" -exec sed -i.bak ...` then delete the `.bak` files).
3. Refresh `.claude/` with `hew init --runtime=claude` so the install artifacts match the new version.
4. Update `CHANGELOG.md` — move `[Unreleased]` content into a new dated `[X.Y.Z]` section.
5. Commit on a feature branch, PR + merge, then tag `main` and push.

---

## Where to look next

| If you need... | Read... |
|----------------|---------|
| Public-facing project overview | [README.md](./README.md) |
| Workspace + module architecture | [ARCHITECTURE.md](./ARCHITECTURE.md) |
| Slash command reference (all 39) | [docs/COMMANDS.md](./docs/COMMANDS.md) |
| Dev setup, MSRV, hooks, release process | [CONTRIBUTING.md](./CONTRIBUTING.md) |
| Release notes | [CHANGELOG.md](./CHANGELOG.md) |
| The methodology bodies the LLM loads | [`skills/`](./skills) |
| The slash command source files | [`commands/`](./commands) |
| Examples of the loop in action | [`examples/`](./examples) |
| Branch-protection config (deferred apply) | [.github/protection/README.md](./.github/protection/README.md) |

When in doubt: read the skill body for whatever loop step you're in (`skills/core/hew-execute.md`, `skills/core/hew-guard.md`, etc.). The methodology is self-documenting.
