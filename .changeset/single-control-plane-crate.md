---
"fkst-hosted": minor
---

Restructure the backend to a single `fkst-control-plane` crate. Removed the
`fkst-worker` deployable and the `fkst-journal` crate, the controller
claim/placement/registry/reassign machinery, the `/internal/v1` worker
protocol, and the in-process session driver; folded `fkst-shared` and
`fkst-engine` back into the control-plane crate. The control plane is now
API-only (a goal trigger records a pending session) pending the
pod-per-session execution model (milestone #9).
