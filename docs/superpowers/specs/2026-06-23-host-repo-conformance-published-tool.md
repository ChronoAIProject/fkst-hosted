# Conformance harness for host repos: a published, versioned rule-pack tool (no per-repo rebuild)

Status: DESIGN — sshx adversarial (minimal/structural/delete triplet + ChatGPT Pro), converged `implement`.
Date: 2026-06-23
Scope: fkst-substrate (engine + published conformance tool), fkst-packages (rule packs + migration), host repos (fkst-website + others: thin runner, delete the copy).

## 1. Problem (verified at source)

The conformance/harness is SPLIT and DUPLICATED per repo:
- fkst-packages has ~25 Python static ratchets `scripts/check_repo*.py` (G-ADAPTER gh/git boundary, G-DEDUP,
  G-PRODUCER-LIVENESS, saga-handler, ingress, forward_direct, monotone_gate, content_truncation, coverage,
  fkst_layout, dogfood_boundary, github_devloop_helpers, ...) + Lua runtime conformance in `testkit`
  (`saga_conformance.lua`, `namespaced_dispatch_conformance.lua`) run by the engine in test mode.
- **The host repo fkst-website ALREADY has a COPIED `scripts/check_repo.py` + `check_repo_test.py`** — it
  rebuilt the infrastructure. Every host repo with its own packages (fkst-website's `site-board`, substrate's
  own dogfood packages) would re-copy the ratchet stack. The copy DRIFTS: it silently falls behind
  fkst-packages' ratchets, so a host's own package escapes a ratchet the platform already enforces.

Boundary (CLAUDE.md): `forge`/`testkit` are PRIVATE to library B; host repos compose B's PACKAGES only via
`pkg.queue` limited names and must NOT consume B's private libraries/scripts unless B publishes a
named/versioned public API. So the shared mechanism must be a PUBLISHED seam, not host repos reaching into B.

## 2. Harness (prior art)

- **Versioned linter platform with rule packs** (ESLint shareable configs + plugins; Python entry-point
  plugins): generic rules authored once, distributed as a versioned product; each consumer provides config
  (which rules, baselines/waivers) and invokes the CLI. "Shared CI tooling" is only the invocation, not the
  ownership model.
- **Policy-as-code** (OPA/Conftest): policies authored centrally, evaluated against each repo's facts.
- **Compiler analysis passes**: intrinsic-validity checks belong in the compiler (engine); style/architecture
  checks are a separable linter.
- CLAUDE.md «框架做稳定公共部分»/«通用>枚举·原语层>业务语义层»/«分层归属»/«Harness本质 PREVENT>DETECT».

## 3. Design — three tiers by ownership (the converged invariant)

> Generic, project-agnostic conformance is authored ONCE as an engine-owned, INDEPENDENTLY VERSIONED,
> PUBLISHED product (a rule-pack linter platform). Any repo — library B or a host — gets it by INVOKING its
> CLI with only its repo-specific config (package roots + baselines/allowlists). Host repos consume only the
> CLI + config schema; they import NOTHING from library B. Per-repo `check_repo*.py` copies are deleted.

| Tier | Home | Owns | Must NOT own |
|---|---|---|---|
| **1. Engine built-in validator** | fkst-substrate (engine runtime) | INTRINSIC package invalidity: malformed package metadata, duplicate runtime identifiers, impossible saga graphs, unresolved refs in a closed-world composition (the engine already does graph-contract / published-seam) | org-specific architecture, migration ratchets, allowlists, B conventions |
| **2. Published `event-conformance` tool** | fkst-substrate monorepo but SEPARATELY RELEASABLE from the runtime; versioned + pinned | generic SOURCE-architecture ratchets as RULE PACKS (engine generic pack: adapter-boundary, dedup, producer-liveness, line/file limits, ingress, forward-direct, monotone-gate, content-truncation, coverage, layout) + an OPTIONAL B-public rule pack; baselines/waivers (the allowlists); rule orchestration; diagnostics (SARIF/JSON + exit status) | B PRIVATE layout knowledge; host-specific rules |
| **3. Engine-run Lua conformance** | fkst-substrate engine test mode (driver) + testkit (the Lua rules) | properties needing EXECUTION: saga runtime/compensation, scheduling, ordering, event-flow, runtime liveness, namespaced dispatch | filesystem traversal / static source scanning |

Host invocation (the published seam):
```
host repo:  conformance.toml  (which rule packs + baselines/waivers + package roots)
            <pinned engine + conformance tool version>
   ▼  invoke ONE public command (thin runner: pin/build fkst-framework, run --self-test + the conformance CLI)
event-conformance CLI
   ├── engine generic rule pack        (tier 2, shared)
   ├── optional B public rule pack      (tier 2, B's published conventions)
   ├── static source/facts analyzer     (tier 2 engine)
   └── engine-run Lua conformance driver (tier 3)
   ▼  diagnostics (SARIF/JSON) + exit status
```

## 4. Generic-vs-specific split (the rule-pack boundary)

- **Generic (engine generic rule pack — shared by every repo)**: package-root discovery/layout, file line
  limits, Lua test shape + helper reachability, gh/git adapter boundary, dedup, producer-liveness, ingress
  fail-closed, forward-direct, monotone-gate, content-truncation, coverage. These apply to ANY fkst package.
- **Library-B-specific (stay in B's own conformance config / a B-private pack)**: `github_devloop_helpers`,
  `dogfood_boundary`, devloop product knowledge. NOT in the shared generic pack.
- **Repo-specific config (each repo provides)**: which packs to enable, the baselines/allowlists (each repo's
  `migration/*.allowlist` become the tool's per-repo waivers), the package roots.

## 5. Versioning (no silent drift — the key trap GPT Pro flagged)

- The `event-conformance` tool is INDEPENDENTLY VERSIONED + PINNED per repo (a lock, like `event-packages.lock`).
- A host CANNOT silently fall behind: the runner pins a version; a stale pin is visible (the lock), and the
  shared CI surfaces the tool version. (Contrast the current copied check_repo.py: invisible drift.)
- A host CANNOT silently opt out of a generic rule: enabling the engine generic pack is the default; disabling
  a rule requires an explicit, visible waiver in `conformance.toml`, not a silent absence.

## 6. Host repos: thin runner + DELETE the copy

- Each host repo keeps only a THIN runner (`scripts/run.sh`): pin/build fkst-framework + the conformance
  tool, run `--self-test`, run the conformance CLI with `conformance.toml`. NO `check_repo*.py`.
- **DELETE fkst-website's copied `scripts/check_repo.py` + `check_repo_test.py`** (and any other host copy),
  replaced by the CLI invocation. This is the «删» — N copies collapse to 1 published seam.

## 7. Migration — phased inventory-ratchet (not big-bang)

Per «迁移=inventory ratchet»: do NOT rewrite all 25 ratchets at once.
1. **Engine command skeleton** (fkst-substrate): `fkst-framework conformance --project-root --package-root
   --config conformance.toml` that runs the EXISTING engine graph/saga conformance + a rule-pack registry
   (empty generic pack initially). Ship it; pin it.
2. **Move generic ratchets into the engine generic rule pack** one-by-one (an inventory manifest of the ~25;
   each migrated ratchet is removed from fkst-packages `scripts/` and added to the pack; fkst-packages itself
   switches to invoking the CLI, proving the seam). Shrink-only: the `scripts/check_repo*.py` count ratchets
   to 0 (B-specific ones move to a B-private pack).
3. **Host adoption**: fkst-website (and substrate's own dogfood) replace their copied `check_repo.py` with the
   thin runner + `conformance.toml`; delete the copies.
4. **At 0**: one generic rule pack, authored once; every repo (B + hosts) invokes it; zero duplication.

## 8. Parallel implementation (the user's 并行实施)

Independent tracks (parallelizable):
- **Track E (fkst-substrate)**: the `fkst-framework conformance` command + rule-pack registry + the engine
  built-in validator tier-1 + the Lua driver tier-3. (engine PRs)
- **Track P (fkst-packages)**: expose the generic ratchets as the engine generic rule pack (migrate one-by-one,
  inventory-ratchet); a B-private pack for B-specific ratchets; switch fkst-packages' own `run.sh` to the CLI.
- **Track H (host repos)**: the thin runner + `conformance.toml` + DELETE the copied check_repo; first
  fkst-website (site-board), then substrate's own dogfood packages.
Track E unblocks P and H (the CLI must exist first); within E/P the ratchet migration is per-ratchet parallel.
COORDINATE with the other machine's `libraries/` refactor (testkit moves) — the tier-3 Lua rules live in
testkit; note the seam.

## 9. Non-goals

- Not a runtime change (conformance is build/CI-time).
- Not baking org-specific ratchets into the engine (those stay in repo config / B-private pack).
- Not a big-bang rewrite (phased inventory-ratchet to 0).
- The tool is separately releasable from the engine runtime (versioned independently).

## 10. Adversarial record

`sshx`: minimal/structural/delete triplet (3× propose) + ChatGPT Pro, converged `implement`.
- **minimal**: engine-first — extend `fkst-framework conformance` (it already owns --project-root/--package-root);
  host keeps a thin runner that pins/builds the engine + runs the CLI.
- **structural**: engine for true package-shape/graph invariants; a published VERSIONED package for the
  generic source ratchets; host invokes with config only (repo root, package roots, baselines).
- **delete**: engine/fkst-substrate as a published versioned package-repo conformance command shipped with the
  engine; host runs ONE public command with repo config; DELETE the copies.
- **ChatGPT Pro**: make it an engine-owned, INDEPENDENTLY VERSIONED, PUBLISHED product (`event-conformance`),
  separately releasable from the runtime; hosts consume only its CLI + config schema, import nothing from B;
  the 3-tier split (engine built-in / published tool / engine-run Lua); rule packs (engine generic + optional
  B public); versioning/pinning so hosts can't silently drift or opt out; mature name = versioned linter
  platform with rule packs (ESLint shareable configs + plugins).

```
[goal: host repos' own packages get conformance WITHOUT rebuilding] ──resolved-by──> [published versioned rule-pack tool]
   │ converges-to (minimal+structural+delete+GPT Pro)                      │
[copied check_repo.py drifts/duplicates] ──deleted-by──────────────────────┘
   │ depends-on                                                            │ depends-on
   ▼                                                                       ▼
[3 tiers: engine built-in / published tool / engine-run Lua] ◀─agree─ GPT Pro (the ownership split)
   │ depends-on                                                            │
   ▼                                                                       ▼
[host invokes CLI with config only; imports nothing from B] ──boundary──> [published seam, not private consumption]
   │ depends-on
   ▼
[independently versioned + pinned → no silent drift / no silent opt-out]
```

Meta-judge `implement`: unanimous on engine-owned-published-tool + 3-tier ownership + host-thin-runner +
delete-the-copy + phased inventory-ratchet; ChatGPT Pro's "independently versioned, separately releasable,
hosts consume only CLI+schema" + the 3-tier table is the keystone resolving where each class lives. No
unresolved conflict edge. Parallel tracks E/P/H, E unblocks P+H.

⟦AI:FKST⟧
