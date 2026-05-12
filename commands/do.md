---
description: Route freeform user input to the right hew skill.
---

Take the user prompt that follows this invocation, decide which skill is
appropriate, and invoke it.

## Routing table

| User intent | Route to |
|-------------|----------|
| "let's build / plan X" | `hew-plan` |
| "break this down / create tasks" | `hew-decompose` |
| "start coding / what's next" | `hew-execute` |
| "fix this one bug / tiny tweak" | `hew-quick` |
| "did we finish / verify" | `hew-verify` |
| "scan this codebase / map this repo / it's an existing project" | brownfield chain (see below) |
| "check health / what's wrong" | `/hew:doctor` (use the binary, not the skill) |

If the intent is unclear, run `hew prime execute` first and let the
ready list inform you.

## Brownfield chain — run all four skills, don't stop between them

When the user says "scan", "map this codebase", "existing project",
or anything that maps to brownfield onboarding, run the **full
chain**: `hew-scan → hew-convention → hew-audit → hew-boundary`.

Run them back-to-back. **Do not pause to ask "continue?" between
steps.** The user already invoked the chain by saying "scan this
codebase"; asking for permission to do each step defeats the
chain. The only legitimate reasons to pause mid-chain are:

- Rule-4 architectural surprise that needs human direction.
- The user explicitly said "scan only" (just the first step).
- A skill prerequisite fails (e.g., `hew-convention` refuses because
  `STATUS:scan` is missing — that's a real error, not a pause).

When the chain completes, write a single summary block to the user
listing all four `STATUS:` markers set. Then either hand back (if
the user only asked to onboard) or continue into `hew-plan` if they
asked to "scan and plan a feature".

## Hand-off shape

After routing, invoke the skill's body directly. Don't ask the user
to confirm a routing decision unless the input is genuinely
ambiguous. Tell them the route in one line ("Routing to hew-plan."),
then start working.
