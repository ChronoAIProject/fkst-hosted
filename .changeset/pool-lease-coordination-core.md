---
"fkst-hosted": minor
---

Add the pool coordination core: a per-package single-owner lease store (`leases` module) with atomic acquire/renew/release over MongoDB, a monotonic fencing token per continuous lease, pod-identity and lease-TTL configuration, a read-only spawn-guard check, and an expired-lease reaping helper.
