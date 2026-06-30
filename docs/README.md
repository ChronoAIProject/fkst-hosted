# fkst-packages docs

Docs are split by audience:

- **[`user/`](user/)** — for **operators** installing and running the system in a host repo:
  onboarding, install, configuration, operation. Read these to *use* fkst.
  - [`user/new-package-repo-bootstrap.md`](user/new-package-repo-bootstrap.md) — bootstrap a new
    fkst package/host repo scaffold.
  - [`user/github-devloop-dogfood-topology.md`](user/github-devloop-dogfood-topology.md) —
    reproduce the per-device `github-devloop` dogfood branch topology on another machine.
  - [`user/control-planes-and-host-repo-composition.md`](user/control-planes-and-host-repo-composition.md) —
    the three control planes (product / host-run / dogfood-operator), how a host repo composes the
    platform via `.fkst-*-ref` pins + `.fkst/local-packages/`, and host-repo conformance with zero rebuild.
  - [`user/global-host-profiles.md`](user/global-host-profiles.md) — XDG-style host-local
    environment profiles for no-repo-pollution FKST runs.

- **[`dev/`](dev/)** — for **contributors** developing the system: design specs, architecture, and
  methodology. Read these to *change* fkst.
  - [`dev/devloop-design.md`](dev/devloop-design.md) — `github-devloop` state machine and design.
  - [`dev/consensus-converge-redesign.md`](dev/consensus-converge-redesign.md) — consensus
    converge/reconcile redesign.
  - [`dev/harness-construction-methodology.md`](dev/harness-construction-methodology.md) —
    harness-first methodology.
  - [`dev/observability-legibility.md`](dev/observability-legibility.md) — local health verdict and
    board classification contract.
  - [`dev/scaffold-install-upgrade-design.md`](dev/scaffold-install-upgrade-design.md) — host-repo
    scaffold install + upgrade + package-reference update design.

The authoritative engine↔package contract lives in fkst-substrate's `docs/package-repo-contract.md`;
repo conventions and commands are in this repo's top-level `README.md`.

（中文：本仓文档按受众分区——`user/` 给装/跑系统的运维者，`dev/` 给改系统的开发者。引擎↔包契约权威在
fkst-substrate 的 `docs/package-repo-contract.md`，包约定与命令在仓根 `README.md`。）

⟦AI:FKST⟧
