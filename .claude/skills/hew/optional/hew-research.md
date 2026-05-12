<!-- hew:version=0.1.0 -->
---
name: hew-research
category: optional
init: hew prime research
---

# hew-research — Domain / Framework Research

You investigate a topic before the rest of the work loop tries to
build against it. This is for genuinely unfamiliar territory — a new
domain (audio, 3D, ML), an unfamiliar framework, an industry-specific
constraint (HIPAA, PCI-DSS, GDPR). The output is a stack of findings
persisted as memories so future sessions inherit the research.

Not installed by default. Most projects don't need a research phase.
When they do, opt in via `hew config set optional-skills.research true`.

## When this skill runs

- The user invokes `/hew:plan --research <topic>` or `/hew:research <topic>`.
- `hew-plan` flags an architectural decision that needs investigation
  (unfamiliar framework, novel domain) and the user authorizes a
  detour.
- Periodically when entering a new codebase area the agent has not
  worked in before.

## Inputs from `hew prime research`

- `memories.factual` and `memories.dep` — what you already know.
  Don't re-research what's already documented.
- The **topic** — passed in by the caller. Be specific: not "auth"
  but "passwordless email-link auth for B2B SaaS in Next.js 14."

## The research loop

```
1. Pin the topic to a single sentence ("X in context Y")
2. Identify the 3–5 sub-questions that, if answered, would let you proceed
3. For each sub-question:
     a. Search for authoritative sources (docs, RFCs, vendor blogs)
     b. Skim 2–3 results, extract the load-bearing fact
     c. Cross-check with a second source if the claim is non-obvious
     d. Write a bd remember with the finding + citation
4. Surface contradictions you couldn't resolve as open questions for the user
5. Mark research complete
```

Use the agent's web search / fetch tools. Don't fabricate sources;
don't summarize from memory.

## What's a "good" research finding

Findings should be **decision-grade**: specific enough that the next
skill (usually `hew-plan` or `hew-decompose`) can act on them without
returning to the same source.

Good:
```
bd remember "RESEARCH: passwordless email-link auth — recommended TTL 15 min for the link, single-use, redirect to same-origin only. Source: NIST SP 800-63B §5.1.3.2."
bd remember "RESEARCH: Stripe checkout — fixed_price subscriptions support trial_period_days but trials cannot be extended via API; user must cancel + re-subscribe. Source: stripe-docs/checkout/subscriptions#trials, verified 2026-05."
```

Bad:
```
bd remember "RESEARCH: auth is complicated, lots of options."
bd remember "RESEARCH: Stripe has trials."
```

If a finding could have been written without doing any research, it's
not a finding.

## Citation discipline

Every finding includes the source it came from. The verbosity is
worth it:

- If the framework's docs changed, you can re-check the original.
- The user can audit the claim.
- "I read it somewhere" loses to "Source: nextjs-docs/app-router#dynamic, verified 2026-05-10."

Format:
```
RESEARCH:<topic-tag> — <finding>. Source: <url-or-doc-ref>, verified <YYYY-MM>.
```

## Cross-checking

When the finding will drive a major decision, cross-check with a
second independent source. If the two sources disagree, surface that
as an open question — *do not silently pick one*.

## Out-of-scope items

If your research surfaces a constraint that's out of the original
scope (e.g., user asked about Stripe, you discover PCI-DSS
requirements), record it as a finding and **flag it to the user**
before they finalize the plan. Better to expand scope deliberately
than to ship something illegal.

## Output

End with a recap to the user:

```
hew-research: passwordless auth in Next.js 14 — done

Findings: 6 (saved as RESEARCH: memories)
  • Token TTL: 15 min, single-use
  • Storage: server-only (no localStorage)
  • Same-origin redirect required (NIST + OWASP)
  • Next.js 14 supports route handlers for the link callback
  • Rate-limit recommendation: 5 attempts / 15 min / IP
  • PII: don't log the token, hash before storage

Open questions for the user (1):
  • Does the product require remembering devices across the link click?
    Yes → add device fingerprinting + DeviceMemory: persistence story.
    No → simpler path; record decision in DECISION:auth.
```

## Step — mark phase complete (optional)

For research that resolves a planning blocker, write a status memory
the planner can check:

```
bd remember "STATUS:research:<topic>:complete — <ISO-8601 timestamp>"
```

## What you don't do

- **Make up facts.** If a search returns nothing useful, say so. No
  hallucinated APIs, no invented version numbers.
- **Skip citations.** Every finding has a source.
- **Replace `hew-plan`.** Research informs planning; it doesn't
  decide architecture.
- **Wander off the topic.** Stay on the user's request. Surface
  out-of-scope discoveries as flags, not as new research loops.
- **Recommend products / vendors** without checking the project's
  procurement constraints. If the team is on AWS, don't research
  GCP-only services.

## Anti-patterns

- **Sourceless findings** ("RESEARCH: just use X"). Useless and not
  auditable.
- **One-pass research** with no cross-check on a load-bearing claim.
- **Re-researching** something already in `memories`. Check first.
- **Findings the size of a paragraph.** One claim per memory.
- **Findings that don't drive decisions.** If you can't say what
  changes because of the finding, don't record it.
