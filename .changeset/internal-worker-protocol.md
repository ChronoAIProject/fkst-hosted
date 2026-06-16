---
"fkst-hosted": minor
---

Add the internal controller ↔ worker protocol — the two-process skeleton the database-free pivot builds on (#134). The role-neutral wire vocabulary (registration, heartbeat with a lifecycle state, work-pull, and the full drain message set) lives in `fkst-shared`; the controller (`fkst-control-plane`) gains an in-memory worker registry + a shared-secret-guarded `/internal/v1/*` router with a stale-worker expiry sweeper, mounted only when `FKST_INTERNAL_AUTH_TOKEN` is set (closed by default); the worker (`fkst-worker`) becomes a real runtime that registers up to `CONTROLLER_URL`, heartbeats, runs a (no-op) pull loop, and serves a local `/health`. No claim/placement authority (that is #135) and no drain behaviour (that is #140) — this ships only the versioned, authenticated transport. The worker still links neither the mongodb driver nor the control-plane (the compiler-enforced boundary).
