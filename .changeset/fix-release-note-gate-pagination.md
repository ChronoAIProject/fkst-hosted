---
---

Fix the `require-release-note` CI gate to page the full PR file list (`gh api --paginate`) instead of the 100-file-capped `gh pr view --json files`, so a large `develop → main` release PR no longer fails the gate when its release note falls past the cap. No package changes.
