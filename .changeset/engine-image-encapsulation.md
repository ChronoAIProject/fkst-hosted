---
"fkst-hosted": minor
---

Encapsulate the fkst-substrate engine inside the backend Docker image. The
multi-stage `backend/Dockerfile` now clones and builds the engine at the commit
pinned in `backend/engine.ref` (enforced by a required `FKST_SUBSTRATE_REF`
build-arg guard), installs `fkst-framework` alongside the API server in a
non-root runtime stage with `git`/`bash` and a writable `FKST_RUNTIME_ROOT`,
and records engine provenance in `/etc/fkst-engine-sha`, `/etc/fkst-engine-version`,
and `io.fkst.engine.*` image labels. A new `docker-build` CI workflow builds the
image on every PR and smoke-tests the engine self-test, launch contract,
non-root uid, runtime-root writability, and label consistency.
