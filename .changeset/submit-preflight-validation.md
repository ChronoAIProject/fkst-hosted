---
"fkst-hosted": minor
---

Add synchronous submit-time pre-flight validation (package correctness + Ornn availability) returning an aggregated 422.

Before any session is spawned, `goals::preflight::validate_submission` now checks a submission comprehensively and, on failure, returns a single **HTTP 422** that lists EVERY problem at once (never first-fail), so the user can fix all of them in one edit cycle (#179, closes gap G3). Two checks run at the API boundary in `fkst-control-plane` (no engine run, no clone, no temp dir):

- **Package correctness** — each requested package is verified via the GitHub **Contents API** (a new App-layer `GithubAppTokens::get_contents` helper that mints through the existing `token_for_repo` path and reuses the typed `NotInstalled { install_url }` install hint): `<repo>/.fkst/packages/<name>/` must exist with a valid entry file `departments/<name>/main.lua`. The engine name rule (`^[A-Za-z0-9_-]+$`), the reserved name `host`, and the `is_department_main` entry-file rule are applied by reference via thin `pub` re-exports in `fkst-engine` (pure re-exports, no behavior change). Contents reads are concurrency-bounded (≤2 calls/package, ≤32 total).
- **Ornn availability** — each pin is confirmed in the Ornn catalog via `skill_versions`/`skillset_versions` + `resolve_pins` over the NyxID proxy with the caller's token (visibility-honoring): a missing skill/skillset, an absent or deprecated-only version, and a closure version conflict each yield a named error.

The gate is wired into BOTH the legacy `POST /goals/:id/trigger` handler and the unified `POST /goals/submit` handler, before `create_for_goal`. The 422 body never echoes the goal prompt, any secret, or any env value.
