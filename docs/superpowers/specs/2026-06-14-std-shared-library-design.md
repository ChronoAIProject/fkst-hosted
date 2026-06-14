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
  `composed.deps`).
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
level (composed packages, `composed.deps`, namespaced `pkg.queue`). A one-way
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
*visible under each package root at run/test time*.

**Vendoring via the assembly boundary.** `.fkst/packages/<pkg>` is already an
**assembly artifact** — a hardlink mirror of `packages/<pkg>` (verified: same
inode `193195037` for `core.lua` in both trees). The vendoring step **projects
`std/` into each assembled package root**, e.g. `.fkst/packages/<pkg>/std/ →`
repo-root `std/` (hardlink or copy). The git source tree stays clean (`std/`
lives once at repo root; `packages/` untouched); only the assembled tree carries
the projection. Tests and `supervise` both run against `.fkst/packages/<pkg>`,
so both see `std`.

> **Implementation task 0 (must confirm first):** locate the code that creates
> `.fkst/packages/<pkg>` as a hardlink mirror. It is **not** in
> `scripts/bin_bootstrap.sh` (grep found nothing). The `std/` projection hooks
> wherever that mirror is produced (likely `scripts/run.sh` setup or a separate
> bootstrap). If no single assembler exists, add one and route all of
> `test`/`run`/`supervise` through it.

## 6. Placement decision: this-repo now → substrate later; **not** a separate repo

- **Now:** `std/` in **fkst-packages** (prototype, vendored). Fast iteration,
  dogfooded on github-devloop, no cross-repo coordination. No other repo
  references it yet — and that is correct, it is not stable enough to commit to.
- **When stable (Rule-of-Three: a second package-repo wants Tier S):** promote
  **Tier S into the substrate** as a blessed Lua authoring stdlib that the engine
  places on every owner's `package.path` by default (like Lua's built-in
  stdlib — no flag needed; a `--lib-root` primitive is only needed for *repo*-level
  Tier R sharing). Conformance gains a "lib dep" accounting (the code-level analog
  of `composed.deps`). **Tier R stays in fkst-packages forever.**
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
`composed.deps` already makes at the event level. Conformance handling:
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

## 9. Testing

- `std/*.lua` Tier S/R modules get their own `*_test.lua` under `std/tests/`,
  discovered by the engine test runner against the assembled root.
- A conformance probe asserts every package resolves `require("std.<m>")` after
  assembly (catches a broken/forgotten projection — fail-closed, not silent).

## 10. Risks / open questions

- **R1 — assembler location (task 0).** The `std/` projection depends on finding
  the single point that builds `.fkst/packages`. Mitigation: if none exists,
  introduce one and route all entrypoints through it.
- **R2 — projection drift.** Hardlink vs copy: hardlink keeps one inode (edits
  propagate, no staleness) but is fragile across filesystems; copy is robust but
  needs re-sync on change. Recommendation: hardlink locally, copy in CI (clean
  checkout each run, so no staleness).
- **R3 — shared-lib blast radius.** A `std` change can break all packages at
  once. Mitigation: it is a monorepo — one CI run catches it; this is the
  accepted price of the uniformity the harness needs.
- **R4 — premature Tier S API.** If Tier S churns, promoting to substrate later
  is painful. Mitigation: do not promote until Rule-of-Three fires (a second
  package-repo needs it); until then it is internal and free to change.

⟦AI:FKST⟧
