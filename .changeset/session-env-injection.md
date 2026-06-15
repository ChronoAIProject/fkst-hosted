---
"fkst-hosted": minor
---

Resolve each session's vault scope into a per-session `env_profile` and inject it into the fkst-substrate engine run (issue #102): the session driver re-fetches the persisted `SessionDoc`, asks the vault for the session's scope (`global` for package sessions, the target repo for goal-triggered ones, repo overlaying global), drops any platform-reserved key, and starts the engine via `start_with_spec(env_profile)` instead of the bare `start()`. Only a non-secret `env_scope` pointer is persisted on `SessionDoc`, so a pod failover re-resolves the profile from the vault (picking up rotated secrets) and never writes secret material to any document; a vault decrypt error fails the start rather than running a session missing its secrets. Secrets stay `SecretString` end-to-end — only env-var key names and counts are logged.
