# Control planes & host-repo composition

fkst-packages holds **three kinds of code**, cleanly separated and mechanically enforced. This doc is the
map: what each plane owns, how a **host repo** (e.g. fkst-website, or fkst-substrate running its own dogfood)
composes the platform on its own content, and how a host repo gets the same **conformance** guarantees
without rebuilding any infrastructure.

## 1. The three control planes

| Plane | Lives in | Owns | Must NOT own |
|---|---|---|---|
| **PRODUCT** | `packages/`, `libraries/` | the platform itself: agent packages (the `github-devloop` trio + the rest) and workspace libraries (`contract` / `workflow` / `testkit` / `forge` / `devloop`), targeting the engine ABI | how a host launches; multi-host orchestration |
| **HOST-RUN contract** | `scripts/host_run.sh` (invoked via `scripts/run.sh supervise`) | ALL launch invariants for **one** host: BIN resolve + freshness rebuild, target `fkst.workspace.toml` package selection, trusted `--platform-root` provenance, runtime-scratch, `--durable-root` (mandatory, fail-closed — never defaulted), the 3-host-shape `--package-root` wiring, `FKST_GITHUB_WRITE` posture, pidfile-based `--restart` (kill -9 + verify-dead, refuses a 2nd supervise on the same durable root) | which hosts run; product logic |
| **DOGFOOD-OPERATOR** | `.claude/skills/dogfood-github-devloop/dogfood.sh` | coordinating **N** hosts: per-machine config, run-checkout sync, `board` / `doctor` / `sync` / `stop`, the integration topology | how **one** host supervises itself — it **delegates** that to the host-run contract |

**Keystone rule**: a single host MUST be runnable without `.claude/skills`. The dogfood operator coordinates
many hosts but must not know how one supervises itself. So `dogfood.sh start/restart` **delegate** to
`scripts/run.sh supervise --project-root <HOST> --platform-root <PKGSRC> --platform-packages "<names>"
--durable-root <path> [--restart]`; the operator constructs no `--package-root`, sets no `FKST_RUNTIME_ROOT`,
and invokes no BIN directly.

**Mechanically enforced** (so the boundary can't rot):
- `scripts/check_repo_dogfood_boundary.py` — the operator's launch functions must delegate via
  `scripts/run.sh supervise` and are forbidden from constructing `--package-root`, setting
  `FKST_RUNTIME_ROOT`, or invoking BIN directly.
- `scripts/host_run_equivalence_test.py` — a golden-master that regenerates the delegated launch for the
  packages/substrate/website host shapes and asserts it matches a committed, machine-independent fixture
  (the «refactoring-is-behavior-preserving» gate).

(Landed: PR #1375 extract host-run contract + #1376 hermetic equivalence test; deployed + soak-validated.)

## 2. How a host repo composes the platform

A **host repo** is a repo whose primary source is NOT Lua packages — fkst-website is website source,
fkst-substrate is the engine — but which RUNS the platform on its own content plus its own small package(s).
It does NOT vendor or copy the platform; it **composes** it and **pins** versions.

```
  HOST REPO (e.g. fkst-website)
  ├── <website source ...>                  # the repo's primary content
  ├── .fkst-substrate-ref                    # PIN: engine (fkst-substrate) SHA
  ├── fkst.workspace.toml                     # declares fkst-packages-platform external source
  ├── fkst.lock                               # locks the platform packages source and artifacts
  ├── .fkst/local-packages/<pkg>/            # the host's OWN package (e.g. site-board), composed into the graph
  ├── .fkst/local-libraries/<lib>/           # host-owned workspace libraries, if any
  ├── .fkst/compose/package-roots             # host composition roots, see ADR 0002
  └── .fkst/conformance/allowlists/           # host conformance allowlists, see ADR 0002
        │ composes (pkg.queue limited names; no cross-require, no vendoring)
        ▼
  PLATFORM (from the trusted fkst-packages checkout supplied as --platform-root)
    packages/{github-devloop, github-devloop-pr, github-devloop-intake, …, consensus, github-proxy, archaudit, idle-detector}
    libraries/{contract, workflow, testkit, forge, devloop}
        │ runs on
        ▼
  ENGINE (a pinned fkst-substrate build)
```

The host supervise resolves the requested platform package names against the target
`fkst.workspace.toml`, hydrates only the external source IDs that own those packages from `fkst.lock`, and
loads those package roots from `<HOST>/.fkst/run/<id>/packages/<pkg>`. For external platform sources, the
target manifest and lock must match the trusted `--platform-root` git URL/path and `HEAD` before hydration;
target files select package ownership but cannot redirect executable platform provenance. Target `workspace`
packages can supply platform packages only when the target root is the trusted platform root itself. Host-owned
packages still come from `.fkst/local-packages/`, all on the same engine BIN — see
`docs/user/github-devloop-dogfood-topology.md` for the dogfood directory layout.

If the target manifest is absent, does not declare a requested platform package, or declares that package in
more than one source, host supervise fails before launch with a narrow diagnostic. `--platform-root` is the
trusted provenance authority for platform execution; target `fkst.workspace.toml` and `fkst.lock` must agree
with it when the target workspace declares external platform packages.

## 3. Host-repo conformance — no per-repo rebuild

The conformance ratchets are a **common, stable part**: authored once, invoked by any repo. A host repo gets
the SAME guarantees (line limits, adapter boundary, dedup, producer-liveness, saga, …) **without copying**
the check_repo infrastructure. Three tiers by ownership:

| Tier | Home | Owns |
|---|---|---|
| **Engine built-in** | fkst-substrate | `fkst-framework conformance --project-root --package-root` — intrinsic validity: graph contract, published-seam, saga |
| **Shared source ratchets** | fkst-packages `scripts/check_repo.py --project-root <repo>` | the generic source ratchets run over ANY repo's tree (discovering packages from both `<root>/packages/*` and `<root>/.fkst/local-packages/*`); library-B-specific ratchets gate on own-repo |
| **Engine-run Lua** | `libraries/testkit` | execution conformance (saga runtime, namespaced dispatch) via the engine in test mode |

A host repo's `scripts/run.sh check` invokes the **shared** `check_repo.py` from the trusted fkst-packages
checkout supplied as `--platform-root`, plus `fkst-framework conformance`, providing ONLY its config (its
package roots + its own waivers). It carries **no copied check_repo**.
(fkst-website's former 610-line copy is gone.)

## 4. Conventions a new host repo follows

- **The platform packages selector** is `fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock`, with
  `fkst-packages-platform` as the source identity. Its git URL/path and locked rev must match the trusted
  `--platform-root` checkout before host supervise executes platform packages. `.fkst-substrate-ref` remains
  the engine toolchain pin when a host uses a checked-out substrate build.
- **The host's own package(s)** live under `.fkst/local-packages/<pkg>/`.
- **Host composition roots** live under `.fkst/compose/package-roots`; host conformance allowlists stay under
  `.fkst/conformance/allowlists/`. See [`docs/adr/0002-host-fkst-layout.md`](../adr/0002-host-fkst-layout.md).
- **`.fkst/` is the host runtime/interface directory** (tracked + ignored mix): committed host-owned bits
  (`local-packages`, `local-libraries`, `conformance`, `compose`) plus gitignored engine scratch
  (`runtime/`, `durable/`).

### Frontend application workflow profile

Frontend application hosts use the same host-repo composition contract as any other non-Lua host repo. The
profile is composition, not a separate platform package:

- Load the platform packages that own the lifecycle: `github-proxy`, `consensus`, `github-devloop`,
  `github-devloop-pr`, and `github-devloop-intake`.
- Put host-specific UI adapters, boards, browser probes, or app metadata under
  `.fkst/local-packages/<host-package>/`.
- List those host-owned package roots in `.fkst/compose/package-roots` so the shared conformance tiers test
  them with the pinned platform graph.
- Keep frontend-specific checks as host package behavior or host CI commands. The platform `github-devloop`
  lifecycle remains the single source of truth for issue intake, implementation, PR review, fixing, and merge.

Do not add a standalone `frontend-devloop` package/profile unless it has a distinct lifecycle contract that
cannot be represented by host composition. This keeps frontend workflow support on the same DRY, single-owner
path as the existing `github-devloop` platform instead of creating a second source of truth.

## 5. The big picture

```
                 PRODUCT  (packages/ + libraries/)            ← what the platform IS
                    ▲ targets ABI            ▲ composed by
                    │                         │
   ENGINE (fkst-substrate, pinned) ───────────┤
                    ▲ launched by             │
                    │                         │
        HOST-RUN contract (host_run.sh)       │  ← how ONE host launches (all invariants)
                    ▲ delegated to            │
                    │                         │
   DOGFOOD-OPERATOR (dogfood.sh) ─ coordinates N hosts, NO launch logic (ratchet-enforced)
                                              │
   HOST REPO (fkst-website / substrate) ──────┘  ← composes the platform via workspace external_sources + lock
                                                    + .fkst/local-packages/, gets conformance via the
                                                    shared tiers with ZERO rebuilt infrastructure
```

The separation is **complete and ratchet-guarded**: the operator can't dribble launch logic into itself, a
single host runs without the operator skill, and a host repo composes + conforms to the platform without
copying any of it.

⟦AI:FKST⟧
