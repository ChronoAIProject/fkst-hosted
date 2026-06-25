# Thin host run.sh delegation to fkst-packages

Status: IMPLEMENTED IN THIS BRANCH
Date: 2026-06-24
Scope: fkst-packages `scripts/run.sh` host entry and docs. Host-side thin
bootstrappers, starting with fkst-website, are follow-up changes in their own
repos.

Superseded note (2026-06-25): ADR 0002 is the canonical host layout source of truth. The current platform
pin is `fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock`; older `.fkst-packages-ref` bootstrap
references in this dated spec are historical. The canonical host composition roots path is
`.fkst/compose/package-roots`.

## Problem

Host repos currently copy launch and conformance plumbing that belongs to the
shared platform. Track H/P already removed the fkst-website `check_repo.py`
copy by publishing `scripts/check_repo.py --project-root <HOST>` from the
pinned fkst-packages checkout. The same duplication remains for `run.sh`.

fkst-website's host runner re-implements `resolve_bin`,
`ensure_fresh_bin`, `build_engine_package_root_args`, and
`run_shared_source_ratchets`. That is the run.sh twin of the old copied
`check_repo.py`: it drifts from fkst-packages, makes each host responsible for
engine/package-root wiring, and turns a shared launch contract into per-host
shell.

The source convention now says host repos compose the platform through
`fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock`, host-owned packages under
`.fkst/local-packages/`, host composition roots under `.fkst/compose/package-roots`,
host conformance allowlists under `.fkst/conformance/allowlists/`, and the one-host supervise contract in
`scripts/host_run.sh`. The user doc
[`docs/user/control-planes-and-host-repo-composition.md`](../../user/control-planes-and-host-repo-composition.md)
defines the three-plane split: PRODUCT, HOST-RUN contract, and DOGFOOD-OPERATOR.
This spec closes the remaining run.sh duplication.

## Converged Design

A host repo's `scripts/run.sh` is a bootstrapper, not a runner:

```sh
#!/usr/bin/env bash
set -euo pipefail

HOST_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PIN="$(resolve_fkst_packages_platform_rev "$HOST_ROOT/fkst.workspace.toml" "$HOST_ROOT/fkst.lock")"
SHARED="$(hydrate_or_reuse_pinned_fkst_packages "$HOST_ROOT" "$PIN")"
exec "$SHARED/scripts/run.sh" host --host-root "$HOST_ROOT" -- "$@"
```

The host bootstrapper owns only the irreducible host-local glue:

- discover its own repository root;
- read the `fkst-packages-platform` full-SHA pin from `fkst.workspace.toml` and `fkst.lock`;
- hydrate or reuse that pinned fkst-packages checkout;
- `exec <shared>/scripts/run.sh host --host-root <HOST> -- <command>`.

Everything else is delegated. The shared fkst-packages runner owns BIN
resolution, freshness, shared source ratchets, engine conformance/test
package-root wiring, and the existing `host_run.sh` supervise lifecycle.

The shared entrypoint is:

```sh
scripts/run.sh host \
  --host-root <HOST> \
  [--platform-root <PKGSRC default=self>] \
  [--local-packages <dir default=<HOST>/.fkst/local-packages>] \
  -- <check|test|supervise [args]>
```

`--host-root` is the repo being checked, tested, or supervised.
`--platform-root` is the fkst-packages checkout supplying platform packages and
shared scripts. When omitted it is this fkst-packages checkout, which is the
common case after a host bootstrapper has hydrated the pinned source and execed
into it. `--local-packages` points at the host's package directory.

## Command Behavior

`run.sh host -- ... check` runs the same two conformance tiers a host needs:

- shared source ratchets from fkst-packages:
  `scripts/check_repo.py --project-root <HOST>`, with
  `<HOST>/.fkst/conformance/allowlists` passed when present;
- engine conformance using package roots from
  `<HOST>/.fkst/compose/package-roots` when present, otherwise discovered
  host-local package roots under `<HOST>/packages/*` and
  `<HOST>/.fkst/local-packages/*`.

`run.sh host -- ... test` runs the host's package tests with the shared BIN
resolution and package-root wiring. Host tests use host-local packages as the
test subjects and include configured platform roots when present.

`run.sh host -- ... supervise [args]` delegates to the existing
`scripts/host_run.sh` contract. The host entry derives package names from the
same package-root config and calls:

```sh
host_run_supervise_contract \
  --project-root <HOST> \
  --platform-root <PKGSRC> \
  --local-packages <dir> \
  --platform-packages "<names>" \
  [--host-packages "<names>"] \
  [supervise args...]
```

The existing host-run contract remains the only owner of durable-root
validation, runtime scratch, pidfile restart, package-root assembly for
supervise, and the final `fkst-framework supervise` invocation.

## Configuration

The next host repo follows these conventions:

- `fkst.workspace.toml` `[[external_sources]]` plus `fkst.lock` pins the fkst-packages checkout by full commit
  SHA under the `fkst-packages-platform` source identity;
- host packages live under `.fkst/local-packages/<pkg>/`;
- host conformance allowlists live under `.fkst/conformance/allowlists/`;
- `.fkst/compose/package-roots` lists engine roots, one per line:
  relative paths resolve from `<HOST>`, absolute paths stay absolute, and
  `fkst-packages:<path>` resolves from `<PKGSRC>`.

Engine conformance uses `<HOST>` as `--project-root`; configured package roots
only supply `--package-root` arguments and package-name derivation. When the
file is absent, `run.sh host` discovers `<HOST>/packages/*` and
`<HOST>/.fkst/local-packages/*` so a minimal host fixture can still run.

## Non-Goals

- Do not change fkst-packages' own non-host `run.sh test`, `check`,
  `test-composed`, `run`, or package-local `supervise` behavior.
- Do not implement the fkst-website thin bootstrapper here. That is the next
  host-side follow-up after this shared entry lands.
- Do not move or refactor `libraries/`; that work is on another machine.
- Do not duplicate `host_run.sh` or copy per-host `run.sh` logic into another
  host. The whole point is one shared implementation.

## Behavior Preservation

This is an additive host-invocable entry. Existing fkst-packages commands keep
their current dispatch and tests. The host entry reuses the existing
`check_repo.py --project-root` seam, shared BIN resolution/freshness functions,
and `host_run.sh` supervise contract, so the per-host copies collapse into one
source of truth without changing the package library's own workflow.

⟦AI:FKST⟧
