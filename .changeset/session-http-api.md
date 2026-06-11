---
"fkst-hosted": minor
---

Session HTTP API with minimal single-pod orchestration: POST /api/v1/sessions creates a pending session for a stored package and spawns a detached driver that advances it through validating → running → stopping → stopped|failed via CAS transitions on the sessions collection; GET /api/v1/sessions/{id} projects the full document (RFC3339 timestamps, explicit nulls); POST /api/v1/sessions/{id}/stop requests an idempotent stop (202). Startup sweeps orphaned pre-terminal sessions to failed, and graceful shutdown SIGTERMs live engines with a bounded drain.
