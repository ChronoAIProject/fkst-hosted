---
"fkst-hosted": minor
---

Provision a per-session NyxID token for fkst-substrate runs (#111): at session start the driver mints one non-expiring NyxID agent API key on the triggering user's behalf and injects it into the engine env as `NYXID_ACCESS_TOKEN` (plus the `NYXID_URL` origin), so the engine's `nyxid` CLI and codex provider act as that user — with no engine change. The key is revoked at teardown; only its non-secret id/prefix are persisted on `SessionDoc` (never the full key). A token-less failover rebuild of a session that had a key escalates (fails) rather than running with broken auth.

Note: the issue spec also asked to add `NYXID_URL` to `ENGINE_ENV_ALLOWLIST`. That was deliberately NOT done: the allow-list copies a var from the parent pod env, and a key on the allow-list is also treated as reserved and dropped from the per-session `env_profile` by `apply_isolated_env`. Since the per-session `NYXID_URL` value lives only in `env_profile` (not the pod env), allow-listing it would prevent it from ever reaching the engine. It is therefore kept non-reserved so the per-session entry is delivered.
