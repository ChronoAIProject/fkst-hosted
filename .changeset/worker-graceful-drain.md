---
"fkst-hosted": minor
---

Worker graceful drain on SIGTERM (#194, #140a). On the shutdown signal the worker now stops pulling new work, announces `Draining` to the controller, and checkpoints + cleanly stops every in-flight session within a bounded `worker_drain_grace_secs` (env `FKST_WORKER_DRAIN_GRACE_SECS`, default 25) — emitting a `Released` per session so the controller can reassign — instead of the previous abrupt placeholder. The drain reuses the existing `stop_session` path (whose supervise loop already journals `finish(Stopped)` as the checkpoint flush and sends `Released`), runs while heartbeats continue so the controller keeps seeing the worker and receives each `Released` mid-handoff, and never hangs on a wedged engine (the per-session stops fan out under a single grace deadline, falling through to the bounded reap backstop). Worker stays database-free.
