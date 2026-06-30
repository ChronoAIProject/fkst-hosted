---
"fkst-hosted": patch
---

Flatten the backend to a single crate rooted at `backend/`. The vestigial Cargo
workspace with its one nested `fkst-control-plane/` member is removed: the
package manifest, `src/`, `tests/`, `Dockerfile`, and `k8s_sample/` now live
directly under `backend/`. Pure structural change — the binary name, public
API, and behaviour are unchanged.
