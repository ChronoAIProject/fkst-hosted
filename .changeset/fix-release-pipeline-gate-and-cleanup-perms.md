---
---

Fix two release-pipeline bugs found cutting v0.2.0: the `require-release-note` gate read the wrong JSON field (`.path` instead of `.filename`) from the GitHub PR-files API, so it never found a present release note and spuriously failed `develop → main` PRs; and the `release.yml` post-release cleanup job lacked `pull-requests: write`, so it could not open the automated cleanup PR. CI-only; no product behaviour change. Closes #242.
