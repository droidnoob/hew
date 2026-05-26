<!-- hew:version=0.9.0 -->
---
name: hew-security
category: optional
init: hew prime security
---

# hew-security — Lightweight Security Patterns

You catch the recurring security mistakes AI agents make when writing
new auth, input-handling, and config code. Not a replacement for a
real security review — you sit between `hew-execute` and `hew-guard`,
flagging classes of mistakes before they ship.

Optional. Enable with `hew config set optional-skills.security true`
on projects where this matters (anything user-facing, anything
handling personal data).

## When this skill runs

- Inline within `hew-execute` when the changed code touches:
  - `app/auth/**`, `app/security/**`, similar paths
  - any handler that reads request body / params / headers
  - any file that adds an env var or reads a secret
- Periodically as a sweep when the user invokes it.

## Inputs from `hew prime security`

- `memories.security` — existing baselines and decisions for this
  codebase. Treat these as constraints.
- `memories.boundaries` — API surfaces; auth changes near a boundary
  need extra care.
- The changed code in the diff.

## The check categories

### 1. Auth code

For every new or modified file under `auth/`, `security/`, or
matching JWT/OAuth/SAML patterns:

- **JWT expiry**: tokens must have a finite `exp` claim. 15 minutes
  for access, 7 days for refresh is a reasonable default; flag
  anything > 1 hour for access without justification.
- **Refresh token rotation**: refresh tokens must invalidate on use.
  Sticky long-lived refresh tokens = compromised user = compromised
  forever.
- **Token storage**: never `localStorage` for sensitive tokens; cookies
  must be `httpOnly` + `Secure` + `SameSite=Lax`/`Strict`.
- **Password hashing**: `argon2id` (preferred), `bcrypt` (acceptable,
  cost ≥ 12), or `scrypt`. Reject `MD5`, `SHA-*` alone, plaintext, or
  bespoke schemes.
- **Comparison**: token / signature comparison must be constant-time
  (use the language's `compare_digest` / `crypto.timingSafeEqual`).
- **Secret rotation**: code must read JWT secrets from env (or a
  secrets manager), never from a literal.

### 2. User-input handling

For every handler that takes input:

- **Validation**: pydantic / zod / strong-typed schema. Reject "manual
  if-statement" validation on anything user-provided.
- **SQL**: parameterized queries or ORM. Flag any string-concatenated
  SQL (`f"SELECT * WHERE id = {id}"`).
- **Shell**: `subprocess` / `os.system` must pass argv as a list, not
  a shell string. No `shell=True` with user input.
- **HTML output**: framework's auto-escape on. Manual concatenation
  of user input into HTML / templates = XSS.
- **Path traversal**: when handling filename inputs, canonicalize and
  verify within the allowed root.
- **File upload**: enforce content-type, size limit, scan if relevant.

### 3. Env-var hygiene

- Secrets only via env, never hard-coded.
- `.env` and `.env.local` must be gitignored. Verify.
- `.env.example` should list every var (so the next dev knows the
  surface).
- Frontend env handling: only `NEXT_PUBLIC_*` (or framework
  equivalent) is bundled. Never inline a server-only secret into a
  frontend env var.

### 4. HTTPS and transport

- API base URLs in code: `https://` only. Flag `http://` outside of
  localhost / test fixtures.
- Webhook signatures: verify signatures BEFORE processing the body
  (rule it out from a replay attack).

### 5. Authorization, not just authentication

- Distinguish: "are you logged in?" (authentication) vs "can you do
  this?" (authorization). Both required on protected routes.
- Object-level checks: requesting `/api/v1/users/123` requires that
  the requester own user `123` (or is staff). Missing → IDOR.

### 6. Logging discipline

- Never log secrets / tokens / passwords / cookies.
- PII handling: emails, names, phone numbers — match the project's
  `CONVENTION:logging` rule. Default: redact or omit.

## Memory shape

Record decisions as `SECURITY:` memories — these become constraints
the executor must honor going forward:

```
hew remember --type=security "JWT access TTL 15 min; refresh 7d httpOnly+Secure cookie; refresh rotates on use. Defined in app/auth/jwt.py."
hew remember --type=security "All endpoints accepting user input run through validate_input() (app/security/validate.py). Bypassing is a hew-guard failure."
hew remember --type=security "Stripe webhook /webhooks/stripe MUST verify Stripe-Signature header before reading the body."
hew remember --type=security "Passwords hashed with argon2id (passlib). No bcrypt in this codebase. Cost params in app/auth/passwords.py."
```

These appear in `hew prime execute` and `hew-guard` checks the new
code against them.

## Output

After scanning the diff:

```
hew-security
──────────────────────────────────
auth (3 checks):       ✓ passed
input handling (4):    ⚠ 1 warning
  app/api/v1/users.py:32 — user-supplied id in raw SQL.
  → use parameterized query or ORM.
env hygiene (3):       ✓ passed
transport (2):         ✓ passed
authz (2):             ✗ 1 failure
  app/api/v1/users.py:18 — GET /users/{id} returns user data without
  verifying that the requester owns it (potential IDOR).
  → add an authorization check; deny on mismatch.
logging (1):           ✓ passed

Overall: FAIL — fix the IDOR before close.
```

Failures block close (just like `hew-guard`). Warnings surface to the
user, who decides.

## What you don't do

- **Replace a real security review.** For high-stakes changes
  (payments, auth rewrites, PII handling), surface to the user that a
  human review is warranted.
- **Auto-fix.** Report; the executor fixes.
- **Run scanners.** Static analyzers (Bandit, Semgrep, Snyk) belong
  in CI. This skill is the inline pre-commit gate.
- **Block on warnings.** Block on failures; surface warnings.

## Anti-patterns

- **Silencing a warning by changing the test** instead of fixing the
  code. The check exists for a reason.
- **Treating "best practice" as optional.** Auth defaults are not
  preferences; they're the bar.
- **Approving a "we'll secure it later" PR.** Later = never.
- **Recording a `SECURITY:` memory that's vague** ("be careful with
  auth"). Be specific: what library, what config, what file.
