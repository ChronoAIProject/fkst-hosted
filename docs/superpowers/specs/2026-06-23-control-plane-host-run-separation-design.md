# Control-plane / host-run separation — design spec

Status: DRAFT (adversarial review pending). Scope: the **control-plane half** of the
fkst-packages architecture separation. The **library half** (`contract → contract/workflow/
testkit`, `std → forge`, generic-liveness extraction + archaudit boundary, `contract.strings`
slimming) is handled separately on another machine; this spec must not touch `libraries/`,
`std/`, or `archaudit/` so the two efforts do not collide.

## 1. Problem

Every **host-launch invariant** — restart-to-deploy (`FKST_RUNTIME_ROOT` scratch /
`FKST_DURABLE_ROOT` stable-reused), BIN resolve + freshness-rebuild, `FKST_RATE_POOL_ROOT`,
`--package-root` wiring, `FKST_GITHUB_WRITE` posture, SIGKILL-to-release-the-redb-lock —
is baked into `.claude/skills/dogfood-github-devloop/dogfood.sh` (lines ~35–261). Therefore a
**single host** (the platform supervising one repo's issues) **cannot be launched without the
dogfood operator skill**. The operator tooling has become the only thing that knows how to run
one host.

This is the **control-plane tangle**: operator-level *orchestration* and host-level *control*
live in one script. It is the same disease as the library tangle (generic primitives living in
the `devloop` pipeline library) — generic/platform concerns fused with operator/pipeline concerns.

## 2. The three control planes (and their homes)

| Plane | What it is | Home | Hard rule |
|---|---|---|---|
| **PRODUCT / platform** | `packages/` + `libraries/` + `std/` + engine ABI contract + conformance + contract-tests | fkst-packages proper | hosts consume it only through package-roots / queue seams |
| **HOST-RUN contract** | how **one** host validly launches/relaunches the platform on **its** issues (the launch invariants) | a uniform per-host runner shipped with the platform (`scripts/run.sh`) | **a single host MUST be runnable without `.claude/skills`** |
| **DOGFOOD-OPERATOR** | coordinating **many** hosts across machines (which repos this machine drives, integration topology, board/doctor/sync sweep) | `.claude/skills/dogfood-github-devloop/dogfood.sh` | **must NOT know how one host supervises itself** — it delegates to the host-run contract |

ChatGPT Pro's framing (the keystone rule): *"A single host must be runnable without
`.claude/skills`. The dogfood skill may coordinate many hosts, but it must not know how one host
actually supervises itself."*

## 3. Design

### 3.1 HOST-RUN contract (`scripts/run.sh supervise`)

A single command launches one host correctly, with no dependency on `.claude/skills`. The CLI
makes the host shape **explicit** — it does NOT auto-discover the project-root's local packages
(auto-discovery would hide the packages-vs-website layout difference and mis-load):

```
scripts/run.sh supervise \
  --project-root      <HOST>          # the repo being supervised (the engine's --project-root)
  --platform-root     <PKGSRC>        # where the platform trio lives (== HOST for fkst-packages; a sibling fkst-packages clone for substrate/website)
  --platform-packages "<names>"       # platform package names, loaded from <PKGSRC>/packages/<name>
  [--host-packages    "<names>"]      # the host's OWN packages (fkst-packages: from packages/; website: from .fkst/local-packages/)
  --durable-root      <path>          # MANDATORY, fail-closed — the stable redb store, reused across launches; NEVER defaulted
  [--runtime-root     <base>]         # scratch base; a fresh child is created per launch (defaults to a fresh temp dir)
  [--restart]                         # SIGKILL the prior supervise holding this durable-root, then launch
```

The three host shapes are modeled explicitly, not hidden behind auto-discovery:

| Host | `--project-root` | `--platform-root` | `--host-packages` source |
|---|---|---|---|
| fkst-packages (its own host) | the packages clone | == project-root | `packages/*` |
| fkst-substrate | the substrate clone | a sibling fkst-packages clone | (none) |
| fkst-website | the website clone | a sibling fkst-packages clone | `.fkst/local-packages/*` |

`fkst-packages-as-its-own-host` is uniform with the others (HOST==PLATFORM-ROOT is the only
difference, expressed as data, not a code branch).

It owns the launch invariants (moved out of dogfood.sh):

- **BIN**: resolve (`BIN` env > `.fkst/env` > PATH > sibling `../fkst-substrate`); rebuild from
  substrate when stale (origin/dev ahead OR any crate `.rs` newer than the BIN), unless
  `FKST_NO_AUTOBUILD`. Export `BIN` (not only `--framework-bin`) so spawned codex can run the suite.
- **Runtime/durable**: `FKST_RUNTIME_ROOT` is **scratch** (fresh per launch); `FKST_DURABLE_ROOT`
  is the **stable** redb store, **reused** across launches. `--durable-root` is **MANDATORY and
  fail-closed** — the host-run contract errors if it is absent. It is NEVER defaulted (defaulting
  to e.g. the project-root's `.fkst/durable` could point at a *different* store than the running
  supervise's and **strand all in-flight work** — #62/#78). Never fresh the durable on a normal
  relaunch (restart-to-deploy). The *choice* of stable path is the operator's (per-machine
  `DUR_*`); the *reuse* invariant is enforced here.
- **Package roots**: from the declared package set + the project-root's own `.fkst/local-packages`.
- **Posture**: `FKST_GITHUB_WRITE` is the only write switch (unset = dry-run).
- **Relaunch**: SIGKILL the prior supervise (release the redb lock) before relaunch.

Result: `scripts/run.sh supervise` is the platform's **"how to run me" contract** — identical for
every host (fkst-packages-as-its-own-host, fkst-substrate, fkst-website). fkst-packages
dogfooding itself is **just another host**; it is not special-cased.

### 3.2 DOGFOOD-OPERATOR (`dogfood.sh`) becomes pure orchestration

`dogfood.sh` keeps ONLY the multi-host / multi-machine concerns and **delegates** the actual
launch:

- per-machine config resolution (`DOGFOOD_ROOT`, `BOT`, `INTEGRATION_BRANCH`, `DOGFOOD_REPOS`,
  the stable `DUR_*` durable roots) — operator topology, not host control.
- checkout management (each target's run-checkouts on the machine's integration branch).
- the multi-target sweep: `status | doctor | config | board | bin | sync` across N hosts.
- `start | restart | stop`: resolve the target's project-root + package set + the **stable
  durable root** (operator-chosen, per-machine), then call `scripts/run.sh supervise …`. The
  durable-root *reuse* invariant is enforced by the host-run contract; the *choice* of which
  stable path (per-machine) stays operator config.

`dogfood.sh` no longer re-implements BIN-freshness, runtime/durable wiring, package-root
construction, or the launch env — it passes the topology to the host-run contract.

### 3.3 Boundary invariant (mechanical — keeps it from re-blurring)

A ratchet in `scripts/check_repo.py` (or a dogfood-skill self-check) asserts the dogfood skill
carries **no host-level launch logic**: `dogfood.sh`'s start/restart path must route through
`scripts/run.sh supervise` and must not itself construct `--package-root` / set
`FKST_RUNTIME_ROOT` / invoke the framework BIN directly. Shrink-only allowlist during migration.

## 4. Migration (incremental — the live dogfood must not break)

1. **Extend `scripts/run.sh supervise`** to the explicit host-run CLI above, owning the full
   launch contract (absorb the invariants from dogfood.sh). Prove a single host launches via
   `scripts/run.sh supervise …` alone (no dogfood.sh) for fkst-packages — supervise up, panic 0,
   loads packages. Add a host-run harness/test exercising all three host SHAPES (packages /
   substrate-trio-external / website-local-packages) so the layout differences are covered.
2. **Refactor `dogfood.sh` start/restart to delegate** to `scripts/run.sh supervise`, passing the
   per-machine topology — CRITICALLY the **existing stable durable roots
   (`DUR_PACKAGES`/`DUR_SUBSTRATE`/`DUR_WEBSITE`) UNCHANGED, byte-for-byte**, as `--durable-root`,
   so no in-flight work is stranded. Before switching, produce an **old/new command-equivalence
   table** proving the delegated launch is identical — same `--project-root`, platform/host
   package sets, durable root, runtime-scratch naming, BIN, and env (`FKST_GITHUB_WRITE`, BOT,
   rate-pool) — for **all three repos**. Keep config / integration-checkout / board / doctor /
   sync orchestration in dogfood.sh.
3. **Add the boundary ratchet** (dogfood.sh start/restart routes through `scripts/run.sh
   supervise` and constructs no `--package-root` / sets no `FKST_RUNTIME_ROOT` / invokes the BIN
   directly), shrink-only allowlist to 0.

After EACH step, verify all three repos (packages / substrate / website) still launch and the
live dogfood keeps flowing (no stranded in-flight work — same durable root reused).

## 6. Collision avoidance (two efforts in parallel)

The library refactor runs on another machine. Verified low collision in the supervise launch
path: `scripts/run.sh` + `dogfood.sh` do **not** reference `libraries/contract`, `std`, or
`archaudit` paths that the `std→forge` / contract-split rename moves. The one **shared file** is
`scripts/check_repo.py` (this spec adds a boundary ratchet there; the library refactor may add its
own ratchets). Mitigation: keep this spec's `check_repo.py` edit to a single additive ratchet
module + one registration line, rebase on the library refactor if it lands first, and never
touch `libraries/`, `std/`, or `archaudit/` here. If `scripts/run.sh` test-wiring references a
library path the rename moves, that line is the library refactor's to change, not this one's.

## 5. Constraint (the goal's hard gate)

All three host repos — **fkst-packages, fkst-substrate, fkst-website** — must remain runnable
throughout: each `scripts/run.sh supervise` launches its host, and the live dogfoods keep
flowing. The migration is reversible at each step (the durable root is reused, so no work is
stranded).

⟦AI:FKST⟧
