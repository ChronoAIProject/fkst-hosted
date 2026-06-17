---
"fkst-hosted": patch
---

Fix goal sessions spawning the fkst-substrate engine with no GitHub credential (issue #106): the session driver now mints the repo-scoped GitHub App installation token and builds a `GoalContext` before starting the engine, calling `start_with_spec(goal: Some(..))` so `GITHUB_TOKEN`, `FKST_GITHUB_TOKEN_FILE`, and `FKST_GOAL_FILE` are set and the `0600` token file + `goal.json` are written at t=0 — rather than the token only arriving via the periodic refresh ~55 minutes in. The same `GoalContext` path serves both the initial start and the failover rebuild (the token is never persisted, always re-minted from the `SessionDoc`), and the refresh clock is seeded to the actual mint instant. The token value is never logged.
