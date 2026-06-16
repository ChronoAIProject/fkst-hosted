---
"fkst-hosted": minor
---

Move fkst-packages to repo-scoped: remove all package-management HTTP endpoints (`/api/v1/packages*`, `/api/v1/packages/generate`) and the Mongo package store, and load packages from the goal repo at `<repo>/.fkst/packages/<name>/`. Goal sessions now clone the repo with the GitHub App installation token (via the credential helper — no token in the clone argv or `.git/config`) and resolve each `package_names` entry against the cloned `.fkst/packages/` (absent dir fails the spawn). The classic `POST /api/v1/sessions` create path is removed (sessions are created via goal trigger); goal package-name validation is now format-only (existence resolved at spawn). Lease keys stay `goal-<uuid>`, which is already repo+package scoped.
