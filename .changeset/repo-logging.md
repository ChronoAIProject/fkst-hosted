---
"fkst-hosted": minor
---

The run-session pod now writes its session log to `.fkst/log/<run_key>.log` and
commits checkpoints to a dedicated `fkst/session-<id>` branch (via git plumbing,
isolated from the engine working tree), replacing the removed fkst-journal
(milestone #9).
