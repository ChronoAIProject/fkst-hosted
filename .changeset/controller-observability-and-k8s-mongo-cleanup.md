---
"fkst-hosted": minor
---

Controller observability endpoints (`/api/v1/admin/state` + `/metrics`) and k8s Mongo-manifest cleanup (issue #144).

Adds two observability surfaces to the datastore-free controller (`fkst-control-plane`): `GET /api/v1/admin/state` (admin-gated on `fkst:admin`) returns the live in-memory state — claims (from the `ClaimMap`), workers (from the `WorkerRegistry`, with heartbeat age + controller-authoritative load + liveness), and sessions (from the in-memory `SessionRepo`) — with secrets reported as PRESENCE booleans only, never serialized; `GET /metrics` (unauthenticated, like `/health`) hand-renders the Prometheus gauges `fkst_pending_work`, `fkst_workers_registered`, and `fkst_workers_alive` with no new dependency. The shared `Arc<ClaimMap>` + `WorkerRegistry` are threaded into `AppState` so the routes read the same live authorities placement uses. GitHub Issues remain the durable audit trail; this is the live ephemeral view.

Cleans the per-deployable `k8s_sample` manifests now that MongoDB is gone (#143): deletes the control-plane `mongodb-service.yaml` + `mongodb-statefulset.yaml` and the `wait-for-mongodb` initContainer, drops all `MONGODB_*` / `MONGO_INITDB_ROOT_*` config + secret keys, and collapses the control-plane to a single authoritative `Recreate` replica with a `maxUnavailable: 1` PDB (the in-memory claim authority is single-writer — multiple replicas would split-brain). `FKST_PLACEMENT_MAX_LOAD` is kept (still read by `Config` for worker-dispatch placement). The READMEs now describe the datastore-free, single-replica controller that rebuilds its in-memory state from worker self-reports + claimed goal-issue labels on restart, while the worker fleet autoscales.
