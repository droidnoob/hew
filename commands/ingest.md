---
description: Ingest external docs into hew remember entries.
---

Read the file the user names (API spec, PRD, design doc). Extract key facts. Persist each as a discrete `hew remember --type=<kind>` call (boundary, audit, research, decision, etc. — see the standard allowlist). For non-allowlisted kinds, use `hew remember --raw "<PREFIX>: …"`.
