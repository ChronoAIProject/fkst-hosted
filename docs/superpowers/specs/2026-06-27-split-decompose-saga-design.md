# Split / Decompose Saga — Design Spec

Status: design (sshx adversarial consensus, 1 fix pass after review-triplet reject). Author: operator (sshx). Date: 2026-06-27.

## Core decision (the convergent answer)

**Do NOT build a new "split" package.** The decompose saga lives in the *existing* `github-devloop-decompose` package, reached only through its published `devloop_decompose` seam. **PR-split is out of scope** — an oversized/stuck PR is already decomposed *through the backing issue* when the fix loop / merge gate exhausts (`github-devloop-pr/departments/review_result/main.lua:130-147` and `core/merge_executor.lua:65-84` raise `fix_reconcile` + `devloop_decompose`). A separate PR-split saga would be over-split.

This is what 模式服务当前问题 + 三次法则 + 守包边界 + 包间走前门 + the over-split guard force. But — **this spec is an EXTENSION/migration, not "the existing decompose already does it"**: the review triplet (grounded in the code) showed the existing decompose covers only ONE of the three triggers and lacks native sub-issues, the parent gate, and a parent-close rule. The sections below separate **what exists today** from **what this spec adds**, so the work is honest and implementable.

## What exists today (verified against code — the baseline)

- The `devloop_decompose` seam is real and is the right front door: `github-devloop-decompose/departments/decompose/main.lua:5-7` consumes + publishes `devloop_decompose`; `github-devloop` / `github-devloop-pr` reach it via the seam, not via decompose internals (doctrine-conformant).
- It accepts **only the fix-loop PR shape**: `libraries/devloop/decompose.lua:44-57` requires schema `github-devloop.decompose.v1`, a positive `pr_number`, `round == version_fix_round(version)`, the forward/replay dedup shape, and a PR `source_ref`; `departments/decompose/main.lua:251-258, 311-318, 355-360` require a PR source_ref matching `pr_number` plus a visible `fix-reconcile` marker and `state=blocked`.
- It is a **single `workflow.saga.department`** (`saga.lua:53-84`; `main.lua:415-421` accept/done/act), NOT a multi-state restart lifecycle. The only restart coverage is the **parent `blocked` lifecycle row** (`libraries/devloop/restart/issue/transitions/blocked.lua:11-82` redrives the decompose queue with `decomposed`-marker + issue-create output obligations).
- On accept it writes a `decomposed` ledger marker to the **PR** comment stream (`main.lua:201-220`) and raises `github-proxy.github_issue_create_request` (`main.lua:399-410`). The child body gets lineage/child markers and `parent_comment_target` pointing at the **PR** (`core/decompose.lua:71-103`) — **no native sub-issue relation, no `post_create_blocked_by`, parent stays `blocked`**.
- `github-proxy` issue-create CAN add blockedBy after creation via `post_create_blocked_by` (`core/issue_create.lua:365-403`), but has **no native sub-issue public seam**. Native linking exists only as `forge.github issue_add_sub_issue`, called **directly** by the slicer.
- `github-ratchet-migration-slicer` is **not** a seam producer: `departments/ratchet_migration_driver/main.lua:9-15` has `produces = {}`; it creates slice issues + native sub-issues + ledger **directly** (`:400-404, :462-469`). It is a *second* child-creation authority today.

## What this spec adds (the actual work — none of this exists yet)

### 1. Three evidence-bearing triggers (only trigger 1 exists)

The `devloop_decompose` seam must accept a `trigger_kind` + evidence, each with explicit schema + validation (fail-closed on missing evidence):
1. **`fix-loop-exhaustion`** — EXISTS today (the PR/fix-reconcile shape). Keep it.
2. **`operator-intake`** (NEW) — an oversized issue, payload carries `source_ref` + machine-readable reason evidence (NOT text-length). New schema + validator.
3. **`inventory-ratchet`** (NEW) — a deterministic manifest payload. New schema + validator.

No autonomous text-length splitter (over-split disease). Missing evidence ⇒ **structured WHY terminal** (not the current benign unsupported-payload skip) so liveness/safety can see it — the implementation must make this mechanically testable.

### 2. Native sub-issue child model (does not exist)

- Add a **`github-proxy` public seam for native sub-issue creation** (decompose must NOT reach `forge.github issue_add_sub_issue` directly — 守包边界). Children are created as GitHub **native sub-issues** of the **parent issue** (today `parent_comment_target` is the PR — must change to the parent issue), idempotent via `dedup_key = (parent source_ref, child slot)` (intent-before-create).
- Populate `post_create_blocked_by` so inter-child ordering (if the manifest declares it) is a native edge.

### 3. Parent terminal / close rule (the materially-wrong claim — CORRECTED)

The original spec said "reuse `dependency_wait` for parent resume". **That is wrong**: `dependency_wait` is a prerequisite gate that re-derives blockedBy and **releases to `ready`** (`github-devloop/core/ready_split.lua:178-192`), i.e. it would **re-run implementation on the parent**, not close it. A decomposition parent's DoD is "all children merged → the parent's work is now the children" — it must **deterministically CLOSE/mark-done**, never re-implement.

So this spec adds a **distinct decomposition-parent terminal rule** (NOT dependency_wait-to-ready): a new parent lifecycle state (e.g. `decomposed-awaiting-children`) that is gated on the native sub-issues, and whose ONLY release is **close-the-parent-as-decomposed** when all children reach `merged`. This is an umbrella-close, not a gate-then-implement. It must be a package-owned `restart_transition_table` row with budget + termination + WHY, and must NOT collide with the existing `blocked` row or the `dependency_wait`→`ready` path. (Reuse the blockedBy *fact source*, not the dependency_wait *release semantics*.)

### 3a. Canonical completion fact — single source (resolves quality re-review)

The decomposition-close row must read **ONE canonical completion fact**, or an implementation could create native sub-issues with no parent `blockedBy` edges and a `blockedBy`-based close row would see zero blockers and close *early* (make-illegal-states-unrepresentable violation). **Decision (pin option a):** `decompose-creating` MUST populate `post_create_blocked_by` so the parent has a native `blockedBy` edge to **every** child; the `decomposed-awaiting-children` close row reads **the GitHub-native `blockedBy` re-derivation as its sole completion fact source** (the same fact source `dependency_gate` already re-derives), and closes the parent iff every blocker child is `merged`. It reuses the blockedBy *fact source* but NOT `dependency_wait`'s release-to-`ready` — so it does not entangle with the #1574 dependency_wait/awaiting-pr resume path. Child creation + the blockedBy edge are one idempotent unit: a child created without its parent-blockedBy edge is an illegal partial state (`decompose-creating` redrives until both exist, then `decomposed`). The parent ledger/count surface is therefore the native sub-issue + blockedBy graph (NOT the PR-stream `decomposed` marker, which stays a per-trigger audit artifact only).

### 4. Saga states + restart rows (must be defined, not hand-waved)

If `decompose-proposing` / `decompose-creating` are real lifecycle states (vs the current single event-idempotent department), the spec/plan must define **package-owned restart rows** for each: single responsibility, liveness class, driving queue, budget, guaranteed termination to a WHY-bearing terminal, output obligations — and add the conformance rows (`restart_contract_test`). Otherwise scope them as one idempotent department + the new parent `decomposed-awaiting-children` row, and say so explicitly. No hand-waving "every row exists".

### 5. Slicer migration (it is a second authority today)

`github-ratchet-migration-slicer` must be **migrated** to emit the `devloop_decompose` seam manifest payload and STOP directly creating issues / native sub-issues, with `event_deps` wiring + a test proving it emits the seam payload and no longer direct-creates/links. Until migrated, it stays a separate authority (declare this as a tracked migration step, not "already feeds the seam").

## Reference frame

Saga (Garcia-Molina) + divide-and-conquer / fork-join over child work items + the existing fkst inventory-ratchet slicer as the concrete child-manifest shape + make-illegal-states-unrepresentable for the trigger gate. Known-good shape: a bounded coordinator that fans a parent into independently-deliverable native-sub-issue children, then yields the parent to a **decomposition-close** rule (NOT a re-implement gate).

## Scope / non-goals

ONE saga, issue-decomposition only. No new package. No PR-split saga. No autonomous text-length splitter. No engine (Rust) change anticipated (package-layer). **Explicitly NOT** reusing `dependency_wait`'s release-to-`ready` semantics for the parent (it re-implements; corrected above).

## Required tests (from review)

Validator tests for all three trigger payloads + missing-evidence WHY-terminal; forge/proxy fake tests for idempotent native-sub-issue creation + blockedBy; a slicer test proving it emits the seam payload and no longer direct-creates; restart-table conformance rows for the new parent close state; and a regression proving the parent **closes** (not re-implements) after children merge — especially given the live `dependency_wait`/awaiting-pr resume gap (#1574), which this spec must not entangle with the decomposition-close path.

## Adversarial provenance

3 grounded codex thinking (read the real code + doctrine): minimal=revise, structural=propose, delete=revise — converged on "extend the existing decompose as the single split saga, 3 triggers, native sub-issues, PR-split out of scope". ChatGPT Pro 4th cross-model angle DEGRADED (runtime could not mount the repo; design in an inaccessible download) — recorded as an unavailable advisory cross-check, not synthesized agreement. Review triplet: architecture=reject, quality=comment, tests=reject — all grounded in code, showing the original spec overstated "formalize existing" and had a materially-wrong dependency_wait parent-close claim. This revision (1 bounded fix pass) separates current-vs-new, corrects the parent-close rule, requires a native-sub-issue proxy seam, defines restart rows, and scopes the slicer migration. Meta-judge: the DIRECTION is endorsed by all seven angles; the IMPLEMENTABILITY corrections are incorporated here.

⟦AI:FKST⟧
