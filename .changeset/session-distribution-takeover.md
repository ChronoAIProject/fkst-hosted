---
"fkst-hosted": minor
---

Pool-manager session distribution and redo-on-failover takeover: new sessions are placed on the least-loaded healthy pod under the package lease (a live lease for another session now answers POST /api/v1/sessions with 409); session drivers are fenced by the lease (claim CAS pins pod_id + fencing_token, periodic renewal, self-termination without document writes on lease loss, release on terminal exit); a per-pod reaper takes over sessions whose holder pod died — re-acquiring the lease under a strictly greater fencing token and redoing the session from scratch (status back to pending, pid/runtime_dir cleared) — and retries unplaced pending sessions. Startup gains fail-closed distribution config (FKST_LEASE_RENEW_INTERVAL_SECS, FKST_TAKEOVER_SCAN_INTERVAL_SECS, FKST_TAKEOVER_GRACE_SECS, FKST_PLACEMENT_MAX_LOAD), lease index creation, the boot orphan sweep now also releasing leases, and the reaper loop tied to graceful shutdown.
