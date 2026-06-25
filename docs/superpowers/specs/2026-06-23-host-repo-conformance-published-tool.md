# Host-repo harness: one coordinate per source, atomic SHA consolidation now, a typed pack-execution contract later

Status: Part A IMPLEMENTED; Part B DESIGN CONVERGED (Round-3), ready to implement as a vertical slice.
- **Part A (§4) — atomic fkst-packages SHA consolidation in fkst-website: IMPLEMENTED** (fkst-website PR;
  one fkst.lock coordinate, all four pin consumers migrated, `.fkst-packages-ref` deleted, single-pin guard).
- **Part B (§5–§8) — package-owned declarative conformance packs that travel with the package: DESIGN
  CONVERGED** after Round-3 sshx (3 Codex + ChatGPT Pro). Round-2 had left the execution contract as open
  questions; Round-3 answered them: declarative rule DATA in the package + one generic engine interpreter +
  typed locked artifacts + reachability activation + a hard owner-only scope boundary. Implement as Slice 1
  (mechanism + host-travel) then Slice 2 (ownership-deletion), each landing across substrate + fkst-packages
  + host. Engine Rust lands in fkst-substrate; rule DATA in fkst-packages; binding in the host.
Date: 2026-06-24 (revised; original 2026-06-23 draft + Round-1 in git history)
Scope: fkst-substrate (engine validator + conformance runner + resolver), fkst-packages (rule-pack data +
migration), host repos (fkst-website first; substrate-dogfood + future hosts).

Superseded note (2026-06-25): Part A has since retired the side pin described in the pre-migration inventory.
ADR 0002 is the canonical host layout and pin source of truth: the current platform pin is
`fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock`, and host composition roots live at
`.fkst/compose/package-roots`.

## 0. Verified pre-Part-A state (2026-06-24 — seek truth from facts)

Round-1's premise ("fkst-website has a COPIED check_repo.py") is STALE. The copy is gone, replaced by
**fetch + a new lock**, and the defect evolved into a worse one. Verified by reading both repos:

- fkst-website had NO `check_repo*.py` copy. `scripts/run.sh check` cloned fkst-packages at the side-pin SHA
  into `.fkst/run/fkst-packages-platform/` and ran THAT (B-private)
  `check_repo.py --project-root <website>`, then `$BIN conformance`.
- A new cross-repo dependency mechanism exists: `fkst.workspace.toml` `[[external_sources]]`
  (`id=fkst-packages-platform`, git+rev, `libraries=["contract"]`) resolved into `fkst.lock`
  (`[external_source.resolved] rev` + `tree_sha256`, `[[external_source.libraries]] contract exports_sha256`).
- **VERIFIED UGLY — two divergent pins to the same upstream (a split brain):**
  `.fkst-packages-ref` = `45ef0324…` (harness fetch) vs `fkst.lock` `external_source.resolved.rev` =
  `1734c42e…` (contract library). They are not merely different: `45ef0324` is the commit that ADDS the
  host-facing ratchet interface (`scripts/check_repo_config.py` + `check_repo.py --project-root`), and
  `1734c42e` predates it (verified by the quality reviewer via `git show <rev>:scripts/check_repo_config.py`).
  `scripts/run.sh:108` even printed "bump .fkst-packages-ref to a Track P commit with the shared host-repo
  interface". So the conformance result was the accidental product of two clocks pointing at incompatible
  commits.
- `.fkst-packages-ref` (or its checkout) had FOUR consumers (verified):
  1. `scripts/run.sh` `run_shared_source_ratchets` → fetched `check_repo.py --project-root` (run.sh:103-124).
  2. `scripts/run.sh` `build_engine_package_root_args` → resolves `.fkst/compose/package-roots` entry
     `fkst-packages:packages/idle-detector` from the SAME checkout (run.sh:127-155, 258).
  3. `.github/workflows/ci.yml:26-35` independently read `.fkst-packages-ref` and pre-cloned the checkout
     BEFORE `scripts/run.sh` runs.
  4. `scripts/run.sh:72-82` `FKST_PACKAGES_CONFORMANCE_ROOT` — a local-only override (a second checkout
     authority).
- `check_repo.py` is host-aware: `check_repo_config.is_own_repo` gates the ~8 github-devloop-hardcoded
  ratchets (skipped for an external project-root). This is a compatibility PATCH, not a public API: the
  inverted-dependency ugly (a "generic" harness hardcoding one package's name) survives under a blanket.

Verified pre-Part-A corrections to the earlier draft (do not assert these as already-solved):
- The engine `fkst-framework conformance` command EXISTS, but `host_conformance.rs` registers only an
  `EngineRulePack` + layout/schema/graph checks (runtime-layout, project-layout, locale-catalogs,
  graph-scan, department-non-empty, schema-validation). It does **not** demonstrably run testkit/devloop
  Lua saga/dispatch conformance via `--config`; `conformance --config` currently parses the TOML into an
  untyped value and otherwise **ignores** it (registry comment reserves future pack selection). Whether
  behavioral Lua conformance runs via `test`/`--self-test` mode vs the `conformance` command is TO-VERIFY,
  not an established fact.
- The resolver currently models ONLY `libraries`: `ExternalSourceDecl` / `ExternalSourceLock` carry
  `libraries` only, and `validate_source_decl` REJECTS an external source with no libraries
  (`manifest_workspace.rs`, `manifest_external.rs`). Typed `conformance_packs` / `tools` / external
  `packages` are unimplemented resolver work.
- The external `idle-detector` package is NOT a locked external artifact today; it is consumed by an
  untyped `fkst-packages:*` path in `.fkst/compose/package-roots`.

## 1. The corrected thesis (Round-2 converged direction; the contract is still TBD — see Part B)

> A host should consume harness through its declared dependency graph: per dependency source, ONE
> authoritative coordinate; ONE public engine command executes conformance; package-owned policy travels
> with the package, not by host copies or a redundant second pin into the same upstream.

Two precise corrections the review forced:
- **"One lock" means one authoritative coordinate PER dependency source, NOT one universal lock for
  everything.** `.fkst-substrate-ref` (the engine toolchain pin) is a LEGITIMATE separate coordinate — like
  `Cargo.lock` pinning dependencies while NOT pinning the `cargo`/`rustc` binary interpreting it. The pin to
  DELETE is the *redundant second fkst-packages* pin (`.fkst-packages-ref`), not the substrate toolchain pin.
- **Rejecting a standalone `event-conformance` product with its own pin is right; but "ride the lock" only
  fixes provenance/integrity of bytes. It does NOT by itself define the execution contract** (how a pack is
  declared, activated, scoped, versioned, trusted). That contract is the real structural design (Part B), not
  a downstream implementation detail.

## 2. Harness (prior art)

- **Lockfile single-source-of-truth** (Cargo/npm): one resolver owns each upstream coordinate; the lock
  records every artifact obtained from it. A lock prevents *unintended* drift; it deliberately permits
  *indefinite staleness* — preventing stale pins needs a SEPARATE min-supported-version / expiry / update
  policy (so "a host cannot silently fall behind" is NOT a property of an ordinary lock and must not be claimed).
- **Versioned linter platform / policy-as-code** (ESLint plugins + shareable configs; OPA/Conftest): generic
  policy authored once and distributed as versioned data; BUT only meaningful with a published pack format,
  activation model, and runner protocol — not just pinned bytes.
- **Compiler vs linter ownership / capability-vs-scan**: intrinsic validity is the engine's (capability);
  static source policy is separable data owned by whoever owns the semantics; a runtime library must not
  secretly scan source (the Round-1 trap).
- **Published API vs private consumption**: consuming declared, versioned artifacts is clean; reaching into a
  repo's private `scripts/check_repo.py` by filesystem path is not.
- CLAUDE.md «守住包边界 / published seam»·«分层归属»·«Harness本质 PREVENT>DETECT»·«通用>枚举»·«迁移=inventory ratchet»·«DRY 单一真相源»·«禁 god-package».

## 3. Tiers by ownership (the stable frame; tier-2 execution contract is Part B)

| Tier | Home (code) | Owns | Must NOT own |
|---|---|---|---|
| **1. Engine built-in validator** | fkst-substrate runtime | intrinsic package invalidity: malformed metadata, duplicate runtime identifiers, impossible saga graphs, unresolved refs, published-seam legality | org/package policy, migration ratchets, allowlists |
| **2. Static rule packs** (policy-as-DATA, executed by the public runner) | authored by the OWNER of the semantics; executed by `fkst-framework conformance` | generic source rules (engine/std-owned pack) + per-package rule packs (package-owned). **The declaration/activation/scope/version/trust contract is UNDESIGNED — Part B.** | a monolithic "B god-pack"; B-private layout baked into the generic pack; arbitrary unsandboxed code |
| **3. Engine-run Lua conformance** | fkst-substrate test driver + testkit/devloop Lua, via lib_deps | properties needing EXECUTION (saga runtime, scheduling, ordering, liveness, dispatch). NOTE: whether the `conformance` command runs these today is TO-VERIFY (see §0). | static source scanning smuggled into a runtime library |

---

# PART A — atomic fkst-packages SHA consolidation in fkst-website (READY TO IMPLEMENT)

## 4. Kill the split brain: one coordinate, atomically, across all four consumers

Goal: eliminate the divergent second pin so conformance runs against ONE coherent platform commit. This is
independent of Part B and unanimously endorsed, with a hard atomicity condition.

Acceptance criteria (ALL must hold; the change is NOT done until each is true):
1. `fkst.lock`'s `external_source.resolved.rev` is bumped to a SINGLE coherent fkst-packages commit that
   satisfies EVERY consumer: it contains the host-facing ratchet interface (`scripts/check_repo_config.py` +
   `check_repo.py --project-root`), `packages/idle-detector`, and the `contract` library. The lock is
   REGENERATED (re-resolve `tree_sha256` + `contract exports_sha256`) — not hand-edited. (The current lock
   rev `1734c42e` is BEHIND the host interface, so adopting it as-is would regress; the consolidation bumps
   the lock to ≥ the `.fkst-packages-ref` rev, i.e. a Track-P commit with the host interface.)
2. `scripts/run.sh` derives its single fkst-packages checkout from the lock's resolved rev (one
   `ensure_fkst_packages_checkout` keyed on the lock, not on `.fkst-packages-ref`), and BOTH
   `run_shared_source_ratchets` and `build_engine_package_root_args` consume that same checkout; the engine
   invocation still includes `packages/idle-detector`.
3. `.github/workflows/ci.yml` hydration step (lines 26-35) resolves the checkout from the lock rev, not from
   `.fkst-packages-ref`.
4. `FKST_PACKAGES_CONFORMANCE_ROOT` local override is removed, OR retained only with an explicit assertion
   that its `git rev-parse HEAD` equals the lock's `external_source.resolved.rev` (no second checkout authority).
5. `.fkst-packages-ref` is DELETED, along with `read_fkst_packages_pin` / `FKST_PACKAGES_PIN_FILE` and the
   README/CLAUDE references to it.
6. A concrete CI guard (a real test, not prose): assert that (a) no `.fkst-packages-ref` (or any second
   `*-ref` side pin to the fkst-packages git URL) exists, and (b) the resolved fkst-packages checkout's
   `git rev-parse HEAD` equals `fkst.lock`'s `external_source(id=fkst-packages-platform).resolved.rev`.
   `.fkst-substrate-ref` is a DISTINCT, legitimate toolchain coordinate and is explicitly out of scope of
   this guard.
7. `scripts/run.sh check && scripts/run.sh test` pass green on fkst-website after the change (the lock bump
   also moves the `contract` library that `site-board` consumes — verify nothing regresses).

Honestly-inventoried migration DEBT (Part A does not pretend to be the one-resolver end state):
- The host still MANUALLY clones the lock rev because the engine `deps` command fetches/validates locked
  external sources but does NOT expose a public source-root lookup, and a manual clone may bypass the lock's
  `tree_sha256` verification. This is acknowledged shrink-only debt, tracked toward Part B, NOT the final
  form. Part A's win is precise and real: one coordinate, no split brain, every consumer coherent — today.
- The host still fetch-runs B-private `check_repo.py`. Tolerated as shrink-only debt until Part B lands; it
  now runs at the SAME locked commit as everything else, so it is no longer a second clock.

---

# PART B — package-owned declarative conformance packs (CONCRETE DESIGN — Round-3 converged)

Round-3 sshx (minimal/structural/delete + ChatGPT Pro) converged the execution contract that Round-2 left
open. The shape: a package ships its static rules as DECLARATIVE DATA inside the package; the engine runs
them via ONE generic compiled interpreter; a typed lock makes the pack a content-hashed artifact resolved
from the one upstream coordinate; activation follows package reachability so referencing a package activates
its owned rules; and a hard scope boundary keeps a package's pack from scanning anything but its own tree +
graph facts that reference it. This removes the inverted dependency (no engine/repo-global hardcoded package
name), the god-runner (no arbitrary code — declarative only), and private consumption (no path-to-script, no
host copy).

## 5. The execution contract (the converged tier-2 design)

- **Form = declarative rule IR as DATA, interpreted by ONE generic Rust pack.** NOT compiled Rust per package
  (a Lua package can't ship Rust without an engine rebuild), NOT a Lua/path-to-script scanner (arbitrary
  transitive code), NOT a sandboxed plugin ABI (worst complexity/value here), NOT a Rego/OPA clone (a second
  language). It is "typed grep over known facts." Add `crates/fkst-framework/src/declarative_conformance.rs`
  with a `DeclarativeRulePack` implementing the existing `RulePack` trait, registered through the reserved
  `RulePackRegistry::from_options` seam (#159).
- **Pack file (TOML)** at `packages/<pkg>/.fkst/conformance/pack.toml`, declared from the package manifest
  `packages/<pkg>/fkst.package.toml` `[conformance] pack = ".fkst/conformance/pack.toml"`. The pack repeats
  its owner: `schema = 1`, `runner_protocol = "fkst-declarative-rulepack@1"`, `owner_package = "<pkg>"`,
  `[[rules]] id, severity="error", kind, scope, ...selectors..., message`. The engine validates: the manifest
  names `<pkg>`; the pack path is package-relative and inside the package root (no `..`/abs/symlink escape);
  `owner_package` equals the unit name; the lock records the same owner. **The host cannot override the pack
  path** (else rules stop traveling with the package).
- **v1 rule kinds (small, stable):** `max_line_count` (per included text file ≤ N), `text_forbid_regex`,
  `text_require_regex` (Rust `regex`, static message, no capture interpolation), `path_exists`, `path_forbid`,
  and `graph_field_regex` (regex over a fixed enum of normalized parsed-`Config` fields — package/queue/
  department/raiser/limit refs — NOT raw source). Each rule has `include`/`exclude` globs.
- **🔑 Scope boundary (the ugliest risk, fenced by construction):** a package-owned pack may inspect ONLY
  (a) `owner_package_files` — files under its own resolved package root — and (b) `graph_refs_to_owner` —
  parsed graph facts that reference the owner package. It may NOT read host files or other packages.
  Otherwise any transitive dependency could ship regex probes over a private host repo (a transitive
  host-source scanner — privacy, false positives, dependency-driven lint language) even without code
  execution. Generic HOST-wide linting is a different ownership model (a standalone published lint pack via an
  explicit `conformance_deps`), out of Part B's package-owned scope.
- **Strict + fail-closed:** unknown fields/kinds/scopes, unsupported `schema`/`runner_protocol`, invalid
  glob/regex, absolute/escaping paths, hash mismatch, a referenced package missing from the artifact index,
  and a referenced package that declares-but-is-missing its pack ALL fail conformance (non-zero exit, a
  `HostCheck` from an `engine.conformance-pack-loader` id). Absence of a pack file for a package that declares
  none is NOT a failure. No `includes`, no subprocess, no network, no fs reads outside the resolved roots.

## 6. Activation + resolver + lock (host names PACKAGES, packs are auto-discovered)

- **Activation = package reachability, reusing the package edge (do NOT add `conformance_deps` for owned
  packs).** Build the active-package set from package roots + the normalized graph's package references
  (`event_deps.packages` and any package-ref field). For each active package, resolve it to exactly ONE typed
  package artifact (coordinate = `external_source.id + package.name`; ambiguous short names fail until the
  binding is explicit), load its owned pack, validate, and run. So `site-board event_deps.packages=
  ["idle-detector"]` → `idle-detector`'s pack is active in fkst-website automatically. (A separate typed
  `conformance_deps` is reserved ONLY for later standalone generic lint packs that scan host files — a
  different ownership model — never for package-owned rules.)
- **Host lists PACKAGES, not packs.** `[[external_sources]] ... packages = ["idle-detector"]`. The resolver
  DISCOVERS the package-owned pack from the package manifest and locks it automatically — the rule travels
  with the package, it is NOT an opt-in the host could omit. `conformance_packs = [...]` is only for
  standalone non-owned packs.
- **Typed lock + resolver** (the verified 10-step extension): add `packages`/`conformance_packs` to
  `ExternalSourceDecl`; RELAX `validate_source_decl` from "≥1 library" to "≥1 artifact of any kind" (but never
  empty); add `ExternalPackageLock{name, unit, exports_sha256}` and `ExternalConformancePackLock{name,
  owner_package, unit, pack_sha256, schema, runner_protocol}` + arrays on `ExternalSourceLock`; extend
  cataloging from libraries-only to all artifact kinds (same `resolved.tree_sha256` content-addressed store —
  no new hash layer; plus a per-pack `pack_sha256` for reviewability/integrity); **stop discarding the
  `ExternalSourceCheckout.root`** in `deps_cli` (the runner needs it to read locked pack + scan locked source).
- **Symmetry (deletes `is_own_repo`):** fkst-packages resolves the SAME way via a workspace-local source
  (`source_id="workspace"`, root = repo root; packages/<pkg> cataloged with the same structs, just not
  serialized as external lock entries). The runner converts both local and external into ONE `ActivePackage
  {source_id, package_name, package_root, conformance_pack_path, tree_sha256?, pack_sha256, origin}` and does
  not care which. After this, the `is_own_repo` gate + the github-devloop hardcoding in `scripts/check_repo*`
  are deleted: a rule runs because its OWNER package is active, not because of repo identity.

## 7. Trust, conflict, waivers, bootstrap, ownership-honesty

- **Trust:** declarative data only — engine owns all capabilities; bounded regex/glob; package-local paths;
  owner match; no eval/shell/network/foreign-fs. The beautiful form is "engine owns capabilities, package
  supplies bounded policy data," not "a safer script."
- **Conflict:** pack coordinate `(source_id, owner_package, pack_name)` appears once; two active packs may not
  emit the same `pack/id` for the same package unless byte-identical by `pack_sha256`; conflicting severities
  fail closed (not last-writer-wins).
- **Waivers are HOST-owned, never package self-exemption:** `.fkst/conformance/waivers.toml` keyed by
  `{pack_coordinate, rule_id, target_package, target_path?, expires?, reason, owner}`. A consuming host does
  NOT honor an upstream package's self-waiver unless copied into the host waiver file. Owners publish
  invariants; consumers own local exceptions; neither silently disables the other's checks.
- **Bootstrap / toolchain:** `.fkst-substrate-ref` stays a DISTINCT toolchain coordinate (Cargo.lock pins deps,
  not rustc). The runner is part of the publicly-versioned `fkst-framework` toolchain; the lock pins package
  DATA (+ `runner_protocol` compat metadata), NOT the interpreter. No second package manager, no `tools`-lock
  self-reference circle.
- **Ownership honesty:** package-specific rules are package-owned packs (per-owner, never a B god-pack).
  Truly universal invariants do NOT become a "B engine-generic pack"; they are promoted into the engine
  schema/capability (see §8) so future hosts don't couple to B.

## 8. Migration order — promote / travel / delete, inventory-ratchet to 0

The ~8 github-devloop-hardcoded ratchets split three ways (don't preserve weak scans as the final form):
- **Promote to engine schema/capability** (make illegal states unrepresentable; substrate work, not data):
  produces ⊆ own+published-seam, namespaced-queue fidelity, event_deps shape, source_ref-on-
  reliable shape, saga spec-head/restart-row + liveness budget/actionable-epoch/responsibility_signature,
  span-contract completeness, monotone-gate declarations. (Anything a graph/schema fact can make impossible.)
- **Travel as package DATA** (pure static source-shape residue): e.g. G14 helper-clone, span wording,
  G-FORWARD-DIRECT marker-gated allowlist (interim, until queue authority is typed).
- **Delete as scaffolding** once the structural field exists: G-SAGA-SPLIT, G-MONOTONE-GATE(-DSL), and the
  source-scan parts of G8/G11/G12 after their concept is a typed restart-table/capability field.

Sequence (each migrated rule is deleted from Python in the SAME PR that adds its data/engine form — no dual
enforcement; a `migration/devloop-hardcoded-ratchets.inventory` shrinks N→0):
1. **Part A now** (§4) — done in fkst-website (split-brain consolidation), independent.
2. **Slice 1 (mechanism + host-travel proof):** substrate `DeclarativeRulePack` v1 + typed `packages`/
   `conformance_packs` lock/resolver + expose checkout root + scope boundary + activation; `idle-detector`
   ships one real declarative rule (`max_line_count`); fkst-website declares `packages=["idle-detector"]`,
   regenerates `fkst.lock`, drops the `.fkst/compose/package-roots` entry for it, and runs
   `fkst-framework conformance` so the SAME rule fires in fkst-packages (local self) AND fkst-website (locked
   external). Proves transport + symmetry across both repos.
3. **Slice 2 (ownership-deletion proof):** port `github-devloop/G-FORWARD-DIRECT` from `scripts/check_repo*.py`
   into `packages/github-devloop/.fkst/conformance/pack.toml` as `text_forbid_regex`, delete the Python branch
   + its `is_own_repo` gating. Proves the inverted dependency is removed (a github-devloop rule runs because
   github-devloop is the active owner, not because of repo identity).
4. **Then** inventory-ratchet the rest (promote/travel/delete per the split above) to N=0; retire host
   execution of B-private `check_repo.py`; `is_own_repo` deleted entirely.
5. **At 0:** one coordinate per source, one public command, per-owner typed packs that travel with the
   package, identical in fkst-packages and every host; zero duplication, zero second pin, zero private-script
   consumption, zero engine/repo-global hardcoded package names.

## 9. Non-goals

- Not a runtime change (conformance is build/CI-time).
- Not baking org/package policy into the engine.
- Not a big-bang rewrite.
- Not a second package manager / second fkst-packages pin / host config that duplicates the workspace graph.
- Not a general policy language (no Rego/OPA clone, no per-package code, no host-file scanning by package packs).
- Part A is NOT the one-resolver end state (it is honest interim debt); the v1 rule IR is deliberately tiny —
  rules that can be made structural belong in engine schema (§8), not preserved as ever-cleverer regex.

## 10. Adversarial record

### Round-3 DESIGN (2026-06-24) — 3 Codex (minimal/structural/delete) + ChatGPT Pro, converged on the §5–§8 contract
- **delete** (revise): the root issue is repo-global Python conflating structural engine invariants (promote)
  with a small package-owned source-scan residue (travel as data); avoid a general declarative IR / mini-OPA;
  one tiny data pattern-pack interpreted by one Rust pack.
- **minimal** (propose): the smallest proof is one declarative `max_line_count` rule carried by `idle-detector`
  (already referenced by fkst-website), one generic runner, activated by package reachability — proves
  transport + symmetry with the fewest moving parts; the lock/resolver typed-artifact is the irreducible
  engine work.
- **structural** (propose): make it a versioned fact contract (`rule_ir`/`runner_protocol`, fail-closed),
  per-owner pack coordinates, host-owned waivers, conflict fail-closed; don't overload `event_deps` for
  cross-package lint (reserve a later `conformance_deps` for standalone host-scanning packs only).
- **ChatGPT Pro** (keystone): host names PACKAGES (pack auto-discovered + locked from the package manifest, so
  rules travel and are not opt-in); the ugliest risk is a package pack scanning arbitrary HOST files — fence
  the scope to `owner_package_files` + `graph_refs_to_owner` only; v1 IR = "typed grep over known facts"
  (max_line_count / text_forbid_regex / text_require_regex / path_exists / path_forbid / graph_field_regex);
  first slice should DELETE a real hardcoded github-devloop ratchet (G-FORWARD-DIRECT), but the HOST-travel
  proof must use a package the host actually references (idle-detector) — do not add a fake host dependency.
Meta-judge `meta-layer convergence`: the §5–§8 design; Slice 1 (idle-detector, host-travel) + Slice 2
(github-devloop/G-FORWARD-DIRECT, ownership-deletion). No unresolved conflict edge.

### Round-2 REVIEW (2026-06-24) — 3 Codex (architecture/quality/tests) + ChatGPT Pro, ALL `reject` → `fix`
- **quality** (reject): the two revs are functionally incompatible (`45ef0324` has the host ratchet interface,
  `1734c42e` does not); deleting the side pin while keeping the stale lock rev regresses `run.sh check`; the
  pin has multiple consumers — the first step is a proxy fix unless it is an atomic single-rev move.
- **architecture** (reject): a THIRD consumer — `.github/workflows/ci.yml` pre-hydrates from
  `.fkst-packages-ref` independently; plus `FKST_PACKAGES_CONFORMANCE_ROOT` local override is a second
  checkout authority; verified the resolver schema is libraries-only (substrate source).
- **tests** (reject): the CI guard was under-specified to be a real test; the "engine conformance already
  loads testkit/devloop Lua conformance" claim is NOT supported by `host_conformance.rs` (6 checks +
  EngineRulePack, no testkit/saga runner); "~25" unverified; `conformance --config` currently ignores config.
- **ChatGPT Pro** (reject): keystone — the spec designed WHERE bytes are pinned, not the public semantic
  CONTRACT that makes them mandatory/scoped (pack declaration → named activation graph edge → fact model →
  declarative-IR-or-sandboxed-ABI → fail-closed → trust). "One lock" overstated → bootstrap circle
  (`.fkst-substrate-ref` is a legitimate toolchain coordinate; decide externally-pinned-toolchain vs
  bootstrapped-lock-artifact). `deps fetch` discards checkout locations (no public source-root lookup) →
  Part A is honest debt, not the one-resolver form. "Host cannot silently fall behind" is false for a lock.
  External `idle-detector` is not a locked artifact. Strongest objection (blocking): without the execution
  contract, implementation collapses to inverted-dependency / god-runner / private-consumption.
  Verdict: the SHA-consolidation can proceed independently if all consumers migrate atomically; the
  structural spec must not merge in its present form.

Meta-judge exit: `fix` → this revision SPLITS the spec: Part A (atomic consolidation) is implementable now;
Part B is downgraded to explicit open design questions (no false `implement`).

### Round-2 THINKING (2026-06-24) — converged the direction (one coordinate / per-owner packs / no standalone product)
minimal/structural `revise`, delete `reject (the standalone product)`, ChatGPT Pro `refute-shape/keep-thesis`.
### Round-1 (2026-06-23) — established published-seam thesis + 3-tier ownership (superseded in shape). History in git.

⟦AI:FKST⟧
