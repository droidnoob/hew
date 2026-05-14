# Changelog

All notable changes to this project are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed

- **Codex adapter: malformed `AgentRoleToml` schema** (#13). The
  `.codex/agents/hew-*.toml` emitter wrote `name` + `category` +
  `body`, none of which Codex's `AgentRoleToml` accepts. Codex
  silently dropped all 20 hew roles at startup. Emitter now writes
  the correct shape (`name` + `description` + `developer_instructions`)
  and uses TOML literal multi-line strings so regex escapes (`\s`,
  `\b`) pass through untouched.

### Added

- **Codex adapter: skills emitter** (#13). `hew init --runtime=codex`
  also writes `.agents/skills/hew-<name>/SKILL.md` per skill —
  Codex's auto-discovered skill primitive. Hew methodology is now
  natively invokable in Codex chat, not just spawn-able as a sub-agent
  role. File count emitted by `Runtime::Codex` install bumps 21 → 41
  (20 roles + 20 SKILL.md + AGENTS.md).

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
