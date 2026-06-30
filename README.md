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
library B: it defines Lua package development source under `packages/`, with departments, raisers,
package-local shared code, and tests. The engine never loads from repo-root `packages/` directly:
`scripts/run.sh` regenerates `.fkst/local-packages -> ../packages` for this repository's own
packages, and also loads any external runtime packages present under `.fkst/packages/`.

Packages communicate through event queues. Flat packages are self-contained and use bare queue
names internally. Composed packages are first-class packages that adapt or combine sibling package
queues and declare those siblings in `[event_deps]` so composed conformance can test the union
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

For host-local, no-repo-pollution runs, keep machine facts in an XDG-style profile such as
`${XDG_CONFIG_HOME:-$HOME/.config}/fkst/host.env`; see
[`docs/user/global-host-profiles.md`](docs/user/global-host-profiles.md) and
[`docs/user/host-profile.env.example`](docs/user/host-profile.env.example). The profile feeds the
existing explicit `scripts/run.sh host --host-root ... --platform-root ... -- ...` contract; it does
not introduce a named profile resolver.

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

Shared repo-root code is split by positive library boundary. `libraries/contract/` contains only
publishable value/protocol primitives (`contract.source_ref`, `contract.payload`,
`contract.error_facts`, and scalar `contract.strings`). Runtime orchestration helpers live in
`libraries/workflow/`, test and conformance tooling in `libraries/testkit/`, forge-facing GitHub/Git
adapters in `libraries/forge/`, and the github-devloop product kernel in `libraries/devloop/`.
Packages declare direct `lib_deps` such as `["contract", "workflow", "testkit"]`,
`["contract", "workflow", "testkit", "forge"]`, or
`["contract", "workflow", "testkit", "forge", "devloop"]` in their `fkst.toml`; the engine's scoped
resolver grants access to modules from those manifest dependencies rather than from per-package
filesystem symlinks. New and migrated `gh`/`git` access goes through `forge.github`/`forge.git`
(production wiring via `forge.ports`); remaining raw call sites are migration debt in
`migration/gh-git-adapter.allowlist` that the G-ADAPTER ratchet shrinks. Tests use
`testkit.testing` with `forge.github_fake` / `forge.git_fake`.

Runtime package roots live only under `.fkst/`. In this library repository, `.fkst/local-packages`
is a regenerated relative symlink to `packages/` and represents this repo's own packages.
`.fkst/packages/` is reserved for external referenced packages assembled by the operator or
dogfood host; it is empty for the package library itself. Both paths are runtime-only and
gitignored. The committed `.fkst/` contents are only `.fkst/substrate-ref` and `.fkst/env.example`;
generated runtime, durable, and board-cache state goes under `.fkst/run/`.

## Package Catalog

Flat packages:

- `github-proxy`: bridges GitHub issue and PR facts into fkst events, and handles dry-run-by-default
  outbound GitHub comments, labels, PR creation, and related requests.
- `github-external-pr-intake`: detects third-party PRs and materializes exactly one normal
  devloop-ready bridge issue per PR, leaving implementation, review, and merge authority in
  `github-devloop`.
- `consensus`: source-agnostic multi-angle `codex` consensus over abstract `proposal` events,
  producing `consensus_reached` or bounded `consensus_converge` events.

Composed packages:

- `autochrono`: maps its own `issue` protocol into `consensus.proposal` and maps reached consensus
  back into its own `reply` protocol.
- `github-autochrono`: composes `github-proxy` and `autochrono` as a GitHub issue-to-reply adapter.
- `github-devloop`: composes `github-proxy` and `consensus` into the autonomous GitHub issue to PR
  loop, using trusted GitHub marker facts, version-CAS state transitions, head-bound PR review, and
  deterministic merge gates.
- `frontend-devloop`: declares the UI-application host profile for composing the existing GitHub
  devloop package family with frontend host scripts and source-ref-only UI artifact handoff.

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
under `packages/` and `scripts/`; package test naming rules; helper reachability checks; and
selected repository-shape checks. Engine tests remain the authority for real package behavior.

Repository-shape guards include G9, which forbids peer cross-package `require` and keeps sharing on
workspace libraries; G10, which shrinks the saga-handler allowlist toward `workflow.saga.department`;
G-LIB-DEP, which locks the library dependency DAG and contract publishable surface; and G-ADAPTER,
which shrinks the `gh`/`git` command-construction allowlist toward `forge.github` / `forge.git`.
Ports-using `gh`/`git` business tests use injected fakes through `testkit.testing`;
existing/adapter-contract tests may still use `fkst.test.mock_command` while the migration proceeds. Other external CLIs such
as `codex` still use the engine command mock; no fake `gh`/`git`/`codex` binaries are generated,
and unmocked external commands fail closed.

## Runtime Posture

GitHub writes are dry-run by default. `FKST_GITHUB_WRITE=1` is the only write posture switch; when it
is unset or any other value, outbound GitHub operations are not mutated. Real supervisor runs also
need host-stable runtime, durable, and rate-pool roots:

- `FKST_RUNTIME_ROOT`: scratch runtime state for local worktrees, locks, logs, cache, and once marks;
  defaults to `.fkst/run/runtime`.
- `FKST_DURABLE_ROOT`: durable delivery store for reliable subscriptions; defaults to
  `.fkst/run/durable`.
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
- [`docs/superpowers/specs/2026-06-15-ports-adapters-design.md`](docs/superpowers/specs/2026-06-15-ports-adapters-design.md):
  historical ports/adapters rationale, now implemented through `forge.github`, `forge.git`, port
  wiring, and fake-port tests.

The authoritative engine-package contract lives in `fkst-substrate` at
`docs/package-repo-contract.md`.

For host repo layout normalization, see [`docs/adr/0002-host-fkst-layout.md`](docs/adr/0002-host-fkst-layout.md).

## Contributing and Security

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for contribution workflow, package conventions, language
policy, testing expectations, and PR rules. See [`SECURITY.md`](SECURITY.md) for supported scope and
vulnerability reporting.

⟦AI:FKST⟧
