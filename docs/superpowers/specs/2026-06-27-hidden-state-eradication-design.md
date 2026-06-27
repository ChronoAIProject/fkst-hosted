# Hidden-State Eradication — Design Spec (harness + refactor)

Status: design (sshx high-adversarial: audit triplet + design triplet unanimous + review triplet, **1 fix pass** correcting stale premises the review caught). Date: 2026-06-27.

## Problem (user-as-oracle: "hidden state")

A **hidden state** is a lifecycle transition whose advancing condition is a **durable, re-derivable fact** (GitHub marker / git / external fact) but is consulted **only on a transient event path** with **no level-triggered poll re-derive** — so a missed event strands the state forever (liveness-blind: zero error facts). The known-good shape is `dependency_wait`: poll-driven, re-reads the durable fact every round, idempotent; the event callback is a hint, the poll re-derive is the safety net (CLAUDE.md «信任契约»).

## Honest correction (the review triplet, grounded in the current checkout, caught stale audit premises)

The first-draft audit/inventory was **partly stale** — the review (reading the actual restart-table + replayer) corrected it. Recorded honestly per verify-before-narrative:

- **HS-3 (awaiting-pr) is NOT "no poll path".** The current `awaiting-pr` restart row ALREADY declares `observe_surfaces = {issue, liveness_scan}` + `required_facts {state, pr-delegation, child-state}` (`libraries/devloop/restart/issue/transitions/awaiting_pr.lua:39,95`); ordinary issue poll already reaches `replay_from_table` (`observe_issue/main.lua:151,156,642`); `liveness_scan` already force-fetches the delegated PR (`liveness_scan/main.lua:52`); tests cover both parent-poll and pr-entity-change replay. So the earlier GPT-Pro diagnosis ("resume only on `pr-entity-change`") was a **partial/stale view**. The real #1588 stall is therefore a SUBTLER bug inside an existing resolver — most likely the **split-topology rollup-ancestry gate** (awaiting_pr_replayer requires `merged.head_sha` to be an ancestor of upstream; squash/rebase/native-PR heads fail this) and/or the resolver still **trusting `facts.current_pr` if present** instead of force-fresh (`awaiting_pr_replayer.lua:127`). **#1588 must be re-verified against this; the spec no longer claims "no poll path".**
- **HS-1 (intake follow-through) lives OUTSIDE the issue restart-table.** Intake is `github-devloop-intake` / `github-devloop-intake-default`, governed by `check_repo_intake_routing.py` (forbids intake raisers, lifecycle forward queues, issue-list self-reads, state-marker writes). The issue restart lifecycle starts at `thinking` (`core.lua:42-51`; `restart/issue/transitions/index.lua`). The enable→thinking edge is `execute_start` consuming `devloop_execute_request`. So HS-1 needs its OWN intake-level poll contract (not the issue restart-table row contract), or is out of this spec's scope.

## Audited inventory (corrected)

| # | Transition | Durable truth | Current status (review-corrected) |
|---|---|---|---|
| HS-2 | `thinking` → `blocked` via convergence reconcile | `converge-round:v1` marker | `thinking` row declares only `state` as required_fact (`thinking.lua:75`) but `replayer.lua:453,500` re-derives from `converge-round` — the advancing fact is **implicit in replay code, not declared** → genuine hidden-state-ish gap |
| HS-3 | `awaiting-pr` resume/close (#1591) | delegation marker → child-PR merged | Poll structure EXISTS; real bug is a subtler resolver gate (rollup-ancestry / force-fresh) — **tighten the existing resolver, do not add a new one** |
| HS-4 | `ready` re-derives the dependency gate | gate fact | `ready` declares only `state` (`ready.lua:61`) but re-derives the gate in `ready_split.lua:210` — same implicit-advancing-fact gap as HS-2 |
| (good) | `dependency_wait` | blockedBy | reference correct shape — declares non-state advancing facts (`dependency_wait.lua:82`) |

The real, mechanically-common defect the review converged on: **some rows' advancing predicate is a durable fact that is re-derived only in replay code, NOT declared as an advancing fact on the row** — so conformance cannot tell "this row advances on a durable fact (needs a poll resolver)" from "this row reads markers only to rebuild payload". THAT is the crux to harden.

## Core decision (direction endorsed by all 7 angles; corrected)

Reuse the existing `restart_transition_table` + `replayer.lua` seam (no new framework). Two precise pieces:

### 1. Harness — a TYPED advancing-predicate classifier (the crux the review demanded)
Add to each restart row a typed declaration distinguishing **advancing facts** (a durable fact whose visibility advances the state to a successor) from payload-reconstruction facts. Concretely: a typed `advancing_facts` field (each entry: `fact_family`, `successor`, `observe_surfaces`, `source_ref_derivation`). **Conformance invariant**: for every lifecycle-authority row, every fact the replayer uses to choose a successor MUST appear in `advancing_facts` (cross-checked against the replayer's fact reads — not a prose field), and every `advancing_facts` entry MUST be re-derivable on a declared poll surface. A row whose replay derives a successor from a durable fact NOT declared as an advancing fact → **CI red**. This is mechanically enforceable because the classifier is the declared-vs-used cross-check, not a hand-maintained list. Tier ② declarative schema/conformance; a tier-③ runtime assert (named resolver exists + registered at replay time) is a later backstop.

### 2. Refactor — poll re-derive is the canonical path, event is a hint
For each row with an `advancing_facts` entry, the normal poll/liveness surface MUST re-derive the successor from the durable fact (force-fresh; treat passed-in `facts.current_pr`/edge facts as hints requiring a freshness proof — fixes `awaiting_pr_replayer.lua:127`). **"Delete per-state hooks" means delete EVENT-ONLY trigger paths, NOT the domain-specific resolver logic** (keep awaiting-pr's rollup/issue-close checks `awaiting_pr_replayer.lua:139-181`; keep convergence/ready domain logic). The dispatcher gains a resolver **registry** with: name→function validation, deterministic order, idempotent/short-circuit semantics, fail-closed when a declared resolver is missing/unregistered, and a conformance check that each declared observe surface actually invokes the dispatcher (`replay_from_table` currently calls exactly one state function and returns after the first handled replay — extend to run/validate declared advancing resolvers).

### 3. Migration — shrink-only ratchet with EXACT inventory
Seed `migration/hidden-state-resolver.allowlist` with EXACT row/fact/successor tuples (design-time precise, not deferred): HS-2 (`thinking`/`converge-round`→`blocked`), HS-4 (`ready`/dependency-gate→ready-release), HS-3 (`awaiting-pr` resolver tightening — declare child-merged as the advancing fact + force-fresh + fix the rollup-ancestry gate). HS-1 (intake) is tracked separately as an intake-package poll contract, NOT in this allowlist. CI fails on allowlist growth; ratchet to 0.

## Required tests (review-specified)
Synthetic row with a durable advancing fact + no resolver → conformance fails; resolver declared but unregistered → fails; resolver declared for an observe surface the dispatcher doesn't invoke → fails; a non-lifecycle/telemetry/no-durable-advance row → does NOT false-positive; a visible terminal `converge-round` marker raises `devloop_reconcile` from ordinary issue/liveness poll with NO `github_comment_written` edge; each migrated successor re-derives on ordinary poll with NO edge event (awaiting-pr already exemplifies: `awaiting_pr_poll_reconcile_test.lua:233-245`); separate live-defer heartbeat facts from successor-deriving facts (don't conflate `liveness_signal_producers` with advancement resolvers).

## Scope / non-goals
ONE harness (the typed advancing-fact classifier + conformance) for the class; ONE resolver registry/dispatcher. Keep domain resolver logic; delete only event-only trigger paths. No new engine capability for the primary prevention. HS-1 intake handled by a separate intake-level poll contract. Package: `github-devloop` (rows + `replayer.lua` dispatcher) + the conformance where restart-table invariants live; intake contract in the intake packages.

## Reference frame
Kubernetes-controller level-triggered reconciliation (reconcile from observed durable state every loop; watch events are hints) + the in-repo `dependency_wait` as the reference shape + make-illegal-states-unrepresentable (the illegal shape = a row deriving a successor from an undeclared durable fact).

## Adversarial provenance
Audit triplet (3 codex) + design triplet (3 codex, unanimous on the direction) + ChatGPT Pro (DEGRADED on every stage — runtime could not mount the repo/`/tmp`; honest unavailable advisory, NOT synthesized agreement) → review triplet (architecture=comment, quality=reject, tests=reject) which, grounded in the current checkout, caught that **the inventory was partly stale (HS-3 awaiting-pr already has the poll structure; HS-1 intake is a separate package) and the harness crux (the advancing-fact classifier) was under-specified**. This 1 fix pass corrects the stale premises, pins the typed advancing-fact classifier as the mechanically-enforceable crux, sharpens the dispatcher registry, makes the migration inventory exact, and adds the focused conformance tests. Open implementation items: (a) re-verify #1588's exact resolver bug (rollup-ancestry vs force-fresh); (b) the precise intake-level poll contract for HS-1. Implementation (Stage 5) is staged: harness keystone → migrate HS-2/HS-3/HS-4 via the ratchet → 0.

⟦AI:FKST⟧
