# libs re-decomposition design (audit-driven)

Status: design (2026-06-23). Driven by a 4-perspective adversarial audit (sshx codex minimal/structural/delete + ChatGPT Pro), strongly converged, of the post-stdlib-split library division. ⟦AI:FKST⟧

## Problem (audit findings, file:line-verified)

The stdlib split produced `contract` / `devloop` / `std`(forge). The audit confirmed the division is directionally right but has real boundary/naming debt:

1. **`contract` is a junk drawer (negative definition).** It is "everything with no github/git dependency", not a concept. Its 15 modules are mostly mutually independent and span unrelated concerns: value contracts (`source_ref`/`payload`/`error_facts`), saga/runtime orchestration (`saga`/`registry`/`dead_letter`/`env`/`logging`/`sweep`/`codex`/`oracle`), and test/conformance tooling (`testing`/`saga_conformance`/`namespaced_dispatch_conformance`). A negative-definition library inevitably regrows a god-lib (the next "pure utility" feels legal to add). It is also exported all-public (`libraries/contract/fkst.toml`).
2. **`std` name lies** — it is now only the GitHub/Git/ports forge layer (`std/github.lua`, `std/git.lua`, `std/ports.lua`), not a standard library.
3. **`archaudit → devloop` is a boundary violation** — archaudit declares `lib_deps=["std","devloop","contract"]` but its only devloop use is generic liveness validation (`packages/archaudit/core.lua:187-209` requires `devloop.liveness` + `devloop.restart_liveness_contract`); those modules hardcode `fkst:github-devloop` markers (`libraries/devloop/liveness/signal.lua`). Generic validation is trapped in the product library.
4. **`contract.strings` is itself mixed** — forge-specific helpers (github bot-login normalization, repo parsing, comment extraction, git-ref validation) plus a temporary JSON encoder (`strings.json_string`, a #976 stopgap) live beside generic scalar helpers.
5. **Cross-repo publishable surface too large** — publishing all of `contract` would expose env/saga/testing/codex/oracle/registry as "forever public API".

`devloop` (20k lines): mostly REFUTED as a problem — a large cohesive single-product kernel (one consumer family), not a god-lib. The only confirmed leak is the generic liveness validation (problem 3). Deeper devloop splits (merge subsystem, autonomy_ledger, commands, schema/policy) are DEFERRED until a real trigger (no current non-devloop consumer).

## Target decomposition (converged 忠于本质 shape: 5 libraries)

Positive definitions, by change-reason; dependency DAG depends toward stability:

```
contract ← workflow ← devloop
contract ← testkit
contract ← forge    ← devloop
contract / workflow / testkit / forge ← packages (each declares only what it uses)
```

| library | positive definition | modules | publish? |
|---|---|---|---|
| **contract** | dependency-free, deterministic, stable VALUE/PROTOCOL primitives, safe to share across products/repos | `source_ref`, `payload`, `error_facts`, **slim** `strings` (scalar/value helpers only, incl. generic `json_string`) | **publishable cross-repo** (the only one) |
| **workflow** | substrate runtime-orchestration authoring machinery | `saga`, `dead_letter`, `env`, `logging`, `codex`, `oracle`, `registry`, `sweep`, + a NEW **generic** liveness/restart-contract validator extracted from devloop (no `github-devloop` strings; markers/queues injected as data) | private |
| **testkit** | test + conformance tooling | `testing`, `namespaced_dispatch_conformance`, generic part of `saga_conformance` | private |
| **forge** (rename of `std`) | GitHub/Git/ports adapters | current `std/*` (github*/git*/ports/fakes/github_view/github_debug_stamp/gitref) + forge-specific helpers moved out of `contract.strings` + forge write-classification moved out of `saga_conformance` | private |
| **devloop** | the github-devloop issue→PR→review→merge product kernel | current `libraries/devloop/*` minus the extracted generic liveness/restart | private |

`lib_deps`: contract=[]; workflow=[contract]; testkit=[contract] (+workflow if needed); forge=[contract]; devloop=[contract, workflow, forge] (+testkit for its tests).

## Per-package lib_deps rewiring (from verified usage)

- contract-only-era packages now declare the specific libs they use:
  - `autochrono` → `[contract, workflow, testkit]` (source_ref/payload/error_facts/strings + saga + namespaced_dispatch_conformance)
  - `consensus` → `[contract, workflow, testkit]` (error_facts/strings + codex/dead_letter/env/saga + ndc)
  - `github-autochrono` → `[contract, workflow, testkit]`; `idle-detector` → `[contract, workflow, testkit]`
- forge users: `github-proxy`, `github-external-pr-intake`, `github-ratchet-migration-slicer` → `[contract, workflow, testkit, forge]`
- **`archaudit` → `[contract, workflow, testkit, forge]` (devloop REMOVED)** — rewire its liveness use to workflow's generic validator; remove archaudit from `devloop` `[visibility]`.
- devloop family (5): `[contract, workflow, testkit, forge, devloop]`.

## Intra-module splits (careful sub-tasks, behavior-preserving)

- **`strings`**: keep generic scalar/value helpers (incl `json_string`) in `contract.strings`; MOVE forge-specific helpers (github bot-login normalization, repo parsing, comment body extraction, git-ref validation) into `forge` (e.g. `forge.strings`/`forge.refs`); rewire callers.
- **`saga_conformance`**: keep generic saga conformance in `testkit`; MOVE forge-specific command/write classification into `forge`.
- (Optional, note-only) `source_ref.version_order_key` is misnamed (it is marker/version ordering, not source-ref shape) — may rename within contract; non-blocking.

## Guards (make the regrowth illegal — the ugliest risk is `workflow` becoming the new junk drawer)

- CI ratchet: `workflow` modules MUST NOT contain product/forge policy strings (`github-devloop`, `fkst-dev:`, `std.github`/`std.git`/`forge.github`, raw `gh`/`git` command heads); product markers/queues must be injected as data. A new `workflow` module with such a string fails CI.
- `devloop` `[visibility]` excludes all non-devloop-family packages, so `archaudit → devloop` (or any peer) cannot reappear.
- `contract` stays the only `public`/publishable library; conformance asserts contract has zero outgoing lib_deps and only value/protocol modules.

## Migration (behavior-preserving, staged; clean break, no compat shim)

Slices (each: move/rename modules + rewrite require sites + rewire lib_deps + update ratchets/.competence + verify suite/deps/check_repo green; like the prior contract extraction):
1. Extract `workflow` from `contract` (saga/dead_letter/env/logging/codex/oracle/registry/sweep) + rewire consumers.
2. Extract `testkit` from `contract` (testing/namespaced_dispatch_conformance/generic saga_conformance) + rewire.
3. Rename `std` → `forge` (module path `std.*` → `forge.*`) + move forge-specific helpers out of contract.strings/saga_conformance.
4. Extract a generic liveness/restart validator from `devloop` into `workflow`; rewire `archaudit` to workflow; drop archaudit from devloop visibility + lib_deps.
5. Add the CI guards (workflow no-policy-string ratchet; devloop visibility lock; contract value-only conformance).
`std` is removed entirely (no deprecated shim — clean break per repo doctrine). substrate-ref unchanged (no engine change; this is a package-repo refactor on the existing libdep + cross-repo engine).

## Cross-repo publishable subset (resolves audit problem 5)

Publish ONLY `contract` (source_ref/payload/error_facts/slim strings). fkst-website's `[[external_sources]]` already references `contract`; after slimming, its surface becomes honest automatically. Do NOT publish workflow/testkit/forge/devloop.

## DEFER (with triggers)

- devloop internal splits — `merge` subsystem (trigger: a non-devloop merge consumer / 3 independent merge lanes), `autonomy_ledger` (trigger: AVM consumed outside devloop), `commands`→forge (trigger: a 2nd non-devloop caller), `devloop.contract` schemas vs impl (trigger: another package family consumes the schemas without devloop policy).
- the oracle's finer granularity (`substrate` vs `workflow`, `forge_testkit`, `devloop_protocol`) — adopt only if a concrete consumer need appears (三次法则); the 5-lib shape is the conservative converged core.
