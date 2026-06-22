# stdlib split + cross-repo versioned library design

Status: design (2026-06-22). Converged from a 4-perspective adversarial design pass (sshx codex triplet minimal/structural/delete + ChatGPT Pro oracle) plus operator analysis, all strongly agreeing. ⟦AI:FKST⟧

## Problem: `std` is a god-lib

Measured: `std/` = 131 Lua files / 23,879 lines. ~80% of that is the `std.devloop_*` family (replayer/base/state/markers/restart/convergence/liveness/merge/payloads/requests/commands/...) — the github-devloop machinery, consumed by ONLY 5 of the 11 packages. The genuinely-general stdlib (strings, git, github, ports, saga, saga_conformance, source_ref, oracle, registry, error_facts, namespaced_dispatch_conformance) is the minority.

So the god-lib is two things wearing one coat: a small general stdlib + the entire github-devloop product machinery, fused into one `library` unit. That fusion is the bulk and the coupling.

Two requirements, two largely-independent sub-problems:
- **(A) Kill the god-lib** = split `std` into layered libraries (notably extract `devloop`). In-repo; uses the EXISTING engine library-dependency primitive (library→library `lib_deps`); no new engine mechanism. This is the cure and the biggest win.
- **(B) Cross-repo versioned reference** (git sha/tag + lock) so OTHER repos (库 C / fkst-website, ...) can consume a small stable core. A heavier NEW engine capability, needed only when a second repo has a real direct code-reuse need.

## Guiding decision (unanimous): do NOT build a package manager

Add ONE narrow mechanism: root-workspace-owned external library sources, locked to exact git commits, cataloged as read-only `library` units, consumed only through existing direct `lib_deps` visibility. Publish only a tiny stable core first; keep `devloop` private. No SemVer solver, no registry, no implicit transitive visibility, no external package roots, no ambient `require` path.

Architecture rule (pre-established, built upon, not relitigated): 库 B's std is PRIVATE to 库 B by default; 库 C integrates with 库 B via EVENTS (pkg.queue) — NOT cross-require — unless 库 B PROMOTES a part of std to a named, versioned, public platform library. This design is that promotion mechanism, used sparingly.

## Phase 1 — the split (fkst-packages, in-repo, current engine)

Split `std` into 5 coarse LAYERED libraries by stability boundary + dependency direction (respect the internal DAG; do NOT make 26 micro-libs — over-fragmentation is an anti-pattern):

| library | contents | depends on | publish? |
|---|---|---|---|
| `fkst.platform.contract` | pure contracts: schema/payload helpers, marker-field/transition/status helpers, registry builders, validation, saga, saga_conformance, source_ref, oracle, strings, error_facts, namespaced_dispatch_conformance. NO github/git/devloop. | — | **publish now** (the only cross-repo core) |
| `fkst.platform.git` | generic git ops (`std/git/*`, git_fake) | contract | maybe, after a real 2nd consumer |
| `fkst.platform.github` | generic GitHub ops (`std/github/*`, github_view, github_debug_stamp, github_fake) | contract | maybe, after a real 2nd consumer |
| `fkst.platform.repo_ports` | combined ports needing git+github (`std.ports`) | git, github | private by default |
| `fkst.devloop` | all ~42 `std.devloop_*` modules | the platform layers | **private** (workflow/product code, used by 5/11 pkgs — the signal it is NOT substrate stdlib); optionally later split into `devloop.contract` (event/state/payload/liveness schemas) + `devloop.impl` (prompts/wiring/policy) |

Packages rewire `lib_deps` from `["std"]` to the specific layers they use (e.g. github-devloop family → `["fkst.platform.contract", "fkst.platform.github", "fkst.platform.repo_ports", "fkst.devloop"]`; consensus/autochrono → contract-only).

Engine support needed for Phase 1: library→library `lib_deps` (direct-only resolution), which the merged primitive already provides. So Phase 1 needs NO substrate change — it is a package-repo refactor.

### Smallest first slice of Phase 1: extract `devloop`

Extract the `std.devloop_*` family into the `fkst.devloop` library (family-visibility to the 5 devloop packages), depending on the (still-monolithic-for-now) remaining `std`. This ALONE shrinks `std` by ~80% and removes the god-lib coupling. Unblocked: the `std.devloop_prompts → require("prompts.*")` DI inversion is DONE (Candidate C, merged); loning's devloop changes are all on dev. Behavior-preserving: same modules, same resolved values, only the library grouping + `[visibility]` + consumer `lib_deps` change.

Then a second slice splits the remaining `std` into contract / git / github / repo_ports.

## Phase 2 — cross-repo versioned reference (substrate engine; DEFER; loning's domain)

Only when a second repo has a verified direct code-reuse need (currently none — 库 C integrates via events). Spec for the substrate engine:

1. **Declaration (Cargo-style intent vs lock separation).** External sources in `fkst.workspace.toml`, not in leaf packages:
   ```toml
   [[external_sources]]
   id = "fkst-platform"
   git = "ssh://git@github.com/org/fkst-packages.git"
   tag = "platform-contract-v0.1.0"   # human intent; NOT the build pin
   libraries = ["fkst.platform.contract"]   # allowlist of what may enter this repo's catalog
   ```
   Consuming units stay simple: `[lib_deps] libraries = ["fkst.platform.contract"]`.
2. **Lock = build truth (`fkst.lock.toml`, checked in).** Records `resolved.rev` (exact SHA) + `tree_sha256` + per-library `exports_sha256`. The tag is update intent; the locked SHA + tree hash is what builds. `--locked` uses the lock; `fkst deps update <id>` is the only thing that moves a resolved rev. ILLEGAL STATE UNREPRESENTABLE: a moved tag does not change the build; manifest-vs-lock divergence cannot silently take effect.
3. **Acquisition/cache.** Two-level: `~/.cache/fkst/git-mirrors/<url-hash>.git` + content-addressed `~/.cache/fkst/store/sha256-<tree-hash>/`. Flow: resolve rev (deref tag → commit) → fetch mirror → clean checkout by commit → compute tree_sha256 (exclude .git) → read the source repo's own `fkst.workspace.toml` → catalog ONLY `kind=library` units → admit only consumer-listed + source-`[visibility]`-allowed → write lock.
4. **Resolver/cache-key.** External lib_dep resolution = the same direct-only, owner-scoped lexical resolver, with module cache keyed by `(source_id, library_name, module)` so two repos / two versions never collide. No external package roots, no consumer-upward require, no `package.path` leakage.
5. **Modes.** `fkst deps lock` (write lock), `deps update <id>` (move one source), `deps fetch` (fill cache from lock), `--locked` (use locked SHA, never rewrite), `--frozen` (no network, cache/vendor only), optional `deps vendor` (hermetic snapshot). Cache + checked-in lock is primary; vendor is optional, not default.
6. **Cross-repo composed conformance (resolves the «solo conformance gap»).** A 库 C package's `event_deps` on a 库 B package uses the SAME external UnitRef; the engine fetches that package at the locked rev, adds its root to the local composed graph, and validates queues/published-seams locally — without granting cross-repo `require`, without symlink spelunking, without manual `--package-root` wiring. This is also the proper fix for the interim `run.sh` "recognize libdep-composed → skip single-root" patch landed in fkst-website #20.

Smallest Phase-2 slice: publish `fkst.platform.contract` from 库 B (visibility allowlisted to one consumer repo first, not public-world); in the consumer repo add `[[external_sources]]` + `fkst.lock.toml` + `deps lock/fetch` + `--frozen`; one package consumes it. Defer transitive external deps, version ranges, registry, solver.

## Ownership / coordination

- Phase 1 (the split) — fkst-packages, current engine — implementable now by this repo's pipeline.
- Phase 2 (the engine mechanism) — fkst-substrate — loning's domain; lands as a substrate spec/PR when a real second consumer exists. Until then, 库 C keeps its own small std + integrates via events (as fkst-website already does).

## Is cross-repo std sharing worth it? Mostly no.

Default: each repo keeps its own small std; integrate across repos via events. Cross-repo publication is reserved for a tiny stable contract core, pinned by sha + lock, when a real second consumer appears. The high-value, immediate work is Phase 1 (kill the god-lib by extracting `devloop` + layering the remainder), which needs no new engine mechanism.
