# DI refactor: retire the ambient-M service-locator via make_department(caps)

Adversarially-built SPEC (sshx codex triplet + ChatGPT Pro cross-model, high-adversarial per the
standing goal). Resolves the root the prior audit exposed: the `install(M)` ambient composed core
is a **service-locator god-table**, and migrating read-sites (installer ratchet 1091→45) left the
**ambient surface intact** (ratchet-gaming). This SPEC retires the surface itself.

## Root (confirmed by ground-truth)

- The composed `core` (M) is **100% package-side**: `core.lua` is a composition root that builds
  `M = {}`, calls `require("devloop.{logging,state,commands,…}").install(M)` (~167 methods), and
  returns M; 36 department `main.lua` do `local core = require("core")` then read `core.X` — the
  **service-locator anti-pattern**.
- The engine only calls `pipeline(event)`; it does not construct M. **No engine change is strictly
  required.** An *optional* engine capability-declaration graph-scan (like `M.spec.produces` /
  `published_seam`) would make DI mechanically enforceable — a later fkst-substrate PR, not a
  blocker.
- A **proven DI pattern already exists**: `forge.ports` — a department defines
  `make_department(ports)` closing over injected handles, returning `{spec, pipeline}`;
  `forge.ports.install` wires production handles; tests inject fakes. Only 5 depts use it.
- The earlier self-containment arc (PRs #1810–#1817) already made `devloop.{logging,state,commands}`
  **independently require-able self-contained modules** — the foundation this DI wiring builds on.

## Target shape (GPT-converged)

Departments receive **narrow, role-based capabilities**, never the whole core:

```lua
-- departments/<dept>/main.lua
local spec = { name=..., produces=..., published_seam=..., caps = { requires = {
  "log", "state.cas", "entity.reader", "commands.emit",   -- declared authority
}}}
local function make_department(caps)
  local log   = caps.log
  local state = caps.state.cas
  local emit  = caps.commands.emit
  local function pipeline(event) ... end
  return { spec = spec, pipeline = pipeline }
end
return { spec = spec, cap_deps = spec.caps.requires, make_department = make_department }
```

**Not** `make_department(core)` (god-lib-by-parameter) and **not** `make_department(all_caps)`.
One named `caps` arg, only declared role handles, close over locals, no ambient lookup after
construction.

### Capability grouping — role-based, not source-module-based

Granularity rule: *a capability = one coherent authority a test would naturally fake* — between
one huge bundle and 167 one-method handles. **Forbidden**: `caps.core`/`caps.devloop`/`caps.all`
(renamed service locators), `cap_deps = {"*"}`. Initial taxonomy:

| cap | authority | notes |
|---|---|---|
| `caps.log` | structured logging | broad OK (cross-cutting, low-risk) |
| `caps.state.read` | read-only state | inspectors |
| `caps.state.cas` | versioned CAS mutation | the lifecycle authority — narrow, auditable |
| `caps.state.lifecycle` | boot/migrate/recover | root-only usually |
| `caps.commands.emit` | emit/enqueue commands | separate from registry |
| `caps.entity.reader` / `.writer` | resolve / mutate entities | writer given sparingly |
| `caps.egress.gh` / `.git` | external effects | reuse the proven forge.ports handle |
| `caps.clock` / `caps.ids` / `caps.config` | determinism | typed slices, not raw tables |

**Package-local core methods** a department reads (e.g. `build_reconcile_pr_state_label_request`,
defined in the package's own `core/*.lua`) are **not capabilities** — they are intra-package
sharing; the department `require`s the package's own submodule directly (allowed, not god-lib).
So `core` dissolves into three explicit sources: injected platform caps + directly-required
package-local modules + already-require-able workflow/forge libs.

## Composition root + strict sealed projector (prevents caps becoming a second core)

`core.lua` (→ `devloop/di/`) becomes: `providers.build_all(env)` builds `all_caps` from the
self-contained modules; `build_department(name)` **projects only the department's declared
`cap_deps`** through `select_caps.project` — a **strict sealed** table (metatable errors on
undeclared access / mutation). A department **never receives `all_caps`**, only its projected
view. `registry` holds module **names** (avoid require cycles). `validate` checks the department
declares known caps and the root provides them.

## Two ratchets (the anti-ratchet-gaming core — directly fixes the prior audit)

The prior audit caught: moving reads `core.X → caps.X` while still building everything through M
= fake success. So **two** shrink-only ratchets, both to 0:

1. **Read-site ratchet** (`G-DEVLOOP-SERVICE-LOCATOR`): department `require("core")` 36→0 +
   department `core.X` reads →0 + tests passing whole core 18→0.
2. **Implementation ratchet** (`G-DEVLOOP-AMBIENT-SURFACE`): `install(M)` providers →0 + ambient
   M exported methods (~167) →0 + compat-only core methods →0.

CI during migration: counts may decrease or stay flat, **never increase**. Per migrated
department: a denylist rule (`departments/<migrated>/` may not `require("core")` / read `core.*` /
pass fake_core). Adapters that wrap legacy M during transition are **counted as debt** (ratchet 2).

## Staged plan (no big-bang)

- **Stage 0 — inventory + freeze**: build both ratchets + baselines (`department_core_requires=36`,
  `department_core_member_reads=N`, `test_core_parameter_sites=18`, `install_m_definitions=N`,
  `ambient_methods=~167`). Freeze: no increases.
- **Stage 1 — cap contracts + strict projector**: add `devloop/di/{capdefs,providers,select_caps,
  validate}.lua` + `registry`. Define cap names; reject wildcard deps. No dept migration yet.
- **Stage 2 — build caps (ambient compat preserved)**: providers build narrow handles from the
  self-contained modules; caps may temporarily wrap M (counted as debt); `core.lua` becomes a
  compat shim generated FROM the caps (not the reverse).
- **Stage 3 — migrate departments in batches**: each dept → `make_department(caps)`, rewire
  `core.X → caps.role.X` or `require(pkg-local)`, declare `cap_deps`; verify-green + sshx-triplet
  + GPT per slice; ratchet 1 shrinks. Ambient M kept as compat until the last dept.
- **Stage 4 — delete the surface**: `require("core")`=0 → delete `install(M)` scaffolds + ambient
  M; ratchet 2 →0. **Genuine dissolution.**
- **Stage 5 — high-adversarial re-audit** (sshx + GPT): confirm the ambient surface is GONE this
  time (the exact failure the last audit caught), caps are sealed/narrow, no god-lib-by-parameter.

## Biggest risks

- Package-local core methods defined **inline** in `core.lua` (not a submodule) — must extract to
  require-able modules first. - Sheer count (36 depts). Mitigate: two shrink-only ratchets + per-dept
  slices + ambient-M compat until the last dept. - Caps taxonomy drifting into a renamed locator —
  the **sealed projector + no-`caps.all` rule** is the guardrail.

## Engine (Rust) decision

Package-side is sufficient for the mechanism. Optional later fkst-substrate PR: a graph-scan that a
department declares `spec.caps.requires` and the composition root provides them — makes DI a
mechanical invariant (like `produces`/`published_seam`), not just convention. Deferred; not a
blocker.

⟦AI:FKST⟧
