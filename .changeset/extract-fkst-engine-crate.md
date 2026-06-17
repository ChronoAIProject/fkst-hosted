---
"fkst-hosted": patch
---

Extract the engine execution primitives into a new worker-reachable `fkst-engine` workspace crate (#159, phase 1 of #151). The `engine/` module (`runner`, `process`, the #136 `breadcrumb`/`adopt`, `clone`, `materialize`, `config`, `goal_token`, `logs`, `util`) moves out of `fkst-control-plane` and decouples its three ties to the control-plane crate: `EngineConfig::load_*` now returns a crate-local `EngineConfigError` (bridged to `AppError` via a `From` impl), the `dir_age`/`RUNTIME_DIR_PREFIX` runtime-dir helpers move engine-side, and `RepoRef` resolves from `fkst-shared`. `fkst-control-plane` re-exports it as `crate::engine`, so every consumer is unchanged; `fkst-worker` now links it (with no `mongodb`/`fkst-control-plane` in its tree), unblocking the engine→worker execution move. Behavior-preserving refactor.
