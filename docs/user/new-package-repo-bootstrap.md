# New `fkst` Package-Repo Bootstrap Checklist

This checklist turns a new repository into a reproducible `fkst` package-repo scaffold. The
authoritative contract is `fkst-substrate/docs/package-repo-contract.md`; this document lists only
the minimum implementation steps.

1. Create the runtime interface directory `.fkst/` with tracked `.fkst/env.example` and `.fkst/substrate-ref`; ignore `.fkst/packages`, `.fkst/local-packages`, `.fkst/run/`, and `.fkst/env`. Everything else under `.fkst/` is runtime-generated.
2. For a Lua-primary package repo, keep committed development source in `packages/<pkg>/...`. `scripts/run.sh` regenerates `.fkst/local-packages -> ../packages` as the own-package runtime view. `.fkst/packages/<pkg>/...` is only for external referenced packages assembled by an operator or dogfood host; it is empty for a package library itself.
3. Create the package directories: `packages/<pkg>/core.lua`,
   `packages/<pkg>/departments/<dept>/main.lua`, optional
   `packages/<pkg>/raisers/<raiser>.lua`, and `packages/<pkg>/tests/*_test.lua`.
4. For a composed package, set `kind = "package.composed"` in `packages/<pkg>/fkst.toml` and declare
   sibling packages that must be loaded together for composed conformance under
   `[event_deps] packages = [...]`. For a flat package, use `kind = "package"` and do not declare
   `[event_deps]`.
5. Copy `scripts/run.sh`, `scripts/check_repo.py`, `.fkst/env.example`, and
   `.github/workflows/ci.yml` from the scaffold.
6. Write the source pin in `.fkst/substrate-ref`. The default may be `dev`; downstream reproducible
   repositories should use an `fkst-substrate` tag or SHA. This is a Git source pin, not semver and
   not binary distribution.
7. For a local `fkst-framework` binary cache path keyed by an explicit source pin, use the pure helper `substrate_bin_cache_path(cache_root, owner, repo, ref)` from `scripts/bin_cache.py`. Path contract: `<cache_root>/fkst-substrate-bin/v1/<owner>/<repo>/<ref>/target/debug/fkst-framework`, with `owner`, `repo`, and `ref` encoded as independent UTF-8 byte percent-encoded path components. `/`, space, `.`, `..`, `%`, and other special characters stay data, not separators or dot-segments, so distinct `(owner, repo, ref)` triples cannot collide through separator replacement. `scripts/run.sh` uses that path only after all ordinary `BIN` sources miss, then serializes clone/fetch/checkout/build with a per-cache lock. Invalid explicit `BIN` or `.fkst/env BIN=` fails closed instead of falling back. `FKST_NO_AUTOBUILD=1` disables the network/build fallback.
8. Copy `.fkst/env.example` to `.fkst/env` and set
   `BIN=/path/to/fkst-substrate/target/debug/fkst-framework`. CI checks out engine source from
   `.fkst/substrate-ref` and builds `fkst-framework` itself.
9. Run `scripts/run.sh test` from the repository root. With no package argument, it first runs
   `fkst-framework --self-test`, then enumerates own packages to test from `packages/*`. The engine
   actually loads `.fkst/local-packages/<pkg>`, and composition / run / supervise paths also include
   external roots present under `.fkst/packages/*`. Flat packages run single-package
   `conformance + test`; composed packages skip single-package conformance but still run tests. The
   command then recursively runs composed conformance from every `fkst.toml` `[event_deps]` entry.
   When unset, `scripts/run.sh` uses `.fkst/run/runtime` and `.fkst/run/durable`; the board cache is
   written to `.fkst/run/board-cache.json`.
10. Use `scripts/run.sh check` when you only need static repository guards, and
    `scripts/run.sh test-composed` when you only need composed conformance.

When adding a package, keep payloads small and stable: reliable delivery should carry only
`source_ref`, `schema`, `dedup_key`, and control fields. Consumers fetch large issue bodies, PR
diffs, comments, code, or file contents from source through `source_ref`.

⟦AI:FKST⟧
