---
"fkst-hosted": minor
---

Isolate every fkst-substrate engine subprocess in its own process environment: the supervise and conformance spawn seams now `env_clear()` and rebuild the child env from a curated host allow-list, the platform-managed `FKST_*` roots, and an optional per-session `env_profile` (with reserved keys dropped), so the pod's ambient environment can no longer leak into a session. Adds the shared `ENGINE_ENV_ALLOWLIST`/`RESERVED_ENV_KEYS`/`is_reserved_env_key` contract and threads an `env_profile` through `StartSpec`.
