---
"fkst-hosted": minor
---

Session runner (engine-integration service): materialize a stored package into a plain temp dir with the 2-key fkst.env, run the conformance pre-flight under a wall-clock cap, spawn fkst-framework supervise in its own process group with a fresh FKST_RUNTIME_ROOT/FKST_DURABLE_ROOT, derive readiness from the empirical stderr markers (half-alive guarded), expose live status / child-log tail / PID liveness, and stop via group SIGTERM with SIGKILL escalation — idempotent, zombie-free, and temp-dir-leak-free on every terminal path.
