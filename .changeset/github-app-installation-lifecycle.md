---
"fkst-hosted": minor
---

GitHub App installation lifecycle: persist installations (user OR org) in a `github_installations` collection keyed by installation id, add a signature-verified webhook endpoint (`POST /api/v1/github/app/webhook`, unauthenticated, HMAC-SHA256 over the raw body verified before parse) handling `installation` and `installation_repositories` events, resolve installations from persistence before probing GitHub (surviving pod restarts), fail active sessions whose repo loses the App on uninstall/repo-removal, and surface an org-owner-aware install hint after creating a new repo.
