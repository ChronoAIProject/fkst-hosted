# Design: Ports & Adapters — a `gh`/`git` anti-corruption layer in `std`

Status: proposal v5 · Date: 2026-06-15 · Repo: fkst-packages
v5 changelog: makes fake-bound tests determinate with package-side `raise` capture, preserves production `_G.pipeline` binding through `std.department`, and defines the saga oracle's combined S1/S2 effect set.
Builds on (read these first):
- `2026-06-14-std-shared-library-design.md` — the `std/` shelf, the Tier S/R split, the symlink vendoring, and the doctrine that earmarks **`gh`-shaped helpers as Tier R**. This spec puts the largest Tier R inhabitant on that shelf.
- `2026-06-14-saga-harness-design.md` — the `std.department{done,act}` department shape and its ①②③ idempotency oracle. §6 of this spec composes that Tier S oracle with Tier R GitHub/git fakes without making the oracle depend on GitHub types.

---

## 1. Problem (实证)

The system programs **directly against concrete commands**. Business logic and the
`gh`/`git` command bridge are tangled in the same files: a department that decides
"should this PR merge?" also knows the exact spelling of `gh pr merge --merge
--match-head-commit`, the JSON shape of `gh pr view`, and how to `shell_single_quote`
a branch name. CLAUDE.md already names the cure as doctrine and the code already
violates it:

> CLAUDE.md(边界模式固定): "外部系统接入优先用 Adapter，把 `gh`、`codex exec`、文件和网络
> 形态转成包内稳定结构；副作用边界集中，业务函数保持可单测。"

**Census of the violation** (scan of `packages/`, 2026-06-15):

| Surface | Count | Where it lives |
|---|---|---|
| `gh`/`git` command builders | **148 functions** | `github-devloop/core/commands.lua` (87, 777 lines) · `core/branches.lua` (26, 483) · `github-proxy/core.lua` (21, 938) · + scattered across 12 more files |
| exec call sites (`exec_sync`/`gh_exec`) | **~240** | 26 departments + core; two **separate** `gh_exec` wrappers (`github-devloop/core/base.lua:918`, `github-proxy/core/gh_rate.lua:38`) |
| parsers (`parse_*` + inline `json.decode`) | **~145** | `github-devloop/core/parsers.lua` (878 lines) · `github-proxy/core.lua` · 10+ more |
| structured domain types | **1** (`source_ref`) | everything else is raw decoded tables; no `Issue`/`PR`/`Comment`/`Worktree` type |

**Three concrete symptoms of the tangle:**

1. **`github-proxy` is a *leaky, partial* adapter.** It is a write-side saga endpoint
   (schema `github-proxy.v1`) but its command builders are embedded in its own
   `core/*`, and **reads bypass it entirely**: `github-devloop` builds its own `gh`
   read commands directly and does **not** `require("github-proxy")` at all. So the
   same operation is implemented twice — e.g. `gh_issue_assign_cmd` exists in *both*
   `github-proxy/core/claims.lua` and `github-devloop/core/claims.lua`; entity views
   are duplicated in `github-proxy/core/entity_view.lua` and
   `github-devloop/core/github_proxy_entity_view.lua`.

2. **Business logic carries GitHub's vocabulary.** `github-proxy/core.lua` holds
   `current_devloop_state` / `compare_state_marker` (the devloop **state-machine**
   version-CAS) right next to `gh_issue_list_cmd` (a raw `gh api` string). The
   "decide" and the "spell the command" responsibilities are interleaved, so neither
   can be tested or changed without the other.

3. **Tests are coupled to command *strings*, not behavior.** ~100 tests mock by exact
   literal command string (`fkst.test.mock_command("git fetch 'origin' '"..branch.."'", …)`).
   Any change to flag order, quoting, or builder spelling reddens unrelated business
   tests. This is the standing brittleness tracked as **#633** ("harness over-couples
   to exact gh-command counts"); #678's worktree fix already broke four tests this way.

The user's north star for the whole repo: **"脚本可以用最简单的代码（没有重复代码）来表达
业务逻辑，框架把公共部分做好做稳定。"** Business should read like *domain decisions*; the
"common, stable part" (the `gh`/`git` mechanics) belongs in one solid, shared place.

## 2. Goal / Non-goals

**Goal.** Introduce a **Ports & Adapters (Hexagonal / anti-corruption) boundary** for
the external `gh`/`git` world. The boundary has three explicit surfaces:

- **S1 neutral adapters** in Tier R `std.github` / `std.git`: build commands, execute
  them, parse stdout, and return neutral normalized Lua tables.
- **S2 package-owned write intents**: durable request schemas such as
  `github-proxy.v1`, carrying `source_ref`, small control fields, bounded short
  control text snapshots, and package `body_source` handles where a deterministic
  renderer is available.
- **S3 package-owned marker/CAS guards**: trusted-bot filtering, marker grammar,
  current state, version-CAS, and expected proposal/state/version checks under the
  existing per-entity locks.

Business departments call S1 for synchronous reads, raise S2 for durable writes, and
run S3 before any guarded write execution. The core intent is unchanged: packages stop
spelling `gh`/`git` commands in business logic, and tests assert behavior at the port or
intent boundary instead of literal shell strings.

**Non-goals.**
- **Not** `codex`/fs/network now. They may use the same pattern later, but this plan
  emits no tasks for them (§7).
- **Not** a change to the **delivery topology**. Which writes are durable-event-mediated
  vs synchronous, and the `source_ref` / content-not-in-payload constitution, are
  **preserved exactly** (§5.4). This is a structural relocation of command-construction
  and parsing behind an adapter, plus a sharper write-intent/guard split.
- **Not** an engine change. Waves 0 and all vertical slices use the existing `exec_sync`
  primitive and need **zero** fkst-substrate change.
- **Not** moving the devloop **state machine** out of `github-proxy`. That
  `current_devloop_state` lives in `github-proxy` is a real layering smell, but
  relocating state-machine *ownership* is a separate concern; this spec extracts the
  `gh`/`git` **command+parse mechanics** and notes the rest as follow-up (§10 R6).

## 3. Position in the `std` stack (how the three specs compose)

```
  std-shared-library  ──  the shelf + the doctrine (peer-require forbidden, std allowed)
        │                 (Tier S / Tier R; symlink vendoring; verified)
        ├── saga-harness  ── department CONTROL-FLOW shape:  std.department{done, act}
        │                    "来了就做，做过就不做";  ①②③ oracle
        └── ports-adapters ── EXTERNAL-WORLD boundary: neutral gh/git adapters,
   (THIS spec)               package write intents, package marker/CAS guards
```

The two department-level specs are **orthogonal axes of the same rewrite**:

- `saga-harness` decides *how a department's control flow is shaped*: `done(event)`
  (idempotency predicate, read-only, re-derives truth) and `act(event)` (effects).
- `ports-adapters` decides *what vocabulary `done`/`act` speak to the outside world*:
  neutral adapter reads, durable write intents, and package-owned guards, not raw
  `gh`/`git` strings.

They reinforce each other: `done(event)` usually re-derives truth through an S1 read
operation (`read_issue`/`read_pr`), and `act(event)` either raises an S2 write intent or
executes a local `std.git` operation. A department's migration should adopt **both** axes
in one vertical slice (`done/act` + port/intent vocabulary) rather than touching
`main.lua` twice (§9).

This spec lightly extends the saga-harness composition pattern without changing
`done(event)` / `act(event)` arity. A migrated department module should expose a
package-local constructor such as `make_department(ports)`. That constructor closes over
the injected port handles, builds `std.department{done=..., act=...}`, and returns the
engine-facing module shape. The default module export binds production ports from
`exec_sync`; tests call the same constructor with fakes. Ports enter through this
factory/context boundary, not through global module state.

## 4. The Three Surfaces

The old dividing question "GitHub vocabulary vs fkst vocabulary" is still useful, but it
is not enough. Guarded writes need a third home. Every line in the 148 builders / 145
parsers belongs to one of these surfaces:

| Surface | Owner / home | Knows | Speaks | Never |
|---|---|---|---|---|
| **S1 neutral adapter** | Tier R `std.github` / `std.git` | `gh` CLI flags, GraphQL field names, `git` plumbing, stdout/JSON shapes, shell quoting, rate-limit strings | adapter handle operations such as `github.read_issue`, `github.create_pr`, `git.ensure_worktree`; returns neutral normalized Lua tables | marker grammar, devloop states, trusted-bot policy, proposal IDs, version-CAS, consensus |
| **S2 write-intent layer** | package queues + payload schemas, primarily `github-proxy.v1` | durable delivery schema, `source_ref`, `dedup_key`, expected proposal/state/version control fields, bounded short `body` / `title` snapshots, package `body_source` / `title_source` handles | intent names such as `github_issue_comment_request`, `github_issue_create_request`, `github_pr_open_request`, `github_issue_label_request` | command strings, stdout parsers, template rendering by `std.github` |
| **S3 marker/CAS guard modules** | package-owned modules under package root / departments | trusted-bot filter, `current_devloop_state`, marker schemas, version-CAS order, expected proposal/state/version/head checks, dependency gate policy | guarded-write decisions made under existing `with_lock` keys before S1 execution | `gh`/`git` flag spelling, GraphQL field strings, low-level shell quoting |

S2 operation names are intentionally **distinct** from S1 execute operation names. For
example, `github-proxy.github_pr_open_request` is a durable intent; `github.create_pr`
is the neutral adapter execution. The same name must not mean both "request this durable
effect" and "run this CLI command."

Worked boundary cases:

| Case | Surface | Reason |
|---|---|---|
| `parse_issue_state` / REST comment parsing | **S1** | decode GitHub JSON into neutral tables |
| `gh_issue_view_loop_cmd`, `gh_pr_diff`, `shell_single_quote`, `url_encode`, `is_git_ref_safe` | **S1** | command spelling and shell mechanics |
| `github_issue_comment_request` payload validation | **S2** | durable request schema and bounded control fields |
| `current_devloop_state`, `compare_state_marker`, `merge_ready_fact` | **S3** | marker grammar and version-CAS are fkst policy |
| "filter comments to the trusted bot" | **S3** | the adapter only surfaces `author_login`; trust is package policy |
| "check current state before writing labels/comment/PR marker" | **S3 before S1** | guarded-write business logic belongs in the package, under the entity lock |

## 5. Architecture

### 5.1 Public surfaces and operations

There is no `interface` keyword in Lua; the surfaces are documented module contracts plus
tests. Packages use the blessed `all→std` direction (`require("std.github")` /
`require("std.git")`) for S1, package queues for S2, and package modules for S3. No peer
package require is introduced.

**S1 reads — synchronous ("re-derive truth from source"):**

| Operation | Returns | Replaces (today) |
|---|---|---|
| `github.read_issue(source_ref)` | neutral issue table with comments, labels, assignees, blocked_by | `gh issue view`/REST + `parse_issue_state` |
| `github.read_pr(source_ref)` | neutral PR table | `gh pr view` + `parse_pr_view_head_state` |
| `github.read_pr_diff(source_ref)` | `string` (full diff, no truncation — caller is in-process) | `gh pr diff` |
| `github.list_open_issues(repo)` | `{issue_summary,...}` | `gh api …/issues` + `parse_entity_list` |
| `github.list_open_prs(repo)` | `{pr_summary,...}` | `gh api …/pulls` |
| `github.find_pr_for_head(repo, branch, base?)` | PR summary or `nil` | `gh_pr_list_head_cmd` + `parse_pr_list_for_head` |
| `github.read_check_runs(repo, sha)` | check-run summary | `gh_commit_check_runs` |
| `github.read_blocked_by(source_ref)` | `{source_ref,...}` | GraphQL `blockedBy` |
| `github.list_repo_labels(repo)` | `{name,...}` | `gh label list` + `parse_repo_labels` |
| `git.show_ref(branch)` | ref table or `nil` | `git show-ref` + `parse_git_show_ref_head` |
| `git.is_ancestor(a, b)` | `bool` | `git merge-base --is-ancestor` |
| `git.remote_branch_head(branch)` | `sha` or `nil` | `git ls-remote` |
| `git.merge_tree_empty_delta(base, head)` | `bool` | `git merge-tree` |
| `git.list_worktrees()` | `{worktree,...}` | `git worktree list` |

**S1 execute operations — mechanics only, called after S3 where guards are needed:**

| Operation | Returns | Notes |
|---|---|---|
| `github.create_issue(request)` | issue table | takes neutral mechanics only: rendered `title_text` plus `body_text` or `body_file`; no package template ids |
| `github.create_comment(target_ref, body_text_or_file)` | comment table | writes already-rendered text or a temp body-file path via `--body-file`; no package template ids |
| `github.edit_comment(comment_ref, body_text_or_file)` | comment table | command mechanics only; stale-target classification belongs to adapter, retry policy to package |
| `github.reconcile_labels(target_ref, add, remove)` | `bool` | ensures repo labels exist; S3 decides whether the label write is still allowed |
| `github.add_blocked_by(blocked_ref, blocker_ref)` | `bool` | GraphQL `addBlockedBy`; #660's `issueId` fix stays in the adapter mechanics |
| `github.assign_issue(issue_ref, login)` / `github.unassign_issue(issue_ref, login)` | `bool` | assignee claim policy remains package-owned |
| `github.create_pr(request)` | PR table | adapter push/create mechanics only; S3 validates state/head/claim first |
| `github.merge_pr(pr_ref, opts)` | merge result table | `gh pr merge --merge --match-head-commit`; merge authorization remains S3 |
| `github.close_issue(issue_ref)` | `bool` | command execution only |

**S2 durable write intents — package-owned request schemas:**

| Intent / queue | Carries |
|---|---|
| `github_issue_comment_request` / `github_pr_comment_request` | target `source_ref`, `dedup_key`, replace/hand-off control fields, bounded short `body` or package `body_source` |
| `github_issue_create_request` | parent/lineage control fields, `dedup_key`, labels/assignees, bounded short `title` / `body` or package `title_source` / `body_source`; large generated bodies stay unmigrated |
| `github_pr_open_request` | issue `source_ref`, branch/head/base control, expected proposal/state/version/head, bounded short `title` / PR body / issue-comment body or package source handles; large generated bodies stay unmigrated |
| `github_issue_label_request` / `github_pr_label_request` | target `source_ref`, expected proposal/state/version, add/remove labels |
| `github_issue_blocked_by_request` | blocked/blocking `source_ref`, `dedup_key`, bounded marker body or package marker `body_source` |

**S3 guarded-write modules — package-owned policy:**

Each guarded write slice introduces or reuses a small package module that runs under the
current per-entity lock before S1 execution. Examples: `comment_guard`, `label_guard`,
`pr_open_guard`, `merge_guard`, `blocked_by_guard`. These modules read current neutral
comments/issues/PRs through S1, apply trusted-bot filtering and marker/CAS policy, and
return a narrow decision (`apply`, `already_done`, `stale`, `blocked`) plus the exact S1
execute request. When an S2 intent carries a package `body_source` / `title_source`
handle, the package-owned executor resolves it through a package-owned renderer before
calling S1. `std.github` never knows package template ids such as
`github-devloop.reviewing-comment.v1`; it sees only rendered text or a body-file path.
S3 modules do not build command strings.

Guard decisions have explicit reliable-delivery semantics:

| Guard outcome | Delivery behavior |
|---|---|
| `apply` | execute the S1 operation; ack only after the write path has completed |
| `already_done` | ack as an idempotent no-op |
| `stale` | ack terminal because a known newer version/head/state superseded this intent |
| `blocked` | ack terminal with a bounded, grepable WHY |
| `marker-not-yet-visible`, read failure, ambiguous guard fact, ambiguous/non-terminal CAS read failure | fail-closed by raising an error so at-least-once delivery retries |

This mapping preserves the activity ⟂ safety doctrine: no silent ACK of a lost write,
and no infinite retry of a terminal stale or explicitly blocked write. A CAS loss is
`stale` only when the guard has positively observed the newer version/head/state; if the
read is ambiguous, incomplete, or non-terminal, it is a fail-closed retry instead of a
benign ack.

**Local `git` writes — synchronous within a department worktree (idempotent; not saga-mediated; topology unchanged):**

| Operation | Note |
|---|---|
| `git.ensure_worktree(branch, path)` → worktree table | **idempotent**; the #678 force-clean (`worktree remove --force; rm -rf; prune`) lives *inside* this op, not at call sites |
| `git.fetch(branch)` / `fetch_pr_head(pr)` / `fetch_pr_merge(pr)` | command mechanics behind the adapter |
| `git.commit(worktree, message)` → `sha` · `git.push(branch, opts)` | `opts`: normal / force-with-lease / update |
| `git.merge_no_ff(...)` / `fast_forward(...)` / `force_clean_worktree(path)` | |

### 5.2 Neutral normalized return shapes, grown per operation

Do **not** front-load a full `Issue`/`PullRequest`/`Comment`/`Worktree` constructor and
validator universe. v1 uses **per-operation documented plain Lua tables**, grown only as
vertical slices need them. A shared type module emerges by Rule-of-Three: when three or
more public operations share the same shape and tests would otherwise duplicate shape
normalization, extract that shape into `std/github/types.lua` or `std/git/types.lua`.

Initial slice examples:

```
read_issue(source_ref) -> {
  number, title, body, state, url, updated_at,
  author_login,
  assignees = { login, ... },
  labels = { name, ... },
  comments = { { id, author_login, body, created_at, updated_at }, ... },
  blocked_by = { source_ref, ... },
}

read_pr(source_ref) -> {
  number, title, body, state, url, updated_at,
  head_ref_name, head_ref_oid, base_ref_name,
  head_repository, is_cross_repository,
  mergeable, merge_state_status,
  labels = { name, ... },
  comments = { { id, author_login, body, created_at, updated_at }, ... },
}

git.list_worktrees() -> {
  { path, branch, head_sha }, ...
}
```

`source_ref` stays **Tier S** (`std/source_ref.lua`, per the std-shared-library drain
§8) and is reused unchanged as the read/write addressing token. S1 is the boundary that
turns a `source_ref` into a fetched neutral table; S3 interprets that table using package
marker/CAS policy.

### 5.3 Adapter internals (file layout respects the 1000-line cap)

The targets are large (777/483/878/938 lines); a flat `std/github.lua` would blow the
hard cap immediately. The adapter is therefore **multi-file, split by stable
responsibility** (SRP), with thin entry aggregators:

```
std/
  github.lua            -- entry: exposes new(exec), no module-global mutable handle
  github/
    shell.lua           -- shell_single_quote, url_encode, is_git_ref_safe, validators (private toolkit)
    exec.lua            -- the ONE canonical gh_exec wrapper (rate-limit detection, error_class facts)
    issue.lua           -- read_issue / list_open_issues / create_issue / close_issue / assign
    pr.lua              -- read_pr / read_pr_diff / list_open_prs / find_pr_for_head / create_pr / merge_pr
    comment.lua         -- create_comment / edit_comment / comment reads
    label.lua           -- reconcile_labels / list_repo_labels / ensure_repo_label
    graphql.lua         -- blocked_by / node-id / named GraphQL constants (from core/github_graphql.lua)
    check.lua           -- read_check_runs / dispatch_ci
    types.lua           -- created later only when Rule-of-Three justifies shared shapes
  git.lua               -- entry: exposes new(exec)
  git/
    exec.lua            -- git exec wrapper
    worktree.lua        -- ensure_worktree / list_worktrees / force_clean (#678)
    branch.lua          -- fetch / push / show_ref / is_ancestor / remote_branch_head
    diff.lua            -- diff_check / merge_tree_empty_delta / unmerged_paths / conflict_markers
    merge.lua           -- merge_no_ff / fast_forward
```

Builders and parsers move only when their **own vertical slice** moves. Inside the
adapter they are **private** helpers for one public domain operation; no public
low-level builder API is exposed and no transitional shim is left behind. The two
`gh_exec` wrappers (`base.lua` + `gh_rate.lua`) collapse into one `github/exec.lua`.

**Nested-require risk (R1):** the std spec only verified *flat*
`require("std.saga")`. `require("std.github.issue")` resolves under the **same** `?.lua`
package.path substitution (`std.github.issue` → `std/github/issue.lua`), one directory
deeper. Wave 0 includes a spike to confirm this; if the engine loader rejects nested
dirs, fall back to flat naming (`std/github_issue.lua`, `require("std.github_issue")`)
— same modules, flatter paths, verified-to-resolve.

### 5.4 Read/write delivery topology and `body_source`

The refactor is **behavior-preserving** only if it changes *how a command is built and
parsed*, never *which path carries the effect*. Concretely:

- **Reads** were already synchronous in-process; they stay synchronous. The S1 operation
  wraps build+exec+parse. This realizes "回源 derive 真相" cleanly (the adapter fetches,
  business decides) with no payload-staleness.
- **GitHub mutations** that go through `github-proxy.v1` durable requests today still do
  — the executor calls S1 after S3 accepts the write. Reliable delivery, idempotent
  markers, and per-entity locks are untouched.
- **Local `git` writes** done synchronously in a worktree (implement/fix/merge) stay
  synchronous; they move behind `std.git` operations with idempotency (e.g. #678) folded
  into the operation.

The content-not-in-payload constitution constrains **large re-derivable content** such
as issue bodies, PR diffs, code, file contents, and long comment histories. It does not
ban short, bounded, authored control text in a durable intent. Current write intents
freeze `body` / `title` at intent creation for comments, issue creation, and PR opening;
that snapshot is part of their current idempotency behavior because a fixed
`dedup_key` produces fixed visible text.

Write bodies therefore split into two classes:

1. **Short control text**: state/status markers, short comments, short titles, and
   bounded marker comments. Each migrated write intent must declare its body class and,
   for this class, a numeric byte cap in the intent schema. The default cap is **4096
   bytes for `body` text and 256 bytes for `title` text** unless a slice documents a
   smaller cap; exceeding the cap is a validation failure, not a reason to silently
   truncate. These fields are for bounded control text, not codex prose. They may remain
   in the durable S2 intent as rendered `body` / `title` text when they are part of the
   command's idempotency key. They may also be represented by a package `body_source` /
   `title_source` template handle when rendering is deterministic and the rendered
   result is still within the declared cap.
2. **Large authored/generated text**: consensus review bodies, review-result prose,
   implementation-failure output, decomposed issue bodies, spec-amendment bodies, or
   other codex-authored text with no clean durable home under current primitives. These
   writes are **out of scope for this spec's write migration**. They stay on the current
   path until fkst-substrate has a real durable-artifact primitive (§11).

For deterministic templates, the S2 intent carries the package-owned handle and only
immutable bounded params or version/digest-pinned facts:

```
body_source = {
  kind = "template",
  template = "github-devloop.short-status-comment.v1",
  source_ref = { kind = "external", ref = "owner/repo#issue/42" },
  params = { proposal_id = "...", version = "...", pr_number = 7 }
}
```

The package executor resolves that handle through a package-owned renderer into final
text or a temp body-file before calling S1. Rendering from a mutable live source at
execute time is **forbidden** when the visible text could change under the same
`dedup_key`; template inputs must be immutable bounded params or pinned facts.
Versioned template ids are immutable contracts: changing template text, layout, or
meaning mints a new version id (`...v2`, not an edited `...v1`) so execute-time
rendering is deterministic under a fixed `dedup_key`. There is no `artifact_ref`
placeholder in this spec. Until a substrate durable-artifact primitive exists, a slice
with large non-re-derivable authored text cannot be moved to the new write path without
changing reliable-delivery and idempotency semantics.

### 5.5 `github-proxy` transformation + exec/error consolidation

- **`github-proxy` stops owning gh/git command+parse mechanics.** Its GitHub command
  builders and GitHub-JSON parsers move slice-by-slice into `std.github`; its
  departments consume S2 intents, run S3 guards under the existing locks, then call S1
  execute operations. `github-devloop` read paths call S1 directly. Duplicated builders
  (`claims`, `entity_view`) collapse when the relevant slice lands.
- **`github-proxy` does not become "pure saga wiring" in this spec.** State-machine
  ownership remains where it is for now. Marker/CAS guard logic is explicitly package
  business logic (S3), not adapter logic. Moving that state-machine vocabulary out of
  `github-proxy` is R6 future work after the gh/git mechanics are clean.
- **One `gh_exec`.** `github-devloop/core/base.lua:918` and
  `github-proxy/core/gh_rate.lua:38` unify into `std/github/exec.lua` (rate-limit
  detection + `error_class`/`fingerprint` facts). `error_facts.lua`'s GitHub error
  taxonomy (`gh-rate-limited`/`gh-command-failed`) moves with it (Tier R; the *generic*
  L1/L2 error-fact shape stays Tier S).

**Sequencing with saga-harness.** Both refactors rewrite the same ~20 department
`main.lua` files. To avoid double-touching, a department's vertical slice adopts **both**:
`std.department{done, act}` **and** port/intent/guard vocabulary inside `done`/`act`.
The port lands *with or just before* the `done/act` rewrite, because `done` = an S1 read
and `act` = an S2 intent or S1/S3 write.

### 5.6 Injection seam (the #633 load-bearing mechanism)

The adapter handle is obtained through an explicit constructor, not by monkey-patching a
cached Lua module:

```
local github_mod = require("std.github")
local git_mod = require("std.git")

local ports = {
  github = github_mod.new(exec_sync),
  git = git_mod.new(exec_sync),
}
```

`std.github.new(exec)` and `std.git.new(exec)` return handles whose operations are bound
to the injected `exec` primitive. If `exec` is missing, the constructor fails loudly.
The handle has no hidden mutable singleton and no module-level fake switch.

Departments compose this with the engine contract through a package-local constructor.
`make_department(ports)` builds the engine-facing department via `std.department{done,act}`
and returns a `{ spec, pipeline }` table. Note (verified against `std/saga.lua:71-75`):
`std.department` sets `_G.pipeline = wrapped` and returns **only** `{ spec = ... }` — it does
**not** put `pipeline` in its returned table. So `make_department` reads the just-set
`_G.pipeline` and returns `{ spec = dept.spec, pipeline = _G.pipeline }`, making
`dept.pipeline` available for the direct fake-bound test call without changing `std/saga.lua`
(a no-engine-change, std-only concern):

```
local std = require("std.saga")

local function make_department(ports)
  local function done(event)
    local issue = ports.github.read_issue(event.payload.source_ref)
    ...
  end

  local function act(event)
    ...
  end

  local dept = std.department {
    consumes = { "devloop_ready" },
    produces = { ... },
    done = done,
    act = act,
  }
  -- std.department sets _G.pipeline and returns { spec = ... } only; surface pipeline as a
  -- table field so tests can call make_department(fakes).pipeline(event) directly.
  return { spec = dept.spec, pipeline = _G.pipeline }
end

local function production_ports()
  return {
    github = require("std.github").new(exec_sync),
    git = require("std.git").new(exec_sync),
  }
end

local M = make_department(production_ports())
M.make_department = make_department
return M
```

This is the least invasive shape: `done(event)` and `act(event)` keep the
`std.department` contract; `ports` enter through the factory/context that builds the
department, not through extra `done`/`act` parameters. A free-form department that has
not yet adopted `std.department` uses the same idea: `make_department(ports)` returns
`{ spec = ..., pipeline = ... }`, with `pipeline(event)` closing over the ports.

The production `main.lua` top level calls `make_department(production_ports())` at load
time and returns that module, or returns an `M` table with `M.spec` / `M.pipeline`
bound from the same constructor. Therefore the existing engine path-loader sees the
global `pipeline` / spec exactly as today: production departments already set a global
`pipeline` directly, and `std.department` preserves that binding by assigning
`_G.pipeline = wrapped`. `fkst.test.run_department("departments/observe_pr/main.lua",
event, opts)` and the real router keep loading the real-adapter-bound department
unchanged. This needs no engine change and preserves production wiring.

Unit and department tests that need fake ports do **not** use path-based
`run_department`, because the engine primitive takes a path string and loads the
production module with real adapters. They load the module, bind the same constructor to
fakes, and invoke the returned department directly in-process. `make_department` is a
pure constructor and is callable repeatedly, so each test gets a fresh fake-bound
department and does not depend on the production global:

```
local main = require("departments.observe_pr.main")
local std = {
  github = { fake = require("std.github.fake") },
  git = { fake = require("std.git.fake") },
}

local function capture_raises(fn)
  local raised = {}
  local old = raise
  raise = function(queue, payload)
    table.insert(raised, { queue = queue, payload = payload })
  end
  local ok, result = pcall(fn)
  raise = old
  if not ok then error(result) end
  return result, raised
end

local model = require("std.github.fake_model").new()
local dept = main.make_department({
  github = std.github.fake.new(model),
  git = std.git.fake.new(model),
})
local result, raised = capture_raises(function()
  return dept.pipeline(event)
end)

-- assert on `raised` for S2 write-intents and on `model` for fake-recorded S1 writes
```

That direct `make_department(fakes).pipeline(event)` call runs under the same engine
test-mode globals (`raise`, `once`, `with_lock`, `cache_*`, locks, and command mocks
exposed through `fkst.test`) while the Tier R fake model is the stateful external truth.
The spy swaps the injected `raise` global around the direct `dept.pipeline(event)` call;
`once` / `with_lock` / `cache_*` test-mode behavior is unchanged because the call still
runs under the engine's test-mode globals. No engine change and no path-based
`run_department` are needed for the fake path.

This package-side effect-capture pattern already exists: `packages/github-devloop/tests/claim_contract_test.lua:57`
defines `capture_raises(fn)` by temporarily replacing the injected `raise` global,
calling `pcall(fn)`, restoring `raise`, and returning the captured `{ queue, payload }`
records. Fake-bound business tests should use this shape, not command-string matching.
This closes the verification half of #633: business tests deterministically assert
raised write-intents plus fake-recorded adapter writes, instead of shell command strings.

A small shared test helper is a Wave-0 deliverable so departments do not hand-roll this
spy. For example, `std.testing.run_fake(dept, event) -> { result, raises, writes }`
should wrap `capture_raises`, call `dept.pipeline(event)`, and read the Tier R fake's
recorded adapter writes from the model. Tests that want the real production adapter path
continue to call `run_department(path, event, opts)`, whose result already exposes
captured raises as `result.raises`. Tests that want fake injection call
`make_department(fakes).pipeline(event)` through the shared helper. Both paths compose
without changing fkst-substrate, closing #633 concretely.

Global `exec_sync` closure inside business helpers and module monkey-patching are
**forbidden as the injection seam**. Production may mention `exec_sync` only in the
production port factory at the module boundary; business logic receives a port handle:

```
local function done(event)
  local issue = ports.github.read_issue(event.payload.source_ref)
  ...
end
```

Tests use the same seam in two ways:

- adapter contract tests call `std.github.new(fake_exec)` / `std.git.new(fake_exec)` and
  assert command construction + parse behavior in one isolated suite;
- business tests call `make_department({ github = std.github.fake.new(model), git =
  std.git.fake.new(model) })` and assert domain reads, S2 intents, and S3 guard
  decisions without ever matching command strings.

This mirrors existing package-side injection precedent (`read_env(name, exec)`,
`gh_exec(..., exec)`) and makes the #633 payoff real: command spelling is swappable at
the adapter boundary, not through global monkey-patching.

## 6. Test boundary (the payoff: behavior, not strings)

Today ~100 business tests mock exact command strings (#633 brittleness). After the
refactor the test boundary is split by the same three surfaces:

1. **Business tests inject ports through §5.6.** They construct the department with
   `make_department({ github = std.github.fake.new(model), git =
   std.git.fake.new(model) })` and call the returned `.pipeline(event)` directly under
   engine test-mode globals through a `capture_raises(fn)`-style spy. They use Tier R
   fakes and assert on neutral reads, durable S2 write intents captured from
   `raise(...)`, fake-recorded S1 adapter writes, and S3 guard outcomes: `read_pr`
   returns a PR table with `head_ref_oid=…`, a comment intent was raised once for
   `issue#42`, a local `git.push` adapter write was recorded once by the fake model, or
   a guard returned `already_done` because a trusted marker is visible. Counting
   "`post_comment` command string invoked once" disappears from business tests.

2. **The saga-harness oracle stays Tier S and GitHub-agnostic.** It defines an abstract
   effect/truth interface: record write-class effects, replay reads from external truth,
   and compare delivery-1 vs delivery-2 behavior under the ①②③ restart contract. It must
   never depend on GitHub-specific shapes, marker names, or `std.github` APIs.

3. **`std.github.fake` / `std.git.fake` are Tier R implementations of that abstract
   interface.** They are the in-memory GitHub/git models used by business tests through
   the constructor seam. The oracle's effect set for a migrated department is the union
   of **(a)** S1 adapter writes recorded by the Tier R fake model and **(b)** S2
   write-intents captured from `raise(...)` by the package-side spy. Migrated business
   departments write to GitHub by raising durable requests such as
   `raise("github-proxy.github_issue_comment_request", payload)`, not by calling
   `std.github.fake` directly, so the oracle must observe captured raises in addition to
   fake-recorded adapter calls. The upgrade from "command multiset" to "intent/effect
   multiset" survives, but the Tier S harness is not identical to the GitHub fake and the
   fake is not `fkst.test.command_calls()`.

4. **The real adapter is tested once, in isolation, for API-contract fidelity.** Only
   `std.github` / `std.git` tests verify that a public operation builds the expected
   command and parses the expected stdout shape. This is the single place command-string
   coupling is allowed to exist. It is concentrated adapter-local brittleness, not
   business-test brittleness.

There is one explicit cross-spec contradiction to reconcile. The current saga companion
spec (§4) defines the ①②③ oracle concretely as
`fkst.test.command_calls()` plus a write-class command multiset. That does not compose
with this ports/adapters seam, because a fake-bound department bypasses command
execution and runs through `make_department(fakes).pipeline(event)`. The required
reconciliation is: the saga ①②③ oracle observes the combined effect multiset for a
migrated department: Tier R fake-recorded S1 adapter writes plus package-spy-captured S2
write-intents. It then compares delivery-1 vs delivery-2 over that combined multiset at
the abstract effect/truth boundary. It must not use `fkst.test.command_calls()` as the
business-test oracle for migrated departments, and it must not rely only on fake-recorded
adapter calls because S2 GitHub writes are raised as durable intents.

This spec does **not** edit the saga companion spec; that is a separate workstream. It
records a coordination dependency: when the saga workstream lands, its oracle section
must be amended from "write-class command multiset from `fkst.test.command_calls()`" to
"combined abstract effect multiset: fake-recorded S1 adapter writes plus captured S2
write-intents." Adapter contract tests may still use command calls locally, because
command spelling is the product there.

This directly serves "让问题都在测试解决": business tests become readable behavioral
assertions, the restart oracle remains generic, and command spelling is pinned only
where command spelling is the product.

## 7. Generalization hook (future, not in this plan)

`codex`, filesystem, network, and record-replay hardening can adopt the same adapter
shape later; this plan emits no tasks for them.

## 8. Conformance teeth (the "严格约束" — make the boundary permanent)

A new `scripts/check_repo.py` G-gate uses the same ratchet mechanism as the
saga-harness allowlist, but it must be **context-aware**:

- **No gh/git command construction outside migrated adapter files.** Flag a string that
  is built as a `gh`/`git` command and passed to `exec_sync`, `gh_exec`, `git_exec`, or an
  equivalent wrapper outside `std/github` or `std/git`. Do **not** flag ordinary textual
  mentions in prompts, tests, docs, comments, fixture bodies, marker text, or issue
  templates.
- **No direct gh/git execution outside the adapter.** A direct `exec_sync` / `gh_exec` /
  `git_exec` call whose command head is `gh` or `git` is red once that file's slice has
  migrated. `codex` and other non-gh/git exec uses are unaffected.
- **Per-migrated-file ratchet.** During migration the allowlist is file-scoped and only
  shrinks. A vertical slice closes the gate on the files it touches; untouched files keep
  their temporary allowance until their slice lands.
- **Port-only dependency.** Packages reach the gh/git world through
  `require("std.github")` / `require("std.git")`; peer cross-package require stays
  banned (existing G9).

The gate enforces the Adapter doctrine without banning harmless prose. It catches the
decay mode that matters: newly constructed or directly executed gh/git commands leaking
back into business code.

## 9. Migration — vertical-slice strangler

Each slice is independently mergeable and CI-gated. There is no bulk public-builder
relocation wave, no public low-level builder API, and no compatibility shim.

- **Wave 0 — Foundations (one PR, behavior-neutral).**
  - Consolidate the two `gh_exec` implementations into the S1 adapter exec module shape.
  - Create empty `std.github` / `std.git` skeletons with `new(exec)` constructors.
  - Create the Tier S abstract oracle interface that the saga-harness will observe, and
    coordinate the companion saga spec's §4 wording from command-multiset to
    abstract write-intent/effect oracle (§6, §11).
  - Spike-verify nested `require("std.github.issue")` and decide the R1 fallback.
  - Move no builders/parsers and change no business behavior.

- **Per-slice loop — repeat one high-level operation at a time.**
  - Pick one public port operation, reads first: `read_issue`, then `read_pr`,
    `read_pr_diff`, `read_check_runs`, and so on.
  - Move only the builders/parsers that operation needs into the adapter as private
    internals.
  - Expose exactly one neutral domain operation returning the per-op shape from §5.2.
  - Migrate that operation's call sites.
  - Rewrite those tests to use the §5.6 port fake / S2 intent assertions instead of
    command-string mocks.
  - Delete the old builder/parser copy in the same slice.
  - Close the §8 ratchet on the touched files.
  - If the department is also adopting saga-harness `done/act`, do the `done`/`act`
    rewrite in the same slice.

- **Reads first.** Read operations are synchronous and idempotent, so they are the
  lowest-risk way to prove the boundary and fake seam.

- **Guarded writes second.** Each write slice introduces the needed S2 intent schema
  cleanup for bounded short text or deterministic package `body_source` /
  `title_source`, plus the S3 guard module, before moving the S1 execution mechanics.
  Large non-re-derivable authored bodies are skipped by this migration until §11's
  durable-artifact substrate primitive exists. The order is: intent contract, guard
  decision under lock, package render if needed, adapter execute op, tests, delete old
  command code, close ratchet.

- **Coordinate with saga-harness.** A department's `done`/`act` rewrite adopts the port
  vocabulary in the same slice. The oracle observes S2 intents / abstract effects
  recorded by the Tier R fake, not `fkst.test.command_calls()`; the business test
  injects Tier R fakes through §5.6 and invokes `.pipeline(event)` directly.

Waves and slices need no engine change. Adapter-local command-string contract tests
cover command fidelity now.

## 10. Risks / open questions

- **R1 — nested `std/` require unverified.** Mitigation: Wave 0 spike; flat-naming
  fallback (§5.3). Cheap, decided before any operation slice.
- **R2 — collision with the concurrent saga-harness dept rewrite.** Both touch every
  `main.lua`. Mitigation: §9 sequencing — one PR per dept adopts both axes; the port is
  the vocabulary `done`/`act` already need.
- **R3 — behavior drift during per-slice extraction.** Moving builders/parsers by hand
  could silently change command semantics. Mitigation: each slice has adapter-local
  command+parse contract tests before business tests are rewritten, and the old
  builder/parser copy is deleted in the same slice.
- **R4 — 1000-line cap during the move.** The adapter is pre-split by responsibility
  (§5.3) so no submodule approaches the cap; `parsers.lua` (878) and `commands.lua` (777)
  *shrink* as their contents distribute across `issue/pr/comment/label/graphql`.
- **R5 — `read_pr_diff` and large content.** Reads return full content **in-process** to
  the caller (no payload, no truncation) — consistent with the content-not-in-payload
  constitution, which constrains *delivery payloads*, not in-process port returns. The
  port must not be used to stuff diff text into a durable event; §5.4 keeps writes
  source/address-handle based.
- **R6 — state-machine vocabulary still in `github-proxy`.** `current_devloop_state` /
  version-CAS remain business logic mislocated in `github-proxy`. Out of scope here (§2);
  noted as a follow-up once the gh/git mechanics are clean (the neutral comments it
  depends on will by then come from `std.github`, making the later move smaller).
- **R7 — injection seam bypass.** Developers may accidentally call `exec_sync` directly
  in migrated files. Mitigation: §8 context-aware gate plus tests that construct ports
  through `std.github.new(exec)` / `std.git.new(exec)`.
- **R8 — non-re-derivable large authored text.** Some current `body`/`title` payloads
  freeze generated prose that is too large or too semantically rich to treat as bounded
  control text, and it cannot be safely re-rendered later. Mitigation: §5.4 explicitly
  excludes those writes from this migration; they stay on the current path until a real
  substrate durable-artifact primitive exists.
- **R9 — saga oracle spec drift.** The companion saga spec currently describes the
  ①②③ oracle as `fkst.test.command_calls()` plus a write-class command multiset, while
  this design requires an abstract write-intent/effect interface observed through Tier R
  fakes. Mitigation: §6 makes the contradiction explicit and §11 records a coordination
  dependency for the saga workstream to amend its oracle wording before migrated
  departments rely on the fake-bound oracle.

## 11. Substrate dependencies (what is package-side vs fkst-substrate)

| Item | Home | Blocking? |
|---|---|---|
| `std.github` + `std.git` adapters, constructor seam, per-op slices, S2 intent cleanup, S3 guards, port-level tests | **fkst-packages** (this repo) | core deliverable |
| Saga ①②③ oracle wording update: replace `fkst.test.command_calls()` command-multiset with the abstract write-intent/effect interface implemented by the stateful truth fake | saga-harness companion workstream | **coordination dependency**; this spec records it but does not edit that file |
| `exec_sync` primitive (already exists) | fkst-substrate | already available; no change |
| Record-replay test mode / substrate #88 | future optional hardening | **non-blocking**; not in this plan |
| Durable authored-artifact primitive for large generated issue/comment/PR bodies | fkst-substrate follow-up, sibling to #88 | **blocking only for migrating §5.4 class (ii) writes**; not in this plan |

The package-side refactor is self-contained and needs **no** engine change — the
strongest de-risking property of this design.

⟦AI:FKST⟧
