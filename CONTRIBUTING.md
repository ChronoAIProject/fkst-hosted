# Contributing to fkst-packages

`fkst-packages` is the behavior-layer package library for the separate `fkst-substrate` engine.
Contributions should change Lua packages, tests, scripts, or documentation in this repository only.
Engine Rust changes belong in `fkst-substrate`.

中文补注：本仓只维护 package 行为层；引擎能力和 Rust 实现属于 `fkst-substrate`。

## Development Setup

1. Build or obtain a local `fkst-framework` binary from `fkst-substrate`.
2. Copy `.fkst/env.example` to `.fkst/env`.
3. Set `BIN=/path/to/fkst-substrate/target/debug/fkst-framework`.
4. Run `scripts/run.sh test` from the repository root before submitting changes.

Useful commands:

```sh
scripts/run.sh check
scripts/run.sh test
scripts/run.sh test <package>
scripts/run.sh test-composed
scripts/run.sh doctor
scripts/run.sh run <package> <department> '{"payload":{}}'
scripts/run.sh supervise <package>
```

`run` and `supervise` default to `.fkst/run/runtime` and `.fkst/run/durable` when the corresponding host
facts are unset. `run` never sets `FKST_GITHUB_WRITE`. Real GitHub writes happen only when
`FKST_GITHUB_WRITE=1`.

## Branch and PR Workflow

- Use `dev` as the integration branch.
- Do not commit directly to `dev`; open a PR into `dev`.
- Use branch names of the form `<type>/<kebab-topic>`, where `<type>` is one of `feat`, `fix`,
  `docs`, `chore`, `refactor`, or `test`.
- Keep each commit to one coherent logical change.
- Use English-primary commit messages, PR titles, and PR bodies. Chinese may be added as auxiliary
  context, but the English text is authoritative.
- PR bodies should include motivation, changes, and test evidence with commands and results.
- Merge with squash after CI is green.
- AI-generated PR bodies or change notes should end with `⟦AI:FKST⟧`.

## Package Structure

Packages live under `packages/<pkg>/` as committed development source. The engine loads runtime
package roots only from `.fkst/`: `.fkst/local-packages` is regenerated as a relative symlink to
`packages/` for this repository's own packages, and `.fkst/packages/` is reserved for external
referenced packages. Both runtime load directories are gitignored.

```text
packages/<pkg>/
  core.lua
  departments/<dept>/main.lua
  raisers/<raiser>.lua
  tests/*_test.lua
```

Package-local shared code belongs at package root, usually `core.lua`, and is required as
`require("core")`. Departments may split local responsibilities beside `main.lua` and require them
as `require("departments.<dept>.<module>")`. Do not cross-require sibling packages. Cross-package
composition must use event queues and, for composed packages, `[event_deps]`.

Flat packages must be self-contained, use their own bare queue names internally, avoid external
package namespace references, and pass single-root conformance. Composed packages may reference
sibling package queues such as `<pkg>.<queue>` and must declare the loaded siblings in
`[event_deps]`.

## Source and Documentation Language

Source files such as `.lua`, `.sh`, `.py`, and `.rs` use English for comments, docstrings, log
messages, error text, template strings, and identifiers. Localized outward text values may use
UTF-8 target-language strings only when they are explicit localization resources and remain readable
and grep-friendly.

Outward artifacts such as documentation, issues, PRs, comments, commit messages, and change notes
are English-primary. Chinese may be included as auxiliary context. Code identifiers, paths, crate
names, command names, protocol names, test assertions, and quoted source text stay verbatim.

## Design Rules

- Keep package behavior deterministic where possible and fail closed on unknown input.
- Treat GitHub and other external systems as eventually consistent fact sources.
- Use stable `source_ref`, `schema`, `dedup_key`, version, and short control fields in durable
  delivery payloads.
- Do not serialize large issue bodies, PR diffs, comments, code, or files into reliable delivery
  payloads. Consumers should fetch full content from source when needed.
- Do not store business state in the source tree or runtime scratch paths to survive crashes.
  Re-derive state from git, external sources, or explicit host facts.
- Keep external side effects dry-run by default. `FKST_GITHUB_WRITE=1` is the only GitHub write
  posture switch.
- Do not add deprecated shims, compatibility layers, `.old` files, `_legacy` paths, or dual-mode
  behavior for old contracts. Change the current contract completely and remove obsolete code.

## File Size and Test Discipline

Source files under `packages/` and `scripts/` have a hard 1000-line limit for `.lua`, `.sh`,
`.py`, and `.rs` files. Split by stable responsibility before a file reaches the limit. Do not use
empty forwarding files or compatibility shells to satisfy the limit.

Tests belong in `packages/<pkg>/tests/` and should be named `*_test.lua`; shared test helpers should
be named `*_helpers.lua`. External commands such as `gh` and `codex exec` must be mocked through
`fkst.test.mock_command` and inspected through `fkst.test.command_calls`. Do not create fake CLI
binaries in tests.

For behavior changes, add or update focused tests and run the narrow package test first when useful,
then run the full suite:

```sh
scripts/run.sh test
```

## Security and Side Effects

Do not put secrets in tests, fixtures, docs, examples, or issue templates. Do not modify GitHub
state, labels, comments, branches, PRs, or repository settings as part of local development unless a
task explicitly requires it and the required write posture is configured.

See [`SECURITY.md`](SECURITY.md) for vulnerability reporting.

⟦AI:FKST⟧
