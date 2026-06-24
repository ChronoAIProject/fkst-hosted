# Design: `std` — a package-repo shared library (Tier S / Tier R)

Status: proposal · Date: 2026-06-14 · Repo: fkst-packages
Companion spec: `2026-06-14-saga-harness-design.md` (the harness is `std`'s first inhabitant; that spec depends on this one).

---

## 1. Problem (实证)

Cross-package Lua duplication is already bleeding. A scan of the three real
`core.lua` libraries shows the same infrastructure re-implemented per package:

```
M.persistence_class      × 3 packages
shell_single_quote       × 2
is_bounded_string        × 2
read_env / read_env_command × 2
+ version_* (CAS order tokens), source_ref, url_encode, trim, stable_hash, ...
```

`github-proxy/core.lua` (947 lines) and `consensus/core.lua` (882 lines) each
carry overlapping low-level helpers. There is no place to share them, because
**the engine forbids cross-package `require`**:

> `docs/package-repo-contract.md:225` — "每个 graph root 用 fresh Lua state，package
> owner 只看自己的 root … `--package-root` 不是跨包 `require` 授权。"

The engine sets each owner's `package.path` to **its own package root only**
(`mlua_init.rs` owner-scoped `package.path`; contract `:244` lists
"package-root require isolation" as an enforced invariant). So today the only
ways to "share" code are (a) duplicate it per package, or (b) push it into the
Rust engine. Both are wrong for repo-level Lua helpers: (a) drifts, (b)
crystallizes an unproven, fast-moving authoring lib into the slow-stable engine.

We need a third thing the user named directly: **"一种库，不在引擎，但在包之间共享"**
— a *repo-level shared library*.

## 2. Goal / Non-goals

**Goal.** A single source of truth for cross-package Lua, requirable by every
package in this repo, that respects the engine's owner-scoped `package.path`,
needs **zero engine change** to start, and is shaped so its universal parts can
later be promoted into the substrate with **zero rework**.

**Non-goals.**
- Not a manifest / dependency resolver / version solver (same restraint as
  `[event_deps]`).
- Not peer cross-package coupling (see §4 — that stays forbidden).
- Not a separate repository (see §6 — rejected with reasons).

## 3. The two tiers (Python analogy: stdlib vs site-packages)

The shared lib has two kinds of content with different ultimate homes:

| Tier | Content | Who wants it | Analogy | Final home |
|---|---|---|---|---|
| **Tier S** (substrate-contract) | `department{done,act}` harness + idempotency oracle + `source_ref` helpers + version-CAS order tokens + `persistence_class` | **any** package-repo on the substrate (executable form of the engine contract) | **stdlib** | substrate (eventually) |
| **Tier R** (repo-domain) | `gh`-shaped helpers, devloop-specific helpers, generic utils (`trim`, `url_encode`, `shell_single_quote`) | **only** fkst-packages | **site-packages** | fkst-packages (forever) |

`source_ref`, version total-order, and `persistence_class` are already defined in
the *substrate contract doc* — they are doctrine, not host business — so they are
Tier S. Anything `gh`/devloop-shaped is Tier R.

This split is the key design decision: it lets us build everything in this repo
now, while keeping Tier S in a form that can move to the engine later.

## 4. Doctrine revision (explicit — requires user sign-off)

This design **consciously revises** CLAUDE.md, which currently says
"只做包内共享——不跨包 require、不建 `fkst/` 目录". The revision splits one
prohibition into two distinct cases:

| Form | Direction | Verdict |
|---|---|---|
| **peer cross-package require** (pkg A requires pkg B's internals) | lateral, bidirectional | **stays forbidden** — this is the tangle the rule was protecting against |
| **hierarchical shared-lib require** (every pkg requires the repo's blessed `std`) | one-way, layered | **newly allowed** — like a language stdlib |

Justification: the repo **already** accepts non-self-containment at the *event*
level (composed packages, `[event_deps]`, namespaced `pkg.queue`). A one-way
shared *code* lib is the symmetric analog at the code level. The prohibition
becomes: *no peer cross-package require; a single blessed shared-lib root is
allowed.*

Proposed CLAUDE.md edit (replaces the "包内共享库" paragraph's prohibition):
> 包内共享库放 package-root `core.lua`；跨包共享放 repo-root `std/`（单向、分层，
> 由装配投影进各包根，见 std spec）。**禁 peer 跨包 require（A→B 内部）**；
> **允许唯一 blessed 共享库根（all→std）**。`std` 不是 manifest/版本解析。

## 5. Architecture (now: Lua, zero engine)

```
fkst-packages/
  std/                      ← single source of truth, lives once in git
    saga.lua                ← Tier S (harness — see companion spec)
    source_ref.lua          ← Tier S
    version.lua             ← Tier S (CAS order tokens)
    strings.lua             ← Tier R (trim, url_encode, shell_single_quote)
    ...
  packages/<pkg>/           ← unchanged git source
```

**Requiring.** A package does `require("std.saga")`. For the engine's
owner-scoped `package.path` (= package root) to resolve that, `std/` must be
*visible under each package root*.

**Vendoring via a per-package symlink (verified).** `.fkst/packages` is a
**symlink** to `packages/` (verified: `.fkst/packages -> ../packages`; the `-H`
flag in `scripts/run.sh`'s `find` exists precisely to follow it). There is **no
separate assembly artifact** to project into — the package root *is*
`packages/<pkg>/`. So the vendoring is a single **git-committed symlink per
package**:

```
packages/<pkg>/std -> ../../std     # one symlink per package, committed to git
```

The engine sets `package.path` to the package root, so `require("std.saga")`
resolves to `packages/<pkg>/std/saga.lua` → repo-root `std/saga.lua` through the
symlink. `std/` lives once at repo root; each package gains one symlink.

> **Verified by spike (2026-06-15):** created `std/probe.lua`, symlinked
> `packages/github-proxy/std -> ../../std`, added a test doing
> `require("std.probe")`, ran `scripts/run.sh test github-proxy` (a **flat**
> package — the strictest single-root conformance gate). Result: `134 passed, 0
> failed`, exit 0. The symlinked `std` module resolves via `require` **and** flat
> single-root conformance accepts it. The earlier "hardlink mirror / assembly
> projection" assumption was wrong and is replaced by this.

## 6. Placement decision: this-repo now → substrate later; **not** a separate repo

- **Now:** `std/` in **fkst-packages** (prototype, vendored). Fast iteration,
  dogfooded on github-devloop, no cross-repo coordination. No other repo
  references it yet — and that is correct, it is not stable enough to commit to.
- **When stable (Rule-of-Three: a second package-repo wants Tier S):** promote
  **Tier S into the substrate** as a blessed Lua authoring stdlib that the engine
  places on every owner's `package.path` by default (like Lua's built-in
  stdlib — no flag needed; a `--lib-root` primitive is only needed for *repo*-level
  Tier R sharing). Conformance gains a "lib dep" accounting (the code-level analog
  of `[event_deps]`). **Tier R stays in fkst-packages forever.**
- **Not a separate repo.** Tier S is *release-coupled* to the engine contract
  (`department{done,act}` means "the conforming way to author a department for
  *this* engine version"). A separate repo would create an engine ↔ stdlib ↔
  package-repos version dance with no independence benefit, because the lib is not
  independent of the engine. (Genuinely engine-independent utilities *could* later
  spin out, but the harness core must not.)

"Can others reference it?" — **Tier S: yes, everyone, but they get it *from the
engine*** (it ships with the substrate), never by reaching into fkst-packages.
**Tier R: no, only this repo's packages.** A sibling package-repo must never
`require`/vendor from fkst-packages — that would be backward peer coupling
between package-repos.

## 7. Conformance accounting (the cost)

A flat package that uses `std` is **no longer single-root self-contained** — it
depends on the `std` root being projected in. This is the same compromise
`[event_deps]` already makes at the event level. Conformance handling:
- Flat single-root conformance runs against the **assembled** package root
  (which includes the projected `std/`), so `require("std.X")` resolves.
- A new check (in `scripts/check_repo.py`, the existing G-gate home) asserts:
  packages only `require("std.<module>")` for modules that exist in `std/`, and
  **never** `require("<sibling-package>.…")` (peer cross-package require stays
  banned — this is the teeth of the §4 revision).

## 8. Migration (seed, then drain duplication via ratchet)

1. Create `std/` + the vendoring projection + the conformance accounting (this spec).
2. Seed `std/saga.lua` as the first inhabitant (companion spec).
3. Drain existing duplication into `std` one module per PR: `source_ref`,
   `version`, `persistence_class`, `shell_single_quote`, `is_bounded_string`,
   `read_env*` … Each PR moves the source into `std/` and replaces per-package
   copies with `require("std.<m>")`. A G-gate ratchet forbids *new* duplicated
   copies of a helper that already lives in `std`.

### 8.1 Bounded utility drain for `has_bounded_source_ref` and `decimal_checksum`

The #840 drain is governed by DRY/Single Source of Truth, constrained by AHA and
Fowler's Rule of Three: only byte-identical, dependency-free helpers may be
promoted, and each promoted helper needs a cohesive `std` owner. The inventory is
closed to this narrow utility class:

| Helper | Prior local copies | `std` owner | Tier | Recurrence decision |
| --- | --- | --- | --- | --- |
| `has_bounded_source_ref` | `packages/autochrono/core.lua`, `packages/github-devloop/core/base.lua` | `std.source_ref` | Tier S, because `source_ref` is substrate-contract shape | Exhausted for this helper; no package-local definitions remain. |
| `decimal_checksum` | `packages/consensus/core.lua`, `packages/github-devloop/core/base.lua` | `std.strings` | Tier R, generic repo utility | Exhausted for this helper; no package-local definitions remain. |

This is a recurrence waiver for a point hoist, not a new open-ended utility
program. Existing sibling proposals (#842, #843, #844, #835, #834, #838) must
carry their own duplicate inventory and module taxonomy before adding or growing
`std` surface. They are not implicitly approved by #840.

## 9. Testing

- `std/*.lua` Tier S/R modules get their own `*_test.lua` under `std/tests/`,
  discovered by the engine test runner against the assembled root.
- A conformance probe asserts every package resolves `require("std.<m>")` after
  assembly (catches a broken/forgotten projection — fail-closed, not silent).

## 10. Risks / open questions

- **R1 — symlink portability (resolved for current targets).** Git-committed
  symlinks work on macOS + Linux (the dev + CI targets) and `core.autocrlf`/
  symlink support is on by default there. Windows is not a target. The spike
  confirmed resolution + conformance on macOS; CI (ubuntu) must be confirmed in
  the first plan task.
- **R2 — one symlink per package to maintain.** Each new package needs its
  `std -> ../../std` symlink. Mitigation: a conformance probe asserts every
  package has a resolvable `std` (fail-closed if missing — see §9), so a
  forgotten symlink is caught, not silently degraded.
- **R3 — shared-lib blast radius.** A `std` change can break all packages at
  once. Mitigation: it is a monorepo — one CI run catches it; this is the
  accepted price of the uniformity the harness needs.
- **R4 — premature Tier S API.** If Tier S churns, promoting to substrate later
  is painful. Mitigation: do not promote until Rule-of-Three fires (a second
  package-repo needs it); until then it is internal and free to change.

⟦AI:FKST⟧
