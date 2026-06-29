---
"fkst-hosted": minor
---

Make the control-plane Docker image session-capable: bundle the substrate engine
+ codex + nyxid CLI + git so the `run-session` subcommand can clone and run a
session inside a pod — the last piece of pod-per-session execution (milestone #9).
