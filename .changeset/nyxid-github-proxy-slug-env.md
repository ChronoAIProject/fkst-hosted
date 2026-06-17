---
"fkst-hosted": patch
---

Make the NyxID GitHub credential-injection proxy slug configurable via `FKST_NYXID_GITHUB_PROXY_SLUG` (default `api-github`) instead of a hardcoded path, so operators can align it with their NyxID deployment without a rebuild.
