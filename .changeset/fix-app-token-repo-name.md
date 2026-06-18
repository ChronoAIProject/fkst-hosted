---
"fkst-hosted": patch
---

Fix GitHub App installation tokens being scoped to the repository owner instead of the repository name, which made every App-token operation (`.fkst` scaffolding, engine git) fail with a GitHub 422.
