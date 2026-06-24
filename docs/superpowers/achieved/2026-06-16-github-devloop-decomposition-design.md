# Design: github-devloop decomposition - installer groups + deferred dogfood overlay

> **Status:** decomposition design **accepted** (2026-06-16) via sshx worker-delegated inline consensus, with scope **re-bounded** below. Review history: thinking triplet → convergence → implementation → 3 review rounds. v1 rejects (dependency direction; github-proxy coverage) closed; v2 rejects (target/slice mismatch; board/convergence coverage) closed; v3 rejects folded as the scope re-bound + corrections below. Accepted by explicit user decision — stop paper-enumerating a pervasively-coupled package; remaining seams are discovered compiler/test-guided during implementation.
>
> ### Scope re-bound (authoritative — overrides any wider claim in the body)
> This spec covers the **github-devloop INTERNAL decomposition only**:
> - **Physically split** the 5 mixed god-modules (`observability`, `config`, `prompts`, `payloads`, `reconcile`) + the secondary `commands`/`branches` caller-splits, using the dependency-inversion seams proven in the body.
> - **Classify in place** (install-group comment in `core.lua`, no physical move) every other single-concern module — including the **merge heart** (`queue` / `merge_batch` / `merge_gate` / `reason_classes` / `pr_safety`).
> - **Orchestrator tier (new):** `observe_issue`, `observe_pr`, and the observability `main` are **orchestrators** allowed to span planes (they carry the design's diagnosis hooks). The "no cross-plane call" invariant applies **only to pure leaf modules**, not orchestrators. Shared observe-read facts flow through injected `ports.github` / `ports.git` as departments migrate; any still-raw observe-read command helper remains migration debt under G-ADAPTER, while detector-only domain helpers are diagnosis. The deleted `observability.lua` installer role (`M.observe_devloop_entities`) is taken over by a named orchestrator module so it stays installed.
> - **Board builders:** public `M.build_board_*` names are **preserved** (port); only the implementation moves to ops `board_context`; lifecycle callers (`context_bundle.lua:401`, `intake_judge:57`, `observe_issue`, `loop`, `review_pr`, `review_loop`, `replayer`) receive **injected** board context — no rename, no dangling caller, no lifecycle→ops dependency.
>
> ### Ports-layer reconciliation (current state — 2026-06-16)
> The accepted decomposition model is unchanged. The landed ports layer now owns `gh`/`git` **command mechanics** in repo-std Tier R `std.github` / `std.git`, wired by `std.ports.install(make_department)` and injected as `ports`; production handles come from `std.ports.production_handles()` via `std.github.new(run)` and `std.git.new(run)`. `github-proxy` is the GitHub **protocol/queue adapter** for poll events and durable issue/PR/comment/label intents; it is not the `gh`/`git` command adapter.
>
> Migration is early. Today only `packages/github-devloop/departments/consensus_result/main.lua` uses `make_department(ports)`. Most raw `gh`/`git` construction in `github-devloop` and `github-proxy` remains migration debt under `migration/gh-git-adapter.allowlist`; #738 is migrating observe-read departments (`decompose`, `intake_scan`, `observe_pr`, `loop`) and shared observe-read helpers. This spec references [Ports & Adapters](./2026-06-15-ports-adapters-design.md) as the command-adapter authority and does not duplicate it.
>
> ### NOT in this spec (named follow-ups)
> - **github-proxy `fkst-dev` leak cleanup** = a **separate contract-migration spec** (it changes the github-proxy↔github-devloop contract; not a behavior-equivalent move). It is not part of this spec's numbered slices.
> - **Full physical relocation** of single-concern modules into `core/{foundation,lifecycle,ops,diagnosis}/` subdirs.
> - Any **physical move of the merge heart**.
>
> ### Seam discovery (honest coverage)
> lifecycle↔diagnosis coupling is **pervasive** (observe departments *are* diagnosis hooks; observe-view commands are shared). This spec does **not** pre-enumerate every edge; it provides the target + the **dependency-inversion seam pattern** + proven examples + the orchestrator tier + the per-slice equivalence proof + a conformance guard that checks **leaf purity** (not orchestrators). Remaining edges are discovered and cut **compiler/test-guided during implementation** (strangler-fig). This is the repo's *先找 harness* doctrine: the right tool for a pervasively-coupled module is compiler-guided seam discovery, not exhaustive upfront enumeration.


Status: decomposition spec accepted · Date: 2026-06-16 · Repo: fkst-packages
Expands: [Capability Layering: generic dev mechanism / fkst run-ops / self-diagnosis](./2026-06-15-capability-layering-design.md), specifically P1 god-package decomposition.
Scope: behavior-equivalent refactor of `packages/github-devloop`; `github-proxy` protocol/queue contract cleanup is a named follow-up; staged package extraction is recorded as Option 2 but deferred.
中文摘要：本规格把已接受的 P1 分层设计展开成可逐 PR 落地的拆分方案。每个 slice 都必须保持行为、队列、schema、marker、dedup、source_ref 与 CAS 语义不变；`github-proxy` contract cleanup 已移出为后续规格。

---

## 1. Context / why

`github-devloop` currently carries Plane 1 lifecycle, Plane 2 fkst run-ops, and diagnosis in one composed package. That package should remain composed because it genuinely depends on `github-proxy` and `consensus` through namespaced queues; flattening would duplicate the `github-proxy` protocol/queue adapter and the consensus engine.

Two adapter boundaries are explicit. `std.github` / `std.git` are repo-std Tier R `gh`/`git` command adapters: they are not engine APIs and they are not `github-proxy`. `github-proxy` is the package protocol/queue adapter that translates GitHub poll events and durable GitHub write intents into the package event stream.

The accepted capability-layering design decided the product boundary. This spec decides the code boundary: split the package internally first, without behavior change, so later extraction is possible without destabilizing the self-driving loop that must merge these refactor PRs. The accepted P1 also requires removing `github-proxy` knowledge of devloop protocol/guard semantics; that is a separate contract-migration follow-up because it intentionally changes the protocol/queue adapter contract instead of only moving code. `gh`/`git` command mechanics belong to the ports layer, not that follow-up.

## 2. Scope

### Option 1 - primary, actionable

Keep `packages/github-devloop` as one composed package. `packages/github-devloop/fkst.toml` keeps these `[event_deps]` packages:

```text
github-proxy
consensus
```

The loop is GitHub-coupled. It is not a flat package, and this refactor must not try to make it one.

External GitHub/git command mechanics are handled by [Ports & Adapters](./2026-06-15-ports-adapters-design.md): `std.github` / `std.git` behind `std.ports.install(make_department)`. This decomposition must reuse that seam and must not create a second raw-command adapter inside the lifecycle, ops, or diagnosis planes.

Option 1 changes only module ownership, installer grouping, and the explicitly allowed split paths inside `github-devloop`. It must preserve:

- all public functions installed on `M`;
- every `M.spec` consume/produce/fanout/stall/retry declaration;
- all queue names, including `devloop_*`, `github-proxy.*`, and `consensus.*`;
- all schemas, including `github-devloop.*`, `github-proxy.v1`, and `consensus.*`;
- all marker prefixes, especially `fkst:github-devloop`, `fkst:github-proxy`, and `fkst:dashboard:v1`;
- all dedup key formulas;
- all `source_ref` shape and re-derivation behavior;
- CAS and marker trust semantics.

### Named follow-up - `github-proxy` protocol/queue leak cleanup

Do not implement the `packages/github-proxy` leak cleanup in this spec. It is not a move-only behavior-equivalent slice: it changes the `github-proxy` <-> `github-devloop` contract so the protocol/queue adapter stops knowing `fkst-dev` state semantics. The replacement contract is generic caller-supplied filtering and guards, with `github-devloop` providing devloop-specific policy through request payloads, queues, or neutral protocol hooks.

That follow-up must preserve delivery safety by re-deriving GitHub facts before writes, keeping dedup keys stable where requests already have them, and making new guard identifiers explicit and grepable. It must remove devloop protocol/guard leaks from `github-proxy`; it must not move `gh`/`git` command construction there, because command mechanics drain to `std.github` / `std.git` under G-ADAPTER.

### Option 2 - deferred follow-on

Define a future `packages/fkst-dogfood` composed overlay that owns fkst ops plus diagnosis. Do not implement it until one of the trigger criteria in §10 fires.

## 3. Target layout

This target is a shrink-only physical split, not a full `core/{foundation,lifecycle,ops,diagnosis}` relocation.

`packages/github-devloop/core.lua` becomes a grouped installer only:

1. create `M`;
2. declare `M.persistence_class()` returning `saga`;
3. install existing and newly split modules in dependency order using `@capability` comments: foundation -> lifecycle -> ops -> diagnosis;
4. return `M`.

Only these surfaces are physically split in this effort:

- mixed god-modules: `core/observability.lua`, `core/config.lua`, `core/prompts.lua`, `core/payloads.lua`, `core/reconcile_requests.lua`;
- secondary mixed caller-split modules: `core/commands.lua`, `core/branches.lua`;
- department-local mixed observability files: `departments/observability/common.lua`, `census.lua`, `dashboard.lua`, and `reaper.lua`;
- reconcile department implementation body: `departments/reconcile/main.lua` is reduced to spec/dispatch/failure wrapping, with handlers split into department-local modules;
- the timeout-specific helpers currently in `core/convergence.lua`, which move to the diagnosis timeout owner while Plane 1 convergence helpers stay in `core/convergence.lua`.

All other single-concern cohesive modules stay at their current `core/*.lua` paths in this spec. They are classified only by the `@capability` install-group comments in `core.lua`, not by physical relocation. This includes `core/base.lua`, `core/state.lua`, `core/markers.lua`, `core/entity.lua`, `core/requests.lua`, `core/dependencies.lua`, `core/claims.lua`, `core/validators.lua`, `core/restart.lua`, convergence helpers in `core/convergence.lua` except the timeout helpers named above, `core/saga.lua`, `core/queue.lua`, `core/merge_batch.lua`, `core/merge_gate.lua`, `core/merge_gate/reason_classes/*`, `core/pr_safety.lua`, `core/liveness.lua`, `core/doctor.lua`, `core/error_facts.lua`, `core/state_gap.lua`, `core/queue_starvation.lua`, `core/conflict_telemetry.lua`, `core/rollup_health.lua`, `core/release_notes.lua`, `core/ensure_repo.lua`, `core/substrate_ref.lua`, `core/github_proxy_entity_view.lua`, `core/forks.lua`, `core/parsers.lua`, `core/strings.lua`, `core/registry.lua`, and any other existing cohesive single-concern module not named in the physical split allowlist.

Allowed new directories are only for modules produced by the allowlisted splits, for example:

```text
packages/github-devloop/
  core.lua                         # grouped installer with @capability comments
  core/
    base.lua                       # example cohesive stay-in-place module
    queue.lua                      # merge heart stays in place
    merge_batch.lua                # merge heart stays in place
    merge_gate.lua                 # merge heart stays in place
    merge_gate/reason_classes/*    # merge heart stays in place
    pr_safety.lua                  # merge heart stays in place
    convergence.lua                # Plane 1 convergence stays, timeout helpers move out
    foundation/env.lua             # split out of core/config.lua
    lifecycle/config.lua           # split out of core/config.lua
    lifecycle/prompts/*.lua        # split out of core/prompts.lua
    lifecycle/payloads.lua         # split out of core/payloads.lua
    lifecycle/reconcile_requests.lua
    ops/config.lua                 # split out of core/config.lua
    ops/prompts.lua
    ops/board_context.lua
    ops/dashboard.lua
    diagnosis/timeout_reconcile.lua
  departments/
    observability/main.lua         # queue/spec wrapper and orchestration boundary
    reconcile/main.lua             # queue/spec dispatcher only
    reconcile/convergence.lua      # deterministic Plane 1 reconcile handlers
    reconcile/timeout.lua          # diagnosis timeout reconcile handler
```

Old mixed files in the physical split allowlist are deleted as their functions move. Cohesive stay-in-place modules are not deleted, moved, or renamed by this spec. There is no shim, compat require, legacy filename, or duplicate installer path.

Full physical relocation of cohesive modules into `core/{foundation,lifecycle,ops,diagnosis}` is a named follow-up, `github-devloop full capability-directory relocation`, and is out of scope for these slices.

中文补充：本规格只允许 shrink-only 拆分混合职责文件；单一职责文件本轮只在 `core.lua` 安装注释中分组，不搬家。

## 4. Per-module split

| Current surface | Target owner | Required move |
|---|---|---|
| `core/observability.lua` | observability orchestrator plus department wrapper | Delete the file. Its exported `M.observe_devloop_entities(event)` must remain installed with identical public behavior, but the implementation belongs to the observability orchestration surface and calls owned pieces in order: lifecycle census -> lifecycle reaper -> diagnosis patrols including state gap -> ops dashboard. The orchestrator is not lifecycle; it is the boundary that preserves today's public return shape for dashboard/tests while enforcing direction between lifecycle, ops, and diagnosis. |
| `departments/observability/main.lua` | department wrapper | Keep `M.spec` unchanged: consumes `devloop_observe_tick`; produces `github-proxy.github_issue_create_request` and `devloop_merge_queue_tick`; `retry=false`; `stall_window=2m`. It stays the queue/spec wrapper and orchestration boundary. |
| `departments/observability/census.lua` | lifecycle read-model helper plus observability orchestrator wrapper | Move only lifecycle-only trusted entity fact normalization to lifecycle-owned split modules, preserving pure census helpers such as entity collection, stall suspect age/threshold calculation, and observe entity log-line formatting. Departments/orchestrators that fetch GitHub facts close over injected `ports` via `make_department(ports)` and pass precomputed neutral facts into those pure helpers; lifecycle census must not build raw `gh`/`git` commands and must not create a parallel fake/injector seam. Keep the public `collect_observability_entities` wrapper on the observability orchestration surface because today it computes and returns `state_gap_report` for dashboard consumption. That wrapper must call lifecycle census first, then call diagnosis-owned state gap calculation, log the state gap report, and return the same shape before dashboard rendering consumes it. Lifecycle census must not call diagnosis. |
| `departments/observability/reaper.lua` | lifecycle | Move orphan managed-PR cleanup to lifecycle-owned split modules, preserving `reap_orphan_prs` and the existing close/comment behavior. This is lifecycle because it reconciles managed PRs whose parent issue is terminal. |
| `departments/observability/dashboard.lua` | ops | Move board rendering and publishing to an ops-owned split module, preserving `render_observability_dashboard` and `publish_observability_dashboard`. Dashboard marker prefix and label behavior must not drift. Existing cohesive helper modules that the dashboard already calls, such as `core/state_gap.lua`, stay at their current paths. |
| `departments/observability/common.lua` | dissolved | Move constants/helpers to the actual owners: dashboard constants and JSON/input helpers to ops; reaper limits to lifecycle; observe repo/bot/fetch policy helpers to lifecycle census or the observability orchestrator as appropriate. Any GitHub/git reads use the existing `std.ports` handles from `make_department(ports)`; do not replace `common.lua` with a new raw-command seam. Deadline/error helpers move to the owners that call them. Delete `common.lua`. |
| `core/state_gap.lua` | diagnosis classification, stay-in-place | Do not relocate this file. Keep `state_gap_report`, `state_gap_marker_stream`, `state_gap_edges_for_entity`, `state_gap_log_line`, `append_state_gap_dashboard_section`, and all current public `M.*` names at `core/state_gap.lua`. Classify it under diagnosis in the `core.lua` install comments for this effort; any function-level physical split between detector and dashboard formatting is part of the later full directory relocation follow-up, not this spec. |
| `core/queue_starvation.lua` | diagnosis classification, stay-in-place | Do not relocate this file. Keep queue starvation detection and repair issue request building at `core/queue_starvation.lua`, preserving `observe_queue_starvation`, recent-closed merge evidence, queue-head logic, and existing issue-create payloads. |
| `core/conflict_telemetry.lua` | diagnosis classification, stay-in-place | Do not relocate this file. Keep conflict file logging parser, hotspot detection, `build_conflict_hotspot_issue_create_request`, and `observe_conflict_hotspots` at `core/conflict_telemetry.lua`. The `FKST_DEVLOOP_CONFLICT_LOG_CMD` knob is ops config, but the detector module stays at its current path in this effort. |
| `core/config.lua` | foundation + lifecycle + ops | Delete the file. Move `read_env_command`, `env_present_command`, `read_env`, and `env_present` to foundation env primitive access. Move lifecycle posture and neutral lifecycle configuration to lifecycle config: `write_mode`, `max_inflight`, `managed_sibling_repos`, `max_fix_rounds`, `max_converge_rounds`, `default_test_command`, `test_command`, `intake_probe_gate`, repo/bot identity access, and the target/base-branch accessor consumed by lifecycle departments. Move dogfood topology policy to ops config: upstream/integration/rollup/sync knobs, release-notes fallback, board command, conflict-log command, and rollup red-window knobs. Preserve `branch_config` and `devloop_config` public names and return shapes while unmigrated callers still need them, but split internals so lifecycle reads base-branch data from foundation/lifecycle config and never reaches into ops. Any git-derived branch facts required by this seam come through the `std.git` port supplied to a department/orchestrator, then enter pure config helpers as data. Ops may feed topology policy into the neutral branch config; it does not own lifecycle base-branch resolution. |
| `core/prompts.lua` | lifecycle + ops | Delete the file. Move shared preamble/template rendering and parsers to lifecycle prompt common: `output_language`, `prompt_preamble`, `review_observation_boundary_clause`, `short_review_observation_boundary_clause`, `render_prompt_template`, `parse_intake_action`, and `parse_review_meta_action`. Move lifecycle builders to lifecycle: `build_implement_prompt`, `build_fix_prompt`, `build_review_meta_prompt`, `build_intake_prompt`, and `build_decompose_prompt`. Move `build_sync_conflict_prompt` to ops, and move the resource `prompts/sync_conflict.lua` to the ops prompt resource path. |
| `core/payloads.lua` | lifecycle + injected board context | Delete the file. Move proposal, transition, handoff, review-gap, and commit-subject builders to lifecycle: `is_gate_owned_review_gap`, `is_out_of_contract_review_gap`, `build_devloop_ready_payload`, handoff verifiers, `build_devloop_reviewing_payload`, `build_current_head_reviewing_payload`, `build_devloop_open_pr_payload`, `build_devloop_fixing_payload`, `build_replayed_fixing_payload`, `build_devloop_review_meta_payload`, `fix_reflection_dedup_key`, `build_devloop_fix_reflection_payload`, `build_devloop_merge_ready_payload`, `build_devloop_intake_candidate_payload`, `build_proposal`, `build_loop_proposal`, `build_pr_review_proposal`, `build_pr_review_loop_proposal`, `implement_commit_subject`, and `fix_commit_subject`. Lifecycle owns proposal building. Move board feed/digest computation to ops board context: `board_digest_block`, `append_board_digest_to_proposal`, and board digest source handling. Replace board-decorated proposal wrappers with the existing ports-backed dependency-inversion seam: departments/orchestrators close over injected `ports` via `make_department(ports)`, ops/diagnosis computes `board_digest`/`board_context` using those ports where GitHub/git facts are needed, and lifecycle proposal builders receive precomputed board data as ordinary arguments. Pure lifecycle/core helpers never call `std.github`, `std.git`, raw `gh`/`git`, ops, or diagnosis. Update every production board-digest/proposal caller in the same slice: `departments/observe_issue/main.lua`, `departments/loop/main.lua`, `departments/review_pr/main.lua`, `departments/review_loop/main.lua`, `core/replayer.lua`, `core/context_bundle.lua`, and `departments/intake_judge/main.lua`. The `core/context_bundle.lua` board file path receives precomputed board context/digest through its args instead of calling the ops board source directly. `intake_judge` builds the same proposal by passing injected board context into the lifecycle proposal builder rather than calling an ops-owned board wrapper. After these call-site updates, deleting the old board-decorated wrapper path leaves no dangling production caller and no lifecycle -> ops dependency. |
| `core/reconcile_requests.lua` | lifecycle + diagnosis | Delete the file. Move convergence request builders to lifecycle: `build_reconcile_label_request`, `build_review_reconcile_label_request`, `build_fix_reconcile_label_request`, `build_reconcile_comment_request`, `build_fix_reconcile_comment_request`, and `build_review_reconcile_comment_request`. Move timeout reconcile request building to diagnosis; the current timeout comment builder is local to `departments/reconcile/main.lua`, so promote it only inside the timeout slice if needed. |
| `core/convergence.lua` | lifecycle Plane 1 convergence, stay-in-place except timeout helpers | Keep `core/convergence.lua` at its current path for Plane 1 convergence helpers: source-ref/converge digests, base-version helpers, thinking/review/fix reconcile payload builders, supported-payload validators, and convergence/reconcile marker readers. Move only the timeout-specific helpers to the diagnosis timeout owner while preserving the public `M.*` names and exact schemas, queues, markers, dedup, and state-version strings: `build_devloop_timeout_reconcile_payload`, `timeout_reconcile_state_version`, `is_supported_timeout_reconcile`, `timeout_reconcile_marker`, and `has_timeout_reconcile_marker`. |
| `departments/reconcile/main.lua` | dispatcher | Keep `M.spec` identical: consumes `devloop_reconcile`, `devloop_review_reconcile`, `devloop_fix_reconcile`, `devloop_timeout_reconcile`; produces GitHub issue/PR comment and issue label request queues; `stall_window=2m`. Split implementation into `convergence.lua` and `timeout.lua`; `main.lua` dispatches by schema and wraps failure exactly as today. |
| `departments/reconcile/convergence.lua` | lifecycle | Own the current thinking/review/fix deterministic reconcile paths. Preserve schemas `github-devloop.reconcile.v1`, `github-devloop.review-reconcile.v1`, `github-devloop.fix-reconcile.v1`; marker strings; dedup keys; state versions; lock keys; trusted-bot checks; and CAS branches. |
| `departments/reconcile/timeout.lua` | diagnosis | Own `github-devloop.timeout-reconcile.v1` handler behavior. Use the diagnosis timeout helper owner for `timeout_reconcile_state_version`, `timeout_reconcile_marker`, `has_timeout_reconcile_marker`, `build_devloop_timeout_reconcile_payload`, and `is_supported_timeout_reconcile`. Preserve dedup key formula and version-pinned blocked transition behavior. |
| `core/commands.lua` | residual non-command helpers after ports migration | Delete or shrink the file only as G-ADAPTER slices drain raw `gh`/`git` command construction into `std.github` / `std.git`. This decomposition may split residual non-command domain/policy helpers by caller plane, but it must not create new per-plane raw command-builder homes. Preserve public command-builder `M.*` names only while unmigrated allowlisted callers still need them; remove or collapse each name when the corresponding port slice drains it. |
| `core/branches.lua` | residual branch policy after ports migration | Delete or shrink the file only as G-ADAPTER slices drain raw git command construction/parsing into `std.git`. This decomposition may split residual non-command branch policy by caller plane, but it must not create new lifecycle/ops/diagnosis git-command modules. Preserve public branch `M.*` names only while unmigrated allowlisted callers still need them; shrink the surface when a port slice removes the command construction. |

## 5. Merge-heart constraint

This spec contains no slice that relocates, deletes, splits, or redesigns the merge heart:

- `core/queue.lua`
- `core/merge_batch.lua`
- `core/merge_gate.lua`
- `core/merge_gate/reason_classes/*`
- `core/pr_safety.lua`

These files stay at their current paths in this effort and are only classified under lifecycle by the `@capability` install-group comments in `core.lua`. Reason classes stay under `core/merge_gate/reason_classes/*`; they do not move with merge gate because merge gate itself does not move.

The running loop uses these files to merge the PRs that perform this decomposition. Moving them now would concentrate self-modification risk in the exact path responsible for landing the work. Any future merge-heart split or relocation is a separate supervised design and is explicitly out of scope for these slices.

## 6. Dependency rules

These are ownership rules. During this spec, many cohesive modules remain at `core/*.lua`, so directory path alone is not the source of truth; the `core.lua` `@capability` install comments and the split-module paths together define ownership.

- Foundation-owned modules have no inward dependencies on lifecycle, ops, or diagnosis.
- Lifecycle-owned modules depend only on foundation and neutral foundation/lifecycle contracts.
- Ops-owned modules may depend on foundation and may read lifecycle facts/read-models.
- Diagnosis-owned modules may depend on foundation and may read lifecycle facts/read-models.
- Lifecycle must never require or call ops or diagnosis, including board context, dashboard, queue-starvation, state-gap, or timeout-diagnosis helpers.
- Ops and diagnosis must not become the fact source for lifecycle transitions.
- Departments may require `core` and department-local modules only.
- The observability orchestrator may sequence lifecycle, diagnosis, and ops steps, but it is not lifecycle and must not be used as a lifecycle dependency.
- Port injection has one mechanism: a department exposes `make_department(ports)`, production exports through `std.ports.install(make_department)`, and tests rebuild the same department with fake ports. Do not add a parallel injector, fake mechanism, or raw-command seam for this decomposition.
- Neutral seams are allowed only where they preserve the direction rule: lifecycle base-branch access reads foundation/lifecycle config and receives any git-derived facts through `ports.git`; board digest/context is computed by an orchestrator/ops helper using injected ports where needed and is passed into lifecycle proposal builders as data; lifecycle census receives precomputed neutral facts and returns facts, then the orchestrator computes diagnosis state gap after census.
- Cross-package composition is only through namespaced queues plus `source_ref` re-derivation. Never peer-require sibling package internals.

## 7. Migration order

Each numbered slice is one PR. Slices 0-6 are behavior-equivalent. `github-proxy` protocol/queue cleanup is handled only by the named contract-migration follow-up in §2. The order is Mikado-style: leaf and low-risk moves first, callers isolated before movers, and the merge heart untouched so the loop can continue to merge each slice. Physical moves are limited to the shrink-only allowlist in §3; cohesive single-concern modules remain at their current paths and receive only installer comment classification.

Coordination with G-ADAPTER is mandatory. Ports migration slices have precedence on files #738 is migrating (`decompose`, `intake_scan`, `observe_pr`, `loop`, and shared observe-read helpers). Decomposition PRs that overlap those files rebase after the ports slice, preserve any existing `make_department(ports)` shape, and must never reintroduce raw `gh`/`git` command builders or grow `migration/gh-git-adapter.allowlist`. A Slice 2 or Slice 4 caller overlapping #738 either waits or updates the already-ported path.

### Slice 0 - classification manifest and installer comments

Files:

- Create or update this spec as `docs/superpowers/specs/2026-06-16-github-devloop-decomposition-design.md`.
- Modify `packages/github-devloop/core.lua` comments only.

Work:

- Add a classification manifest naming every current installer as foundation, lifecycle, ops, diagnosis, or merge-heart lifecycle.
- Regroup the existing `require(...).install(M)` list with `@capability` comments only. Do not change require paths or order except comments and blank lines.
- State explicitly in comments that install order target is foundation -> lifecycle -> ops -> diagnosis.
- State the physical split allowlist and the stay-in-place rule in the comments or adjacent manifest text, so later slices cannot infer a full subdirectory relocation target.

Equivalence proof:

- `git diff --find-renames --color-moved` shows comments/docs only.
- `scripts/run.sh test github-devloop` passes.
- `scripts/check_repo.py` passes.

### Slice 1 - prompts

Files:

- Create `packages/github-devloop/core/lifecycle/prompts/common.lua`.
- Create `packages/github-devloop/core/lifecycle/prompts/builders.lua`.
- Create `packages/github-devloop/core/ops/prompts.lua`.
- Move `packages/github-devloop/prompts/sync_conflict.lua` to an ops-owned prompt resource path.
- Delete `packages/github-devloop/core/prompts.lua`.
- Update installer paths and direct `require` paths only.

Work:

- Move lifecycle prompt preamble, template rendering, lifecycle builders, and parsers without changing text, parser accepted tokens, or fail-closed behavior.
- Move only sync-conflict prompt builder/resource to ops.

Equivalence proof:

- Existing prompt tests, including intake parser, review meta parser, context layer, sync conflict, and core flow tests, pass via `scripts/run.sh test github-devloop`.
- Contract grep shows no drift in parser labels `⟦FKST:INTAKE⟧`, `⟦FKST:CLASS⟧`, `⟦FKST:REASON⟧`, or review meta labels.

### Slice 2 - payloads and board context seam

Files:

- Create `packages/github-devloop/core/lifecycle/payloads.lua`.
- Create `packages/github-devloop/core/ops/board_context.lua`.
- Delete `packages/github-devloop/core/payloads.lua`.
- Update installer paths and direct `require` paths only.
- Update board-digest/proposal call sites in `departments/observe_issue/main.lua`, `departments/loop/main.lua`, `departments/review_pr/main.lua`, `departments/review_loop/main.lua`, `core/replayer.lua`, `core/context_bundle.lua`, and `departments/intake_judge/main.lua`.

Work:

- Move all proposal/payload builders to lifecycle.
- Move board digest computation to ops board context.
- Replace board-decorated proposal wrappers with the `std.ports`-backed dependency-inversion seam: departments/orchestrators expose `make_department(ports)`, compute board context or board digest through ops/diagnosis helpers using `ports.github` / `ports.git` where GitHub/git facts are needed, then pass that precomputed data into lifecycle proposal builders as parameters. Lifecycle must not require or call ops, diagnosis, `std.github`, `std.git`, or raw `gh`/`git`.
- Preserve current call-site behavior for `observe_issue`, `loop`, `review_pr`, `review_loop`, replay, context bundle generation, and intake judge direct proposal generation by computing the same digest before calling the lifecycle builder or bundle writer.
- `core/context_bundle.lua` must not call `M.board_digest_block` directly after this slice; it receives precomputed board context/digest in its args and writes the same `board.txt` content.
- `departments/intake_judge/main.lua` must not call `core.build_board_proposal` as an ops-owned wrapper; it receives precomputed board context/digest and passes it to the lifecycle proposal builder.
- Keep proposal body bounds and board digest truncation behavior byte-equivalent.
- After the listed call sites are updated, deleting the old board-decorated proposal wrapper path leaves no dangling production caller and no lifecycle -> ops dependency.

Equivalence proof:

- `scripts/run.sh test github-devloop` passes.
- Board digest tests and proposal/review/replay/context-bundle/intake tests pass.
- Contract grep over `schema =`, `devloop_`, `github-proxy.`, `consensus.`, `dedup_key`, and `source_ref` shows no intentional drift.
- Static checks show no lifecycle reference to `board_context`, `board_digest_block`, `append_board_digest_to_proposal`, `build_board_proposal`, `build_board_loop_proposal`, `build_board_pr_review_proposal`, `build_board_pr_review_loop_proposal`, or other ops-owned board digest functions.

### Slice 3 - config and env seam

Files:

- Create `packages/github-devloop/core/foundation/env.lua`.
- Create `packages/github-devloop/core/lifecycle/config.lua`.
- Create `packages/github-devloop/core/ops/config.lua`.
- Delete `packages/github-devloop/core/config.lua`.
- Update installer paths only.

Work:

- Move env primitive access to foundation.
- Move lifecycle posture and base-branch resolution consumed by lifecycle departments to lifecycle config.
- Move dogfood topology policy to ops config while preserving public `branch_config` and `devloop_config` names and return shapes.
- Define the seam explicitly: ops owns upstream/integration/rollup/sync policy inputs, but foundation/lifecycle config exposes the neutral target/base-branch accessor that lifecycle calls. Lifecycle callers of `branch_config()` must not reach through ops to resolve their base branch. Any git fact needed for branch resolution is read by the department/orchestrator through injected `ports.git` and passed into pure config helpers as data.
- Preserve the exact allow-list for `FKST_GITHUB_BOT_LOGIN`, `FKST_GITHUB_REPO`, `FKST_GITHUB_WRITE`, `FKST_DEVLOOP_UPSTREAM_BRANCH`, `FKST_DEVLOOP_INTEGRATION_BRANCH`, `FKST_DEVLOOP_MAX_INFLIGHT`, `FKST_DEVLOOP_MANAGED_SIBLING_REPOS`, `FKST_DEVLOOP_ROLLUP_MERGE`, `FKST_DEVLOOP_ROLLUP_RED_WINDOW_MINUTES`, `FKST_DEVLOOP_RELEASE_NOTES_FALLBACK`, `FKST_DEVLOOP_CONFLICT_LOG_CMD`, `FKST_DEVLOOP_BOARD_CMD`, `FKST_DEVLOOP_INTAKE_PROBE_PROOF`, `FKST_DEVLOOP_TEST_COMMAND`, `FKST_OUTPUT_LANG`, `GH_TOKEN`, and `GITHUB_TOKEN`.

Equivalence proof:

- `scripts/run.sh test github-devloop` passes.
- `core_basics_test` and env/config tests prove identical defaults and validation failures.
- No new env var name appears without manifest classification.
- Static checks show lifecycle departments and lifecycle modules call only foundation/lifecycle config for base-branch resolution, with no ops config dependency.

### Slice 4 - observability decomposition seam

Files:

- Create lifecycle-owned census and orphan reaper modules under `core/lifecycle/`.
- Create an ops-owned dashboard module under `core/ops/`.
- Keep diagnosis detector modules at their current paths: `core/state_gap.lua`, `core/queue_starvation.lua`, and `core/conflict_telemetry.lua`.
- Delete `packages/github-devloop/core/observability.lua`.
- Delete `packages/github-devloop/departments/observability/common.lua`.
- Move or drain `departments/observability/census.lua`, `dashboard.lua`, and `reaper.lua` into their owning modules or the observability orchestrator as specified in §4.
- Keep `packages/github-devloop/departments/observability/main.lua` as the queue/spec wrapper.

Work:

- Preserve the orchestration order: lifecycle census -> diagnosis state gap report/logging -> lifecycle reaper -> `observe_queue_starvation` -> `observe_conflict_hotspots` -> dashboard render -> dashboard publish.
- Keep public `collect_observability_entities` behavior and return shape stable by leaving a wrapper in the observability orchestration surface. The orchestrator closes over injected `ports` via `make_department(ports)`, fetches any needed GitHub/git facts through those ports, calls lifecycle-only census with precomputed data, then runs diagnosis state gap calculation, and only then returns data to dashboard consumers.
- Preserve `devloop_observe_tick` behavior, retry posture, dashboard marker CAS behavior, orphan PR close/comment behavior, state gap log lines, and diagnosis issue-create payloads.
- Do not physically move `state_gap`, `queue_starvation`, or `conflict_telemetry` in this slice; they are already single-concern cohesive modules and are only classified by install comments.
- Lifecycle census must never require or call diagnosis, ops, `std.github`, `std.git`, or raw `gh`/`git`.

Equivalence proof:

- `scripts/run.sh test github-devloop` passes.
- Observability, dashboard, fair rotation, state gap, queue starvation, and conflict telemetry tests pass.
- `scripts/run.sh test-composed` passes because this slice touches namespaced queue production.
- Focused tests assert `collect_observability_entities` still returns `state_gap_report` before dashboard rendering consumes it.
- Static checks show lifecycle census has no state-gap or diagnosis dependency.
- `git diff --find-renames --color-moved` shows only allowlisted observability mixed-file movement plus installer/require path edits; `core/state_gap.lua`, `core/queue_starvation.lua`, and `core/conflict_telemetry.lua` remain at their current paths.

### Slice 5 - residual commands/branches after ports migration

Files:

- Create lifecycle residual domain/policy modules under `core/lifecycle/` only for non-command helpers still mixed into `commands.lua` or `branches.lua`.
- Create ops residual domain/policy modules under `core/ops/` only for non-command helpers still mixed into `commands.lua` or `branches.lua`.
- Create diagnosis residual domain/policy modules under `core/diagnosis/` only if detector-only non-command helpers remain mixed into `commands.lua` or `branches.lua`.
- Delete or shrink `packages/github-devloop/core/commands.lua` as port slices drain raw `gh`/`git` command builders into `std.github` / `std.git`.
- Delete or shrink `packages/github-devloop/core/branches.lua` as port slices drain raw git command builders/parsers into `std.git`.
- Update installer paths only.

Work:

- Do not create new per-plane raw `gh`/`git` command-builder homes.
- Raw command construction/parsing drains into repo-std Tier R `std.github` / `std.git` under G-ADAPTER. This slice only splits residual non-command domain/policy helpers by plane.
- Preserve public command and branch `M.*` names only while unmigrated allowlisted callers need them. When a port slice removes the corresponding command construction, shrink the public surface instead of keeping a compatibility wrapper.
- Keep lifecycle, ops, and diagnosis residual helpers split by caller plane, not by superficial `gh` vs `git` spelling.
- Do not use this slice to move cohesive modules such as `ensure_repo`, `substrate_ref`, `release_notes`, `liveness`, or detector modules; only residual mixed helpers from `commands.lua` and `branches.lua` are split.

Equivalence proof:

- `scripts/run.sh test github-devloop` passes.
- `scripts/run.sh test-composed` passes when namespaced GitHub request payloads or composed wiring are touched.
- `scripts/check_repo.py` and G-ADAPTER pass.
- Contract grep over schemas, queues, branch names, marker strings, and dedup formulas shows no drift. Any raw command-string drift belongs to the ports slice that moved that command into `std.github` / `std.git`, not to this decomposition slice.

### Slice 6 - reconcile split

Files:

- Create `packages/github-devloop/departments/reconcile/convergence.lua`.
- Create `packages/github-devloop/departments/reconcile/timeout.lua`.
- Reduce `packages/github-devloop/departments/reconcile/main.lua` to `M.spec`, schema dispatch, and failure wrapper.
- Create lifecycle and diagnosis reconcile request modules under `core/lifecycle/` and `core/diagnosis/`.
- Create a diagnosis timeout helper module, for example `packages/github-devloop/core/diagnosis/timeout_reconcile.lua`.
- Delete `packages/github-devloop/core/reconcile_requests.lua`.
- Keep `packages/github-devloop/core/convergence.lua` at its current path except for the timeout helper extraction named below.

Work:

- Move thinking, review, and fix true-stall reconcile handlers to `convergence.lua`.
- Move timeout reconcile handler to `timeout.lua`.
- Move the timeout-specific helpers currently in `core/convergence.lua` to the diagnosis timeout owner while preserving their public `M.*` names: `timeout_reconcile_state_version`, `timeout_reconcile_marker`, `has_timeout_reconcile_marker`, `build_devloop_timeout_reconcile_payload`, and `is_supported_timeout_reconcile`.
- Preserve `core/liveness.lua` behavior: timeout escalation still raises `devloop_timeout_reconcile` using `M.build_devloop_timeout_reconcile_payload` with the same queue, schema `github-devloop.timeout-reconcile.v1`, marker, dedup key, and version string.
- Keep Plane 1 convergence helpers in `core/convergence.lua`.
- Keep consumed/produced queues, schema dispatch, marker names, state versions, dedup keys, lock keys, CAS outcomes, and trusted-bot enforcement identical.

Equivalence proof:

- `scripts/run.sh test github-devloop` passes.
- Reconcile, review loop, fix reconcile, timeout, unsupported payload, comment localization, and restart contract tests pass.
- `scripts/run.sh test-composed` passes.
- Before/after grep for `github-devloop.reconcile.v1`, `github-devloop.review-reconcile.v1`, `github-devloop.fix-reconcile.v1`, `github-devloop.timeout-reconcile.v1`, `reconcile:`, `review-reconcile:`, `fix-reconcile:`, `timeout-reconcile:`, `timeout_reconcile_state_version`, `timeout_reconcile_marker`, `has_timeout_reconcile_marker`, `build_devloop_timeout_reconcile_payload`, and `is_supported_timeout_reconcile` shows no drift except owner path.

Stop here. This spec contains only Slices 0-6 and no slice that moves `queue`, `merge_batch`, `merge_gate`, merge-gate reason classes, or `pr_safety`. `github-proxy` devloop protocol/guard cleanup is a separate contract-migration follow-up.

## 8. Done criteria for every slice

Every behavior-equivalent slice PR must prove equivalence with the same checklist:

- `git diff --find-renames --color-moved` shows only the allowlisted relocation plus installer/require path edits for that slice.
- When a slice physically splits an allowlisted mixed file, the old mixed file is deleted in the same PR. Cohesive stay-in-place files are not deleted, relocated, or renamed.
- No shim, compat module, legacy require path, or duplicate implementation remains.
- `scripts/run.sh test github-devloop` is green.
- `scripts/run.sh test-composed` is green when composed wiring, namespaced queues, or cross-package request payloads are touched.
- `scripts/check_repo.py` is green.
- G-ADAPTER must not grow: `migration/gh-git-adapter.allowlist` must not gain a file or command head. A touched file stays allowlisted only for unmigrated debt, and it shrinks only when the corresponding port migration removes its command construction.
- Contract grep over `schema =`, `devloop_`, `github-proxy.`, `consensus.`, `fkst:github-devloop`, marker prefixes, dedup keys, and `source_ref` shows no drift.
- Every source file remains <= 1000 lines.
- Runtime, durable delivery state, and GitHub marker/comment state are never hand-edited.
- Rollback is an ordinary git revert of the slice PR.

The separate `github-proxy` protocol/queue contract-migration follow-up must define its own contract proof. It is not part of this move-only equivalence checklist.

## 9. Conformance guard slice

Add a future `scripts/check_repo.py` invariant set so the capability seams cannot re-tangle. This guard is complementary to G-ADAPTER, not a replacement for it. It may land immediately after Slice 0 as a ratchet, or after Slice 6 once the physical split allowlist has shrunk. It does not touch or relocate the merge heart.

G-ADAPTER already enforces `gh`/`git` command-construction location and shrink-only migration through `scripts/check_repo_gh_git_adapter.py` plus `migration/gh-git-adapter.allowlist`: new raw command heads outside `std.github` / `std.git` fail, and removed heads require allowlist shrink. The capability-layering guard below enforces plane ownership, leaf purity, merge-heart stay-in-place, no new mixed modules, and no shims.

Required guards:

- Track ownership from both sources of truth: `@capability` installer comments for cohesive stay-in-place modules, and physical paths for the new split modules under `core/foundation/*`, `core/lifecycle/*`, `core/ops/*`, `core/diagnosis/*`, and owned department-local modules.
- Forbid lifecycle-owned modules, regardless of path, from requiring ops-owned or diagnosis-owned modules.
- Forbid lifecycle departments from requiring ops or diagnosis modules directly.
- Because installers write onto one shared `M` table, also scan call-graph-shaped references, not only `require`: flag forbidden cross-group `M.*` usage where a lifecycle-owned file calls a function classified as ops or diagnosis, and flag ops/diagnosis calls that make them authoritative for lifecycle transitions.
- Check installer ownership and naming: every installed `M.*` function must have a manifest owner; grouped install blocks must match foundation -> lifecycle -> ops -> diagnosis; new names that imply board/rollup/sync/dashboard stay out of lifecycle, and names that imply state transition/CAS/merge authorization stay out of ops and diagnosis.
- Enforce the shrink-only physical split allowlist from §3. The guard must not require every module to live under `core/{foundation,lifecycle,ops,diagnosis}` in this spec.
- Forbid new mixed top-level `packages/github-devloop/core/*.lua`. Existing cohesive top-level modules are allowed only with a single `@capability` owner comment, and adding a new top-level module requires proving it is single-concern.
- Forbid relocation, deletion, or path rewrites of `core/queue.lua`, `core/merge_batch.lua`, `core/merge_gate.lua`, `core/merge_gate/reason_classes/*`, and `core/pr_safety.lua` by any slice generated from this spec.
- Require `fkst.toml` `[event_deps]` entries for any sibling namespaced queue reference.
- Keep the existing peer cross-package require ban.
- Ban shim, compat, legacy, `.old`, and `_legacy` names in new files.
- Keep existing line-limit, hidden-text, error-class, and source hygiene guards.

The guard should use a shrink-only capability allowlist during migration. Adding a new mixed module, widening the physical split allowlist, or silently converting the target into full subdirectory relocation is red. Raw `gh`/`git` detection stays in G-ADAPTER; do not duplicate that scanner here.

Static checking is a ratchet, not the whole architecture. A require-only guard cannot catch all shared-`M` couplings, and even `M.*` scans cannot prove dataflow intent. The actual direction is enforced by the seams in this spec: neutral foundation/lifecycle base-branch contracts, `std.ports`-backed injection into departments/orchestrators, injected board digest/context for proposal building, and an observability orchestrator that computes diagnosis state gap after lifecycle census instead of letting lifecycle call diagnosis.

## 10. Option 2 - deferred `packages/fkst-dogfood`

Future package:

```text
packages/fkst-dogfood/
  fkst.toml
  core.lua
  core/ops/*
  core/diagnosis/*
  departments/*
  raisers/*
  tests/*_test.lua
```

`fkst.toml` `[event_deps]` declares `github-devloop`. It also declares `github-proxy` and `consensus` only if `fkst-dogfood` directly references their queues. Do not use transitive dependency assumptions as an API.

Owned surfaces after extraction:

- ops departments: `sync_scan`, `sync_conflict`, `pr_freshness_scan`, `rollup_scan`, `rollup_merge`, `ensure_repo`, `substrate_ref_scan`;
- diagnosis departments: observability diagnosis patrols, `dead_letter`, `doctor`, `liveness_scan`;
- matching raisers: branch, ensure-repo, substrate-ref, observability, liveness, and any future doctor raiser;
- ops core: sync, rollup, dashboard, release notes, ensure repo, substrate pin, dogfood topology config;
- diagnosis core: state gap, queue starvation, conflict hotspot, rollup health, dead letter issue drafting, liveness redrive, doctor checks.

Hard rule: `fkst-dogfood` must not require `github-devloop` internals. It composes only via namespaced queues, `source_ref` re-derivation, and generic repo-std ports for local `gh`/`git` command mechanics. Examples:

- produce `github-devloop.devloop_timeout_reconcile` for timeout escalation;
- optionally bounded-redrive `github-devloop.devloop_merge_queue_tick`;
- produce `github-proxy.github_issue_create_request` for diagnosis issue filing;
- re-fetch GitHub issue/PR facts from `source_ref` rather than trusting stale payload snapshots.

P2 down-shift boundary: `fkst-substrate` gets only project-agnostic observe/queue/DLQ/error/cron/delivery facts and generic primitives. The `gh`/`git` command adapter stays in this repository's Tier R `std.github` / `std.git` ports, not the engine. GitHub marker parsing, `fkst-dev` labels/state, dogfood topology, board rendering, and protocol queues stay package-owned.

Trigger criteria to actually implement Option 2:

1. a second non-fkst deployment needs the lean generic GitHub dev engine without dogfood topology;
2. ops/diagnosis release cadence diverges from lifecycle;
3. P2 has exposed generic observe/queue/DLQ/error/cron/delivery facts from `fkst-substrate` so the overlay shrinks instead of copying today's large observability surface.

Until one of those fires, Option 2 is permanent surface without payoff. Keep it deferred.

## 11. Coverage of accepted P1

This spec covers the accepted P1 target list from the capability-layering design with a narrower physical slice set:

- `github-devloop` move-only decomposition covers the mixed god-modules and caller-split modules only: observability, reconcile, prompts, payloads, config, commands, and branches through Slices 1-6.
- Cohesive single-concern modules are covered by installer `@capability` classification only and stay at their current paths. Full physical relocation is the named follow-up in §3.
- The merge-heart stay-in-place rule keeps queue, merge batch, merge gate, reason classes, and PR safety cohesive during this effort.
- `core/convergence.lua` remains the Plane 1 convergence owner, except the timeout-specific reconcile helpers are moved to the diagnosis timeout owner in Slice 6.
- `github-proxy` devloop protocol/guard leak cleanup is not covered by this spec; it is a separate contract-migration follow-up. This spec only records the pointer so that work does not re-enter the decomposition scope.
- `gh`/`git` command mechanics are covered by the ports-adapters/G-ADAPTER migration, not by this decomposition except where a slice must preserve or reuse an already landed `make_department(ports)` path.

## 12. Risks and rollback

The central risk is self-modification: the package is refactoring the code that merges its own refactor PRs. The mitigation is the migration order and the merge-heart stay-in-place constraint. Leaf moves go first; command and reconcile moves are later and supervised; the merge heart is not moved or split.

Other risks:

- A moved module can accidentally change installation order. Mitigation: grouped installer comments first, then one module family per PR.
- A split can silently alter a dedup key, marker, or schema. Mitigation: before/after contract grep and existing integration tests.
- A config split can invert ownership and make lifecycle depend on ops. Mitigation: neutral base-branch config seam plus conformance guard.
- Board digest can invert ownership by making lifecycle call ops. Mitigation: board context is injected data; every caller, including context bundle and intake judge, is updated in the same slice.
- Observability can reintroduce lifecycle -> diagnosis coupling. Mitigation: lifecycle census is pure; the observability orchestrator computes and logs state gap after census and preserves the public return shape.
- Timeout reconcile can be half-owned by convergence and diagnosis. Mitigation: Slice 6 moves the exact timeout helper set out of `core/convergence.lua` while preserving public `M.*` names and wire contracts.
- Ports migration can race the decomposition and reintroduce raw commands into a touched file. Mitigation: G-ADAPTER slices have precedence on overlapping observe-read files; decomposition preserves existing `make_department(ports)` shapes and does not grow `migration/gh-git-adapter.allowlist`.
- The separate `github-proxy` contract cleanup can weaken write safety. Mitigation for that follow-up: caller-supplied guards are re-derived immediately before mutation and fail closed when stale or ambiguous.
- Observability and reconcile write external state in real mode. Mitigation: no hand-edited runtime/durable/GitHub marker state; dry-run tests first; rollback by git revert only.

Rollback for any failed slice is an ordinary git revert of that PR. Do not patch runtime state to compensate for a bad refactor.

## 13. Self-drive note

Safe for `github-devloop` to auto-implement through the normal issue -> PR -> review -> merge path:

- Slice 0, because it is comments/docs only.
- Slice 1, because prompts are leaf text builders with strong parser tests.
- Slice 2, if board digest tests are kept in the same PR and the injection seam is verified across all listed callers.
- Slice 3, if env/config tests are expanded before moving callers and the base-branch seam is verified.

Needs supervision or a narrow manually reviewed issue bound:

- Slice 4, because observability can publish dashboards, close orphan PRs, and file diagnosis issues.
- Slice 5, because it must coordinate with G-ADAPTER and branch topology while avoiding new raw command-builder homes.
- Slice 6, because reconcile writes terminal blocked markers under CAS.
- The separate `github-proxy` protocol/queue contract-migration follow-up, because it changes the adapter contract and must land with paired github-devloop caller changes.
- The conformance guard slice, because a bad guard can block every future autonomous PR.

Any PR generated from this spec has no authority to relocate or redesign `core/queue.lua`, `core/merge_batch.lua`, `core/merge_gate.lua`, `core/merge_gate/reason_classes/*`, or `core/pr_safety.lua`. A separate bugfix or supervised merge-heart design may touch them on its own merits, but it is not part of this decomposition spec.

⟦AI:FKST⟧
