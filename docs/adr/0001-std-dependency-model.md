# ADR 0001: `std` / Library Dependency Model: Triggers Have Fired, Adopt Engine Primitives

- Status: Accepted; this revision supersedes the older defer / scanner-only posture.
- Date: 2026-06-21
- Decision process: `sshx` ontology triplet 3/3 unanimous + two ChatGPT Pro cross-model reviews
  converged + explicit user decision that the time had arrived.
- Related: `std/init.lua` header comments (Tier S/R), `scripts/check_repo.py` G9,
  `composed.deps`, `fkst-website` `CLAUDE.md` (host repo structure + private B-std + two-boundary
  distribution), and the `fkst-substrate` companion spec / ADR (pending).

---

## Context

The earlier ADR made the right call for its time. `std/` was still an implicit universal framework
library; real dependencies could be derived statically from `require("std.x")`. Without an enforcing
consumer, a handwritten manifest would have become a second source of truth. A symlink tier split
would either fail to provide selective visibility or force repository-wide `require` rewrites that
would collide with loning's active refactor zone. The old decision was therefore to use a scanner as
the fitness function, keep room for a future `--lib-root`, and avoid premature engine work.

This ADR is a living document. The previous version explicitly listed triggers: if a real need for
selective visibility appeared, if multiple named roots appeared, if a second shared root appeared, or
if third-party / host consumers could not reach implicit `std`, then the deferred engine
`--lib-root` / manifest primitive should move into the present. Those triggers have now fired:

- loning's saga split turned `github-devloop` into a family: `github-devloop`,
  `github-devloop-intake`, `github-devloop-decompose`, `github-devloop-pr`, and
  `github-devloop-integration`.
- The state-machine / restart / marker / liveness / merge-gate code shared by those five packages
  moved into universal `std` as `std.devloop_*`. At this revision, the current worktree had 42
  top-level `std/devloop*.lua` files and 11,252 LOC; those modules were required only by the five
  devloop-family packages, and by zero other packages.
- This is no longer a harmless `std` tail that every package may naturally see. It is a second
  library: `devloop`. It has its own consumer set, visibility boundary, versioning / publication
  semantics, and should not grant ambient visibility to non-devloop packages.
- The user explicitly required the dependency model to be formalized, auditable, and standardized so
  other repos / projects can reference it. Per-package symlinks were judged implicit, unauditable,
  and not standardizable.

Therefore, this ADR evolves from "defer the engine primitive; scanner-only for now" to "adopt a
formal engine library-dependency primitive". This does not overturn the old ADR; it is the next phase
after the triggers that the old ADR named have fired.

The goals are unchanged: the dependency graph must be clear, version boundaries explicit,
auditable, and mechanically verifiable; no drifting second source of truth; no interruption of
loning's current active migration zone; and no engine Rust implementation in this repository.

The old trigger table evolves into the current adoption table:

| Previously deferred item | Old trigger | Current state | Decision in this revision |
|--------|------|------|------|
| Handwritten per-package `std.deps` manifest + `actual-uses ⊆ declared` enforcement | Engine `--lib-root` named-root authorization / third-party public B-std publication surface / second shared root with a different trust-publication cycle | Triggered: `devloop` is the second shared root, and the user requires a formalized / auditable / standardized dependency graph | Adopt per-unit manifests; scanner becomes the declared-vs-actual validator |
| Physical folder / tier split (`std-core` / substrate / forge) | A module group has an independent lifecycle and a mechanism that preserves `require` paths | Partly triggered: `devloop` has a family-scoped lifecycle, but loning is still in an active zone | Decide only the `devloop` library boundary; actual extraction waits for saga-split stability and does not reorganize layers |
| Engine `--lib-root` (named shared root replacing symlinks) | Real selective-visibility need / multiple named roots / non-layered split library DAG / cross-repo consumers cannot reach `std` | Triggered: `std` + `devloop` multiple named roots, devloop-family allowlist, cross-repo standardization need | Adopt a formal engine library-dependency primitive |
| Promote Tier S through substrate | Another package-repo needs a Tier S platform contract published by substrate | Not yet triggered as an independent publication need | Preserve as a future publication boundary; do not block this library primitive |

---

## Decision

### 1. Adopt Two Unit Kinds: `package` and `library`

The `fkst` manifest uses the neutral umbrella term **unit**. A unit has two sibling kinds:

- `package {flat|composed}` is a runtime unit: it has departments / raisers / `M.spec` / queues /
  lifecycle, and is loaded and run by the engine.
- `library` is a require-only code unit: it has exports / `lib_deps` / visibility / version, has no
  runtime presence, produces no queues, and runs no departments.

`library` is not a third package subtype. `std` was already a library, hidden by symlinks. The
appearance of `devloop` makes libraries plural, so the concept must become explicit.

This aligns with established prior art, without claiming one-to-one equivalence: Cargo, .NET, Bazel,
and similar systems express different target roles such as libraries and apps / binaries inside one
build / manifest world. `fkst` adopts the same idea: shared manifest / lock / registry language, but
distinct `package` and `library` unit kinds.

### 2. The Engine Provides a Formal Library-Dependency Primitive

The engine replaces per-package `std` symlinks and the G9 no-peer-require convention by enforcing
require scope:

- `require()` can load only the current unit's package-private modules or public exports from
  libraries **directly declared** by the current unit manifest.
- Undeclared library access fails closed.
- A library has its own declared require scope. A library may depend on another library, but must
  explicitly declare `lib_deps`; the engine resolves them, limits capability, and checks cycles.
- Library access is **direct-only**: a unit can `require()` only public modules from libraries it
  directly declares. Transitive `lib_deps` are resolved / linked by the engine for the depended-on
  library, but do not automatically grant the upstream unit direct require rights. If the upstream
  unit wants to require the transitive library directly, it must declare it itself and pass
  visibility.
- Module ownership is declared by the library manifest, not inferred from directory names or the
  first segment of `require()`.
- Require scope is a capability, not ambient filesystem visibility; the manifest is the auditable
  dependency graph.

This promotes "the single canonical way" from scanner convention to engine capability: business code
no longer sees all shared code merely because a symlink exists.

### 3. Lua Require Scope / Module Cache Isolation Is Part of the Engine Primitive

Require scope cannot only change path search; it must also preserve the owner-scoped module isolation
that current G9 depends on:

- Each unit resolves `require()` inside its declared scope. Package-private modules belong only to
  that package / unit; library-private modules belong only to that library.
- A library's public exports are resolvable only inside consumer units that declare the library.
  Private modules must never be directly resolvable by consumers.
- The engine must model Lua module resolution / cache keying by owner scope and requesting-unit
  capability, or use an equivalent mechanism with the same property: one unit cannot obtain another
  unit's private module through `package.loaded` / same-name module cache collision, and cannot
  obtain a public module from an undeclared library.
- Every unit load context must run with an isolated `package.loaded` / searcher environment, or an
  equivalent substrate isolation mechanism. Lua modules are mutable singletons cached through
  `package.loaded`; if a library module is cached only by module name, two consumer packages share
  the same mutable table and owner-scoped isolation breaks. The correct cache key is **resolved
  module identity + content/version identity + top-level consumer/load context**. Repeating
  `require()` for the same resolved module inside one unit load context still returns the same
  instance; different top-level consumer/load contexts get isolated instances even if they require
  the same library export.
- The resolver must not silently resolve ambiguity through searcher order. If two declared libraries
  export the same module name, or a package-private module name conflicts with a declared library
  export, the manifest / resolver must require canonical prefixes to remove ambiguity or fail
  closed. It must never default to the first searcher path.
- This preserves owner-scoped module isolation while promoting enforcement from "no symlink, no
  visibility" to "no declared capability, no visibility".

This is a requirement for the `fkst-substrate` resolver; this repository does not recreate cache
isolation at the package layer.

### 4. One Manifest Per Unit, With Typed Dependency Sections

Use a unified per-unit manifest (example name: `fkst.toml`) instead of extending `composed.deps`
into generic `deps`, and instead of long-term parallel files.

The manifest must distinguish two dependency planes:

- `lib_deps`: code plane. Declares which libraries this unit can require.
- `event_deps`: event plane. **Only for composed packages, with semantics exactly equal to today's
  `composed.deps`**: declares the sibling packages referenced by namespaced queues in this composed
  package's composed graph, according to `M.spec.consumes` / `M.spec.produces` / `M.spec.fanout` and
  the declared produced queues used by actual `raise()` calls. Composition conformance uses this to
  load those sibling package roots together with this package and validate the composed graph. It is
  not a generic "event dependency" system, does not describe runtime delivery order, deployment
  dependencies, version solving, or library dependencies, and does not list individual queues.

The workspace root file (example name: `fkst.workspace.toml`) discovers units and configures
registries. The lockfile (example name: `fkst.lock`) pins concrete versions / content. Internal
workspace units are versionless; third-party consumption of a named library pins a Git ref / content
ID without semver solving.

Existing `composed.deps` files in the current worktree are the migration source for `event_deps`.
For example, `packages/autochrono/composed.deps` is `consensus` because `propose` produces / raises
`consensus.proposal` and `reply` consumes / fanouts `consensus.consensus_reached`.
`packages/github-devloop-pr/composed.deps` is
`github-proxy consensus github-devloop-decompose`, covering the `github-proxy.*`, `consensus.*`, and
`github-devloop-decompose.*` references in its graph. The typed manifest only lifts these line-based
texts into verifiable fields.

Library manifests declare at least:

```toml
kind = "library"
name = "devloop"
stable_id = "fkst.library.devloop"
version = "workspace"
lib_deps = ["std"]

[exports]
public = ["devloop.*"]

[visibility]
units = [
  "github-devloop",
  "github-devloop-intake",
  "github-devloop-decompose",
  "github-devloop-pr",
  "github-devloop-integration",
]
```

`std` itself also becomes a declared library with public visibility: all units, packages and
libraries, may declare it.

```toml
kind = "library"
name = "std"
stable_id = "fkst.library.std"
version = "workspace"
lib_deps = []

[exports]
public = ["std.*"]

[visibility]
public = true
```

Composed package manifests contain both planes but do not mix them:

```toml
kind = "package"
name = "github-devloop-pr"
stable_id = "fkst.package.github-devloop-pr"
version = "workspace"
package_kind = "composed"
lib_deps = ["std", "devloop"]
event_deps = ["github-proxy", "consensus", "github-devloop-decompose"]
```

`[visibility]` is the library consumer allowlist over **units**, not packages only. Entries may be
package units or library units; `public = true` means all units may declare that library. The current
`devloop` visibility allowlist is the devloop-family package consumers. If another library consumes
`devloop` in the future, it should be added to the same allowlist as a unit name. Non-devloop
consumer units do not declare it and cannot require it.

### 5. `fkst deps` Becomes the Dependency Audit / Rendering Entry Point

Add `fkst deps`, which reads the workspace + unit manifests + lockfile, renders the full DAG, and
checks invariants:

- dependency graph acyclic；
- declared `lib_deps` covers actual static requires;
- undeclared library require fails;
- visibility allowlist is obeyed;
- public exports exist and have no owner conflicts;
- no orphan libraries;
- `event_deps` has the same semantics as current `composed.deps`: each composed package's declared
  sibling package set must cover the sibling-package namespaced queues referenced by `consumes` /
  `produces` / `fanout` in its composed graph, and is used only for composed-conformance
  package-root inclusion. It must not introduce broader event-dependency semantics such as
  deployment or runtime ordering.

The old G-STD-DEP / requires-as-truth scanner is not deleted; it is upgraded into a manifest
validator. It continues deriving actual requires from source and validates that declared `lib_deps`
cover actual requirements. The meaning of single source of truth evolves by phase: in the old phase,
the source was `require()` itself; in the new phase, the source is the manifest grant, and
`require()` is code-derived audit counterevidence. They must agree and cannot drift.

### 6. The First Concrete Library Is `devloop`

Extract the `std.devloop_*` family code into library `devloop`:

- Paths like `std.devloop_saga` become `devloop.saga`.
- Drop the redundant `devloop_` prefix to avoid `devloop.devloop_saga` stutter.
- Do not choose names such as `std-devloop` or `devloop_std` that preserve old `std` semantic
  residue.
- Do only mechanical prefix removal + manifest declaration.
- Do not reorganize the hierarchy of the 42 modules in this boundary migration. Relayering is
  semantic refactoring and would turn a boundary change into large review / conflict risk.

`devloop` visibility is a consumer-unit allowlist. The current allowlist is the devloop-family
packages; non-devloop consumer units do not declare it and cannot require it.

### 7. Extraction Precondition: `std.devloop_prompts` Must First Invert Package-Local Prompt Inversion

Current worktree facts: shared prompt orchestration in `std/devloop_prompts.lua` directly
`require("prompts.implement")`, `require("prompts.fix")`, `require("prompts.sync_conflict")`,
`require("prompts.review_meta")`, `require("prompts.intake")`, `require("prompts.decompose")`, and
similar modules. Those `prompts/<name>.lua` files live in the consuming package's package-local
`prompts/` directory. This is equivalent to template-method inversion: shared orchestration lives in
`std`, while concrete prompt content is supplied by the consumer package's package-local modules.

This worked in the symlink era. But if `devloop_prompts` moves unchanged into the `devloop` library,
it becomes a library -> consumer package-private module back-reference. The new require scope must
fail closed: a library cannot see a consumer package's private `prompts.*`.

Therefore, before extracting `devloop`, first **invert the inversion by dependency injection**:

- Per-package prompt content remains owned by each package.
- The DI boundary is a formal provider port, not an implicit require path. The consuming package
  constructs a small `prompts` provider table, for example
  `local prompts = { implement = function(ctx) ... end, fix = function(ctx) ... end, sync_conflict = function(ctx) ... end, review_meta = function(ctx) ... end, fix_reflection = function(ctx) ... end, intake = function(ctx) ... end, decompose = function(ctx) ... end }`.
  The library entry is shaped as `devloop_prompts.run(ctx, prompts)`, or an equivalent entry with the
  same port semantics. `devloop` orchestration calls only these named functions / values.
- Package-local prompt content remains owned by the consuming package. Concrete templates /
  renderers may stay under the consumer package's `prompts/`; the consumer requires them and inserts
  them into the provider table.
- The `devloop` library must not `require("prompts.*")`.
- Do not wrap each package's prompts back into tiny libraries and make `devloop` depend on them.
  That would promote package-owned content into library dependencies, invert the dependency graph,
  and recreate a library -> consumer-specific edge.
- `fkst deps` / G-STD-DEP validator must treat std/library -> package-private require as an
  extraction blocker.

This is a precondition for `devloop` library extraction, not a later cleanup. It matches the existing
G-STD-DEP report-only finding that `std` must not depend on packages.

### 8. Engine Work Belongs in `fkst-substrate`; This Repository Records the Decision

Rust primitives belong in `fkst-substrate`: manifest parsing, require-scope
resolution / enforcement, preserving owner-scoped module isolation, `fkst deps`, and
lockfile / versioning are all implemented in companion `fkst-substrate` specs / ADRs and PRs.

This repository does not implement the engine and does not change Rust. This ADR is the package-repo
decision record that drives substrate design.

### 9. Migration Is Staged, Not Big-Bang

Migration order:

1. Now: land this ADR and write the companion design / ADR in `fkst-substrate`.
2. Implement engine primitives in `fkst-substrate`: manifest parsing, require scope, `fkst deps`, and
   lockfile.
3. Declare `std` as a library in this repository and write unit manifests for existing packages. No
   behavior changes in this phase; a compat path may still support it.
4. Before extracting `devloop`, complete the `std.devloop_prompts -> prompts.*`
   dependency-injection inversion.
5. After loning's saga split stabilizes, mechanically extract the `devloop` library. Keep short-term
   compat aliases such as `std.devloop_*` until consumers migrate.
6. After engine enforcement covers the graph, delete per-package `std` symlinks and G9
   no-peer-require scanner as the primary boundary. Keep scanner logic as the `fkst deps`
   validator / ratchet.

Key discipline: do not touch the `std.devloop_*` files currently moving in loning's active zone.
Midstream extraction would create many conflicts and mix architecture-boundary change with business
migration.

Compat aliases are not enforcement bypasses. They are scanner-tracked, removal-gated migration debt:

- Each alias must be listed in the shrink-only allowlist maintained by `fkst deps`, with source
  alias, canonical target, consumer units, and tracking reason.
- The alias allowlist is the canonical visibility enforcement gate, not a report-only scanner
  exception. If `std` remains public and `std.devloop_*` aliases are treated as ordinary public
  `std` modules, any unit that declares `std` can bypass `devloop` visibility and directly reach
  devloop code. That is a capability bypass.
- Therefore alias resolution must limit capability according to the canonical target grant: only
  consumer units listed in the allowlist, and that have declared and are allowed to use the
  canonical `devloop` library, can resolve the corresponding `std.devloop_*` alias. Units that did
  not declare `devloop` or are not in the alias consumer allowlist must fail closed even if they
  declared public `std`.
- Declared-vs-actual mismatch may warn only for alias modules + consumer units explicitly listed in
  the allowlist.
- Unlisted aliases / undeclared library access fail closed.
- After an alias is removed from the allowlist, the corresponding old path immediately fails closed
  under normal rules.
- `fkst deps` must include a removal gate ensuring the allowlist can only shrink to zero, so
  `std.devloop_*` cannot become a permanent dual entry point.

### 10. Explicit Supersede / Preserve

**Superseded**：

- "defer `--lib-root` / engine primitive": the triggers have fired; adopt a formal primitive.
- "scanner-only now": scanner is no longer the final boundary, only the validator.
- "universal `std` symlink is enough": `devloop` proves a second shared root and selective
  visibility need exist.

**Preserved**：

- evolutionary architecture: upgrade the mechanism only after triggers fire;
- fitness functions: use mechanical validators to guard require / manifest / visibility invariants;
- single source of truth: reject symlink as an implicit, unauditable dependency; manifest grant is
  the dependency graph and scanner is counterevidence;
- the value of requires-as-truth: keep it as the actual-use scanner instead of discarding it;
- do not interrupt loning's active zone: extract `devloop` after her saga split stabilizes;
- staged migration: decision and substrate primitive first, then manifest, then library extraction,
  then deletion of symlink / G9 as the primary defense.

---

## Rationale

### The Triggers Have Fired; Continuing to Defer Becomes a Band-Aid

The old ADR's deferral was the last responsible moment, not "never". There are now plural
libraries: `std` is universal and `devloop` is family-scoped. Keeping `devloop` inside universal
`std` grants ambient visibility to non-consumers and leaves the dependency graph to convention and
symlink guessing.

After the user explicitly required formalized / auditable / standardized dependencies, the defects
of symlinks became the core problem: they cannot express declared capability, cannot express a
visibility allowlist, and cannot become a cross-repo standard interface.

### `package` and `library` Must Be Sibling Unit Kinds

Making `devloop` a new package subtype would confuse two planes: runtime lifecycle and code require.
A `package` runs and has queues and departments; a `library` does not run and only provides module
exports. Squeezing a library into the package type would force the engine, docs, and audits to
explain "a package that does not run", which is the wrong ontology.

With unit kinds, package and library share manifest / lock / registry language while keeping
different typed sections. `lib_deps` governs code; `event_deps` governs event composition. This also
explains where `composed.deps` belongs: it is not generic deps, but a formal event-plane field.

### Engine Primitives Are the Clean Solution; Symlink / Scanner Are Migration Tools

The clean solution follows ground truth: whoever is granted require capability can require; whoever
did not declare it fails closed. Symlink is ambient filesystem visibility, and scanner is after-the-
fact detection; neither is the final boundary.

This follows the repository harness doctrine strength gradient: when capability restriction is
possible, do not stay at scan long-term. `fkst deps` remains important, but its job is to audit that
declared grants match actual requires, not to replace engine enforcement.

### The Way to Avoid a Second Source of Truth Has Changed

In the old phase, there was no manifest consumer, so handwritten `std.deps` would drift and
`require()` was the only source of truth. In the new phase, the engine itself becomes the enforcing
consumer. Manifest grant is no longer duplicate documentation; it is the capability source. The
scanner's correct role is then validator: derive actual requires from code and reconcile them with
the manifest.

This preserves the old ADR's core insight: do not let two handwritten narratives drift. In the new
model, one side is the grant source and the other is code-derived evidence; conflict is failure.

### Extracting `devloop` Is Boundary Migration, Not Semantic Refactoring

The current risk is not "the names are ugly"; it is "the boundary is not explicit". Therefore the
first step is only mechanical prefix removal plus manifest declaration. Relayering 42 modules may be
valuable later, but it is an independent refactor requiring its own plan, tests, and review. It
should not be mixed into library-boundary migration.

### Cross-Repo Ownership Is Clear

This repository is a package repo and does not change engine Rust. The real need is an
`fkst-substrate` module resolver / manifest / lockfile primitive. This ADR records package-library
decisions and migration constraints as input to the companion substrate design.

---

## Consequences

**Positive**:

- The dependency graph changes from implicit symlink to manifest DAG: auditable, renderable, and
  pinnable.
- The `std` / `devloop` boundary becomes explicit: universal code and family-scoped code no longer
  share one ambient namespace.
- `composed.deps` gets the right home: it evolves into `event_deps` and does not pollute the code
  dependency plane.
- Scanner investment is preserved: scanner-only becomes manifest validator and CI fitness function.
- Third-party / host repos can reference named library + pinned version instead of copying symlink
  conventions.
- Engine enforcement makes undeclared require fail closed, stronger than G9 convention.

**Costs / Notes**:

- Requires new `fkst-substrate` primitives and companion spec / ADR; this is not an implementation
  that a docs PR in this repository can complete alone.
- Manifest / lockfile / registry should land after the design is stable, to avoid freezing temporary
  fields into a public contract.
- Extracting `devloop` must wait for loning's saga split to stabilize; moving files now would create
  conflicts.
- Short-term compat aliases are migration tools, not permanent dual modes; they must be deleted
  after consumers migrate.
- `fkst deps` must cover both code plane and event plane, or dependency narratives will split again
  into two unreconcilable systems.

**Migration Complete When**:

- Every unit has `fkst.toml`; the workspace has `fkst.workspace.toml` and `fkst.lock`.
- `std` and `devloop` are both declared libraries.
- Devloop-family packages declare `lib_deps = ["std", "devloop"]`; non-devloop packages declare only
  the libraries they actually need.
- `fkst deps` renders the DAG and validates declared-vs-actual, visibility, acyclic, orphan, and
  `event_deps` invariants.
- Engine require-scope enforcement is active.
- Per-package `std` symlinks and G9 as the primary boundary are removed; scanner remains as
  validator.

⟦AI:FKST⟧
