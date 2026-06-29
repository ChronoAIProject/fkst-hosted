---
"fkst-hosted": minor
---

Add `PodSessionLauncher`: creates one per-session Kubernetes Secret + Job
(run-session mode, 0400 creds mount, ownerReference cascade, at-most-one-Job
idempotency) — the spawn half of pod-per-session execution (milestone #9).
