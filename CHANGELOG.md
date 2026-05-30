# Changelog

All notable changes to this project are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.12.0] — 2026-05-31

### Added

- **Project-local config file `.hew.toml` (`hew-c0pa`).** Hew settings
  now layer across user-global (`~/.config/hew/config.toml`) and
  project-local (`<repo>/.hew.toml`, `hew.toml` legacy fallback). `hew
  init` emits a starter `.hew.toml` with header + `version = 1`. `hew
  config set` takes `--global` / `--project` flags (mutually exclusive)
  to pick the target; refuses to silently write user-global when a
  project file exists. `hew config show` renders the merged effective
  config with `(user-global)` / `(project)` / `(merged)` / `(env)` /
  `(default)` attribution per key, in text and `--json`. Merge rules:
  scalars project-wins, `Option<T>` falls back via `or`, arrays
  concat+dedupe, maps extend, tables recurse. Discovery anchors on the
  first `.beads/` / `.git` ancestor (root-only; no ancestor walk in
  v1). New `HEW_USER_CONFIG` env var overrides the XDG user path
  without bypassing layering (`HEW_CONFIG` retains single-file bypass
  semantics). See [`docs/CONFIG.md`](docs/CONFIG.md).

### Fixed

- **`hew loop run --jobs >= 2` now actually runs N workers (`hew-zt4z`).**
  The parallel dispatcher claimed N tasks at slot-fill time, but the
  per-worker loop then re-queried `bd.ready()` independently — and saw
  an empty list (since the dispatcher's own claim removed those tasks
  from ready). Result: 0 iters per worker, tasks stranded in
  `in_progress`. Fix threads the assigned `ReadyTask` into the worker
  via `Worker.assigned_task`; the worker prepends it to its
  `bd.ready()` poll for the first iter, then falls back to the normal
  query for iter 2+. Verified end-to-end: 2 workers, real `claude -p`
  spawns, real per-worker worktrees at `~/.hew/wt/<run-id>/{0,1}/`,
  real merge-back, both worker branches landed cleanly.
- **`hew loop run --jobs=1 --scope=epics` now honors the scope filter
  (`hew-s9mb`).** The serial path's `bd.ready()` poll skipped the
  scope-descendant check that the parallel `Dispatcher::dispatch_tick`
  enforces. Result: agent-explicit `--scope=epics --epics=<id>` could
  silently consume any bd-ready task. Fix re-resolves the descendant
  set at the worker's bd.ready() poll and filters every iter, matching
  dispatcher semantics. Verified: with P1 standalones at the top of
  `bd ready` and a P2 in the scoped epic, the loop now picks the P2.
- **`hew loop summary` renders an in-flight view instead of erroring
  with `No such file` (`hew-cn2y`).** The command crashed with raw
  `× read run.json: No such file or directory` when called against a
  run whose first iter hadn't completed yet. Fix classifies four
  cases: no run-dir → today's "not found" error; run-dir with
  manifest.json but no run.json → render parallel in-flight view from
  manifest worker states; run-dir with iter-*.json but no run.json →
  render serial in-flight view from latest iter; empty run-dir →
  minimal "iter 0 in flight" view with elapsed time. Includes a
  `note:` line pointing at re-running summary after iter 1 ends.

### Changed

- **`/hew:auto` slash description** corrected from the legacy
  "Run plan → decompose → execute → verify autonomously" to the
  in-conversation epic walk it actually does (per the rewrite in
  `hew-6n0v` / 0.11.0).

## [0.11.0] — 2026-05-30

### Added

- **`hew loop graph` DAG renderer (`hew-m7lq`).** Renders the loop's
  iter + batch + run + manifest history as a directed graph in
  mermaid (default), GraphViz `dot`, or terminal ASCII. Each iter is
  a node labelled with task id, outcome glyph, duration, and tokens;
  edges distinguish sequential next-iter, agent-suggested,
  planner-suggested, fallback (`bd ready`), and backpressure
  rollbacks. Unhappy paths render distinctly: incomplete iters get a
  dashed border (`⋯`), cancelled-mid-run iters get `⊘` with the stop
  timestamp, runtime errors with empty stderr annotate as `possibly
  hung`, backpressure failures draw a `↺ rolled back` self-edge, and
  verify outcomes get a coloured tail node (`Verify ✓` / `Verify ✗` /
  `Verify (skipped)`). Parallel runs (`--jobs >= 2`) render
  per-worker swimlanes from `manifest.json`. CLI:
  `hew loop graph [--run-id ID] [--format mermaid|dot|ascii]
  [--out PATH] [--all]`; `--out` ending in `.md` wraps the mermaid
  body in a fenced \`\`\`mermaid block. `--all` aggregates every run
  under `.hew/loop/` into one document with each as its own subgraph.
  Pre-batch-plan legacy runs render with sequential edges only. See
  `docs/LOOP.md` § Loop graph. Closes epic `hew-lf40`.
- **End-of-run verify step for `hew loop` (`hew-bon7`).** Opt-in
  mandatory test step that runs after the last iter (and after
  merge-back on `--jobs N >= 2`) to prove the final stacked state is
  green. Conditional on both a resolvable test command (CLI
  `--verify-command` > `loop.end_of_run.verify_command` > project-
  authored signals via `hew_core::gate::detect`) and an explicit
  opt-in (`--verify-tests` or `loop.end_of_run.verify_tests = true`).
  Outcome (`Passed` / `Failed` / `Skipped` / `TimedOut`) persists as
  `Run.verify_outcome` in `run.json`, shows up as a `verify:` line in
  `hew loop summary`, and on failure files a
  `STATUS:loop-verify-failed:<run-id>` memory + exits non-zero so CI
  branches on it. Closed tasks are **not** rolled back on failure —
  the memory + summary line + exit code are the durable signals.
  Defaults are byte-identical to today (`verify_tests = false`). CLI:
  `--verify-tests`, `--no-verify-tests`, `--verify-command=...`.
  Config: `[loop.end_of_run] verify_tests`, `verify_command`,
  `verify_budget_wall` (default `"10m"`). See `docs/LOOP.md` §
  End-of-run verification.
- **Batch planner for `hew loop run --jobs N` (epic `hew-lf40`).**
  Parallel runs now layer two informed signals on top of `bd ready` to
  choose each iter's dispatch batch: (1) a `next_iteration:` block in
  the iter agent's close output (cheapest, in-band), and (2) a
  dedicated planner subprocess spawned between iters when (1) is
  absent — capped by `loop.planner.budget_tokens` (default `10_000`)
  and skipped rather than truncated when over budget. `bd ready`
  remains the safety floor: agent / planner suggestions can only
  narrow the candidate set, never expand it
  (`DECISION:loop-batch-planner-floor`). Each iter persists a
  `batch-NNN.json` artifact (`schema_version: 1`) under the run dir; a
  future `hew loop graph` (`hew-m7lq`) replays them. The end-of-run
  summary gains a single-line `planner: agent=N, runtime=M,
  fallback=K` row right after `scope:` (omitted entirely for legacy /
  serial runs). CLI: `--no-planner`, `--planner-budget`. Config:
  `[loop.planner] enabled = true`, `budget_tokens = 10_000`. v1 only
  triggers under `--jobs >= 2`; `--jobs=1` skips the layer. See
  `docs/LOOP.md` § Batch planner.
- **`hew loop run --scope={ready|epics}` — scoped run queue
  (`hew-b3yl`).** Operators (and calling agents) now declare which
  slice of `bd ready` counts as the queue for a run: `--scope=ready`
  (everything ready — current behavior) or
  `--scope=epics --epics=<csv>` (only tasks transitively under the
  listed epics). The dispatcher resolves descendants once at startup
  and filters every tick against that set. Interactive runs get a
  picker; non-interactive runs without `--scope` fail with
  `HewError::MissingFlag { flag: "scope" }` so an agent-driven loop
  never accidentally consumes the rest of the graph. `run.json` gains
  a `scope` field; legacy runs without it load as `None` and
  `hew loop summary` renders them as `scope: ready (legacy)`. End-of-
  run summary now carries a `scope:` line between `outcomes` and
  `tokens`. See `docs/LOOP.md` § Scope.
- **`hew loop --jobs N` — parallel worker slots via per-worker git
  worktrees.** Default `1` keeps today's single-threaded loop
  byte-for-byte (no worktree, no merge-back, no manifest). `N >= 2`
  switches to a dispatcher path that lays down one git worktree per
  slot at `~/.hew/wt/<run-id>/<n>/`, drains the ready queue in
  parallel, then merges each `loop/<run-id>/w<n>` branch back onto
  launch HEAD at shutdown. Clean merges prune the worktree on the way
  out (`hew-kt5q`); conflicted merges file a `[merge-conflict]` bug
  task and leave the worktree for hand-resolution. Trust-the-graph
  per `DECISION:loop-parallel-overlap-policy`: any `bd ready` task is
  parallelizable; ordering belongs in dep edges, not overlap
  metadata.
- **`hew loop prune-worktrees [--apply]`** — garbage-collect orphan
  worktrees left behind by crashed parallel runs. Dry-run by default
  (lists what would be removed); `--apply` actually deletes. A
  worktree is "orphan" when its `<run-id>` has no live run-dir under
  `<project>/.hew/loop/` (or that run-dir's `run.json` already
  records a `stop_reason`). Per-worker branches survive in the
  project's git history.
- **`hew loop summary` per-worker breakdown.** Parallel runs render a
  `wkr | iters | closed | runtime | tokens | stop` table with a
  totals row before the aggregate summary block (`hew-h0tu`). Serial
  runs (no `manifest.json`) are unchanged.
- **`hew_core::git::reset_hard_in(worktree, sha)`** — per-worker
  rollback helper. Runs `git -C <worktree> reset --hard <sha>` so the
  parallel loop's gate-fail revert is scoped to one worker's worktree
  and never touches siblings (`DECISION:loop-parallel-overlap-policy`).
  `loop_cmd::git_reset_hard` now delegates here.
- **`hew_core::worktree::branch_exists` + `create()` collision guard.**
  Reusing a `run_id` after a crashed run would land `git worktree add
  -b` on a stale branch; `create()` now pre-checks via `rev-parse
  --verify` and returns a clear `GitNonZero` rather than silently
  overwriting. Branch naming stays the documented `loop/<run-id>/w<n>`
  (see `worktree::branch_name`).
- **Per-task model selection in `hew loop`.** Heavy tasks can route to a
  stronger model without changing the rest of the queue. Precedence
  (highest first): description tag `<!-- hew:model=opus-4-7 -->`, label
  `model:<name>`, config `loop.model.by_priority.<P>`,
  `loop.model.by_type.<type>`, `loop.model.default`. The resolved model
  is passed to the spawner per iter as `--model` / `-m` and logged in
  `iter-NNN.json::model`. `hew loop summary` adds a "by model"
  breakdown table (iters, tasks, input/cached/output/total) when at
  least one iter recorded a model; hidden otherwise. See
  `docs/LOOP.md` "Per-task model selection" for syntax + the
  per-model prompt-cache caveat.
- **`hew init` re-run UX.** Re-running `hew init` in an already-inited
  directory now detects the prior install via per-runtime artifact
  markers (`.claude/skills/hew/SKILL.md`, `.agents/skills/hew-execute/SKILL.md`,
  `HEW:BEGIN` in `.cursorrules`/`.windsurfrules`, `CLAUDE.md` for generic)
  and routes to one of three modes instead of silently re-prompting and
  overwriting `~/.config/hew/config.toml`: **Refresh** (default —
  re-lay skill files only, keep config), **Reconfigure** (full prompt
  chain + config overwrite, opt in via `--reconfigure`), or **Cancel**
  (no changes). Interactive runs get a 3-option picker; non-interactive
  runs without `--reconfigure` default to Refresh. The summary panel
  header reflects the chosen mode (`Setup complete` / `Refreshed` /
  `Reconfigured`). Fresh installs are unaffected. (hew-0wa)

### Changed

- **`hew-decompose` skill documents `bd create --graph` for batch task
  creation.** Hand-rolling `hew task new` in a loop hits
  `GOTCHA:zsh-cmd-substitution` on apostrophes / backticks / `$()` in
  multi-line descriptions. The skill body now includes a graph-JSON
  example (nodes + edges + parent_key) and calls out batch mode as the
  recommended path for >3-task plans. No new `hew` code — `bd create
  --graph` is a documented hold-out (alongside `bd orphans` / `bd lint`).
  Also fixes two stale references to a `bd gate create` primitive that
  never existed, and drops `gate` from the task-type table (external
  blockers use the dedicated `hew gate` surface, not a task type).

### Added

- **`hew gate` — external-state gates for `/hew:ship` and friends.**
  `hew gate new --gh-pr=N --title="…"` creates a bd task labelled
  `hew-gate` with the wait condition stored as typed metadata.
  `hew gate poll [<id>]` queries the external surface (currently
  `gh pr view <N> --json state,mergedAt`) and closes any task whose
  condition has fired — `MERGED` resolves, `OPEN` / `CLOSED-without-merge`
  stay pending, unknown states surface as warnings. Pair with
  `hew dep add <next-epic> <gate-id>` to block downstream work on a
  PR merge. Future backends (`gh:issue`, `gh:run`, `cmd:`) are
  scaffolded behind the same `GateKind` enum.

### Changed

- **`/hew:ship` skill body now uses `hew gate`.** Replaces the previous
  step 3 (`bd create --type=gate --await-type=gh:pr --await-id=N`,
  which referenced flags that don't exist in bd v1.0.3) with the
  working `hew gate new --gh-pr=N` flow plus an explicit `hew gate
  poll` step so resolved gates flip closed.

- **`hew init --runtime` accepts multiple runtimes.** Comma-separated
  (`--runtime=claude,codex`) and repeated (`--runtime=claude
  --runtime=codex`) forms both parse to the same list; the install loop
  then iterates each runtime in order with a per-runtime stdout banner.
  The interactive picker becomes a multi-select checkbox with
  currently-detected runtimes pre-checked.
- **`hew loop --runtime=codex` drives codex-cli as a first-class
  runtime.** The new `CodexSpawner` mirrors `ClaudeSpawner`: same
  `RuntimeSpawner` trait, same `HEW_LOOP_*_BIN` override pattern, same
  failure classification + token accounting. Codex's `--sandbox` enum is
  mapped from hew's `allowed_tools` list (`Edit`/`Write`/`MultiEdit`/
  `NotebookEdit` → `workspace-write`; everything else → `read-only`).
  See `docs/LOOP.md` for the runtime table and the lossy-translation
  caveat for `Bash(git:*)`-style restrictions.
- **Fallback runtime with primary-sticky cooldown.**
  `--fallback-runtime=<claude|codex>` (config:
  `loop.fallback_runtime`) routes iters to a secondary runtime when the
  primary returns a `RuntimeError`. `--fallback-cooldown-iters N`
  (config: `loop.fallback_cooldown_iters`, default 3) controls how long
  the loop sticks with the fallback before retrying primary. Worked
  example + cooldown semantics in `docs/LOOP.md`.
- **`SpawnFailureClass`** sits alongside `SpawnOutcome.success` so the
  runner can distinguish "runtime broke" (try fallback) from "guard
  tripped" / "budget exhausted" (don't). Status-code → kind classifier
  is shared between Claude's JSON error envelope and Codex's nested
  `turn.failed` message.
- **`SpawnOpts { model_override, working_dir }`** threaded through
  `RuntimeSpawner::spawn` for per-iter overrides. No behavior change
  today (call sites pass `SpawnOpts::default()`); substrate for the
  upcoming per-task model resolution epic.
- **Live-runtime e2e tests for both spawners.** `e2e_real_claude_spawn`
  + `e2e_real_codex_spawn` in `hew-core/src/runtime.rs` exercise the
  real CLIs when `HEW_LOOP_E2E=1` and the binary is on PATH. Default
  `cargo test` skips both. Documented in `CONTRIBUTING.md`.

### Changed

- **`hew init` with no `--runtime` and multiple detected runtimes
  refreshes them all** instead of erroring in non-interactive mode.
  Useful for `hew update` re-runs and CI flows that want
  every-installed-runtime regenerated without naming them. Zero detected
  + non-interactive still errors with `MissingFlag` as today.
- **`RuntimeSpawner::spawn` gained a `&SpawnOpts` parameter.** Custom
  implementations need to add `_opts: &SpawnOpts` to the signature.
  Existing call sites (production loop + tests) pass
  `&SpawnOpts::default()`.

## [0.10.0] — 2026-05-28

Makes the loop runner portable across language stacks and adds a way to
re-inspect any past run. Born from a real bug: `hew loop` died on iter 1
in a Python project because the per-iter gate was hardcoded to Rust.

### Added

- **Language-aware loop gate.** The per-iter test/lint gate no longer
  assumes `cargo`. It reads the commands from signals the project
  already wrote — a `test` / `lint` target in a `Makefile`, a recipe in
  a `justfile`, or a script in `package.json` — so the loop gates Rust,
  Python, Go, Node, or anything with those entry points. Detection lives
  in the new pure-logic `hew_core::gate` module. When no signal is
  present the gate is skipped (with a stderr breadcrumb) rather than
  failing the run, and a missing tool binary (ENOENT) degrades to
  skip-pass instead of trapping the loop.
- **`hew loop summary [--run-id]`.** Re-renders the rich end-of-run
  report (token breakdown, cache-hit rate, per-iter spend sparkline,
  symbols touched, stop reason) for any completed or running loop from
  its persisted `run.json` + iter logs — previously that summary only
  printed once, live at the end of a `run`. Backed by a new
  `StopReason::from_label` with a round-trip drift test.

### Fixed

- **`hew loop` no longer traps non-Rust projects.** A Python (or any
  non-cargo) repo previously tripped `GuardTrip` after one iter because
  `cargo test` / `cargo clippy` errored on the missing `Cargo.toml`. The
  loop now drains the queue regardless of stack.
- **Homebrew install path in the README** corrected to
  `brew install droidnoob/hew/hew` (the old `droidnoob/tap/hew` pointed
  at a tap repo that never existed).

### Docs

- New "The autonomous loop" section framing `hew loop` and its
  guardrails (graph-as-state, backpressure gate with rollback,
  byte-stable prompt prefix, budgets and clean stops).
- Terminal demos added to the README: `hew init`, `hew status`, and a
  `hew loop summary` screenshot.

## [0.9.0] — 2026-05-26

The "loop runner" release. `hew loop run` is now a fully wired
autonomous outer harness against real `claude -p`, exercised
end-to-end on a toy CRUD project and instrumented with cache-hit
tracking, symbol-level changelog, and a coloured end-of-run summary.

### Added

- **`hew loop` — process-level outer harness.** A new subcommand
  group that drains the bd ready queue by spawning fresh `claude -p`
  subprocesses, with hard caps on iters, tokens, and wall clock.
  Per-iter outcome, token spend, prefix hash, decisions / deferred
  ids, stderr tail, and (when treesitter is on) the symbol-level
  changelog get atomically logged to `.hew/loop/<run-id>/iter-NNN.json`.
  - `hew loop run` — drive the loop until a stop fires.
  - `hew loop list` — recent runs + state.
  - `hew loop logs --tail N` — pretty-print iter rows for a run.
  - `hew loop cancel` — touch the stop-file of a running loop.
- **Backpressure gate.** `cargo test` + `cargo clippy` run after each
  non-error iter. On `Verdict::Fail` the loop runs `git reset --hard
  <pre-iter-sha>` to revert the iter's commits, overrides the outcome
  to `BackpressureFail`, and files a `STATUS:loop-iter-failed:<run>:
  <iter>:<iso>` memory.
- **SIGINT → clean stop.** A `ctrlc` handler flips the shared
  `CancelFlag` so Ctrl+C produces `StopReason::Cancelled` in
  `run.json` (the in-flight iter finishes; no orphaned subprocess).
- **`--unattended` flag + decision-resolution flow.** Walks memory →
  code → research and files either `DECISION:<topic>` or
  `DEFERRED:<topic>` depending on provenance. When `--unattended` is
  on, the loop polls bd for new `DEFERRED:` memories the agent filed
  during an iter and tries to resolve them via prior art
  (case-insensitive memory match + `git grep -n -i -F` for code
  citations).
- **Out-of-band closure detection.** If an iter's task disappears
  from `bd.ready()` after the spawn, the outcome is promoted to
  `Closed` even when the agent closed via the Bash tool (whose stdout
  doesn't propagate into the model's final reply text).
- **Rich end-of-loop summary, auto-shown after every run.** Small
  magenta "hew" ASCII banner, outcome breakdown (colour-coded:
  green=closed, yellow=no_close, red=backpressure_fail), token split
  (input / output / cache_read / cache_create with percentages),
  cache-stability line computed from prompt_prefix_hash run-length,
  decisions / deferred counters, 8-block Unicode sparkline of
  per-iter token spend, symbols touched. `NO_COLOR` strips the ANSI.
- **`hew blast` as a first-class signal in two surfaces.**
  - **Loop iter logs** gain `symbols_touched: Vec<String>` populated
    from `blast::compute_blast_with(pre_iter_sha)` after each
    non-error iter. End-of-run summary aggregates the symbol set
    deduped across the run.
  - **`hew status`** gains a "Since last close" section listing the
    working-tree symbol delta against the default base (upstream →
    main → master). Top-8 + `…(+N more)` footer.
  - **`hew-execute` skill body** Step 6 close-checklist now nudges
    the agent to run `hew blast` pre-close to confirm the symbol
    delta matches task scope.
- **`/hew:loop` slash** wires the loop into Claude Code.
- **`docs/LOOP.md`** — full design + troubleshooting guide, plus a
  "First real run" section capturing the 2026-05-26 E2E with three
  real-claude runs (artifacts under `examples/loop-runs/2026-05-26/`).
- **`DEFERRED:` joins the memory-prefix allowlist** (14th prefix) so
  the loop can file unresolved topics for operator review via
  `hew remember --type=deferred`.
- **`hew_core::time::parse_iso_utc`** — strict reverse of
  `iso_from_unix`, used by the loop summary for wall-clock duration.

### Changed

- **`/hew:auto` slash body** rewritten as a thin pointer at
  `hew loop run --until-empty`. The in-conversation walk is still
  reachable via `/hew:work`.
- **`hew prime <skill>` defaults to text.** `--json` now gates the
  JSON shape for every skill (previously hard-coded JSON except for
  `resume`). Aligns with `FEEDBACK:no-json-piping` — text is the
  agent-facing contract. `--pretty` still implies `--json`.
- **Prompt cache invariant fixed.** The per-iter primer used to live
  inside the cacheable prefix passed to `prompt::assemble`, so
  `prompt_prefix_hash` changed every iter and the Anthropic prompt
  cache missed every spawn. The primer is now captured once at run
  start via `bd.prime_raw()` and held byte-stable across the run;
  per-iter task fields moved into the task brief (tail). Cache hit
  rate is now observable in the rich summary.

### Removed

- **`--research-budget` flag + `research_gate` module.** The flag
  was wired but never consumed — real `claude -p` agents do their
  own web search inside the spawn, never round-tripping a request
  to the loop. -277 lines net. If a future runtime exposes a
  loop-mediated research hook, a typed budget can be re-added at
  that point.

### Internal

- New crate modules: `hew_core::loop_summary`, plus the deletion of
  `hew_core::research_gate`.
- `RuntimeSpawner` trait + `ClaudeSpawner` (production) +
  `MockSpawner` (tests). `GateRunner` trait + `CargoGateRunner`
  (production) + `StaticGateRunner` (tests). Both injectable into
  `run_loop_with` so the integration tests under
  `hew/tests/loop_backpressure.rs` exercise the rollback,
  unattended-resolve, out-of-band-close, and prefix-hash-invariant
  paths against a tempdir git repo without burning real API tokens.
- Captured live `claude -p --output-format json` fixture at
  `hew-core/tests/fixtures/claude-output.json` (redacted); unit test
  parses it to catch field-name drift.

## [0.8.1] — 2026-05-26

Tree-sitter on by default. The 0.8.0 release shipped `hew blast` but
the Homebrew formula and `cargo install hew` produced binaries built
without `--features treesitter`, so end users hit a "rebuild with
--features treesitter" error when invoking it.

### Changed

- **`treesitter` feature is now on by default** in `hew-core` and
  `hew`. Shipped binaries (brew, cargo-dist, `cargo install hew`)
  include `hew blast` out of the box.
- Lean local build path preserved: `cargo build --no-default-features`
  strips every tree-sitter crate. The cfg gates that compile out the
  blast subcommand under no-features stay in place.
- `DECISION:treesitter-feature-gating` memory updated to reflect the
  new default.

## [0.8.0] — 2026-05-26

Tree-sitter symbol extraction + `hew blast`. Five-slice epic
delivering a feature-gated pure library for diff-driven symbol
extraction across six languages, plus a new CLI surface that
consumes it.

### Added

- **`hew_core::treesitter` library** (feature `treesitter`, off by
  default — `cargo build -p hew --features treesitter` to enable).
  Parses Rust / Python / TypeScript / JavaScript / Go / Java sources
  via tree-sitter and extracts `Symbol { name, kind, byte_range,
  line_range }`. Hand-trimmed `tags.scm` queries per language follow
  the tree-sitter org capture convention. Dedupe by `byte_range`
  prefers `Method` > `Function` > `Class`. Error-tolerant — malformed
  source returns Ok with partial results.
- **`hew_core::treesitter::diff::changed_symbols`** — pure
  line-overlap intersection between extracted symbols and a slice
  of changed line ranges.
- **`hew_core::diff_hunks::parse_changed_ranges`** — parser for
  `git diff --unified=0` hunk headers. Pure line math, not feature-
  gated. Skips zero-count pure-deletion hunks.
- **`hew blast` subcommand** (feature `treesitter`). Walks
  `git diff --unified=0 <base>...HEAD` and prints, per file, the
  symbols whose definitions overlap a hunk. Different from
  `git diff` — answers "which functions / classes actually changed,"
  not "which lines moved." Three input modes:
  - default — scan everything in the diff
  - positional file args — intersect with the diff set
  - `--no-diff <files>...` — skip git; emit every symbol in each
    file (combines with `--stdin` for `git ls-files | hew blast …`)
  - `--path <substr>` (repeatable) substring filter
  - `--base <ref>` override
  - `--json` for machine-readable output
- **`/hew:blast` slash command + `hew-blast` optional skill body.**
  Opt-in via `hew config set optional-skills.blast true`.
- **`craft.symbol_trace` config flag.** When true (and the binary
  was built with `--features treesitter`), `hew task close` auto-
  appends a `symbols changed (blast vs <base>): …` block to the
  task's notes via `bd update --append-notes`. Permanent semantic
  trail in the bd graph. Silent best-effort; off under default
  builds. Default `false`.
- **`ReviewBundle.changed_symbols`** field on `hew review bundle
  --json`. Per-symbol slices of the diff with `source_slice` so the
  review skill body can read just the changed regions instead of
  whole files. Populated when treesitter is enabled; absent under
  default builds (`serde(skip_serializing_if = "Vec::is_empty")`).
- **Statusline 1M context fix.** The model id in the transcript
  reliably carries a `[1m]` suffix on the extended window; the
  statusline now uses that as the authoritative ceiling instead of
  the observed-usage heuristic.

### Tests

- 12 unit tests for diff intersection (`treesitter::diff`).
- 30 unit tests for per-language extraction
  (`treesitter::grammars`).
- 7 end-to-end integration tests with per-language fixtures
  (`hew-core/tests/treesitter_integration.rs`) + one non-gating
  perf signal (`HEW_TS_BENCH=1` enforces <5ms warm; off otherwise
  prints).
- 7 hunk-header parser tests (`diff_hunks`).
- Statusline `[1m]`-context-suffix tests (3 new).

### Internal

- Three `GOTCHA:test-counts-drift` counts bumped: skills 20 → 21,
  slashes 39 → 40, install-claude file count 61 → 63, install-codex
  file count 41 → 43.
- `hew_core::blast::compute_blast` / `resolve_base` / etc. accept
  `&dyn GitClient` so library callers (including the review bundle
  enrichment) can drive the pipeline with a mock or owned client.

## [0.7.1] — 2026-05-25

Statusline fixes — `hew statusline` was overwriting Claude Code's
default bottom-bar instead of composing with it, and the scope label
was overflowing the truncation point. The previously-shown
context-usage indicator is now back as its own clearly-labeled
segment.

### Changed

- **Composes with Claude Code's default line.** The CLI now reads the
  session JSON it gets on stdin and renders a Claude-style prefix
  (`<model> | <cwd>` with ANSI color: cyan model, green cwd) ahead of
  the hew segment, separated by `||`. New `--bare` flag emits just
  the hew segment. `NO_COLOR` honored.
- **Scope label condensed.** `condense_title` splits an epic/milestone
  title on the first em-dash (matching the milestone-body convention)
  and truncates the head to 28 chars with an ellipsis. Stops the
  90-char epic descriptions from blowing past Claude Code's
  truncation point.

### Added

- **Context-usage segment** (`ctx <bar> <pct>% · <used-tokens>`).
  Parses `transcript_path` out of the stdin session JSON, walks the
  JSONL backward to the most-recent `type=assistant` message, and
  sums `input_tokens + cache_creation_input_tokens +
  cache_read_input_tokens` as "context used". Renders a color-
  gradient bar:
  - green   < 60%
  - yellow  60–84%
  - red     ≥ 85%
  Context limit inferred from observed usage (200K standard, 1M
  extended). Token count humanized as `847` / `41K` / `1.2M`.
  Best-effort: any IO / parse failure → segment skipped, statusline
  keeps working.
- **Hew segment is now bar-free.** `hew <label> N/M [(phase)]` —
  label-based so the two graphs (context bar vs. epic counter) aren't
  visually competing.

### Tests

- 8 new inline (humanize_tokens, infer_context_limit, claude prefix
  render with/without colors, condense_title em-dash strip +
  ellipsis truncation, TokenUsage::total).
- 3 new e2e (compose with session JSON, --bare skips prefix, ctx
  segment appears when transcript carries a usage block).

## [0.7.0] — 2026-05-25

Ships **`feat/statusline`** — Claude Code agent statusline showing
what hew is working on, end-to-end. Auto-wired by `hew init`, and
self-heals onto installs that predate this release with no `hew
update` needed.

### Added

- **`hew_core::statusline`** — pure render module. `render(input,
  format, width)` is a pure function over `(StatuslineInput,
  StatuslineFormat, width)`; no I/O. Three formats:
  - `Compact`: `<label> <bar> <pct>%`
  - `Medium` (default): adds phase + epic-fraction
  - `Full`: adds `<user> <user-done>/<user-total>`
  Width clamped to `[1, 80]`; total=0 / done=0 short-circuit to all-
  empty; done > total saturates to all-filled. `detect_phase` infers
  `Planning` / `Executing` / `Verifying` from `STATUS:*` markers plus
  task counts. `pick_scope_label` falls through milestone → epic →
  `"(no scope)"`. All types derive `Serialize` / `Deserialize` /
  `JsonSchema`. 17 unit tests cover the documented edge cases.
- **`hew statusline` subcommand** — thin clap wrapper that owns the
  side-effects the pure layer can't: stdin drain, `prime::resume`
  query, env-based `USER` lookup, milestone-memory project label,
  current-epic discovery (in_progress parent → ready epic fallback).
  Flags: `--compact` / `--full` (mutually exclusive; default Medium),
  `--scope=auto|project|milestone|epic`, `--width=N` (default 10,
  clamped not rejected). Stdout is reserved for the line itself;
  errors and bd-not-initialized both exit 0 with empty stdout so
  Claude Code's `statusLine` hook degrades gracefully. 7 e2e tests
  via a PATH-stubbed bd plus 5 inline tests for the lenient JSON
  peek + project-label fallback.
- **`hew init` upserts a top-level `statusLine` block in
  `.claude/settings.json`** carrying `hew_managed: true`. Mirrors the
  SessionStart hook pattern: re-install is idempotent, uninstall
  removes only the hew-owned variant, opt-out works by removing the
  flag. 5 new install tests cover write, idempotency, user-opt-out
  preservation on install, hew-owned removal on uninstall, and user-
  opt-out preservation through the full install / uninstall cycle.
- **Auto-migration** on `hew prime resume` for installs that predate
  this release. `install::auto_migrate_claude_statusline(cwd)` runs
  on every SessionStart; injects the block iff (1) `.claude/settings.json`
  exists and parses, (2) carries a `hew_managed: true` SessionStart
  entry (proves it's a hew install), and (3) has no `statusLine` key
  yet. Silent / fail-closed on every other path — the SessionStart
  hook must never break because of a self-heal misfire. 5 new tests
  cover the happy migration, idempotency when block already present,
  skip-when-not-a-hew-install, missing-settings, and the malformed-
  JSON safety net.

### Documentation

- New "Statusline" subsection in `CLAUDE.md` covering the three
  formats, the `hew_managed` discriminator, the graceful-fallback
  contract, where the pure module lives, and the self-heal path.
- `README.md` "Daily flow" notes the auto-wired statusline.
- `CONTRIBUTING.md` notes the `hew_managed: true` pattern for future
  contributors wiring install plumbing into `.claude/settings.json`.
- `docs/COMMANDS.md` CLI-only surfaces table gains a `hew statusline`
  entry.

## [0.6.1] — 2026-05-25

Fixes **GitHub #40** — `hew prime resume` surfacing a stale
`CHECKPOINT:` instead of the newest one. Root cause was on the
*write* side: the `hew-checkpoint` skill body told the agent to
roll the row by hand with `hew remember --raw "CHECKPOINT:…" --key
…`, which made it easy to produce a body without an ISO timestamp
directly after the `CHECKPOINT:` prefix. Such bodies sorted
lexicographically below well-formed newer entries in
`prime::latest_checkpoint`, silently shadowing them.

### Added

- **`hew checkpoint "<body>"` subcommand.** One-shot helper that
  auto-prepends `CHECKPOINT:<ISO-8601-now> — `, auto-generates a
  `checkpoint-<sanitised-iso>` key, and writes through the same path
  as `hew remember`. Supports `--key` override, `--timestamp`
  override (for back-dating / tests), and `--related` / `--related-task`
  for emitting `LINK:` sidecars in the same call. Body shapes already
  containing a well-formed `CHECKPOINT:<ISO>` prefix pass through
  verbatim; malformed prefixes are rewritten to the canonical shape.
- **`hew_core::checkpoint`** — pure `build_checkpoint_key` /
  `build_checkpoint_body` helpers covering the three input shapes
  (no prefix, broken prefix, well-formed). Eight unit tests pin the
  rewrite behaviour, including the exact bug shape from #40.
- **`hew_core::time`** — promoted the ISO-8601 formatter (previously
  duplicated in `hew/src/commands/compact.rs`) into a shared module
  with a `looks_like_iso_date` recogniser used by both the new
  checkpoint helper and the prime resume hardening.

### Changed

- **`hew_core::prime::latest_checkpoint`** now treats a non-ISO first
  token as "no timestamp" (instead of using whatever lex-sorts there).
  A malformed legacy checkpoint can no longer shadow a newer
  well-formed one in `hew prime resume`. Regression test pins the
  exact bug shape.
- **`skills/core/hew-checkpoint.md`** rewritten to lead with `hew
  checkpoint "<body>"`. The previous `hew remember --raw
  "CHECKPOINT:…" --key …` instructions (and the worked example using
  them) are gone; an explicit "do not hand-roll the shape" anti-pattern
  is now called out.
- **`commands/checkpoint.md`** mirrors the skill rewrite.

### Fixed

- **GitHub #40** — `hew prime resume` surfaces stale CHECKPOINT
  instead of newest. Closed by the combination of the new
  `hew checkpoint` subcommand (write-side fix), the
  `latest_checkpoint` recogniser tightening (defense-in-depth), and
  the skill-body rewrite (no longer recommends the foot-gunny path).

## [0.6.0] — 2026-05-22

Ships the **Memory Links epic** end-to-end — sidecar `LINK:` edges
between memories, with a writer, reader, body-scanner, cascade-aware
forget, compact exemption, and a remember-time suggestion prompt.
Plus a handful of independent fixes (`bd init` stealth/skip-agents,
`hew forget` as a top-level subcommand, gate-syntax + acceptance-flag
corrections in the decompose skill).

### Added

- **`LINK:` row grammar + parser + index** (hew-f75, hew-bfc).
  `hew_core::memories::links` exposes a frozen
  `LINK:<from>->relates_to:(memory|task):<to>` grammar with
  `parse_link_row` / `format_link_row` / `build_link_row_body`, a
  bidirectional `LinkIndex` (outbound, inbound, dangling), and a
  `read_links` builder over a `(key, body)` memory list. Dedupes
  identical rows surfaced from multiple bodies.
- **`hew remember --related <KEY>` / `--related-task <ID>`** (hew-utn).
  Repeatable flags that emit `LINK:` sidecars after the primary
  write. Require `--key` so the link's `<from>` side is
  deterministic. Targets are charset-validated front-door — a bad
  value fails the whole command before any bd write.
- **`hew memories --links <KEY>` reader** (hew-bhc). Text-default
  view of a memory's outbound / inbound / dangling edges, with
  `--json` emitting a stable `{key, outbound, inbound,
  dangling_outbound}` shape for downstream consumers. Picks up
  explicit `LINK:` rows AND inline `[[memory-key]]` / `#bd-task`
  body references in one merged index.
- **Body-reference scanner** (hew-tcy). `scan_body_refs(from, body)`
  extracts `[[memory-key]]` and `#prefix-id` references into the
  same `LinkIndex` as explicit LINK rows. `LinkSource { Explicit,
  BodyScan }` lets readers distinguish authored from inferred
  edges; explicit wins on dedupe. Backslash-escape suppresses
  wikilinks; task refs require a word boundary; trailing sentence
  punctuation is stripped so `#bd-99.` captures `bd-99` but
  subtask dots like `hew-a3f8.1` survive.
- **`hew forget <KEY>` top-level subcommand** (hew-7zi) — ergonomic
  alias for `hew memories --forget <KEY>`. Then extended (hew-jem)
  to **cascade-purge outbound LINK: rows** automatically: when a
  memory dies, sidecars from it die too; inbound rows are
  deliberately left as dangling so authors notice and rewire.
  Cascade target list is locked in *before* the primary forget
  fires, so a step-2 failure doesn't leave orphans.
  `hew memories --forget` remains the no-cascade escape hatch.
- **`hew remember --type=link`** (hew-uxf). Joins the canonical
  type allowlist (14 → 15 prefixes). Bare body gets the `LINK:`
  prefix prepended; pre-formatted full rows still go through
  `--raw`.
- **Interactive "these look related — link any?" prompt at remember
  time** (hew-3wt). Lexical ranker in `hew_core::memories::suggest`
  (token overlap + same-prefix bonus, stop-words + short-tokens
  filtered, top-N with deterministic tie-break) drives an
  `inquire::MultiSelect`. Selections feed back through the
  existing `--related` write path. New flags `--no-suggest` and
  `--suggest-top=<N>` (default 3, `0` = disabled). Silent under
  `--non-interactive` / CI / non-TTY per
  `CONVENTION:cli-non-interactive`.
- **`LINK:` exempted from compaction** (hew-54w). `LINK` joins
  `STATUS:scan/convention/plan/decompose` in
  `HARDCODED_EXEMPT_PREFIXES` so compact never destroys the edge
  graph. Matcher's `starts_with` arm makes bare `LINK` cover every
  `LINK:*` row.
- **`hew init --stealth` flag**. Explicit non-interactive opt-in
  to the existing stealth path (skips the "share the task graph
  in git?" prompt). Mutually exclusive with `--git-track`.

### Changed

- **`hew init` always passes `--skip-agents` to `bd init`**, and
  also `--stealth` when `git_track=false`. Stops bd from writing
  a competing `CLAUDE.md` ("use bd for ALL task tracking" — direct
  conflict with `FEEDBACK:prefer-hew-over-bd`), an `AGENTS.md`, or
  a competing SessionStart hook in `.claude/settings.json`, and
  stops bd from auto-committing beads files when the project
  isn't tracking `.beads/`. Reorders init: git is now initialised
  and `git_track` resolved before `bd init` runs.
- **`hew memories --forget <KEY>`** is now the documented
  no-cascade path; the new top-level `hew forget` is the curated
  ergonomic surface that cascades.

### Fixed

- **`hew-decompose` skill body**: Step 5 gate table used
  `bd create --type=gate --await-type=…` syntax that doesn't exist
  in bd. Replaced with the real `bd gate create --type=human|timer
  |gh:pr|gh:run --blocks=<id> --await-id=<x>` form; the `--blocks`
  flag does the dep wiring inline, dropping the standalone
  `hew dep add`. Step 7 acceptance comment switched from the stale
  `bd update --acceptance` to the existing `hew task update
  --acceptance` wrapper.

### Tests

hew-core test count: 251 (v0.5.2) → 303. New e2e suites for
`hew forget` (7 tests) and `hew remember --related/--links/
--no-suggest` (17 tests across remember + memories).

## [0.5.2] — 2026-05-20

Patch release fixing the bare `hew update` path. PR #28 cut 0.5.1 with
the new `hew next` / `hew ready` subcommands, but users couldn't
actually upgrade onto it via `hew update` itself — the in-process
axoupdater was misconfigured for every channel we ship. This release
fixes that and folds in a long-asked-for UX: skill files now refresh
automatically after a successful binary upgrade.

### Fixed

- **`hew update` works on every distribution channel** (hew-lv2). The
  in-process `axoupdater` call required an `install-receipt.json` that
  cargo-dist never writes (we ship with `install-updater = false`) and
  that brew / `cargo install` never write either — so the bare
  `hew update` failed with "The updater isn't properly configured" for
  every shipped install method. Replaced with explicit routing by
  `InstallSource` (Brew → `brew upgrade hew`; Cargo → `cargo install
  --git … --force`; curl-installer / unknown → axoupdater; dev build
  → refuse with hint). `HEW_INSTALL_SOURCE` env var overrides the
  auto-detected source.
- **Skill files auto-refresh after a binary upgrade.** Previously
  `hew update` upgraded the binary but left every project's
  `.claude/skills/hew/`, `.cursorrules`, etc. running stale skill
  bodies until the user remembered to also run `hew update --local`.
  The bare `hew update` now re-execs the freshly-installed `hew update
  --local` whenever a runtime marker is detected in cwd. Suppress with
  `--no-refresh`.
- **CI stub installer hardened against lingering Linux ETXTBSY.**
  PR #27 made `install_executable_stub` atomic via tmp+rename, but
  CI still occasionally hit "Text file busy" on the very next exec.
  Root cause: post-write chmod dirtied the inode's metadata after the
  write fd closed, and the kernel could still see the writer-count
  drop in-flight when exec consulted it. Fix: set the mode at create
  time (`OpenOptions::mode`), `sync_all()` data + metadata, drop the
  fd, then rename, then fsync the parent directory. Per-call atomic
  counter added to the tmp name so two threads sampling identical
  nanos can no longer collide.

## [0.5.1] — 2026-05-20

Small follow-up release adding the long-missing `hew ready` and
`hew next` CLI surfaces. The `/hew:next` slash skill was documented as
picking the top unblocked task, but the CLI had no subcommand to back
it — agents had to drop to raw `bd ready` / `bd list`, violating
`prefer-hew-over-bd`. Both gaps now closed.

### Added

- **`hew ready` and `hew next` subcommands** (hew-xtg, GH #23). The
  `/hew:next` slash skill was documented as picking the top unblocked
  task, but the CLI itself had no `next` or `ready` command — agents
  had to drop down to raw `bd ready` / `bd list`, violating
  `prefer-hew-over-bd`. New surfaces:
  - `hew ready` mirrors `bd ready --json` through the curated
    `ReadyTask` type. Text-default; `--json` opts in; `--n` truncates.
  - `hew next` claims the top ready task and prints its id + title.
    `--no-claim` peeks. `--branch` additionally creates a feature
    branch (prefix derived from `issue_type` via
    `feat/fix/chore/docs`; slug from task title). `--prefix` and
    `--slug` override the auto-derivation.
  - The bundled `/hew:next` slash command updated to call the new
    CLI directly instead of pivoting through `hew prime execute`.

## [0.5.0] — 2026-05-20

Session-start overhaul plus a sweep of agent-facing bug fixes. The
headline is `hew prime resume`: the SessionStart hook now emits a
plaintext summary by default and surfaces three new bands of context
that the agent previously had to discover by hand — `hew config` knobs
as standing instructions, every claimed in-flight task with its body,
and the working-tree git state (branch, dirty, ahead/behind). Memory
bucketing gained `DECISION`, `GOTCHA`, and `FEEDBACK` as first-class
categories instead of being buried in `factual`.

Alongside that, six bugs filed against the curated wrapper layer
landed: `hew remember --type=feedback` now accepts, `hew task close`
gains `--force`, `hew task show` displays children for epics,
`hew task update` is new, `hew update --local` no longer requires a
working self-updater, and `hew epic tree` is ~3.5× faster on real
graphs.

### Added — prime resume context overhaul (hew-5gb, PRs #15 and #22)

- **Plaintext-by-default for `hew prime resume`** (hew-prime-pt, PR
  #15). The SessionStart hook output is now a readable summary
  instead of a wall of JSON. `--json` (and `--pretty`, which implies
  it) emit the structured `ResumeOutput` for tooling consumers.
  Other `hew prime <skill>` invocations are unchanged — they still
  emit JSON. The bundled `RESUME_DIRECTIVE` in `install.rs` updated
  to describe the new default.
- **`ConfigInstructions` rendered as standing directives** (hew-5gb).
  `hew config` knobs that shape behavior (`branching.strategy`,
  `testing.require`, `craft.max_function_lines`,
  `craft.warn_on_unused`, `review.after_n_tasks`, `review.after_epic`,
  `research.default`, `optional_skills.*`, `git_track`) appear at
  every session start as actionable lines, not a raw dump. Loads via
  `config::load` with default fallback so a missing or malformed
  config never breaks the hook.
- **Claimed in-flight tasks surfaced with their body** (hew-5gb).
  Every `status=in_progress` task appears with id, title, priority,
  and the first ~20 lines of its description. The "what was I doing?"
  signal the previous resume output omitted (only the integer count
  was visible). Routes via `BdClient::run_to_file` per
  `GOTCHA:pipe-deadlock` since `bd list --json` can exceed the OS
  pipe buffer on large graphs.
- **Git working-tree state in the hook output** (hew-5gb). Current
  branch, dirty/clean, untracked count, ahead/behind upstream.
  Best-effort: degrades to `None` when not in a repo or git is
  unavailable.
- **First-class `DECISION`, `GOTCHA`, `FEEDBACK` memory buckets**
  (hew-5gb). Previously bucketed into `factual`. `CLAUDE.md` calls
  these out as "cite when relevant" / "read before debugging" / "honor
  every time" categories — they deserved direct visibility. On the
  hew repo's own graph, that's 35 + 9 + 1 entries pulled out of
  factual.

### Added — task wrappers (hew-fz2, hew-8j7, hew-dm5, PR #24)

- **`hew task close --force`** (hew-fz2 / GH #17). Surfaces `bd`'s
  existing `--force` flag through the hew wrapper. Useful when a
  planner added an over-conservative dep that didn't actually gate
  the work — previously required dropping to `bd close` directly,
  violating prefer-hew-over-bd. Closes still record the deviation
  type via `--type N`.
- **`hew task show <epic>` displays children** (hew-8j7 / GH #20).
  Default behavior now appends a `CHILDREN (N/M complete)` section
  whenever the task has children, mirroring `bd show`'s format.
  `--no-children` flag suppresses for narrow output. `--json` form
  gains a `children: [TaskSummary, ...]` field (omitted when empty).
- **`hew task update`** (hew-dm5 / GH #16). New subcommand for
  editing existing task fields after creation:
  `--title`, `--description`, `--description-file <path>` (mutually
  exclusive with `--description`), `--acceptance`. Removes the
  two-narrative problem with `hew task note` and the
  prefer-hew-over-bd violation of falling back to raw `bd update`
  after a spec pivot.

### Added — `feedback` memory type (hew-45h, PR #24)

- **`hew remember --type=feedback`** (hew-45h / GH #18) — adds
  `feedback` to the `MEMORY_PREFIXES` allowlist (now 14 entries).
  FEEDBACK is a first-class Anthropic auto-memory category for user
  preferences and corrections — distinct from `CONVENTION` (project
  rule), `DECISION` (point-in-time choice), and `GOTCHA` (technical
  pitfall). Was previously forcing users to either miscategorize or
  fall back to raw `bd remember`.

### Fixed — `hew update` is usable again (hew-rr8, PR #24)

- **`--local` bypasses the self-updater entirely** (hew-rr8 / GH
  #19). Previously `hew update --local` ran the axoupdater check
  first; when that returned "isn't properly configured" the user
  couldn't refresh their `.claude/` skills either. Now `--local`
  short-circuits before any updater call, detects every installed
  runtime via `install::detect_runtimes`, and re-runs
  `install::install` for each. Errors clean when no runtime marker
  is found, pointing the user at `hew init`.
- **Self-updater failures now ship a concrete recovery path**
  (hew-rr8). The old message ("install a newer release manually from
  &lt;url&gt;") gave no actionable command. The new
  `MANUAL_INSTALL_HINT` lists `brew install droidnoob/hew/hew`,
  `cargo install --git https://github.com/droidnoob/hew hew`, and the
  releases page. After a successful binary upgrade, `hew update` now
  also reminds the user to run `hew update --local` in each project
  root.

### Performance — `hew epic tree` ~3.5× faster (hew-ara, PR #24)

- **Drop the N+1 query** (hew-ara / GH #21). `hew epic tree` was
  making `2N+1` bd subprocess calls (one show + one children per
  node). Each bd call costs ~0.5s, so a 7-child epic took ~5–7s.
  Two fixes: (a) eliminate the redundant per-node `bd show` since
  `children()` already returns full `TaskSummary` objects;
  (b) leaf-skip heuristic — non-epic, non-milestone tasks never
  parent anything in practice, so their `children()` call is
  skipped. Combined: `O(2N)` → `O(1)` bd calls on a flat-leaf epic.
  Measured on this repo's `hew-4az`: 6.7s → 1.9s (release build).
  Regression test `tree_does_not_query_children_of_leaf_tasks`
  asserts exactly 1 show + 1 children call on a 3-leaf fixture.

### Internal

- `MEMORY_PREFIXES` length is 14 (was 13). The
  `validate_memory_type_accepts_every_allowlisted_value` test
  iterates over the array so it auto-covers the new entry; no
  count-drift bumps needed.
- `prime::ResumeOutput` JSON gained `config`, `in_progress`, `git`
  fields. Existing consumers of the JSON shape continue to work —
  serde's `#[serde(default)]` and `skip_serializing_if = "Option::is_none"`
  guard against missing fields on older runs.

## [0.4.0] — 2026-05-14

First public release. Repo flipped from private to public; cargo-dist
publish and `hew update` come online for end users. The headline
change is the **init v2 flow** — a complete reimagining of
`hew init`, expanding it from a single runtime prompt to a structured
13-step setup wizard that surfaces every config knob the methodology
relies on, with a matching CLI flag for every prompt so scripted
installs stay first-class. Codex adapter fixes that landed in the
0.3.1 unreleased window ship as part of this release.

### Added — init v2 flow (hew-hxr)

- **Tri-state `SkillMode` for plan-chain optional skills**
  (hew-d3r). `OptionalSkills` switches from four booleans to three
  `SkillMode { Yes, No, Ask }` fields (`deps`, `research`,
  `security`); `quick` dropped (it's a here-and-now utility, not a
  plan-chain decision). Default is `Ask` for all three so the
  hew-plan picker stays the source of truth. Hand-rolled
  `Deserialize` accepts legacy bool on-disk configs (`true → Yes`,
  `false → No`) so pre-0.4 users don't see hard parse errors.
- **`hew init` status lines, no install prompts** (hew-c0u +
  hew-8xl). git and beads detection now emits readable status lines
  (`git: ✓ on PATH`, `beads: ✓ installed`). The Confirm prompt for
  git auto-install is gone — interactive runs try the sudo-free
  path automatically and surface a hint on failure. Beads install
  always prints the installer it's using (`brew` or `curl`). Never
  blocks init.
- **`hew init` runs `git init` when no `.git/` is present** (hew-c0u).
  Idempotent: skips when the repo exists.
- **`Share the task graph in git?` prompt** (hew-op2). Git-gated:
  only asked when `.git/` exists and interactive. Persists to
  `git_track` in `~/.config/hew/config.toml`. `--git-track` CLI flag
  still works.
- **Project state detection + prompt** (hew-t4l). New
  `--project-type=new|existing` flag; interactive runs ask with
  cursor pre-positioned on detected default (`existing` if any
  source-like files exist, else `new`). Drives the post-install
  routing hint (`/hew:new-project` vs `/hew:scan`).
- **Auto-branching strategy prompt + default `epic`** (hew-j7h).
  `BranchingConfig::default` bumped from `"none"` to `"epic"`; new
  `--branching=epic|none|always` flag.
- **Optional skills tri-state prompts** (hew-t11). Three Select
  prompts (deps / research / security), each picking
  `yes|no|ask`, with a preamble warning about token cost. New
  flags: `--deps`, `--research`, `--security`.
- **Require-tests prompt** (hew-7pr). New `--require-tests` /
  `--no-require-tests` flag pair persists to `testing.require`.
- **Configure-more gate for advanced knobs** (hew-ajw). Optional
  Confirm gates the `research.default` and review-cadence
  prompts. New flags `--research-default`, `--review-after-n`,
  `--review-after-epic` short-circuit the gate.
- **Summary panel at end of install** (hew-bcz). Replaces the
  one-line "hew installed for X" output with a structured panel
  showing every decided value. `--quiet` keeps the old one-liner
  for scripts.
- **ASCII banner at top of `hew init`** (hew-id2). Six-line block-
  letter "HEW" wordmark + version + tagline. Suppressed in
  `--quiet` and non-interactive runs (`hew/src/ui/banner.rs`).
- **e2e coverage** (hew-efy). 34 tests in
  `hew/tests/init_e2e.rs`, including a catch-all that round-trips
  every v2 flag through the on-disk config.

### Changed

- **`BranchingConfig::default` is now `"epic"`** (hew-j7h). Was
  `"none"` in 0.3.x. New installs get auto-branching on out of the
  box; existing installs that set it explicitly are unaffected.

### Fixed

- **Codex adapter: malformed `AgentRoleToml` schema** (#13). The
  `.codex/agents/hew-*.toml` emitter wrote `name` + `category` +
  `body`, none of which Codex's `AgentRoleToml` accepts. Codex
  silently dropped all 20 hew roles at startup. Emitter now writes
  the correct shape (`name` + `description` + `developer_instructions`)
  and uses TOML literal multi-line strings so regex escapes (`\s`,
  `\b`) pass through untouched.

### Added — adapters

- **Codex adapter: skills emitter** (#13). `hew init --runtime=codex`
  also writes `.agents/skills/hew-<name>/SKILL.md` per skill —
  Codex's auto-discovered skill primitive. Hew methodology is now
  natively invokable in Codex chat, not just spawn-able as a sub-agent
  role. File count emitted by `Runtime::Codex` install bumps 21 → 41
  (20 roles + 20 SKILL.md + AGENTS.md).

### Notes

- Repo flipped public on 2026-05-14 ahead of the originally-planned
  treesitter gate (`hew-sb7`). cargo-dist publish, `hew update`, and
  the deferred branch-protection ruleset are now live — see
  `project:release-gating` memory for the workflow implications.
- Re-running `hew init` in an already-inited dir still overwrites
  config + re-asks every prompt. Detect-and-offer-refresh logic is
  filed as **hew-0wa** for a follow-up release.

## [0.3.0] — 2026-05-13

First feature release of the 0.3 line. Two new CLI surfaces (`hew
epic list`, `hew remember --from-file`, `hew memories --export`), a
real correctness fix in `hew compact apply`, and a methodology
refinement that teaches memory grouping by domain — all on top of
the 0.2.1 branch-protection baseline. Repo stays private until the
treesitter milestone (`hew-sb7`) ships; this tag exists for version
hygiene + local installers.

### Added

- **`hew epic list`** (#6): new verb under `hew epic`. Lists epics
  with status-filter defaults (open/in_progress/blocked/deferred);
  `--all` includes closed, `--closed` filters to closed-only,
  `--n <N>` caps rows, `--json` emits structured output. Routes
  through `hew_core::tasks::list` with `issue_type=epic`. Four unit
  tests cover the status-filter branches.
- **`hew remember --from-file <PATH>`** (#8): bulk insert from a JSON
  array of `{ type?, body, key?, raw? }` entries. All-or-nothing
  semantics — every entry is validated up-front before any
  `bd remember` is called, so a malformed entry in the middle of the
  file rejects the whole batch with `entry[N]: <reason>` and zero
  partial writes. Nine unit tests cover the payload-building logic.
- **`hew memories --export [-o PATH] [--plaintext]`** (#10): dump
  filtered memories to a file. Default format JSON; `--plaintext`
  for a human-readable text listing. Default path when `-o` is
  omitted: `<projname>-memories-<iso-ts>.<ext>` in the current
  directory, with a filesystem-safe ISO timestamp
  (`YYYY-MM-DDTHH-MM-SSZ`, colons replaced with dashes for Windows
  safety). Filters reuse the existing `--prefix`, `--grep`,
  `--research` flags. Six unit tests.
- **Memory-grouping methodology** (#9): three high-volume
  memory-creating skill bodies (`hew-convention`, `hew-audit`,
  `hew-plan`) now teach **one memory per domain** with structured
  sub-sections instead of atomic per-rule writes. Each skill has a
  worked atomic-vs-grouped contrast and a rule of thumb (≥3
  same-domain remembers → fold). The JSON shape for
  `--from-file` is documented once in `hew-convention.md`; the
  others cross-reference. `docs/COMMANDS.md` picks up the new
  `--from-file` row.

### Fixed

- **`hew compact apply` silent data loss on slug collision** (#7):
  bd's `remember` auto-derives a slug from the body. A compacted
  replacement whose body starts with `<PREFIX>:<topic>` (e.g.
  `CONVENTION:subprocess —…`) could auto-slug to the same key as a
  not-yet-forgotten source. bd treated the write as
  update-in-place; phase 2's forget then erased the new entry along
  with the source — silently. `ApplyReport.added` still reported
  success.

  The fix routes every compaction write through an explicit
  `--key` of shape `<prefix-lower>-compact-<topic-slug>[-<idx>]
  [-<n>]`. The `compact-` infix guarantees no auto-derived slug
  can match. A new phase 1.5 read-back verification re-fetches
  memories after writes and bails with
  `HewError::CompactWriteLost { keys }` if any chosen key didn't
  land — **no sources are forgotten on a broken write**, which
  preserves the `DECISION:compact-safety` invariant under failure.
  `ApplyReport` gains `added_keys: Vec<String>`
  (`serde(default)`, additive-only). Four new regression tests
  include simulating the exact failure mode via a `MockBd::drop_keys`
  field.

### Internal

- `MockBd` in `hew-core::compact::tests` now mutates its memory
  store on remember/forget so post-write verification is testable.
- The ISO-timestamp helper in `hew/src/commands/memories.rs`
  intentionally duplicates `iso_from_unix` from
  `hew/src/commands/compact.rs` (~25 lines); a third call site will
  trigger an extraction to a shared utility module.

## [0.2.1] — 2026-05-13

Branch-protection hardening + project-level agent guidance docs. No
functional CLI changes; methodology + tooling refinements that
formalize the "no commits on `main`" contract end-to-end.

### Added

- **Branch protection — local pre-commit hook** (`.githooks/pre-commit`
  + `.pre-commit-config.yaml`): refuses commits while HEAD is on
  `main` / `master`. Emergency override via
  `HEW_ALLOW_MAIN_COMMIT=1`. Mirrored across both the bash hook and
  the pre-commit framework config so installing either gets the
  guard.
- **Branch protection — GitHub ruleset spec** at
  `.github/protection/main-ruleset.json` plus a
  `.github/protection/README.md` documenting when and how to apply.
  The ruleset refuses deletion + force-push, requires PRs to merge,
  requires all 8 CI contexts to pass (rustfmt + clippy + 4-way test
  matrix + cargo-audit + cargo-deny), with no admin bypass. Tracked
  for deferred apply once the repo flips public.
- **Project-level agent guidance**: new `CLAUDE.md` (~200 lines) +
  `AGENTS.md` (thin pointer following the [agents.md](https://agents.md)
  convention). CLAUDE.md is the canonical source — covers project
  shape, branching contract, build/test/lint, how to use the
  methodology on itself, memory prefix semantics, the three-place
  test-count-drift contract, hard-won gotchas (pipe-deadlock, zsh
  heredoc, clippy traps, bd-mol-bond, flaky pre-commit), locked
  behavioral preferences (`FEEDBACK:no-json-piping`,
  `FEEDBACK:prefer-hew-over-bd`, `CONVENTION:commit-messages`),
  locked architectural decisions (craft-enforcement, craft-adaptive,
  compact-safety, review-filing, hew-remember-type-allowlist), and
  the 5-step release process.
- **CONTRIBUTING.md "Branching" section** documenting that `main` is
  protected on both ends, the conventional prefix list, and the
  override env var.

### Changed

- **`hew-plan`** now decides the branch shape (prefix + slug) as
  part of the plan output rather than the agent inventing one at
  claim time. New "Decide the branch shape (don't create it)"
  section right after "When this skill runs". Output recap gains a
  `Branch: <prefix>/<slug>` line alongside Goal / Acceptance /
  Architecture / Order / Graph shape / Open questions.
- **`hew-execute` Step 3a** renamed "create the branch on first
  claim" and restructured around the new skill-boundary contract:
  - **Branch source-of-truth** subsection reads the plan's `Branch:`
    recap, falls back to the epic body, then asks the user once and
    caches.
  - **Protected-branch guard** subsection refuses to proceed
    without a branch decision when HEAD is on `main` / `master` and
    the project uses protected-branch enforcement. Never invents a
    branch name — that's the planner's job.
  - **Opt-in auto-branch strategy** subsection unchanged in spirit
    but now layered cleanly on top of the per-plan decision.
  Catches the "agent walked the loop on main and only discovered
  the problem at commit time" failure mode at claim time instead.
- **README.md** restructured for breathing room: horizontal rules
  between every major section, dense 4–6 sentence paragraphs split
  at natural pivots, numbered phase items broken into lead lines +
  rationale sub-paragraphs, install methods bolded as labels, the
  39-slashes line expanded into a bulleted category list,
  troubleshooting entries split into bold-header + body. 245 → 372
  lines, content byte-identical word-for-word.

### Fixed

- `hew/tests/completions_e2e.rs::manpage_emits_roff_header` no
  longer hardcodes the version string; reads `CARGO_PKG_VERSION` at
  compile time. (Already fixed in 0.2.0 but worth flagging since it
  was the bump-blocker.)

### Memories

- New `CONVENTION:skill-boundaries-plan-vs-execute` codifies the
  rule: `hew-plan` DECIDES, `hew-execute` DOES. Plan never runs
  `git`; execute never makes architectural calls. Apply this lens
  to any new skill-body work that touches actions vs. decisions.
- New `CONVENTION:commit-messages` codifies the no-GSD-by-name rule
  for commit messages and bodies.

## [0.2.0] — 2026-05-13

The Craft + Compaction release. The methodology now adapts to each project's chosen quality dial, surfaces drift as soft warnings without blocking close, and ships a controlled compaction surface for noisy memory prefixes. Plus 12 new slash commands and a docs refresh.

### Added — Craft system

- **Catalogue** — 28 craft principles (SOLID, DRY, KISS, YAGNI, Clean Architecture, Hexagonal, DDD, Idempotence, Fail Fast, Pure Functions, Small Functions, Single Level of Abstraction, Tell-Don't-Ask, Command-Query Separation, Meaningful Names, No Magic Numbers, Consistency With Existing Code, …) at `skills/data/craft-principles.toml`, embedded via `include_str!` and exposed as `hew schema craft-principles`. New `hew_core::craft` API: `load`, `find`, `ids`, `for_stack(stack_id)`.
- **Adaptive selection** — principles are picked per project, not applied universally. Three entry points populate the project's set:
  - `hew-new-project` Phase C surfaces a multi-select picker; defaults from each principle's `default_for_stacks` list. Each chosen principle persists as `CONVENTION:craft.<id>`.
  - `hew-convention` Step 11 (brownfield) extracts the principles the codebase already follows via four heuristics: function-length distribution, layering style, test-to-source ratio, opportunistic style fingerprints.
  - `hew-plan` Craft refinement records per-feature deviations as `DECISION:craft-feature:<plan-id>` memories.
- **Soft-warning enforcement** — `hew_core::guard::craft_warnings(memories, diff, cfg) -> Vec<CraftWarning>` is a pure function the hew-guard skill body calls on the staged diff. Three heuristics ship: `missing-tests` (always-on; promoted from Warn to Fail when `testing.require=true`), `function-length` (gated on `craft.max_function_lines > 0`), `duplication` (gated on a `CONVENTION:craft.dry` memory). Per `DECISION:craft-enforcement`, warnings never block close on their own.
- **Brownfield deference** — `craft.consistency-with-existing-code` defaults on every seeded stack; existing `CONVENTION:*` always wins over a freshly-picked principle.
- **Methodology threading** — every loop skill (`hew-plan`, `hew-decompose`, `hew-execute`, `hew-quick`, `hew-guard`, `hew-verify`, `hew-review`, `hew-adversarial-review`) reads and reacts to the picked set: task descriptions gain `Tests:` + `Craft:` lines, executor has a Step 5a inline craft check, verify has a Maintainability dimension across the batch, review walks picked principles, adversarial-review attacks gaps left by unpicked ones.
- **Brownfield audit** — `hew-audit` gains Craft drift checks as a 7th finding category that greps for code regions contradicting a persisted `CONVENTION:craft.<id>`.

### Added — `hew-new-project` skill

- New core skill bootstraps a project from a 1–3 sentence outline. Phases: Capture + Socratic clarifying (4–6 PROJECT memories) → Parallel research (RESEARCH memories with `[VERIFIED]` / `[CITED]` / `[ASSUMED]` provenance tags) → Synthesis pickers (stack family + craft principles + database + auth + hosting) → Milestone-vocabulary picker → Roadmap construction (one epic per milestone, sequenced via task-level deps) → First-milestone decompose. Idempotency: refuses re-run if `STATUS:new-project:complete` exists unless `--re-bootstrap` is passed.
- Companion `/hew:new-project` slash command.

### Added — Memory compaction

- **`hew_core::compact` module** — pure-data layer with `CompactPlan { prefix, target_clusters, granularity, allow_recompact, clusters }`, `Cluster { topic, source_keys, replacement_bodies }`, `Granularity { Broad, Fine }`, `ApplyReport`, `validate`, and `default_k(n) = ceil(sqrt(n)).clamp(1, cap)`. All schemars-derived.
- **Safety invariants** in `compact::apply`, encoded per the four locked DECISION:compact-* memories: **adds-before-forgets** (replacements written FIRST so a mid-apply crash leaves more memory, not less), **provenance suffix** (every replacement body gets `[compacted-from: k1, k2, ...]` appended), **drift-guard** (sources already carrying the suffix are skipped unless `allow_recompact=true`), **exempt allowlist** (`STATUS:scan/convention/plan/decompose` hardcoded plus user-configured `compact.exempt`).
- **CLI** — `hew compact apply` reads a CompactPlan from stdin (validates before any bd contact); `hew compact list-prefixes` surveys per-prefix memory counts with strict UPPER-SNAKE prefix detection so natural-language colons don't pollute the histogram. Schema variants `compact-plan` + `compact-apply-report`.
- **Config knobs** — `compact.dry_run_default` (true), `compact.granularity_default` (`"broad"`), `compact.target_clusters_cap` (6), `compact.allow_recompact_default` (false), `compact.exempt`.
- **`hew-compact` skill** — nine-step compaction loop documented: survey → pick prefix → read → cluster in-context (K = ceil(√N) capped at 6, dual-prompt granularity) → draft prescriptive replacement bodies → render diff preview → wait for explicit approval → emit CompactPlan JSON and pipe to `hew compact apply` → show ApplyReport. Refuses to compact `DECISION:` / `STATUS:` / `BOUNDARY:` prefixes.
- Companion `/hew:compact <PREFIX>` slash command.

### Added — Slash commands (12 new, 27 → 39 total)

- `/hew:compact` — memory compaction
- `/hew:decompose` — direct invocation of the hew-decompose skill
- `/hew:resume` — manual re-run of the SessionStart-hook prime payload
- `/hew:prime <skill>` — manual primer for a specific skill
- Brownfield chain: `/hew:scan`, `/hew:convention`, `/hew:audit`, `/hew:boundary`, `/hew:migrate`
- Optional skills: `/hew:deps`, `/hew:research`, `/hew:security`
- (Also new: `/hew:new-project` and `/hew:spec` shipped earlier in this cycle.)

### Added — Curated bd wrappers (agent-facing stable contract)

- `hew task {show, list, claim, close, new, reopen, children, note, search}` — stable JSON via `--json`; schemas via `hew schema {task, task-list-filter, new-task, epic}`.
- `hew dep {add, remove, tree, blocked}` — dependency-edge ops.
- `hew epic {show, tree, close, audit, summary}` — epic-level ops.
- `hew remember --type=<allowlist>` — 13-prefix allowlist; `--raw` escape for the 5 non-allowlisted prefixes.
- `hew memories [--prefix|--grep|--research|--recall|--forget]` — curated read/inspect/forget surface. Text-default output; `--json` opts in.

### Added — Review pipeline

- `hew-review` skill + `/hew:review` slash — friendly second-pass code review against CONVENTION/BOUNDARY/SECURITY memories. Files findings as `[Review][BLOCKER|WARNING|INFO]` bd bugs/chores. Includes a Craft pillar walking each `CONVENTION:craft.<id>`.
- `hew-adversarial-review` skill + `/hew:adversarial-review` slash — red-team pass attacking gaps the friendly review can't see. Steelman of the not-taken alternative. Attacks principles the project *didn't* pick.
- `hew review bundle` CLI — assembles the agent-facing input (closed-tasks-in-scope, diff, applicable memories, epic body, last review timestamp) with stable schema.
- Step 10a executor picker fires on `review.after_n_tasks` or `review.after_epic` config triggers.

### Added — Spec-clarity gate

- `hew-spec` skill + `/hew:spec` slash — scores user asks on goal-clarity + acceptance-clarity and loops Socratic questions until the ambiguity gate passes (or 4 rounds elapse). Use before `/hew:plan` when the ask is vague.

### Added — Branching + research detour

- `hew_core::branch` + `hew branch new --prefix=<type> --slug=<text>` — conventional-prefix branch creation. `branching.strategy` config (`none` / `epic` / `always`) controls when `hew-execute` first-claim auto-creates a branch.
- `hew-plan` research-or-decompose tail picker honoring `research.default` config (`ask` / `auto-skip` / `auto-run`).
- `hew-research` skill provenance discipline: every finding tagged `[VERIFIED]` / `[CITED]` / `[ASSUMED]` with a source citation.

### Added — Session resume

- Claude Code `SessionStart` hook wired by `hew init` runs `hew prime resume` on every session entry. Marked `hew_managed: true` so re-installs replace in place.
- `hew prime resume` emits the resume JSON payload (project state, STATUS flags, categorized memories, latest CHECKPOINT) with stable schema for non-Claude runtimes.
- `hew-checkpoint` skill + `/hew:checkpoint` slash dump in-flight state to a `CHECKPOINT:` memory before `/clear`.

### Added — Documentation

- New `docs/COMMANDS.md` — full slash-command reference, 39 entries grouped into 9 categories with descriptions pulled verbatim from command frontmatter.
- README.md restructured to the established Claude-Code-methodology section order; all `>` blockquote-as-code-block misuses fixed; all `&lt;` / `&gt;` HTML entity escapes replaced with literal angle brackets inside fenced blocks.
- ARCHITECTURE.md gains a "Craft-principles data layer + soft-warning model" section covering the three layers (catalog / membership memory / enforcement) and the methodology flow.
- SKILL.md (the always-loaded agent index) gains a "Craft principles" section + a `CONVENTION:craft.<id>` row in the memory-prefix table.

### Changed

- Skill bodies migrated from raw `bd remember "PREFIX:..."` / `bd create --type=...` calls to the curated `hew remember --type=<prefix>` / `hew task new` wrappers. Frees skill bodies from coupling to bd's evolving JSON schema while keeping a stable agent-facing contract.
- Big-output bd queries (`list`, `prime`, `memories`, `ready`) now route through `read_via_temp` (Stdio::from(File)) instead of pipe-buffer reads after `wait_timeout` — fixes a pipe-deadlock that bit large memory stores above the OS pipe buffer (~16KB on macOS / ~64KB on Linux).
- `hew task list` default `--n 20` newest-first; `--n 0` unlimited; `--head` reverses to oldest-first.
- `hew task show` text-default; `--json` opts in (matches `hew memories` + `hew status` pattern).

### Fixed

- HTML-entity-escaped angle brackets in README.md (`&lt;` / `&gt;`) → literal `<` / `>` inside fenced blocks.
- `>` blockquote-as-code-block misuses in README.md → fenced ` ```text ` blocks.

### Configuration

New config keys (full list via `hew config keys`):

| Key | Default | What it does |
|-----|---------|--------------|
| `testing.require` | `false` | When `true`, hew-guard fails close on missing tests instead of warning |
| `craft.max_function_lines` | `0` | Soft-warn when a changed function exceeds this many lines |
| `craft.warn_on_unused` | `true` | Soft-warn on lint-detected unused imports / dead code |
| `compact.dry_run_default` | `true` | `hew compact apply` starts in dry-run mode |
| `compact.granularity_default` | `"broad"` | Strict vs relaxed clustering prompt |
| `compact.target_clusters_cap` | `6` | Upper bound on `default_k(n)` |
| `compact.allow_recompact_default` | `false` | Drift-guard override |
| `compact.exempt` | `[]` | Literal memory keys never forgotten |
| `branching.strategy` | `"none"` | `none` / `epic` / `always` |
| `research.default` | `"ask"` | `ask` / `auto-skip` / `auto-run` |
| `review.after_n_tasks` | `0` | Fire review picker after N closed tasks |
| `review.after_epic` | `false` | Fire review picker on epic close |
| `review.batch_size` | `8` | Default scope size for `hew review-bundle` |

### Stats

- 20 skills (was 14) + SKILL.md index
- 39 slash commands (was 23)
- 28 craft principles in the v1 catalogue, 4 stacks seeded (`ts-next`, `py-fastapi`, `rust-axum`, `go-echo`)
- 25 test binaries, all green; clippy + fmt clean across the workspace

## [0.1.0] — 2026-05-12

Initial release. Methodology + Rust CLI shipped together.

### Added

- **Methodology** — 14 skill markdown files plus a `SKILL.md` index,
  installed verbatim into the agent runtime's skill directory by
  `hew init`:
  - core: `hew-plan`, `hew-decompose`, `hew-execute`, `hew-guard`,
    `hew-verify`
  - brownfield: `hew-scan`, `hew-convention`, `hew-audit`,
    `hew-boundary`, `hew-migrate`
  - optional: `hew-deps`, `hew-research`, `hew-quick`, `hew-security`

- **Slash commands** — 23 markdown command files for Claude Code:
  `/hew:do`, `/hew:next`, `/hew:auto`, `/hew:plan`, `/hew:work`,
  `/hew:quick`, `/hew:verify`, `/hew:ship`, `/hew:test`, `/hew:add`,
  `/hew:drop`, `/hew:epic`, `/hew:note`, `/hew:ingest`, `/hew:debug`,
  `/hew:forensic`, `/hew:review`, `/hew:status`, `/hew:report`,
  `/hew:doctor`, `/hew:config`, `/hew:help`, `/hew:update`.

- **`hew` CLI** with full subcommand surface:
  - `hew init` — interactive or non-interactive setup; detects Claude,
    Cursor, Codex, Windsurf, or falls back to Generic CLAUDE.md.
    Idempotent on re-runs; preserves user content outside the
    HEW:BEGIN/HEW:END markers in single-file adapters.
  - `hew prime <skill>` — emits the JSON contract agents consume:
    project state, parsed `STATUS:` map, prerequisites, ready task
    list, memories categorized by prefix, embedded skill body.
  - `hew status` — human-readable text by default; `--json` for
    machine consumption.
  - `hew doctor [--fix]` — five health checks (bd present, .beads/
    exists, .gitignore lists .beads/, runtime markers detected, skill
    layout intact).
  - `hew config {get|set|list|reset|path}` — TOML persistence at XDG
    config dir; honors `HEW_CONFIG` for test isolation.
  - `hew schema {prime|config}` — JSON Schema draft 2020-12 export for
    agent validators.
  - `hew update [--check-only|--yes|--local]` — self-update via
    axoupdater. Survives the pre-release no-receipt state gracefully.
  - `hew completions {bash|zsh|fish|power-shell|elvish}` and
    `hew manpage` for shell completion + manpage generation.

- **Global flags** — `--non-interactive`, `--json`, `--quiet`,
  `--output={auto,json,text}`, `--verbose`. Auto-detect
  non-interactive mode on `CI=true`, `HEW_NON_INTERACTIVE=1`, or
  non-TTY `stderr`.

- **Runtime adapters** for Claude Code, Cursor, Codex, Windsurf, and a
  generic CLAUDE.md fallback.

- **Passive update notification** — `hew prime` spawns a background
  GitHub-API check (via `curl`, once per 24h) and surfaces the result
  in the `update_available` field on the next prime call. Disabled
  with `HEW_NO_UPDATE_CHECK=1`.

- **Three example walkthroughs** under `examples/`: greenfield SaaS,
  brownfield feature add, single-bug fix.

- **`templates/codebase-scan.md`** — the prompt template invoked by
  `hew-scan` on brownfield projects.

- **CI** — GitHub Actions workflow running fmt, clippy `-D warnings`,
  test matrix (ubuntu + macos × stable + MSRV 1.91), cargo-audit, and
  cargo-deny on every PR and main push.

- **Release infrastructure** — `[workspace.metadata.dist]`
  configuration for cargo-dist 0.31+ targeting linux x64+arm64, macos
  x64+arm64, and windows x64. `install.sh` curl-installer +
  `dist/hew.rb` Homebrew formula stub. Release workflow placeholder
  triggered on `vX.Y.Z` tags.

- **Pre-commit hooks** — `.githooks/pre-commit` (portable bash, 3.2
  safe) and `.pre-commit-config.yaml` (pre-commit framework). Both run
  `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
  and the test suite.

- **Documentation** — README, ARCHITECTURE.md, CONTRIBUTING.md, MIT
  LICENSE.

### Notes

- 118 tests pass; `cargo fmt --all -- --check` and
  `cargo clippy --all-targets -- -D warnings` clean.
- MSRV pinned at `rust-version = "1.91"` in `Cargo.toml`.
- Methodology distilled from observing patterns and anti-patterns in
  [Beads](https://gastownhall.github.io/beads/),
  [GSD](https://github.com/gsd-build/get-shit-done), and similar
  AI-agent methodologies.

[Unreleased]: https://github.com/droidnoob/hew/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/droidnoob/hew/releases/tag/v0.1.0
