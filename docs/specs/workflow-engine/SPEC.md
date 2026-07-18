# Specification: General Workflow Engine Kernel + 5 Workflow Packages

## 0. Overview

This spec extracts the generic machinery of the reference package
`packages/github-devloop-workflow` into **one shared library namespace, `workflow.engine.*`,
inside the existing published `workflow` library**, and then builds **five thin adapter
packages** on top of it:

| Package | Role | Injects codex runner? | Multi-step reality |
|---|---|---|---|
| `github-devloop-workflow` (refactored → *workflow-develop*) | dev platform binding + the **single** devloop intake seat | yes | yes (real) |
| `workflow-security` | multi-step security review; findings filed as issues | yes | yes (real) |
| `workflow-finance` | cost/usage report over its own queue | no (static-only) | degenerate-single-step |
| `workflow-marketing` | content drafting over its own queue | no (static-only) | degenerate-single-step (weakest) |
| `workflow-writer` | authors user `fkst.workflow.v1` templates via a reviewed PR | yes | degenerate-single-step |

### The governing constraint (verified)

`migration/code-dedup.allowlist` is **0 bytes** (verified: `wc -c` = 0). The cross-file
dedup ratchet is therefore **zero-tolerance**: the branching / idempotency / frontier /
marker / CAS-key logic **must live exactly once in the shared library** and can **never**
be copied into a package. The reference already proves the target shape — every
`departments/*/main.lua` is a `spec` table plus a one-line `saga.department(spec, handlers())`
call (verified: `departments/workflow_materialize_next/main.lua`). Each adapter's department
files stay ~3-line wrappers carrying **zero dedupable business logic**.

### Verified facts this spec relies on

- `libraries/workflow/fkst.toml`: `kind=library`, `lib_deps=[contract]`,
  `allowed_lib_deps=[contract]`, `exports.public=["workflow.*"]`. It already ships
  `workflow.codex`, `workflow.saga`, `workflow.env`, `workflow.dead_letter`, `workflow.logging`.
  **Adding `workflow.engine.*` introduces NO new lib_dep**, and `generator.lua`'s
  `require("workflow.codex")` becomes an in-library sibling.
- `libraries/workflow/conformance/pack.toml` **forbids** (severity=error) any of these
  substrings in `**/*.lua` (excluding `tests/**`): `github-devloop`, `forge.github`,
  `forge.git`, `fkst-dev:`, and **raw `gh`/`git` command text**. This is decisive:
  - The marker namespace literal `fkst:github-devloop-workflow` (verified at
    `core/marker.lua:33-36,140,220,353,410`) **would trip this pack** — so the namespace
    **must be parameterized out of the kernel** regardless.
  - The reconcile loop's `require("devloop.*")` imports (verified:
    `materialize_reconcile.lua:1-6` requires `devloop.base_ids/context_bundle/base/claims/entity/logging`)
    exceed `allowed_lib_deps=[contract]` and cannot enter the workflow lib — so reconcile
    **must be generalized** (devloop severed, injected via `platform`), not moved as-is.
  - **Fallback library option** (see §1): if the workflow lib's conformance pack should
    not gate engine code, ship a standalone `libraries/workflow-engine` with
    `lib_deps=[contract, workflow]` instead. Default recommendation: use `workflow.engine.*`.
- `frontier.compute_frontier(plan, ledger_facts, child_status_of)` (verified
  `core/frontier.lua:126`) with the 5-status whitelist `{result_ready, fatal, recoverable,
  running, unknown}` (verified `frontier.lua:5-13`); a non-function reader returns
  `terminal-error("missing-child-status-reader")`.
- `generator.spawn_generated` (verified `core/generator.lua:99-107`) returns
  `nil, "missing-generator-runner"` when no runner is injected — **static-only adapters
  must restrict templates to `kind='static'`**.
- reconcile's dev-intake back-reference `require("workflow_select").load_catalog_for_ctx`
  (verified `materialize_reconcile.lua:56`) and ambient `exec_sync` (`:105`) / `with_lock`
  (`:229`) — all severed into injected seams.
- `blueprint.lua:5-6`: `SCHEMA="fkst.workflow.v1"`, `MAX_WORKFLOW_STEPS=16`.
- archaudit is the de-facto security/audit reuse source: `fkst.toml`
  `lib_deps=[contract, workflow, testkit, forge, devloop]`, `event_deps=[idle-detector, github-proxy]`;
  `core.build_issue_create_request` (`archaudit/core.lua:672`), `dedup_key=512`
  (`core.lua:15`), `raisers/audit_poll.lua`, `departments/{audit,dead_letter}`. **No
  chrono-security / chrono-finance / chrono-marketing package exists on this branch** —
  the reuse mandate is satisfied by the kernel + archaudit + saga/dead_letter/contract, and
  the only genuinely new code is per-adapter bindings + built-in records.

---

## 1. The shared kernel: `workflow.engine.*` (library)

### Background

The reference package's `core/*` splits cleanly into a GENERAL half (deps only on
`core.*` + `contract.*` [+ `workflow.codex`]) and a DEV-COUPLED half (heavy `devloop.*`
+ ambient `_G`). Only the GENERAL half plus a **generalized** reconcile control-flow and a
**departments factory** move into the library. Everything DEV-coupled stays in
`workflow-develop`.

### Affected files

**Reuse (move/adapt from `packages/github-devloop-workflow/core/` into
`libraries/workflow/engine/`):**

| Source | Target module | Disposition |
|---|---|---|
| `core/errors.lua` | `workflow.engine.errors` | move-as-is (no deps) |
| `core/blueprint.lua` | `workflow.engine.blueprint` | move-as-is (deps: engine.errors) |
| `core/catalog.lua` | `workflow.engine.catalog` | move-as-is (deps: engine.blueprint, engine.errors) |
| `core/digest.lua` | `workflow.engine.digest` | move-as-is (deps: contract) |
| `core/marker.lua` | `workflow.engine.marker` | **adapt** — parameterize namespace token |
| `core/materialization.lua` | `workflow.engine.materialization` | move-as-is (deps: engine.marker, engine.digest) |
| `core/frontier.lua` | `workflow.engine.frontier` | move-as-is (deps: engine.blueprint, engine.marker, contract.strings) |
| `core/generator.lua` | `workflow.engine.generator` | **adapt** — keep codex runner injected |
| `core/default_catalog.lua` | *not moved* | **generalize** — kernel keeps only the loader/validator shape; dev blueprints stay per-adapter |
| `materialize_reconcile.lua` (handlers loop) | `workflow.engine.reconcile` | **generalize** — sever devloop + ambient + intake fallback |
| *(new)* | `workflow.engine` (facade) + `engine.make_departments` | **new** |

**New files (in `libraries/workflow/engine/`):**
- `engine/init.lua` — the `workflow.engine` facade re-exporting submodules + `make_departments`.
- `engine/reconcile.lua` — generalized reconcile control-flow.
- `engine/departments.lua` — `make_departments(executor, completion, catalog, platform)`.

### Implementation instructions

1. **Move the pure modules verbatim**, rewriting `require("core.X")` →
   `require("workflow.engine.X")`. `errors`, `blueprint`, `catalog`, `digest`,
   `materialization`, `frontier` need no logic changes. Preserve every verified anchor:
   `blueprint.validate` (`:204`), `blueprint.MAX_WORKFLOW_STEPS=16` (`:6`), duplicate-step-id
   rejection (`:195-198`); `catalog.collect_file_records` / `catalog.validate_records`,
   `MAX_CATALOG_FILES=128` (`:6`), duplicate-id-disqualifies-both (`:185-199`);
   `frontier.compute_frontier` 5-status whitelist and coercion (`:7-13,118-124`),
   predecessor gating (`:182-196`), all-`result_ready`→terminal-done (`:206`).

2. **`marker.lua` — parameterize the namespace.** Replace the four hardcoded parse
   patterns (`:33-36`) and four emitters (`:140,220,353,410`) that embed
   `fkst:github-devloop-workflow` with a `namespace` token taken from a constructor:
   ```
   -- workflow.engine.marker
   function M.with_namespace(ns)  -- ns e.g. "fkst:github-devloop-workflow"
     -- returns a table exposing the same build/parse fns, closed over ns
   end
   ```
   This is a **hard precondition**: (a) the literal would trip the workflow lib
   conformance pack (`github-devloop` forbidden), and (b) co-resident adapters sharing an
   issue must not read/overwrite each other's markers.

3. **`generator.lua` — keep the runner injected.** Leave `require("workflow.codex")` (now
   an in-library sibling). Do **not** hardcode a runner: the static path
   `validate_spec(title, intent)` needs none; the generated path uses
   `deps.spawn_codex`/`deps.spawn_codex_sync` and otherwise returns
   `nil, "missing-generator-runner"` (`:99-107`) — this is the contract static-only adapters
   depend on.

4. **`reconcile.lua` — generalize the control-flow.** Port the decision sequence (read
   issue → skip-closed → recompute terminal → verify claim → load blueprint + digest-match →
   materialization_facts → `compute_frontier` → perform-materialize/terminal, reference
   `materialize_reconcile.lua:227-300`), **severing four dev couplings**:
   - `require("workflow_select").load_catalog_for_ctx` fallback (`:56`) → call
     `catalog_provider.load_blueprint(ctx, workflow_id)` from the injected `catalog` seam.
   - `require("devloop.*")` (`:1-6`) → take `base_ids`, `claims`, `entity`, `logging`,
     worktree helpers from the injected `platform` table.
   - ambient `exec_sync` (`:105`) and `with_lock` (`:229`) → `platform.exec_sync`,
     `platform.with_lock`. **Never read `_G`** (service-locator / devloop-decouple ratchets
     are shrink-only — the kernel must add zero new ambient reads).
   - The step-raise call site (reference `record_created_or_raise_create`,
     `materialize_reconcile.lua:202`) → `executor.raise_step(step_ctx)`; the terminal call
     site (`terminal()`, `:34-50`) → `executor.emit_terminal(scope, origin, state, reason_code)`.
   - The frontier's 3rd arg comes from `completion.reader(scope)`.

5. **`engine.make_departments(executor, completion, catalog, platform)`** returns **lazy
   per-department closures**, NOT eagerly-built departments:
   ```
   return {
     select          = function() ... end,
     materialize_next = function() return saga.department(mat_spec, engine_handlers(seams)) end,
     dead_letter      = function() ... end,
   }
   ```
   **Laziness is mandatory**: `saga.department` mutates `_G.pipeline` (`saga.lua:79`), so
   building all three eagerly makes the last clobber the others. The kernel owns the generic
   `spec` skeleton + accept/done/act/wrap handlers (reference
   `materialize_reconcile.lua:331-346`); the adapter supplies only seam bindings and its own
   `consumes`/`produces` queue names (passed through `platform`/spec overrides).

6. **Library placement.** Put the modules under `libraries/workflow/engine/`, exported by
   the existing `workflow.*` public export. Confirm the extracted, parameterized code
   contains **no** forbidden strings (`github-devloop`, `gh`/`git` command text, `forge.git*`)
   so the workflow conformance pack stays green. If any residue is unavoidable, fall back to
   a standalone `libraries/workflow-engine` (`lib_deps=[contract, workflow]`) and add it to
   each package's `[lib_deps]`.

### Definition of Done

- `workflow.engine.{errors,blueprint,catalog,digest,marker,materialization,frontier,generator,reconcile}`
  and `workflow.engine.make_departments` all resolve inside the `workflow` library.
- `workflow.engine.marker.with_namespace(ns)` produces markers using the injected token;
  no `fkst:github-devloop-workflow` literal survives in the library.
- `reconcile` has **zero** `require("devloop.*")`, zero `require("workflow_select")`, zero
  `_G`/ambient reads — all such needs arrive via injected `executor/completion/catalog/platform`.
- `libraries/workflow` conformance pack (`no-product-or-forge-policy-strings`,
  `no-raw-gh-git-command-text`) stays green; `allowed_lib_deps=[contract]` still holds.
- `make_departments` returns closures (verified lazy) — a smoke test building all three in
  one process does not clobber `_G.pipeline`.
- `scripts/check_repo.py` passes locally (code-dedup allowlist still empty and satisfied).

---

## 2. The fixed seam contract (applies to all 5 adapters)

Every adapter passes exactly four seam objects to `engine.make_departments`:

### EXECUTOR — `executor.raise_step` / `executor.emit_terminal`
```
raise_step(step_ctx) -> "raised" | "exists" | "wait" | fail(reason_code)
  step_ctx = { scope, origin, blueprint_digest, slot, predecessor,
               predecessor_ref_digest, generated_spec, facts, current }
emit_terminal(scope, origin, state, reason_code) -> nil
```
The single primitive the kernel calls to materialize one step into a real platform
artifact. **MUST be idempotent, keyed on
`materialization.child_dedup_key(origin, slot.id, predecessor_ref_digest)`**: a second call
with the same key returns `"exists"`, never double-creates; an in-flight create returns
`"wait"`. The **kernel owns WHEN** (from the frontier decision) and the CAS key; the
**adapter owns HOW** the artifact is raised.

### COMPLETION — `completion.reader(scope) -> child_status_of`
```
child_status_of(child_ref) -> status[, detail]
  status ∈ { result_ready | fatal | recoverable | running | unknown }
```
The pure, side-effect-free reader `frontier.compute_frontier` consumes as its 3rd argument.
Any other/thrown value is coerced to `unknown` (`frontier.lua:118-124`). Kernel semantics:
`result_ready`→advance / may reach terminal-done; `fatal`→terminal-blocked;
`recoverable`/`running`/`unknown`→wait (unknown **never** advances, **never** terminalizes —
the safety default). The kernel binds only to this **function**, never a module.

### CATALOG — `catalog.load_blueprint(ctx, workflow_id)` / built-in `records()`
The injected provider that (a) supplies the adapter's built-in `records()` and (b) merges
files from `FKST_WORKFLOW_CATALOG_ROOT` via `catalog.collect_file_records`, validating both
through the one `blueprint.validate`; duplicate id disqualifies both peers. Replaces
reconcile's dev-intake fallback (`:56`).

### PLATFORM — `{ with_lock, exec_sync, discovery, lease, claims, logging, ... }`
All boundary I/O, taken from the adapter's injected table, **never `_G`**.

**stays-per-adapter (never in the kernel):** the concrete platform binding module; the ONE
intake policy that may consume the devloop candidate; built-in blueprint `records()`; the
marker namespace token; the 3 `departments/*/main.lua` + `fkst.toml`; adapter prompts + the
codex runner (or none).

---

## 3. `workflow-develop` (refactor of `github-devloop-workflow`)

### Background
This IS today's `github-devloop-workflow`, refactored so its generic machinery moves into
`workflow.engine.*`, leaving the package as the **dev platform binding + the single devloop
intake seat**. It is the ONE topology allowed to consume
`github-devloop-intake.devloop_intake_candidate` (INTAKE_POLICY_SET ratchet).

### Affected files

**Reuse verbatim (keep in the package — heavy `devloop.*` / ambient users):**
- `core/materialize/actions.lua` — EXECUTOR realization: `issue_create_request` (`:208`),
  `raise_request` (`:228`), `terminal_request` (`:171`),
  `record_created_or_raise_create`, `record_existing_child_or_created_marker`
  (existence→`exists`, in-flight→`wait` `:397-405`).
- `core/materialize/child_status.lua` — COMPLETION reader (`:210`) with `pr_is_merged`
  (`:48`), delegating to `core/child_result.lua` (`child_result_status` `:124`).
  `child_result.lua` requires `devloop.markers.facts` (`:1`) → **stays here** as the dev
  completion impl.
- `core/materialize/discovery.lua`, `core/materialize/lease.lua`, `devloop.claims.verify_issue_claim`.
- **The intake seat**: `core/default_intake.lua`, `core/intake_class.lua`,
  `core/intake_service_class.lua`, `core/select_request.lua`, `workflow_select.lua`
  (consumes `github-devloop-intake.devloop_intake_candidate`).
- `libraries/workflow/codex.lua` (real codex runner — dev IS a code agent, generated slots
  work) + `prompts/{intake,workflow_select}.lua`.
- `libraries/workflow/{saga,dead_letter,env}.lua`.
- `fkst.toml` event wiring unchanged: `event_deps=[github-devloop-intake, github-devloop, github-proxy]`,
  `lib_deps=[contract, devloop, forge, testkit, workflow]`.

**Write new (thin, no engine logic):**
- `bindings.lua` — the single binding table: `executor` bound to `actions.lua`;
  `completion` bound to `child_status.reader`; `catalog` = provider wrapping
  `workflow_select.load_catalog_for_ctx` (so runtime behavior is byte-identical to the old
  fallback) + `records.lua`; `platform` = `{with_lock, exec_sync, discovery, lease,
  verify_issue_claim, logging}` from injected deps, **never `_G`**.
- `records.lua` — the current `core/default_catalog.lua` content (166 lines of dev
  blueprints) as an adapter-owned `records()` provider (kept out of the shared library).
- marker namespace injection: `"fkst:github-devloop-workflow"` → preserves exact current
  wire markers, read-compatible with existing issues.
- `departments/workflow_materialize_next/main.lua` — ~3-line wrapper:
  `engine.make_departments(b...).materialize_next()`; keeps `consumes=workflow_materialization_tick`,
  `produces=github-proxy.github_issue_create_request + github_issue_comment_request`.
- `departments/workflow_select/main.lua` — thin wrapper delegating to reused
  `workflow_select.handlers()`; `consumes=github-devloop-intake.devloop_intake_candidate`,
  `produces=github-devloop.devloop_execute_request + github-proxy.*`.
- `departments/dead_letter/main.lua` — `saga.department(spec, dead_letter.handlers({package="github-devloop-workflow"}))`.
- `tests/*_equivalence` — characterization tests asserting refactored-onto-kernel behavior
  is byte-identical to `tests/frontier_test.lua` / `tests/materialize_reconcile_test.lua`.

### Implementation notes
- Executor is two-hop, both hops reused verbatim: reconcile materializes a **child issue**
  (`github_issue_create_request`); the reused `workflow_select` lineage branch then raises the
  **code atom** `github-devloop.devloop_execute_request` one hop later. Do not collapse them.
- `catalog` provider wraps `workflow_select.load_catalog_for_ctx` so the injected-provider
  path is behaviorally identical to reconcile's old `:56` fallback — this keeps the single
  INTAKE_POLICY_SET seat clean while preserving dev behavior.
- Ambient `with_lock`/`exec_sync`/`exec_argv` are read on this adapter's injected platform;
  dev's existing service-locator allowlist entries carry over (kernel adds none new).

### Definition of Done
- Package builds against `workflow.engine.*`; no engine logic copied into the package.
- `tests/frontier_test.lua` + `tests/materialize_reconcile_test.lua` pass unchanged (golden);
  equivalence tests confirm byte-identical markers/frontier/reconcile decisions.
- Marker wire format unchanged (`fkst:github-devloop-workflow`), read-compatible with live issues.
- Still the sole consumer of `github-devloop-intake.devloop_intake_candidate`.
- `scripts/check_repo.py` green; empty code-dedup allowlist satisfied.

---

## 4. `workflow-security`

### Background
A genuinely multi-step security-review adapter: profile stack → match dependencies against a
vuln index → audit code/tests → file findings. Each step is a codex-analysis run; completion
is **findings filed as `github-proxy` issues (never a merged PR)**. Claims work via its **own
`workflow-security` label/tick path** — never the dev intake seam. archaudit is the primary
reuse source (single codex pass → multi-step engine pipeline is the delta).

### Affected files

**Reuse:**
- The entire kernel `workflow.engine.*` via `make_departments`.
- `workflow.codex.judgment_codex_opts` (`codex.lua:11`) + dispatch as the injected runner
  (same shape as `archaudit/departments/audit/main.lua run_codex`).
- `github-proxy.github_issue_create_request` seam + its dedup/bounds/marker-safety
  (`github-proxy/core/issue_create.lua`, dedup 512) as the COMPLETION filing mechanism.
- Findings→issue-create builder pattern ported from `archaudit/core.lua:672`
  (`build_issue_create_request`), `dedup_key` (`:622`), `schema="github-proxy.issue-create.v1"`.
- Own-tick + label-claim pattern from `archaudit/raisers/audit_poll.lua` (cron) +
  `has_archaudit_label`/`audit_issue_search`.
- `forge.ports.install` + `devloop.github_factory.github_options(exec_sync)` for the
  production GitHub port; `libraries/testkit` harness; `saga`/`dead_letter`.

**Write new:**
- `fkst.toml` — `lib_deps=[contract, workflow, testkit, forge, devloop]`;
  `event_deps=[github-proxy]` (optionally `idle-detector`); **excludes** `github-devloop-intake`
  and `github-devloop`. `persistence_class="saga"` (or `composed_judgment_pipeline`).
- `bindings.lua` — `{executor, completion, catalog, platform}`; marker namespace
  `"fkst:workflow-security"`.
- `core.lua` — security helpers: findings→issue-create builder (ported archaudit shape),
  `dedup_key`, finding validation, per-step prompt builders.
- `completion.lua` — **fresh** pure `reader(scope)->child_status_of` mapping a codex
  analysis step's durable output to the 5-status enum (NOT ported from `child_result.lua`,
  which is `devloop.markers.facts`-coupled).
- `blueprints/security-review.json` — built-in `fkst.workflow.v1` template, own `records()`
  provider. 4 ordered steps: `profile-stack`(generated) → `match-dependencies`(generated) →
  `audit-code-tests`(generated) → `file-findings`(terminal).
- `departments/security_select/main.lua` — OWN intake: consumes
  `workflow-security.security_review_request` + own tick; claims via `workflow-security` label.
- `departments/security_materialize_next/main.lua` — ~3-line `make_departments(...).materialize_next()`.
- `departments/dead_letter/main.lua` — ~3-line `make_departments(...).dead_letter()`.
- `raisers/security_poll.lua` — own cron tick (`workflow-security_tick`), mirrors
  `audit_poll.lua`.
- `prompts/*.md` — per-step codex prompts.
- `tests/*` — engine-binding + multi-step frontier + issue-create parity (testkit).

### Implementation notes
- **executor.raise_step** materializes a slot into a codex-analysis run (not an issue),
  idempotent on `child_dedup_key`: checks durable output / in-flight run
  (`workflow.codex.live_run_active` + injected discovery) → `exists`/`wait`, else spawns
  (`judgment_codex_opts(prompt,'.')` then dispatch) → `raised`; hard failure → `fail(reason)`.
  **emit_terminal** on all-`result_ready` builds+raises `github-proxy.github_issue_create_request`
  per finding (dedup-marked, `workflow-security` label); terminal-blocked → dead-letter terminal.
- **completion.reader** maps: analysis output present/well-formed/durable → `result_ready`;
  run in flight → `running`; transient nonzero/timeout(124) → `recoverable`; malformed/validation
  failure → `fatal`; unreadable → `unknown`.
- **Injects the codex runner** → generated slots work.

### BLOCKING capability gap — network egress for `match-dependencies`
There is **no generic outbound-HTTP capability** in the repo (verified: `libraries/forge`
egress is `gh`/`git` subprocess only; the workflow lib conformance pack forbids raw egress
command text). Step 2 needs an online vuln index. Options:
- (a) the injected **codex runner reaches OSV/GHSA/NVD itself** (its own network egress);
- (b) a **new forge-style egress port** for the vuln index (a genuinely new capability to approve);
- (c) **recommended zero-new-egress fallback: GHSA via `gh api`** (GitHub Security
  Advisories), which rides existing github-proxy/forge `gh` egress — but `gh` command text
  belongs in a **forge/devloop adapter**, never in the workflow lib.
Do not silently assume (a); flag this for approval before implementing step 2.

### Definition of Done
- Multi-step pipeline (4 steps) drives via the kernel; per-step completion advances frontier.
- Findings filed as dedup-idempotent `github-proxy` issues on terminal-done; re-reconcile
  never double-files.
- Never consumes `github-devloop-intake.devloop_intake_candidate`; own tick + label claim only.
- Marker namespace `fkst:workflow-security`; each `departments/*/main.lua` ~3 lines.
- Network-egress decision recorded (a/b/c) before step 2 ships.
- `scripts/check_repo.py` green; Lua tests pass in CI.

---

## 5. `workflow-finance` (and `workflow-marketing` sibling)

### Background
Finance materializes a cost/usage-report `fkst.workflow.v1` template into its OWN
artifact/queue via the kernel — **static-only** (no codex runner), claiming work off its own
label path. Marketing is a structurally identical sibling (content drafting). **Honest
assessment: both are degenerate single-step for v1** — the engine buys nothing over a plain
package until real predecessor-gated stages exist. **Recommendation: ship both as plain
packages now**, adopt the adapter only when a staged pipeline (finance: collect→compute→draft;
marketing: brief→draft→editorial-review) materializes. The multi-step template shape is
pre-specced so the switch is a template swap, not a rewrite. **No reusable
chrono-finance/marketing package exists** on this branch.

### Affected files (finance; marketing mirrors exactly)

**Reuse:**
- Full `workflow.engine.*` via `make_departments`; `blueprint.validate` +
  `catalog.collect_file_records/validate_records` do all validation.
- `saga.department` (one department per file — `_G.pipeline` mutation).
- `dead_letter.handlers({package="workflow-finance"})` verbatim.
- `workflow.engine.marker` with injected token `"fkst:workflow-finance:*"` (marketing:
  `"fkst:workflow-marketing:*"`).
- `contract.*` via `[lib_deps]` — no new lib_dep.

**Write new:**
- `packages/workflow-finance/fkst.toml` — `kind=package.composed`;
  `[lib_deps] libraries=[contract, workflow]`; event_deps + department consumes/produces name
  **finance-owned** queues (own intake candidate + own report/artifact queue), NOT
  `github-proxy.*`/`github-devloop.*`.
- `bindings.lua` — the only real logic: `{executor, completion, catalog, platform}`.
  `executor.raise_step/emit_terminal` target the finance report artifact/queue;
  `completion.reader` maps report state to the 5-status enum; `catalog` = built-in records +
  `FKST_WORKFLOW_CATALOG_ROOT` files (adapter reads the env — re-homed from
  `workflow_select.lua:37/61`); `platform` from injected table, **never `_G`**.
- `built_in_records.lua` — `records()` returning finance built-in template(s), **`kind=static`
  only** (no runner injected). Replaces dev `default_catalog.lua` content.
- `departments/workflow_materialize_next/main.lua` — ~3 lines
  `engine.make_departments(...).materialize_next()`.
- `departments/finance_select/main.lua` — ~3 lines `.select()`; own-label intake, consumes a
  finance-owned candidate/label queue, **never** `github-devloop-intake.devloop_intake_candidate`.
- `departments/dead_letter/main.lua` — ~3 lines over `dead_letter.handlers({package="workflow-finance"})`.
- `packages/workflow-marketing/*` — identical file set; differences confined to
  `bindings.lua` (token `fkst:workflow-marketing:*`, marketing queues),
  `built_in_records.lua` (content-drafting template), `fkst.toml` queue names, `marketing_select`.

### Implementation notes
- **executor.raise_step** keyed on `child_dedup_key`: existence check (mirrors
  `actions.lua:486-522`) → `exists`; in-flight → `wait` (`actions.lua:397-405`); else raise
  ONE finance report-generation request on the finance-owned queue → `raised`; malformed →
  `fail`. `generated_spec` is always nil; **no runner injected**, so the
  `missing-generator-runner` path (`generator.lua:106`) is never reached — **templates must
  be `kind=static`**.
- **completion.reader** — new ~40-line pure module (NOT `child_result.lua`): report durable →
  `result_ready`; failed job → `fatal`; transient → `recoverable`; in progress → `running`;
  unreadable → `unknown`.
- **Templates**: v1 finance single step `{id='generate-cost-usage-report', kind=static}`.
  Real multi-step (pre-specced): `collect-usage-data → compute-cost-breakdown → draft-cost-report`,
  each `kind=static`, later steps read merged predecessor via `contract.source_ref`. Marketing
  v1 `{id='draft-content'}`; real `content-brief → draft → editorial-review`.
  `selector.labels_any` / `title_contains_any` route origin to the template.
- **workflow-writer interplay caveat**: an authored finance template whose id collides with a
  built-in id silently disqualifies BOTH (`catalog.lua:185-199`) — document this warning.

### Definition of Done
- Both packages build against `workflow.engine.*` with only bindings + records as new code.
- Static-only enforced: no `kind='generated'` slot ships without a runner.
- Marker tokens `fkst:workflow-finance:*` / `fkst:workflow-marketing:*`; no cross-adapter
  marker collisions.
- Neither consumes the dev candidate seam; own-label intake only.
- No new `_G`/`core.X` reads (service-locator ratchet green).
- README/docs note the "ship-plain-until-gated-stages-exist" recommendation and the id-collision warning.
- `scripts/check_repo.py` green; Lua tests pass in CI.

---

## 6. `workflow-writer`

### Background
A department adapter over `workflow.engine.*` that turns a user's `fkst-workflow` issue into
a reviewer-ready `fkst.workflow.v1` template PR: codex drafts the template, the **kernel's**
`blueprint.validate` gates it, and it lands as a file under `FKST_WORKFLOW_CATALOG_ROOT` via a
reviewable PR — **zero new engine code**. Validation is REUSED in three layers, never
re-implemented.

### Affected files

**Reuse:**
- `workflow.engine.blueprint.validate/parse_blueprint` (the one validator);
  `workflow.engine.catalog.validate_records/collect_file_records` (dup-id disqualifies both);
  `frontier.compute_frontier`; `materialization.child_dedup_key`;
  `engine.make_departments(...).materialize_next()` reconcile flow;
  `workflow.engine.marker` with token `fkst:workflow-writer`;
  `generator.run_slot_generator` with an injected runner.
- `workflow.saga`, `workflow.dead_letter.handlers({package="workflow-writer"})`.
- `workflow.codex` (`unrestricted_codex_opts`, `codex.lua:19`) for the drafting agent;
  `workflow.env.read_env` (`env.lua:24`) for `FKST_WORKFLOW_CATALOG_ROOT`.
- github-proxy seams: `github_issue_comment_request`, `github_issue_label_request`,
  `github_entity_changed`/`github_poll` (verified live in `packages/github-proxy`).
- The PR-open **technique** from the github-devloop convention (branch→write→commit→push→
  `gh pr create`, `Closes #issue`) — reused inside the adapter's own executor binding; the dev
  INTAKE is NOT imported, and `gh`/`git` text lives in the adapter/forge boundary, never the lib.

**Write new:**
- `fkst.toml` — `kind=package.composed`; `[lib_deps]=[contract, workflow, testkit, forge]`;
  `[event_deps]=[github-proxy]`. **Omits** `github-devloop-intake` (single-intake rule).
- `bindings.lua` — the only substantive new code: `executor` (raise_step spawns codex + opens
  PR, emit_terminal comments back), `completion` (own PR-state `child_status_of`), `catalog`
  provider, `platform` (exec/with_lock/gh/discovery injected, **no `_G`**), own-label claim
  binding, marker token. No engine business logic.
- `records.lua` — adapter-owned `records()` shipping the ONE built-in authoring template.
- `prompts/author_template.lua` — codex prompt: read request → draft `fkst.workflow.v1` →
  self-validate with `workflow.engine.blueprint.parse_blueprint` → write
  `$FKST_WORKFLOW_CATALOG_ROOT/<id>.json` → open PR; refuse PR on validation failure.
- `departments/workflow_writer_select/main.lua` — ~3-line lazy wrapper; own `fkst-workflow`
  label claim, consumes `github-proxy.github_entity_changed` filtered to `fkst-workflow`.
- `departments/workflow_writer_materialize_next/main.lua` — ~3-line
  `engine.make_departments(...).materialize_next()`.
- `departments/dead_letter/main.lua` — ~3-line lazy wrapper.
- `tests/` — built-in authoring template passes `blueprint.validate`; the package never
  consumes `devloop_intake_candidate`; a catalog-lint tool runs `blueprint.validate` over the
  catalog root for the delivered-file PR's CI (reused, not re-implemented).

### Implementation notes
- **executor.raise_step** (single generated authoring step): spawn codex via the injected
  runner with `author_template`; agent drafts, self-validates with `parse_blueprint`, writes
  `<id>.json` on a fresh branch, opens PR with `Closes #<issue>`. The PR is the child artifact
  keyed by `child_dedup_key(origin,"author",predecessor_ref_digest)`; re-materialization finds
  the existing branch/PR marker in the `fkst:workflow-writer` namespace → `exists` (never a
  second PR); in-flight → `wait`. **emit_terminal** posts the terminal fact (PR link or
  `fail()` reason) as a `github-proxy.github_issue_comment_request`. exec/with_lock/gh from
  injected platform, never `_G`.
- **completion.reader** — thin PURE `child_status_of` (NOT `child_result.lua`): PR open with a
  validating template → `result_ready`; run in flight → `running`; transient failure → `recoverable`;
  irrecoverably invalid draft or id colliding with an existing catalog id → `fatal`
  (commented back); unreadable → `unknown`.
- **Templates**: exactly ONE built-in `workflow-authoring-flow`, `selector.labels_any=["fkst-workflow"]`,
  single generated step `author-template`. The deliverable is the NEW user-authored template
  file the PR adds under `FKST_WORKFLOW_CATALOG_ROOT`, validated identically to any built-in
  record on load.
- **Untrusted-content safety**: the drafted template is only ever DATA through the
  byte-bounded validator (id≤128, version≤64, summary≤512, applies_when≤1024, static
  intent/generator ≤8000, 1..16 contiguous steps, unknown-field rejection). Delivery is a
  reviewable PR (human gate); an id colliding with a shipped flow surfaces FATAL and
  disqualifies both, so it cannot silently override.
- Codex runner is injected (real authoring needs a wired runner; a static-only degraded mode
  is possible only by switching the built-in to `kind=static`).

### Definition of Done
- Zero copied engine code; each `departments/*/main.lua` is a ~3-line lazy wrapper building
  exactly one `saga.department`.
- Built-in authoring template passes `blueprint.validate`; delivered files validate identically.
- Never consumes `github-devloop-intake.devloop_intake_candidate`; own `fkst-workflow` label claim.
- No new lib_dep, no dev-intake dep, no `_G`/`core.X` reads.
- Re-materialization never opens a second PR (idempotent on `child_dedup_key`).
- `scripts/check_repo.py` green; Lua tests + catalog-lint pass in CI.

---

## 7. Template schema (`fkst.workflow.v1`) — reference

One validator `blueprint.validate` (`blueprint.lua:204`), used by both `parse_blueprint` and
`catalog.validate_records`. Top-level object (rejects arrays + unknown keys `:208-211`):
`{ schema=="fkst.workflow.v1", id (≤128B), version (≤64B), summary (≤512B), applies_when
(≤1024B), selector? { labels_any?: 1..16 ≤128B, title_contains_any?: 1..16 ≤128B }, steps }`.
`steps` = contiguous 1..16 ordered slots (`:6,230-242`); each step `{ id (unique within
workflow, dup rejected `:195-198`; ≤128B), title (≤200B), content }`. `content` is a tagged
union on `kind`: `{kind='static', intent ≤8000B}` OR `{kind='generated', generator ≤8000B}`
— exactly one payload field, unknown fields rejected (`:160-184`). Loading:
`catalog.collect_file_records(FKST_WORKFLOW_CATALOG_ROOT)` reads `*.json`/`*.toml` (≤128 files
`:6,23`) through the same validator; any id in ≥2 records disqualifies BOTH peers
(`result.duplicates`, `:185-199`).

---

## 8. Build order (kernel first) & verification boundary

### Build order (dependency-ordered — foundation merged before dependents)
See `build_order` field. Wave the backlog: land + merge the kernel (Wave 1) before any
adapter issue is filed, because each adapter is coded in isolation against a `main` that must
already contain `workflow.engine.*`.

### Verification boundary
- **Local (structural, always):** `scripts/check_repo.py` — enforces the ratchets:
  empty `code-dedup.allowlist` (no copied engine logic), `service-locator`/`devloop-decouple`
  shrink-only (no new ambient/`core.X` reads), `library-layering`, INTAKE_POLICY_SET (single
  dev-candidate consumer), the workflow lib conformance pack (no `github-devloop`/gh/git
  strings). Run this after every change.
- **CI only (behavioral):** Lua tests (department + engine + frontier + issue-create parity)
  run in CI, not locally. Author them but rely on CI (per the chrono company-department
  packages note: Lua tests only run in CI).

## 9. Hard constraints (recap)
1. Kernel is ONE shared library (`workflow.engine.*`), never copied — empty
   `code-dedup.allowlist` is zero-tolerance.
2. Exactly ONE topology (`workflow-develop`) consumes
   `github-devloop-intake.devloop_intake_candidate`; all others claim via their own label/queue.
3. New departments add NO ambient `require("core")`/`core.X`/`_G` reads — inject via `platform`.
4. Templates: `fkst.workflow.v1`, 1..16 ordered static|generated slots, loaded from
   `FKST_WORKFLOW_CATALOG_ROOT`, one validator, id-collision disqualifies both.
5. Marker namespace MUST be parameterized per adapter (also required to pass the workflow lib
   conformance pack).
6. `make_departments` returns lazy per-department closures (one `saga.department` per
   entrypoint file — `_G.pipeline` mutation).