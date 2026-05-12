---
description: Systematic debugging with persistent state.
---

Open a debug session. Persist hypotheses as `hew remember --raw "DEBUG:<topic>: <observation>"` (`DEBUG:` isn't in the standard `--type` allowlist; `--raw` is the escape hatch). Run experiments. If session dies, next session picks up from the DEBUG: memories.
