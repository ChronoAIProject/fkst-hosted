---
"fkst-hosted": patch
---

Remove `backend/README.md`. It documented an architecture that no longer exists
(MongoDB, the multi-crate workspace, the worker deployable) and was actively
misleading after the DB-free pivot, the v1 surface trim, and the backend
flatten. The two inbound links (root README footer, API reference) are updated
so none dangle.
