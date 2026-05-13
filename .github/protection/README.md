# Branch protection for `main`

The local pre-commit hook (`.githooks/pre-commit`) refuses commits on `main`. The matching **server-side** protection — refuse pushes to `main`, require PRs, require CI to pass — lives here as a ruleset JSON ready to apply.

## When to apply

GitHub's branch-protection and ruleset APIs are gated behind GitHub Pro / Team / Enterprise for **private** repos. The free plan allows them on **public** repos.

Apply the moment one of these flips:

- The repo becomes public, or
- The org / owner upgrades to GitHub Pro or higher.

## How to apply

```sh
gh api -X POST repos/droidnoob/hew/rulesets \
  -H "Accept: application/vnd.github+json" \
  --input .github/protection/main-ruleset.json
```

Verify:

```sh
gh api repos/droidnoob/hew/rulesets | jq '.[] | {id, name, enforcement, target}'
```

## What the ruleset enforces

| Rule | Behavior |
|------|----------|
| `deletion` | `main` cannot be deleted |
| `non_fast_forward` | No force-pushes / history rewrites to `main` |
| `pull_request` | Every change to `main` lands via PR. `required_approving_review_count: 0` means PRs can self-merge — solo-dev friendly while still enforcing the PR boundary |
| `required_status_checks` (strict) | All 8 CI contexts must pass before merge: `rustfmt`, `clippy`, `test (ubuntu-latest / stable)`, `test (ubuntu-latest / 1.91)`, `test (macos-latest / stable)`, `test (macos-latest / 1.91)`, `cargo-audit`, `cargo-deny` |
| `bypass_actors: []` | No admin bypass — even the maintainer goes through PRs |

## How to update

If a new required CI check lands, add it to `required_status_checks` in `main-ruleset.json` then re-apply with `gh api -X PUT repos/droidnoob/hew/rulesets/<id>` (the ruleset id from the `gh api .../rulesets` listing).

## Status

Currently **not applied** (repo private + free plan). Local pre-commit hook is the only enforcement until this flips.
