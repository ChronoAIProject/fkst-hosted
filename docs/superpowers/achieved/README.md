# Achieved specs

Design specs whose core mechanism is built and in production use. Moved here so `docs/superpowers/specs/` holds only designs with remaining work. History is in git; the doctrine that resulted lives in the repo `CLAUDE.md`.

- `2026-06-14-std-shared-library-design.md` — shared libraries are built and in use: pure contract helpers now live in `contract.*`, while forge adapters remain in `std.github`, `std.git`, and `std.ports`.
- `2026-06-14-saga-harness-design.md` — the SAGA harness (`contract.saga` `department{done,act}` + `contract.saga_conformance` + `core/restart` transition table + the G10 shrink-only ratchet) is built and conformance-enforced. Residual per-department migration is tracked by an open issue (drive `migration/saga-handler.allowlist` → 0).
- `2026-06-16-github-devloop-decomposition-design.md` — `github-devloop/core.lua` is now a thin installer-group aggregator (`require("core.<mod>").install(M)`); the §8 move-equivalence criteria are met. The separate `gh`/`git` egress migration is tracked by an open issue.

Specs still open in `../specs/` (remaining work, each tracked by an issue): ports-adapters, unify-gh-git-egress (both gated on `migration/gh-git-adapter.allowlist` → 0), capability-layering (staged remediation, largely realized by the above).
