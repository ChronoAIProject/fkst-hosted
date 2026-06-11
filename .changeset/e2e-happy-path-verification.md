---
"fkst-hosted": minor
---

End-to-end happy-path verification (v1 MVP exit criterion): `scripts/e2e/run-e2e.sh` drives a deployed fkst-hosted through health → create the `e2e-minimal` package → start a session → poll to running → stop → poll to stopped, as a POSIX curl+jq black-box client with per-phase exit codes and idempotent re-runs (409-on-create is success). The engine-runnable fixture lives at `backend/tests/fixtures/e2e-minimal/` and is shared byte-identically with the new `e2e_happy_path` integration test, which composes testcontainers MongoDB, the real router/session service, and the real bundled engine in-process.
