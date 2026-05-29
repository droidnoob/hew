# Architecture

This document is for future maintainers (and future-you). It explains
why the code is shaped the way it is, where the load-bearing pieces
live, and what to be careful about when changing them.

## The thesis

Hew exists because GSD's failure modes — markdown state, no crash
recovery, 4:1 token overhead, 41 workflow files for one agent — all
trace back to **the LLM is the source of truth for project state**. Hew
inverts that: Beads is the source of truth, the LLM is a process that
queries the truth.

Every architectural choice below serves that thesis. When in doubt:
ask "where does this state live?" If the answer is "in a markdown
file the LLM has to parse," reconsider.

## Top-level shape

```
┌──────────────────────────────────────────────────────────────┐
│ Agent runtime (Claude Code / Cursor / Codex / Windsurf)      │
│   ├── invokes skill (markdown file installed by `hew init`)  │
│   └── calls `hew prime <skill>` to get state + skill body    │
└──────────────────────────┬───────────────────────────────────┘
                           │ JSON over stdout
                           ▼
┌──────────────────────────────────────────────────────────────┐
│ `hew` binary (this repo)                                     │
│   • thin clap layer in hew/                                  │
│   • all logic in hew-core/                                   │
└──────────────────────────┬───────────────────────────────────┘
                           │ subprocess (OsString args, stdin null)
                           ▼
┌──────────────────────────────────────────────────────────────┐
│ `bd` binary (Beads)  •  Dolt under the hood                  │
└──────────────────────────────────────────────────────────────┘
```

Three boundaries, each with a tightly-scoped contract:

1. **Agent ⇄ hew**: JSON. The schema is the one `hew schema prime`
   emits, derived via `schemars` from `hew_core::prime::PrimeOutput`.
2. **hew ⇄ bd**: stdout JSON when available; text fallback parsed
   permissively. The `BdClient` trait abstracts this.
3. **bd ⇄ Dolt**: opaque to us. We do not depend on Dolt directly.

## Workspace split

```
hew-core/    library, all logic, easy to unit test
hew/        thin binary, clap + tracing init + dispatch
```

`hew-core` has zero dependency on clap, inquire, or any presentation
crate. The split exists so that:

- Tests can drive the library directly without spawning processes.
- A future second consumer (TUI, plugin) can link `hew-core` without
  inheriting the CLI's deps.
- The clippy / fmt rules can be tightened per crate without affecting
  the other.

`hew/src/main.rs` is intentionally ~8 lines. Anything else is bloat.

## The `BdClient` trait — the only mutable boundary

`hew_core::bd::BdClient` is the single seam between us and Beads:

```rust
pub trait BdClient: Debug {
    fn version(&self) -> Result<BdVersion>;
    fn ready(&self) -> Result<Vec<ReadyTask>>;
    fn stats(&self) -> Result<StatsSummary>;
    fn prime_raw(&self) -> Result<String>;
    fn memories(&self) -> Result<BTreeMap<String, String>>;
    fn remember(&self, text: &str) -> Result<()>;
    fn run_raw(&self, args: &[&OsStr]) -> Result<BdOutput>;
}
```

Two implementations:

- **`RealBd`** — shells out via `std::process::Command`. Resolved once
  via `which::which("bd")` at startup, cached. Always passes
  `Vec<OsString>` args (no shell interpolation), always
  `.stdin(Stdio::null())` (so `bd` never waits on a TTY), and bounds
  every invocation with `wait_timeout` (default 30s, soft-kill then
  hard).
- **`FakeBd`** / **`MissingBd`** — in tests. Some tests don't even
  need a fake — they write a shell script to a tempdir, set
  `PATH` to that dir, and let `RealBd::discover()` find it.

JSON shapes are decoded **permissively**: every typed struct uses
`#[serde(default)]` on its fields so a newer `bd` adding fields does
not break us. The `memories()` impl explicitly filters
`schema_version: 1`-style metadata out of the otherwise-`{String: String}`
shape — a real bug we hit during integration.

## Registries — `skills` and `slash`

Both `hew_core::skills` and `hew_core::slash` are **compile-time
registries**:

```rust
// hew-core/src/skills.rs
pub const CORE: &[Skill] = &[
    skill!("hew-plan", Category::Core, "core/hew-plan.md"),
    // ...
];

macro_rules! skill {
    ($name:expr, $cat:expr, $relpath:expr) => {
        Skill {
            name: $name,
            relative_path: $relpath,
            category: $cat,
            body: include_str!(concat!("../../skills/", $relpath)),
        }
    };
}
```

`include_str!` reads the markdown at compile time, so the binary ships
with the methodology baked in. There is no runtime file resolution for
the canonical bodies.

There is a paired **drift test** in `hew-core/tests/skills.rs` that
walks the on-disk `skills/` tree and asserts every file is registered
and vice versa. The same pattern guards `slash::ALL`.

**To add a new skill or command:**

1. Add the markdown file.
2. Add the entry to the registry.
3. Build — the drift test catches you if you forget either.

## The prime contract

`hew_core::prime::PrimeOutput` is the JSON contract every agent
consumes. Adding fields is backwards-compatible (skip-if-none); renaming
or removing is not.

```rust
pub struct PrimeOutput {
    schema_version: u32,
    skill: String,
    project: ProjectInfo,
    status: StatusMap,          // STATUS:phase memories, parsed into structured map
    prerequisites: Prerequisites,
    tasks: TaskInfo,
    memories: MemoryBuckets,    // categorized by prefix
    skill_instructions: String, // the skill body
    update_available: Option<UpdateAvailable>,
}
```

The schema is exported via `hew schema prime` (using `schemars` derive
macros). Consumers can validate against draft 2020-12.

`prime::build()` is intentionally tolerant of partial bd failures —
`unwrap_or_default()` on stats / ready / memories — so a flaky bd
shouldn't break agent context entirely. The contract's "missing
prerequisites" path is the canonical failure surface.

## Memory prefix taxonomy

The executor (and every other skill) routes on memory prefixes:

| Prefix | Treatment |
|--------|-----------|
| `STATUS:` | parsed into the `status` map (not surfaced as memory bucket) |
| `CONVENTION:` | constraint — executor must honor |
| `BOUNDARY:` | check before changing shared interfaces |
| `AUDIT:` | dep health, may open tasks |
| `SECURITY:` | check on auth/input code |
| `MIGRATION:` | check on model changes |
| `DEP:` | informational |
| `PROJECT:` | project-level facts (target user, scale, deployment, constraints). Written by `hew-new-project`. |
| `MILESTONE:` | current milestone identity + acceptance window. Written by `hew-new-project`; updated by milestone-close events. |
| `ROADMAP:` | the milestone chain (ordered list of epics). Written by `hew-new-project`. |
| `RESEARCH:` | investigation findings with provenance tags `[VERIFIED]` / `[CITED]` / `[ASSUMED]`. Written by `hew-research` and the Phase-B research threads of `hew-new-project`. |
| (none) | factual codebase knowledge |

`prime::categorize()` is the canonical implementation. Don't fan this
logic out — keep additions there.

### Memory-write allowlist

`hew remember --type=<x>` validates `<x>` against
`hew_core::tasks::MEMORY_PREFIXES` and rejects unknown values. The
13-entry allowlist mirrors the bucket taxonomy above (plus `gotcha`,
`decision`, `factual`). Non-allowlisted prefixes (e.g. `CHECKPOINT:`,
`SPEC:`, `DEBUG:`, `NOTE:`) write through `--raw` — the escape hatch.

### Stack-conventions data layer

Used by `hew-new-project`'s Phase C synthesis to seed
`CONVENTION:<key>` memories from the chosen tech stack.

- Source: `skills/data/stack-conventions.toml`
- Embedded at compile time via `include_str!` in
  `hew_core::stacks::EMBEDDED`.
- API: `stacks::load()` / `stacks::find(stack_id)` / `stacks::ids()`.
- Schema: `hew schema stacks` (schemars-derived from
  `StackTable` → `Stack` → `StackConvention`).
- Drift: 9 unit tests assert parse, seed presence, id uniqueness,
  convention-key uniqueness per stack, minimum-3-conventions
  guard, non-empty bodies.

Seed entries (v0.1): `ts-next`, `py-fastapi`, `rust-axum`, `go-echo`.
Adding a stack requires only a new `[[stacks]]` table in the TOML;
no Rust code changes. Removing a stack is a breaking change for
projects that bootstrapped against it.

### Craft-principles data layer + soft-warning model

The craft system is the methodology's quality dial: principles like
SOLID, DRY, KISS, Clean Architecture, and Idempotence are catalogued
once and selected per project. The architecture has three loosely
coupled layers.

**1. The catalog** (`hew_core::craft`).

- Source: `skills/data/craft-principles.toml`. v1 ships 28 entries
  across four categories (`code-level`, `architecture`,
  `reliability`, `rule-set`).
- Each entry: stable kebab-case `id`, `name`, `category`, `summary`,
  `when_to_apply`, `when_not_to_apply`, optional `conflicts_with`,
  `example`, and `default_for_stacks` (which seeded stack ids
  preselect this principle in the picker).
- Embedded at compile time via `include_str!` in
  `hew_core::craft::EMBEDDED`.
- API: `craft::load()` / `craft::find(id)` / `craft::ids()` /
  `craft::for_stack(stack_id)`. The last one drives the picker's
  defaults — `hew-new-project` Phase C reads it; brownfield
  `hew-convention` Step 11 reads it for suggestion-style nudges.
- Schema: `hew schema craft-principles` (schemars-derived from
  `CraftTable` → `CraftPrinciple` → `CraftCategory`).
- Drift: ~12 unit tests assert parse, kebab-case id discipline, id
  uniqueness, `conflicts_with` reference validity, breadth across
  categories, `consistency-with-existing-code` defaulting on every
  stack (the brownfield-deference invariant), and signature
  principles being present.

Adding a principle is a TOML-only change. Removing one is breaking
for any project that wrote a `CONVENTION:craft.<id>` against it.

**2. The membership memory** (`CONVENTION:craft.<id>`).

A project's *chosen* principles live as memories — not as config —
because the choice is part of the project's identity, not the agent's
runtime tuning. Three entry points populate them:

- `hew-new-project` Phase C (greenfield picker, defaults from
  `for_stack`).
- `hew-convention` Step 11 (brownfield extraction from layering,
  function-length distribution, test-to-source ratio, style
  fingerprints).
- `hew-plan`'s Craft refinement step writes
  `DECISION:craft-feature:<plan-id>` for per-plan adds/relaxes; these
  are *scoped* to the plan id and don't bleed across features.

The executor and reviewer read both layers when deciding what's in
force for a given task.

**3. The enforcement surface** (`hew_core::guard`).

`craft_warnings(memories, diff, cfg) -> Vec<CraftWarning>` is the
sole programmatic surface. It's a pure function — takes the memories
map, a unified diff, and the loaded `Config`; no bd / git calls —
which makes the heuristics trivially unit-testable on synthetic
diffs. The hew-guard skill calls it on the staged diff and prints
warnings under the seven hard checks.

Three heuristics ship today, each gated differently:

| Rule              | Gate                                  | Severity                              |
|-------------------|---------------------------------------|---------------------------------------|
| `missing-tests`   | always-on                             | `Warn`; `Fail` iff `testing.require`  |
| `function-length` | `craft.max_function_lines > 0`        | `Warn`                                |
| `duplication`     | `CONVENTION:craft.dry` memory present | `Warn`                                |

Per `DECISION:craft-enforcement` (memory), hew-guard does **not**
block close on craft warnings on its own — the only current promotion
path is `testing.require = true` lifting `missing-tests` from
`Warn` to `Fail`, and even then the executor decides whether to
refuse the close. The contract is "surface, don't block."

`Severity::Warn` vs `Severity::Fail` and the `CraftWarning` struct
(`rule`, `severity`, `file`, `line`, `message`, `silence`) are
schemars-derived so future callers (review bundles, CI surfaces) can
deserialize without ad-hoc parsing.

**Methodology flow.**

`CONVENTION:craft.*` threads through every skill in the loop:

```
new-project    → CONVENTION:craft.* picked (Phase C)
hew-convention → CONVENTION:craft.* extracted (Step 11, brownfield)
hew-plan       → DECISION:craft-feature:<plan-id> deviations
hew-decompose  → Tests + Craft lines per task description (Step 3)
hew-execute    → Step 5a inline craft check before writing
hew-guard      → soft warnings on the staged diff
hew-verify     → 5th dimension Maintainability across batch
hew-review     → Craft pillar walks picked principles
hew-adversarial-review → attacks gaps left by unpicked principles
```

Soft enforcement is deliberate: drift is surfaced everywhere it can
appear, but only the project decides which gates promote to hard
blocks (currently only `testing.require`). New per-rule promotion
keys (e.g., `craft.max_function_lines_require`) would slot into
`CraftConfig` the same way.

## Runtime adapters

`hew_core::install::install(runtime, root)` writes the skill tree (and
slash commands, for Claude) into the runtime's expected layout. Five
adapters:

- **Claude** (`.claude/skills/hew/{core,brownfield,optional,custom}/` +
  `.claude/commands/hew/`)
- **Cursor** (`.cursorrules` with `HEW:BEGIN/HEW:END` marker section)
- **Codex** (`.codex/agents/hew-*.toml` per-skill + `AGENTS.md`)
- **Windsurf** (`.windsurfrules` with marker section)
- **Generic** (single bundled `CLAUDE.md`)

Single-file adapters (Cursor / Windsurf / Codex's `AGENTS.md`) use
`upsert_marked_section()` — idempotent inject-or-replace between
marker comments. User content outside the markers is preserved across
re-installs.

The Claude layout is canonical because it has the richest semantics
(per-file skill discovery + per-file slash commands). Other adapters
collapse the structure into whatever the runtime accepts.

## Non-interactive discipline

Every command supports non-interactive mode. Precedence:

1. `--non-interactive` flag (caller sets `force_non_interactive=true`)
2. `HEW_NON_INTERACTIVE=1`
3. `CI=true`
4. `stderr` is not a TTY

Note: **`stderr`**, not `stdout`. stdout is commonly piped to `jq` even
when a human is watching; stderr is the better signal.

In non-interactive mode:
- Every interactive prompt must have a corresponding override flag.
- Missing required values fail loudly with a `MissingFlag` error that
  names the flag.
- No `inquire` calls ever execute.

This is enforced in `hew_core::ctx::Ctx::new()` and observed by every
command via `ctx.interactive`.

## Update flow

Two independent mechanisms:

- **`hew update`** — synchronous, user-invoked. Wraps `axoupdater`
  (with the `blocking` feature) to self-update from GitHub releases.
- **Passive notification** — `hew_core::notify::schedule_if_stale()`,
  fired by `hew prime` once per 24h. Spawns a background thread that
  hits the GitHub API via `curl`, caches the result, and includes it
  in the next `prime` output via `update_available`. Disabled with
  `HEW_NO_UPDATE_CHECK=1`. Uses `HEW_CACHE_DIR` for test isolation.

The cache file lives at `<XDG_CACHE>/hew/update-available`. Failure is
silent — network down, GitHub rate-limited, none of it surfaces as an
error.

## Build metadata

`hew/build.rs` uses `vergen` to populate `VERGEN_BUILD_TIMESTAMP`,
`VERGEN_RUSTC_SEMVER`, and `VERGEN_CARGO_*` env vars at compile time.
These feed `cli.rs::LONG_VERSION`, which `hew --version` displays.

We deliberately do **not** pull in `git` instructions (`gitcl` feature)
because it requires the source tree to be a git repo at build time and
breaks `cargo install` from crates.io. Releases bake the version into
the binary via `CARGO_PKG_VERSION`.

## CI + release

- **`.github/workflows/ci.yml`** runs on every PR and push to main:
  fmt → clippy `-D warnings` → test matrix (ubuntu+macos × stable+1.91
  MSRV) → cargo-audit → cargo-deny.
- **`.github/workflows/release.yml`** is a placeholder. Real releases
  require running `dist init` (cargo-dist 0.31+) to expand the
  generated workflow.
- **`Cargo.toml [workspace.metadata.dist]`** is already configured for
  cargo-dist when it runs.

## Things to be careful about

- **Don't break the prime JSON schema** without bumping
  `schema_version` and giving consumers time to adapt.
- **Don't shell-interpolate** anywhere — use `Vec<OsString>`. Always.
- **Don't read training-data version numbers** when adding deps. Run
  `hew-deps` first (the methodology bites back when ignored).
- **Don't introduce `dialoguer`** as a parallel prompt path; `inquire`
  is the chosen primitive. Two libraries doing the same job is the
  start of drift.
- **Don't widen `BdClient`** without thinking about the mock surface.
  Every method is a thing a fake must implement.
- **Don't fan out memory categorization**; keep additions in
  `prime::categorize()`.

## Loop runner — `hew loop`

The autonomous outer harness, added in 0.9.0. Process-level loop that
drains the bd ready queue across many tasks without staying in one
chat session. Architecturally:

- **Per-iter spawn.** Each iter is a fresh `claude -p` subprocess
  invoked via `hew_core::runtime::ClaudeSpawner` (the
  `RuntimeSpawner` trait — `MockSpawner` for tests). Why fresh: no
  context bloat across iters, prompt cache hits are observable, and
  Ctrl+C never strands an in-memory state machine.
- **Cache-disciplined prompt.** `hew_core::prompt::assemble(skill_body,
  primer, task)` produces a byte-stable prefix (skill body + primer
  captured once at run start via `bd.prime_raw()`) plus a per-iter
  tail (task brief). The Anthropic prompt cache hits the prefix
  across iters; `prompt_prefix_hash` is logged so cache stability is
  observable.
- **Backpressure gate.** `hew_core::backpressure::evaluate` is pure;
  `hew::commands::loop_cmd::CargoGateRunner` shells `cargo test` +
  `cargo clippy`. On `Verdict::Fail` the runner runs `git reset
  --hard <pre-iter-sha>` and files a `STATUS:loop-iter-failed`
  memory.
- **Stop-signal precedence.** `hew_core::stop_signals::Collector`
  gathers CancelFlag (SIGINT), stop-file, wall clock, token budget;
  `hew_core::runner::StopSignals::evaluate` applies precedence
  rules to produce a `StopReason`.
- **Decision resolution.** `hew_core::decide::resolve` walks
  memory → code → research; under `--unattended` the loop polls
  bd for new `DEFERRED:` memories after each iter and converts
  them to `DECISION:` via `BdDecisionContext`.
- **Per-run logs.** Atomic temp-file-then-rename writes under
  `.hew/loop/<run-id>/run.json` + `iter-NNN.json`. `IterLog` shape
  in `hew_core::loop_log`; aggregate Summary in
  `hew_core::loop_summary` (rendered post-run). Parallel runs
  additionally write `manifest.json` at the run-dir root and slot
  per-worker logs under `worker-<n>/`.
- **Parallel dispatch (Epic C).** `--jobs N >= 2` switches the runner
  to a per-worker-worktree dispatcher path. `hew_core::dispatcher`
  owns the slot machine (atomic `bd update --claim` per fill);
  `hew_core::worktree` lays down + prunes worktrees under
  `~/.hew/wt/<run-id>/<n>/` (out-of-tree per
  `DECISION:loop-worktree-location`); `hew_core::merge_back`
  consolidates each `loop/<run-id>/w<n>` branch onto launch HEAD at
  shutdown, files `[merge-conflict]` bug tasks on conflict, and
  leaves the conflicted worktree on disk for hand-resolution.
  `hew loop prune-worktrees` GC's orphans whose run-dir has no
  active `stop_reason = None` entry (active-set check via
  `hew_core::loop_log::active_run_ids`).

The CLI layer in `hew/src/commands/loop_cmd.rs` is a thin wiring
shim: `run_loop_with(ctx, args, bd, spawner, gate, project_root)` is
the testable inner — `hew/tests/loop_backpressure.rs` exercises the
rollback, unattended-resolve, out-of-band-closure-detection, and
prefix-hash-invariant paths against a tempdir git repo without
burning real API tokens. `hew/tests/loop_parallel_e2e.rs` +
`hew/tests/loop_prune_e2e.rs` cover the dispatcher path end-to-end
with a stub spawner + planted orphan worktrees.

Full design + troubleshooting in [`docs/LOOP.md`](./docs/LOOP.md).

## Open questions / future shape

- Multi-agent: Beads supports concurrent claims (`bd update --claim`
  is atomic) but the methodology assumes one agent. Whether Hew gains
  multi-agent affordances or stays single-agent is undecided.
- `hew schema` covers `prime` and `config`. Adding `status` and
  `doctor` schemas is mechanical when needed.
- The Codex adapter writes TOML files because that's the Codex
  convention as of v0.1; if Codex moves to plain markdown, the adapter
  collapses to the marker-section pattern.
