# AGENTS.md

Multi-agent guidance for this repo. **The canonical source is [CLAUDE.md](./CLAUDE.md)** — read it first. This file exists so non-Claude agents (Cursor, Codex, Windsurf, Gemini CLI, etc.) can find the same instructions when they look for `AGENTS.md` per the [agents.md convention](https://agents.md).

## tl;dr

→ See [CLAUDE.md](./CLAUDE.md) for the full agent contract:

- Project shape (Cargo workspace: `hew-core` + `hew`; methodology in `skills/`; slashes in `commands/`)
- Branching rules (`main` is protected; use `hew branch new --prefix=<type> --slug=<text>`)
- Build / test / lint commands + pre-commit hook contract
- Memory prefix taxonomy
- Hard-won gotchas (test-count drift, pipe-deadlock, zsh heredoc, clippy traps)
- Locked behavioral preferences (`FEEDBACK:no-json-piping`, `FEEDBACK:prefer-hew-over-bd`, …)
- Locked architectural decisions
- Release process

## Agent-specific notes

**Cursor / Windsurf** — these runtimes don't currently run `.githooks/pre-commit`. Run `cargo fmt --all` + `cargo clippy --all-targets -- -D warnings` + `cargo test` manually before staging, or rely on CI to catch.

**Codex** — `hew init --runtime=codex` writes per-skill TOML files under `.codex/agents/` plus its own `AGENTS.md` section between `HEW:BEGIN` / `HEW:END` markers. User content outside those markers is preserved across re-installs.

**Generic (CLAUDE.md fallback)** — `hew init --runtime=generic` writes a single bundled `CLAUDE.md` for runtimes without per-skill discovery. Note this is *different from* the project-level `CLAUDE.md` you're being pointed to; the generic adapter file lives elsewhere and is auto-generated.

## Why two files

`CLAUDE.md` is project-specific guidance maintained by humans (and well-behaved agents) for whoever / whatever picks up the repo. It contains the substance.

`AGENTS.md` is a pointer file so non-Claude agents looking for the conventional discovery path land on the same content. Both files are committed; both are checked when changes affect agent guidance. **Update `CLAUDE.md` first.** This file should rarely change.
