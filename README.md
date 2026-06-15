# fkst-packages

`fkst-packages` is the official package library for `fkst`: reusable Lua packages that run on the
separate `fkst-substrate` engine. The repository contains behavior-layer packages, tests, and
package documentation; it does not contain engine Rust code and does not store host application
state.

中文补注：本仓是 `fkst` 的官方包库（库 B），只放运行在 `fkst-substrate` 引擎上的 Lua 行为层 package。

## Project Status

- License: Apache-2.0, see [`LICENSE`](LICENSE).
- CI: `.github/workflows/ci.yml` builds `fkst-framework` from `ChronoAIProject/fkst-substrate` and
  runs `scripts/run.sh test`.
- Default integration branch: `dev`.
- Engine source pin: `.fkst/substrate-ref` when present, otherwise CI falls back to `dev`.

## What This Repository Provides

`fkst` is split into an engine and package repositories. `fkst-substrate` owns the runtime,
delivery, SDK primitives, conformance checks, and `fkst-framework` binary. `fkst-packages` is
library B: it defines Lua packages loaded under `.fkst/packages/`, with departments, raisers,
package-local shared code, and tests.

Packages communicate through event queues. Flat packages are self-contained and use bare queue
names internally. Composed packages are first-class packages that adapt or combine sibling package
queues and declare those siblings in `composed.deps` so composed conformance can test the union
graph.

## Quickstart

Clone this repository, then configure a local `fkst-framework` binary:

```sh
cp .fkst/env.example .fkst/env
$EDITOR .fkst/env
```

Set `BIN` in `.fkst/env` to a built `fkst-framework`, usually from a sibling
`fkst-substrate` checkout:

```sh
BIN=/path/to/fkst-substrate/target/debug/fkst-framework
```

Run the same test entrypoint used by CI:

```sh
scripts/run.sh test
```

For a read-only host preflight:

```sh
scripts/run.sh doctor
```

For a one-shot department run:

```sh
scripts/run.sh run <package> <department> '{"payload":{}}'
```

For a real foreground supervisor:

```sh
FKST_GITHUB_REPO=owner/repo \
FKST_RATE_POOL_ROOT=/var/lib/fkst/rate-pools \
scripts/run.sh supervise github-proxy
```

`scripts/run.sh` resolves `fkst-framework` in this order: explicit `BIN`, `.fkst/env`, `PATH`,
a sibling `../fkst-substrate`, then the `.fkst/substrate-ref` source-cache fallback for local
non-CI runs. Invalid explicit `BIN` values fail closed. CI builds the engine itself and does not
silently use a stale binary.

## Package Layout

Each package root follows this shape:

```text
packages/<name>/
  core.lua
  departments/<department>/main.lua
  raisers/<raiser>.lua
  tests/*_test.lua
```

`core.lua` is package-local shared code and is required as `require("core")`. Larger departments may
split stable local responsibilities into files beside `main.lua`, such as
`require("departments.<department>.<module>")`. Packages do not cross-require sibling package code;
cross-package composition goes through event queues.

The runtime package view is `.fkst/packages/`. In this repository it is a relative symlink to
`packages/`; host repositories may place runtime packages directly under `.fkst/packages/`.

## Package Catalog

Flat packages:

- `github-proxy`: bridges GitHub issue and PR facts into fkst events, and handles dry-run-by-default
  outbound GitHub comments, labels, PR creation, and related requests.
- `consensus`: source-agnostic multi-angle `codex` consensus over abstract `proposal` events,
  producing `consensus_reached` or bounded `consensus_converge` events.

Composed packages:

- `autochrono`: maps its own `issue` protocol into `consensus.proposal` and maps reached consensus
  back into its own `reply` protocol.
- `github-autochrono`: composes `github-proxy` and `autochrono` as a GitHub issue-to-reply adapter.
- `github-devloop`: composes `github-proxy` and `consensus` into the autonomous GitHub issue to PR
  loop, using trusted GitHub marker facts, version-CAS state transitions, head-bound PR review, and
  deterministic merge gates.

## Architecture Overview

The package contract has three levels:

```text
Company
  -> Department
     -> Person
```

- Company: supervisor, framework, and composed graph.
- Department: `departments/<dept>/main.lua` with `M.spec` and `pipeline(event)`.
- Person: one `codex exec` invocation.

The event flow is:

```text
source -> fanout -> route -> spawn -> RAISED
```

Department inputs are `Event{queue, payload, ts}` values. There are no lifecycle hooks, shared
memory, or durable package-local state between pipeline invocations. Durable truth must come from
git, external systems such as GitHub, or explicit host facts. Reliable delivery payloads stay small:
they carry stable pointers such as `source_ref`, schema, dedup keys, versions, and short control
fields. Large issue bodies, PR diffs, comments, code, and files are fetched from source by the
consumer that needs them.

## Testing and Repository Guards

Use `scripts/run.sh test` as the standard local and CI entrypoint. It runs repository static guards,
`fkst-framework --self-test`, package tests, flat-package conformance, and composed conformance.

Useful commands:

```sh
scripts/run.sh check
scripts/run.sh test
scripts/run.sh test github-proxy
scripts/run.sh test-composed
scripts/run.sh doctor
```

Static guards include the 1000-line hard limit for `.lua`, `.sh`, `.py`, and `.rs` source files
under `.fkst/packages/` and `scripts/`; package test naming rules; helper reachability checks; and
selected repository-shape checks. Engine tests remain the authority for real package behavior.

## Runtime Posture

GitHub writes are dry-run by default. `FKST_GITHUB_WRITE=1` is the only write posture switch; when it
is unset or any other value, outbound GitHub operations are not mutated. Real supervisor runs also
need host-stable runtime, durable, and rate-pool roots:

- `FKST_RUNTIME_ROOT`: scratch runtime state for local worktrees, locks, logs, cache, and once marks.
- `FKST_DURABLE_ROOT`: durable delivery store for reliable subscriptions.
- `FKST_RATE_POOL_ROOT`: shared host path for external-command rate pools.
- `FKST_RATE_POOL_GH`: host-owned GitHub rate-pool sizing for the named pool `gh`.

## Documentation

- [`docs/README.md`](docs/README.md): documentation index by audience.
- [`docs/user/new-package-repo-bootstrap.md`](docs/user/new-package-repo-bootstrap.md): package-repo
  scaffold checklist.
- [`docs/dev/devloop-design.md`](docs/dev/devloop-design.md): `github-devloop` state machine and
  design notes.
- [`docs/dev/consensus-converge-redesign.md`](docs/dev/consensus-converge-redesign.md): consensus
  convergence and reconcile design.
- [`docs/dev/harness-construction-methodology.md`](docs/dev/harness-construction-methodology.md):
  harness-first methodology.
- [`docs/dev/scaffold-install-upgrade-design.md`](docs/dev/scaffold-install-upgrade-design.md):
  scaffold install, upgrade, and package-reference update design.

The authoritative engine-package contract lives in `fkst-substrate` at
`docs/package-repo-contract.md`.

## Contributing and Security

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for contribution workflow, package conventions, language
policy, testing expectations, and PR rules. See [`SECURITY.md`](SECURITY.md) for supported scope and
vulnerability reporting.

⟦AI:FKST⟧
