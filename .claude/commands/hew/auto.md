---
description: Run plan -> decompose -> execute -> verify autonomously.
---

Walk the entire workflow without further user input until either:
- the `hew status` ready list is empty (call /hew:verify, then report done), or
- a Rule-4 architectural change blocks you (stop, surface, wait).

Honor all guard / deviation / convention rules. Atomic commits per task.
