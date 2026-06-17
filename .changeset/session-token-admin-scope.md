---
"fkst-hosted": minor
---

Grant substrate session GitHub App installation tokens admin-equivalent access for the whole session (issue #110). `default_permissions()` now requests `administration: write`, `pull_requests: write`, `contents: write`, and `issues: write` (metadata stays implicit), and `TokenPermissions` gained a `pull_requests` field. This lets session automation perform repo-administration tasks (branch protection / rulesets, collaborators, settings) on the target repo. A mint `422` (the App lacks a requested permission) is now logged loudly at the mint site with the rejected-permission detail and an actionable hint, never the token. `docs/github-app.md` documents the elevated set, the required Read & write App-settings declaration for the four permissions, the per-installation re-consent triggered by adding `administration`, the org-owner-only installation consequence, and the deliberate exclusion of `workflows`/`secrets`/`actions`/`repository_hooks`/`environments`.
