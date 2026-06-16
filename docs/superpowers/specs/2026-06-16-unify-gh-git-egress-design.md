# Unify gh/git egress onto a structured-argv adapter + single shell-free engine primitive (Plan B)

Status: design (sshx-converged 2026-06-16; thinking triplet minimal/structural/delete → meta-judge `implement`)
Scope repos: `fkst-substrate` (engine primitive) + `fkst-packages` (adapters, migration). This repo writes only Lua; the engine change lands via a `fkst-substrate` PR.
Anchors: this design composes with `2026-06-15-ports-adapters-design.md` and `2026-06-15-capability-layering-design.md`; it does not contradict them.

## 1. Problem

Lua package code reaches `gh` and `git` in too many ways. Source-verified inventory:

| pattern | command | verdict |
|---|---|---|
| business builds raw string `exec_sync({cmd="gh issue view ..."})` | gh | redundant (migration debt) |
| business builds raw string `exec_sync({cmd="git -C ... status"})` | git | redundant (migration debt) |
| `ports.github.read_issue()` → `std.github` adapter | gh | the intended target (partial) |
| `std.git` adapter | git | intended target, near-empty stub |
| engine `setup_worktree`/`git_log_*` (direct `git` argv via `run_audited`) | git | engine host-authority — NOT a "package writing style" |
| `spawn_codex_sync` (Person; prompt on stdin) | codex | different semantic category |

Debt today: `migration/gh-git-adapter.allowlist` = 30 file sections / 76 raw `gh`|`git` heads; the G-ADAPTER ratchet (`scripts/check_repo_gh_git_adapter.py`) is shrink-only.

Two execution mechanisms coexist for git even inside the "adapter" path: `std.github`/`std.git` build shell strings lowered to `/bin/sh -c` (`sdk_basic.rs:210`), while engine `sdk_git` already runs `git` as direct argv (`sdk_git.rs:472`). The unification eliminates Lua-side shell-string construction and gives gh/git **one** path: a structured-argv adapter calling **one** shell-free engine egress primitive.

## 2. Decision

1. Add a **new, distinct, shell-free engine primitive `exec_argv`** (argv array → direct `Command::new(program).args(args)`), reusing the existing `external_command::run_audited` / `CommandSpec` path that `sdk_git` already proves.
2. `std.github` / `std.git` become real **anti-corruption adapters** that construct **argv arrays** (never shell strings) and call `exec_argv`. Stdout parsing stays. Business code never builds a raw `gh`/`git` head again.
3. **`exec_sync` stays** as the genuine-shell primitive — but is forbidden for gh/git. It is NOT generalized to also accept argv, and its `/bin/sh -c` form is NOT deleted.
4. `codex` (`spawn_codex`) and engine `sdk_git` host-authority helpers are **out of scope**; do not fold them in.

### Why two primitives, not one generalized `exec_sync`

The no-dual-mode rule (CLAUDE.md) decides this. `exec_sync`'s shell form **cannot be deleted**: genuine non-gh/git shell users exist and need shell features argv cannot express — e.g. `context_bundle.lua` uses `printf %s … > path`, `test -r … && test -r …`, `test -d/-e` (redirection, `&&`, shell builtins). If `exec_sync` were generalized to accept `cmd` **or** `argv`, that dual-input interface would be **permanent** (the `cmd` branch never goes away), which is exactly the forbidden dual-mode. Two single-responsibility primitives — `exec_sync` = run-a-shell-command, `exec_argv` = run-a-program-by-argv-without-a-shell — is the clean SOLID split (single responsibility, interface segregation). "One way" applies per capability: **one way for gh/git (argv via adapter)**, not "one process primitive for everything."

## 3. D1 — Engine primitive contract (`exec_argv`)

Lua-visible contract (new global, registered in `sdk_basic.rs` alongside `exec_sync`):

```
exec_argv({
  argv = { program, arg1, arg2, ... },  -- non-empty array of strings; argv[1] is the program
  cwd = "<dir>",                         -- optional
  env = { KEY = "VAL", ... },            -- optional
  timeout = <seconds>,                   -- optional
  read_coalesce = { ... },               -- optional, identical semantics to exec_sync
}) -> { stdout, stderr, exit_code, timed_out?, error_class? }   -- same result shape as exec_sync
```

- `argv` is required, non-empty, all strings. No `cmd` key (reject a table carrying `cmd`). No `rate_pool` key (see D2). No `stdin` for this cut (`CommandStdin::Null`; the engine supports `Bytes` but YAGNI here).
- Maps directly onto the existing struct (`external_command.rs:455`):
  `CommandSpec { program = argv[1], args = argv[2..], cwd, env, stdin = Null, timeout, process_group = timeout.is_some() }`.
- Reuses `execute_spec` → `run_audited` → `build_command` (`Command::new(program).args(args)`, `external_command.rs:589`). Audit lines, mock/cassette record-replay, and `read_coalesce` all already key off `CommandSpec.program/args`, so argv composes with no schema invention.
- Mock path: build `MockCommandInvocation` from argv exactly as `exec_sync` does from `/bin/sh -c`, so test mode + adapter-contract tests + `fkst.test` cassettes work for argv calls.

This is a small additive change to `sdk_basic.rs`; the execution helper is shared with `exec_sync`.

## 4. D2 — Rate / audit / sandbox under argv

- **Delete** the stale Lua `rate_pool` field; do not honor it. `std.github/exec.lua:38` sends `rate_pool={name='gh'}`, but `parse_exec_options` never reads it (`sdk_basic.rs:100`) — it is dead metadata. Remove it from adapters and tests.
- Rate selection: `RatePoolRegistry.acquire_for_program(basename(argv[1]))` (`rate_pool.rs:131`), exactly as `sdk_git` does for `git` (`sdk_git.rs:468`). Rate pools are host posture facts (`package-repo-contract.md:208-210`), not package API. (`exec_sync` keeps `acquire_for_command_text` — command-text parsing is only meaningful for the shell form.)
- Audit, error_class, timeout/process-group, read_coalesce: unchanged — they live in `run_audited`/`CommandSpec` and apply identically.
- Sandbox/posture: argv makes the executed program/args explicit (injection-safe; no Lua-side shell quoting). Boundary-resource registry gains an `argv.process` entry distinct from the existing `shell.process` (`boundary_resource.rs:48`).

## 5. D3 — Adapter surface

- `std.ports.production_handles` injects `exec_argv` (not `exec_sync`) into `std.github.new` / `std.git.new`.
- `std/github/exec.lua` and `std/git/exec.lua` take **argv + opts** (private `_exec_argv`), prepend the program internally (`{"gh", ...}` / `{"git", ...}`), and drop `rate_pool`.
- Command builders become private **argv constructors**: `{"gh","api",path}`, `{"git","show-ref","--verify",ref}`. Shell quoting disappears from gh/git construction; keep only semantically-needed validators (URL/ref/number) — delete `std/github/shell.lua`'s `shell_single_quote` once its callers are gone; keep `url_encode` if still used.
- Delete one-line probe stubs (`std/github/probe.lua`, `std/git/probe.lua`) and fill `std.git` per migrated family (it is a near-empty facade today, `std/git.lua:5`).
- Public adapter surface stays **semantic** and grows per slice (reads first):
  - github reads: `read_issue`, `read_pr`, `read_pr_diff`, `list_open_issues`, `find_pr_for_head`, `read_check_runs`, `read_blocked_by`.
  - github writes: `create_comment`, `edit_comment`, `reconcile_labels`, `assign_issue`, `create_pr`, `merge_pr`, `close_issue`.
  - git: `show_ref`, `is_ancestor`, `remote_branch_head`, `list_worktrees`, `ensure_worktree`, `fetch`, `commit`, `push`, `merge`.
- Business code may pass source_refs/request tables and consume neutral tables; it may not build `gh`/`git` argv, call `exec_sync`/`exec_argv` for gh/git directly, or assert command spelling outside adapter-contract tests.
- Fakes (`std.github_fake`/`std.git_fake`) keep parity (in-memory models; no fake binaries).

## 6. D4 — Strangler staging (foundation by operator; slices by pipeline)

**Foundation (operator-built; not delegated):**
- **S1 (`fkst-substrate` PR):** add `exec_argv` to `sdk_basic.rs`; Rust tests (argv mapping, cwd/env/timeout/process-group, read_coalesce fingerprint, mock record-replay, `acquire_for_program`); add `argv.process` boundary resource; update SDK surface in `package-repo-contract.md`. Additive — no behavior change to existing callers. **Must land before P1.**
- **P1 (`fkst-packages` foundation PR):** `std.ports` injects `exec_argv`; `std.github`/`std.git` exec wrappers → argv; delete stale `rate_pool`; migrate `std.github.read_issue` builders string→argv as the **reference migration**; adapter-contract tests assert `opts.argv`; evolve `scripts/check_repo_gh_git_adapter.py` to ALSO flag argv-form heads (`exec_argv({argv={'gh'|'git',...}})`) outside adapter paths so argv cannot become a new bypass; note `exec_argv` as the gh/git egress in CLAUDE.md. **Establishes the copyable pattern for the pipeline.**

**Incremental slices (decomposed into independently-grabbable GitHub issues for github-devloop):**
- One issue per coherent command family / file group; each issue: migrate that group's allowlisted raw heads to `std.github`/`std.git` argv methods, delete the old string helper in the same PR, shrink `migration/gh-git-adapter.allowlist` monotonically (ratchet enforces shrink-only). Reads before writes. Each issue references P1 as the pattern and S1 as the primitive.
- Family groups (initial): `github-proxy/core.lua` gh (7); `github-devloop/core/commands.lua` gh; `…/commands.lua` git; `…/branches.lua` git; `…/claims.lua` gh; `…/github_graphql.lua` gh api; `…/error_facts.lua`/`release_notes.lua`; `std/github_debug_stamp.lua`; `std/saga_conformance.lua`.

**Shell-compound blockers (classified separately):** ~15 allowlisted helper strings combine genuine shell features with git — env expansion/presence (`core/config.lua:33,40`), command substitution/`&&` (`core/branches.lua:82`), `mkdir &&` worktree helpers (`branches.lua:209`, `commands.lua:769`), redirection/`rm`/`if`/`test` (`commands.lua:724,745,759,828`). These are NOT clean argv migrations. Each such issue must **decompose** the compound into (argv git op via adapter) + (the fs/env step), where the fs/env step either stays on `exec_sync` (genuinely shell) or uses a narrow fs/env helper. No shell pipelines or globs were found as blockers; `|` inside a `git grep` regex is an argv argument, not a pipe.

**Final:**
- Packages: allowlist gh/git heads = 0; delete the gh/git string-builder helpers; tighten G-ADAPTER so new raw gh/git heads (string or argv) outside adapters are impossible.
- Substrate (optional doc-only): document `exec_argv` as the gh/git egress in `package-repo-contract.md`. **`exec_sync` and its `/bin/sh -c` form are retained** for genuine shell users — not deleted (decision §2).

## 7. D5 — Repo split & dependency order

- **fkst-substrate** owns only the generic process capability: `exec_argv` contract/validation, `CommandSpec` mapping, rate/audit/read_coalesce/error_class/cassette behavior, boundary-resource entry, Rust tests. It must learn nothing about `gh`, GitHub source_ref grammar, markers/CAS, branch topology, or `std.github`/`std.git`. Existing `sdk_git` host-authority helpers stay engine concerns and may share the internal argv execution helper without becoming the package git adapter.
- **fkst-packages** owns Tier R adapters, fakes, `std.ports`, G-ADAPTER, adapter-contract tests, the per-family migration, and package docs.
- Dependency order: **S1 → P1 → per-family slices (allowlist↓) → final deletion**.

## 8. Non-goals

- No merging of `codex` or engine `sdk_git` into this egress (different categories).
- No deletion of `exec_sync`/`/bin/sh -c` (genuine shell users remain).
- No dual-mode/compat/opt-in: each slice deletes the old string form it replaces; `exec_argv` and `exec_sync` are two distinct capabilities, not two ways to do the same thing.
- No broad command DSL beyond the allowlisted gh/git families (YAGNI).

## 9. Done definition

- `exec_argv` exists and is the only egress used by `std.github`/`std.git`.
- Zero gh/git reached via `exec_sync` strings; `migration/gh-git-adapter.allowlist` gh/git heads = 0.
- G-ADAPTER rejects new raw gh/git heads in string and argv forms outside adapter paths.
- Stale `rate_pool` field removed everywhere.
- `exec_sync` remains solely for genuine shell users.

## 10. Harness anchoring

- **Single-egress capability boundary** + **argv-not-shell (injection-safe) execution**: the unsafe surface (shell string building) is removed for gh/git; the program/args are explicit.
- **Ports & adapters / anti-corruption layer**: `std.github`/`std.git` own all command construction and parsing; the engine stays generic.
- **Strangler-fig migration**: additive primitive first, monotonic ratchet shrink, old form deleted per slice, zero permanent second way.

⟦AI:FKST⟧
