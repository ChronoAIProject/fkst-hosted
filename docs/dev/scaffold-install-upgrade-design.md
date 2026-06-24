# Scaffold, scaffold-upgrade, and package-reference-update design

Status: design (awaiting review) · Date: 2026-06-14 · Author: operator + sshx inline consensus

This spec covers how an **independent, domain-owned host repo** installs the fkst autonomous
devloop system into its `.fkst/` directory and keeps that install current as the upstream engine
and packages evolve. It was vetted by an sshx thinking triplet (minimal / structural / delete) →
meta-judge `meta-layer convergence`; the worker conclusions are the basis for the decisions below.

（中文摘要：本 spec 定义"域无关的宿主产品仓如何把 fkst 自治 devloop 装进自己的 `.fkst/`，并随上游引擎与
包演进保持更新"，覆盖脚手架/安装、脚手架升级、包引用更新三面。结论经 sshx 三角度共识收敛。）

---

## 1. Problem & goal

An independent product repo (a web app, a service, anything — **not** a fkst library repo and
**not** a fork) wants to install the fkst autonomous devloop (`github-devloop` + its `[event_deps]`
`consensus`, `github-proxy`) into its `.fkst/` directory, so `github-devloop` autonomously develops
the host's own product code. The host root stays domain-owned; fkst lives entirely under `.fkst/`.

Three facets to solve, treated as **one** install/upgrade mechanism:

1. **Scaffold / install** — materialize {engine, tooling, packages} for the host's `.fkst/`.
2. **Scaffold upgrade** — re-materialize when upstream engine/tooling/packages improve.
3. **Package-reference update** — keep the host's installed package set current with upstream.

### The consistent gap

The engine and the engine-owned tooling already have the full mechanism — pin
(`.fkst/substrate-ref`) + fetch-to-cache (`scripts/bin_bootstrap.sh`) + auto-bump PR
(`packages/github-devloop/core/substrate_ref.lua`) + CI gate + regenerate
(`fkst-framework init-package-repo`). The **package layer has none of install / upgrade /
reference-update** for a host repo. Nothing installs `github-devloop` into a host's `.fkst/`, and
nothing keeps it current. Filling that gap — by **copying the proven engine pattern to the package
layer**, not inventing — is the whole job.

---

## 2. Reference frame (prior art)

Governing practice: **dependency lockfile + source pin + module cache, with GitOps-style bump PRs
and CI/conformance as the upgrade gate** (Cargo.lock + crates cache; go.mod + module cache;
Renovate/Dependabot bump PRs; cruft/cookiecutter for template regeneration). The fkst engine edge
already implements exactly this shape (`.fkst/substrate-ref` is a *git source-pin, not semver, not
binary distribution*). The package layer must adopt the same shape. **Vendoring upstream source
into the product repo is the wrong shape** and is rejected (§7).

---

## 3. Decisions (sshx consensus)

- **D1 — package source materialization: `pin + fetch` (M3), not vendored (M2).**
  Unanimous. Package source is upstream library code; committing copies under a host's
  `.fkst/packages/` creates drift and ownership confusion. M3 mirrors the proven engine path and
  keeps the host root domain-owned. The host commits a *pin*, not source.

- **D2 — declaration: a tiny two-part desired-state manifest (a pin + a package selection), not a
  bare one-line pin and not a resolver.** Converged. A bare `fkst-packages@<sha>` cannot say *which
  top-level package* to load; a full manifest risks becoming a resolver the engine contract avoids.
  The resolution is the minimal middle: **a pin (`repo@ref`) plus a top-level package selection**
  — `{ repo, ref, packages: [<top-level>] }` (e.g. `github-devloop`) — with `fkst.toml`
  `[event_deps]` expanded automatically into the `--package-root` set. No per-package versions, no dependency
  solver, no second manifest.

- **D3 — reuse, do not duplicate.** Generalize the existing `substrate_ref.lua` auto-bump from
  "engine pin only" into **one multi-upstream bump mechanism** (engine pin + package pin). Do not
  build a second installer competing with `init-package-repo`; do not make the engine read package
  composition as a resolver unless the substrate contract is explicitly changed.

- **D4 — upgrade is regenerate + bump + gate.** Tooling upgrades stay engine-owned via re-running
  `init-package-repo` (idempotent); pins advance via bump PRs; CI re-fetches at the new pin and
  runs composed conformance as the gate.

---

## 4. Precondition (must land first)

**Unify the split ref-surface before adding any package pin.** Today the engine scaffold
(`init-package-repo`) and `fkst-website` use a **root** `.fkst-substrate-ref`, while package-side
tooling and `substrate_ref.lua` read `.fkst/substrate-ref` (inside `.fkst/`). Adding a parallel
package pin on top of an already-inconsistent engine-pin surface doubles the confusion. Pick **one
canonical location** (recommended: everything under `.fkst/` — `.fkst/substrate-ref`) and migrate
`init-package-repo`, the CI template, and `fkst-website` to it. This is a substrate + tooling
change and is **sub-project SP0** (§8), blocking the rest.

（中文：动包 pin 之前，先把 `.fkst-substrate-ref`(根) 与 `.fkst/substrate-ref`(内) 两处落点统一成一个
canonical 位置，推荐都收进 `.fkst/`。不先解会把不一致翻倍。）

---

## 5. The unified mechanism

### 5.1 `.fkst/` install model (host-committed)

The host repo commits only desired-state + config; everything else is fetched to a cache or is
runtime scratch:

```
<host-repo>/
  .fkst/
    substrate-ref          # engine git source-pin (canonical, post-SP0)
    packages.manifest      # NEW: { repo: "ChronoAIProject/fkst-packages", ref: "<sha>",
                           #        packages: ["github-devloop"] }  (pin + package selection)
    env                    # gitignored host config (FKST_GITHUB_*, FKST_DEVLOOP_*, ...)
    runtime/               # gitignored scratch (locks, worktrees, once marks)
    durable/               # gitignored redb persistent delivery store
  scripts/run.sh           # engine-owned template (init-package-repo), resolves manifest -> roots
  scripts/check_repo.py    # engine-owned template
  .github/workflows/ci.yml # engine-owned template
  <product code ...>       # domain-owned, untouched by fkst
```

The manifest format is a small, stable, human-readable file. Exact serialization (single-line
`owner/repo@sha + packages` vs a 3-line key/value) is an open question (§9-Q1); semantics are fixed
by D2.

### 5.2 Install flow

1. `fkst-framework init-package-repo` writes the engine-owned templates + `.fkst/substrate-ref`
   (existing) **and** an initial `.fkst/packages.manifest` (new behavior — substrate change).
2. A bootstrap step (generalized `bin_bootstrap.sh`) reads both pins: clones `fkst-substrate@pin`
   → builds the engine BIN into cache (existing); clones `fkst-packages@manifest.ref` → exposes its
   `packages/` in cache (new).
3. `run.sh supervise` resolves the manifest's top-level `packages` + their transitive
   `fkst.toml` `[event_deps]` into the repeated `--package-root <cache>/packages/<pkg>` args and supervises
   `FKST_GITHUB_REPO` = the host repo. (In the dogfood today this `--package-root` resolution is
   the manual step done by hand.)

### 5.3 Upgrade flow (scaffold upgrade)

- **Tooling**: an engine-pin bump re-runs `init-package-repo` so `run.sh` / `check_repo.py` / CI
  regenerate from the engine's current templates (idempotent; closes the silent tooling-drift hole
  observed between fkst-packages and fkst-website).
- **Pins**: engine pin and package pin each advance by a dedicated bump PR (§5.4).
- **Gate**: the host's CI re-fetches at the new pin(s) and runs `--self-test` + `test` + composed
  `conformance` against the fetched roots; red CI blocks the bump merge.

### 5.4 Package-reference update (auto-bump, generalized)

Generalize `packages/github-devloop/core/substrate_ref.lua` from a single hardcoded upstream into a
small **multi-upstream** bump driver. For each declared upstream edge:

| edge | pin file/field | tracked upstream | bump PR title |
|---|---|---|---|
| engine | `.fkst/substrate-ref` | `fkst-substrate` dev head | `chore: bump fkst-substrate pin` |
| packages | `.fkst/packages.manifest` `ref` | `fkst-packages` dev head | `chore: bump fkst-packages pin` |

Each edge keeps the proven safety posture of the existing module: `ls-remote` the tracked branch,
SHA pin only, write into the integration branch, idempotent (skip if a bump PR for that head already
open), dry-run unless `FKST_GITHUB_WRITE=1`. This is **one mechanism with a per-edge config table**,
not two copy-pasted modules.

---

## 6. Cross-repo ownership

This design spans all three repos. **fkst-packages only writes Lua + scripts + docs; engine changes
are proposed to `fkst-substrate`.**

| Concern | Owner | Kind |
|---|---|---|
| `init-package-repo` writes `packages.manifest`; manifest→`--package-root` resolution contract; `bin_bootstrap` fetch of the package source; ref-surface unification (SP0) | **fkst-substrate** | engine (Rust) — propose as substrate issues/PRs |
| Generalize `substrate_ref.lua` → multi-upstream auto-bump; package composition metadata | **fkst-packages / github-devloop** | Lua (this repo) |
| The two pins (`substrate-ref`, `packages.manifest`), `env`, `runtime`/`durable`, product code | **host repo** | content |

The engine-owned templates (`run.sh`, `check_repo.py`, CI) live as `init-package-repo` output, so
manifest resolution inside `run.sh` is authored substrate-side (the template) even though it is a
shell script, not Rust. Net: the bulk of SP0/SP1 is substrate work; the auto-bump generalization
(SP2) is the actionable in-repo piece.

---

## 7. What NOT to build (delete-reviewer guardrails)

- **No vendored package installs** — no committing upstream package source under host `.fkst/packages/`.
- **No dependency solver / per-package version matrix** — the manifest is desired-state, not a resolver.
- **No second manifest / second installer** — `init-package-repo` stays the single scaffold reconciler.
- **No sibling-clone-as-source-of-truth** — the host must not depend on a co-located fkst-packages
  checkout on the same disk (the dogfood's manual `--package-root` to `~/fkst-packages` is a
  developer convenience, not the install contract).
- **No engine-side package-composition resolver** unless the substrate contract is explicitly extended;
  `[event_deps]` expansion via the manifest CLI lives in the generated `run.sh`, not as an engine
  scheduling resolver.

---

## 8. Decomposition into sub-projects

This is too large for one implementation plan and spans repos. Decompose + sequence:

- **SP0 (precondition, substrate)** — unify the ref-surface to a single canonical
  `.fkst/substrate-ref`; migrate `init-package-repo`, CI template, `fkst-website`. Blocks SP1.
- **SP1 (substrate)** — `init-package-repo` writes `packages.manifest`; `bin_bootstrap.sh` fetches
  the package source by pin to cache; `run.sh` resolves manifest + `fkst.toml` `[event_deps]`
  via the manifest CLI → `--package-root`.
- **SP2 (fkst-packages, this repo)** — generalize `substrate_ref.lua` → multi-upstream auto-bump
  with a per-edge config table (engine + packages edges). The actionable in-repo deliverable.
- **SP3 (substrate CI template)** — package-pin bump PRs gated by composed conformance at the new pin.

**Sequencing**: SP0 → SP1 → (SP2 ∥ SP3). SP2 can be specified now but its package-pin edge only
becomes live once SP1 ships `packages.manifest`. The first fkst-substrate proposal to open is SP0.

---

## 9. Open questions

- **Q1 — manifest serialization**: single-line `ChronoAIProject/fkst-packages@<sha> github-devloop`
  vs a 3-line key/value. (Semantics fixed by D2; form open.)
- **Q2 — canonical ref location for SP0**: confirm `.fkst/substrate-ref` (inside `.fkst/`) as the
  single home, and whether `packages.manifest` sits beside it as `.fkst/packages.manifest`.
- **Q3 — multi-package hosts**: D2 supports a `packages` list > 1; do we need that now, or start
  with exactly `["github-devloop"]` and let the list generalize later (YAGNI)?
- **Q4 — engine release vs build-from-source**: out of scope here; the host still builds the engine
  from the substrate pin via `bin_bootstrap.sh` (Rust toolchain required). A binary-release track is
  a separate substrate proposal.

---

## 10. Validation evidence

- sshx thinking triplet (codex-cli workers, read-only, peer-invisible): Q1 unanimous **M3**; Q2
  converged on the **two-field manifest**; all three anchored on the lockfile+pin+cache /
  GitOps-bump-PR reference frame; the delete reviewer surfaced the split-ref-surface precondition.
- The pattern is already proven in production on the engine edge (`.fkst/substrate-ref` +
  `bin_bootstrap.sh` + `substrate_ref.lua` auto-bump), observed live during dogfood.

⟦AI:FKST⟧
