---
"fkst-hosted": patch
---

Per-session NyxID keys now self-expire via a configurable TTL (FKST_SESSION_KEY_TTL_SECS, default 24h) instead of a service-account revoke NyxID rejects, fixing the key-cleanup leak.
