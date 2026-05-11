# Contributing to Hew

## Local setup

```sh
# 1. Rust toolchain (MSRV is pinned to 1.91 in Cargo.toml).
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y

# 2. Beads — the runtime dependency for any e2e work.
brew install beads
# or: curl -sSL https://beads.sh/install | sh

# 3. Clone + enable hooks.
git clone git@github.com:droidnoob/hew.git
cd hew
git config core.hooksPath .githooks
```

That last line wires up `.githooks/pre-commit`, which runs `cargo fmt --check`,
`cargo clippy --all-targets -- -D warnings`, and the test suite on every
commit that touches Rust files.

If you prefer the [pre-commit](https://pre-commit.com/) framework, the
repo also ships `.pre-commit-config.yaml`:

```sh
pre-commit install
```

Either is sufficient — pick one, don't run both.

### Escape hatches

- `HEW_SKIP_HOOKS=1 git commit ...` — bypass hooks once. Use sparingly.
- `HEW_HOOK_NO_TESTS=1 git commit ...` — skip the test step but still run
  fmt + clippy.

## Day-to-day workflow

```sh
cargo build                          # 1.x s incremental, ~30s cold
cargo test                           # full suite; ~2s after warm cache
cargo clippy --all-targets -- -D warnings
cargo fmt --all
```

Tests are organised as:

- **Unit tests** alongside each module under `hew-core/src/`.
- **`hew-core` integration tests** under `hew-core/tests/` — drive the
  library against tempdirs and stub `bd` shell scripts.
- **Binary e2e tests** under `hew/tests/` — spawn the compiled `hew`
  binary via `assert_cmd`, hand it a stub `PATH`, and verify
  end-to-end behavior.

Add a test for every behavior change. Snapshots are intentionally
avoided in favor of structural assertions (JSON shape, file presence)
so test churn matches behavior churn.

## Workspace layout

```
hew/
├── Cargo.toml              # workspace manifest + workspace.metadata.dist
├── hew-core/               # all logic — testable, no clap/inquire
│   ├── src/
│   │   ├── bd.rs           # BdClient trait + RealBd subprocess wrapper
│   │   ├── ctx.rs          # Ctx, OutputMode, TTY-aware constructor
│   │   ├── config.rs       # TOML persistence at XDG config dir
│   │   ├── doctor.rs       # 5-check health diagnostics
│   │   ├── error.rs        # HewError + miette diagnostics
│   │   ├── install.rs      # runtime adapters (Claude/Cursor/Codex/...)
│   │   ├── notify.rs       # passive update check (background curl)
│   │   ├── prime.rs        # hew prime JSON contract assembly
│   │   ├── skills.rs       # include_str! skill registry
│   │   ├── slash.rs        # include_str! slash command registry
│   │   ├── status.rs       # text + JSON status rendering
│   │   └── tty.rs          # IsTerminal + env-based non-interactive detect
│   └── tests/              # integration tests against stub bd
├── hew/                    # the thin binary
│   ├── build.rs            # vergen build metadata
│   ├── src/
│   │   ├── main.rs         # 8 lines — parse, dispatch
│   │   ├── lib.rs          # tracing init + run()
│   │   ├── cli.rs          # clap Cli + Command enum + global flags
│   │   └── commands/       # one file per subcommand
│   └── tests/              # binary e2e
├── skills/                 # 1 SKILL.md + 14 skill markdown files
├── commands/               # 23 slash-command markdown files
├── templates/              # codebase-scan template
├── examples/               # 3 walkthrough narratives
├── .github/workflows/      # ci.yml + release.yml
└── .planning/              # the project's own Beads-graph history
```

Keep `hew/` thin. Anything that isn't presentation, arg-parsing, or
process bootstrap belongs in `hew-core/`.

## Coding conventions

- **Errors**: `thiserror` enum (`HewError`) in `hew-core/src/error.rs`,
  derive `miette::Diagnostic` with explicit `code()` and `help()`.
- **Subprocess**: never shell-interpolate. Always pass `Vec<OsString>`
  args, `.stdin(Stdio::null())`, `wait_timeout` for bounded calls.
- **TTY**: check **stderr** for "is a human watching," not stdout.
- **Logging**: `tracing` to stderr; never stdout (which is the agent
  contract for `hew prime`).
- **Output**: `Auto` mode collapses to text (human default); JSON is
  opt-in via `--json` or `--output=json`. `hew prime` always emits JSON
  regardless of flag.

These mirror the `CONVENTION:` memories the methodology itself
captures — useful to read those too (`bd memories`).

## Adding a new skill or slash command

1. Drop the markdown file in `skills/<category>/<name>.md` or
   `commands/<name>.md`. Use the existing files as the template — they
   all start with `<!-- hew:version=X.Y.Z -->` and a frontmatter block.
2. Add the entry to `hew_core::skills::CORE / BROWNFIELD / OPTIONAL`
   (or `slash::ALL`) so `include_str!` picks it up.
3. The drift test (`hew-core/tests/skills.rs`) will fail if the file
   exists but the registry hasn't been updated, and vice versa.

## Releases

Releases are cut by tagging `vX.Y.Z` on `main`. Before the first real
release:

1. Run `dist init` (the cargo-dist binary; install via `cargo install
   cargo-dist` or grab from <https://opensource.axo.dev/cargo-dist/>) to
   regenerate `.github/workflows/release.yml`. The current file is a
   placeholder.
2. Confirm `workspace.metadata.dist` in `Cargo.toml` lists the right
   targets, installers, and tap.
3. Tag and push: `git tag v0.1.0 && git push origin v0.1.0`.

CI runs on every PR and main push (fmt / clippy / test matrix on
ubuntu+macos × stable+MSRV / cargo-audit / cargo-deny). Release
artifacts come from the tagged release workflow.

## Submitting changes

- Open a PR against `main`. CI must be green.
- Prefer atomic commits with conventional-commit subjects
  (`feat(scope):`, `fix(scope):`, `docs:`, `refactor:`, `test:`,
  `chore:`).
- Reference the Beads issue ID in the body if applicable (`Closes
  bd-X.Y` or similar).
- Don't push generated files (`target/`, `Cargo.lock.bak`, etc.); the
  `.gitignore` already covers them.

## Code of conduct

Be kind. Disagree on technical content, not people. If something feels
off, open an issue or DM the maintainer.

## Licence

MIT — same as the rest of the repo. By contributing you agree your
work ships under the same terms.
