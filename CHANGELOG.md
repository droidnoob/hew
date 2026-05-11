# Changelog

All notable changes to this project are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
versioning follows [Semantic Versioning](https://semver.org/).

## [Unreleased]

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
  test matrix (ubuntu + macos × stable + MSRV 1.90), cargo-audit, and
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
- MSRV pinned at `rust-version = "1.90"` in `Cargo.toml`.
- Methodology distilled from observing patterns and anti-patterns in
  [Beads](https://gastownhall.github.io/beads/),
  [GSD](https://github.com/gsd-build/get-shit-done), and similar
  AI-agent methodologies.

[Unreleased]: https://github.com/droidnoob/hew/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/droidnoob/hew/releases/tag/v0.1.0
