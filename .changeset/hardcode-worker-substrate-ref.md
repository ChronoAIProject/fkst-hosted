---
---

Worker Docker build: default `ARG FKST_SUBSTRATE_REF` to the committed `backend/engine.ref` SHA so the engine-laden `fkst-worker` image builds with no build-args, for DevOps platforms that can't pass build-time args. The empty-arg fail-fast guard is preserved, so the CI negative test is unaffected. Temporary workaround tracked by #227 (revert to the mandatory build-arg once the platform supports build-time args). No package changes.
