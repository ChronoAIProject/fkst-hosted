---
"fkst-hosted": patch
---

Remove `backend/docker-compose.yml` and declare the fkst deployables Kubernetes-only (#165). Compose is no longer a supported run path: the local-dev quickstart in `backend/README.md` now starts Mongo from a developer-managed container (or the forthcoming `k8s_sample/` mongodb manifest) and runs the crates with `cargo run -p fkst-control-plane` / `cargo run -p fkst-worker`; the six integration-test doc-comments no longer reference the deleted file; and a "Kubernetes-only deployment" policy line is added to both `backend/README.md` and `CLAUDE.md`'s Quick Rules Summary. No code behaviour changes; Mongo itself is removed later by #143.
