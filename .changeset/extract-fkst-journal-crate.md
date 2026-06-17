---
"fkst-hosted": patch
---

Extract session-progress journaling into a new worker-reachable `fkst-journal` workspace crate (#175, prerequisite of #151). The `journal/` module (activity, comments, config, flush, github, keys, merge, model, parse, signals + tests) moves out of `fkst-control-plane` and decouples its single tie to the control-plane crate: `JournalConfig::from_config(&Config)` becomes a `journal_config_from_app(&Config)` free function on the control-plane side, mapping field-for-field exactly as before. `fkst-control-plane` re-exports the crate as `crate::journal`, so every consumer is unchanged; `fkst-worker` now links it (with no `mongodb`/`axum`/`fkst-control-plane` in its tree) so the worker can journal RAISED events direct to GitHub once the engine→worker execution move lands. Behavior-preserving refactor.
