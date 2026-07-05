# Workflow-Orchestration Layer — Design SPEC (dynamic materialization)

Date: 2026-07-02 (rev 2026-07-03)
Status: PROPOSED (design only). Converged over seven sshx review rounds (3 codex + GPT-Pro oracle each). Rounds 1–5 hardened soundness under an additive just-in-time model; round 6 (aesthetic) found the additive machinery a workaround under a **static** assumption; round 7 surfaced the decisive missing dimension — **dynamic step content** (a later step's issue body may be generated from an earlier step's merged result) — and all four perspectives converged: the beautiful, more-general form is **dynamic result-driven materialization**, of which static content is the degenerate constant-generator case. §17 is the cumulative audit. Premises verified against source (file:line).

## 1. Problem & goal (re-derived at round 7)

The essence is **bounded result-driven decomposition through ordinary devloop atoms**: run the existing **issue → consensus → PR → merge** atom repeatedly, where each next atom's issue content may be **generated from the completed (merged) result of the prior atom**, under a bounded, user-authored workflow contract in `~/.fkst/workflow/`. Static ("run a fixed N-step plan in order") is the special case where each step's generator ignores prior results. Designing for dynamic subsumes static.

## 2. Governing essence — materialize the next work item when its input exists

- **The workflow layer owns MATERIALIZATION of the next issue, not EXECUTION of the issue lifecycle.** Execution stays the unchanged atom (devloop/consensus/PR). The layer does the one thing the atom cannot: turn a prior merged result + a user generation contract into the next issue's content.
- **Non-existence is ontological, not a gate.** A dynamically-generated step issue genuinely cannot exist before its predecessor merges, because its body is not yet knowable. Ordering is therefore a *consequence* of materialization, not an enforced gate. (A core pre-consensus dependency gate may remain useful only as defense for ordinary issues; it is not the workflow's ordering mechanism.)
- **Structure may pre-exist; content may not.** The workflow's shape (ordered slots, generator contracts, bounds) is authored up front and immutable; each step's *content* is a later materialized artifact bound to the concrete inputs it was generated from.

## 3. Three authorities (the beautiful separation)

- **Structure (blueprint):** an immutable origin fact — workflow id, template digest, ordered step slots, per-slot **generator contract**, bounds (max steps, linear v1), trust/model/prompt-version policy. Holds NO rendered issue bodies.
- **Materialization (ledger/CAS):** the authority for what was actually produced — per slot: predecessor result refs/digests, generator-contract digest, generated-spec digest, child dedup key, child ref. This is the single source of truth for "what content was committed for this slot, from which inputs, and which child carries it."
- **Frontier (derived):** the first slot whose predecessor is merged and which is not yet materialized. Ordering falls out of this; it is computed, never stored.

## 4. Architecture — one package `github-devloop-workflow`, two departments

```
 HOST  ~/.fkst/workflow/**/*.toml  -> file.list + toml.decode (read-only)
   v
+== github-devloop-workflow  — additive intake-policy package ===============+
|  [workflow_select]  classifier: on a matching origin, take origin lease,   |
|     write ONE immutable workflow-blueprint (structure + generator          |
|     contracts + bounds); NO step content, NO child. else -> default policy |
|                                                                            |
|  [workflow_materialize_next]  sole result-driven materializer:             |
|     over leased non-terminal instances, compute FRONTIER from blueprint +  |
|     materialization ledger + public child_result_status:                   |
|       predecessor merged & next slot unmaterialized                        |
|         -> read predecessor result by source_ref                           |
|         -> run the slot's generator contract (static=literal / codex=gen)  |
|         -> LATCH generated spec into the materialization CAS               |
|         -> create exactly ONE ordinary devloop issue (lineage header)      |
|       predecessor still running -> wait                                    |
|       child irreversibly failed -> workflow-terminal{blocked, why}         |
|       all slots merged          -> workflow-terminal{done}; release lease  |
+============================|===============================================+
       github-proxy.         |  ensure_child_issue(parent, dedup_key, body)   |
                             v   (durable create-CAS; ONE child per slot)
+== UNCHANGED CORE — zero edits ============================================+
|  each materialized step is an ORDINARY issue: intake enable -> consensus   |
|  -> PR -> merged. Its merged result feeds the NEXT slot's generator.       |
+===========================================================================+
```

## 5. `workflow_select` — blueprint author (no content, no child)

Sole active consumer of `devloop_intake_candidate` (§13). Precedence, before codex: descendant-of-a-workflow-step (transitive lineage) → default; existing blueprint present → no-op (immutable authority); origin matches a workflow → take the **origin materialization lease** (§10, verified re-read self-only) and write ONE immutable `workflow-blueprint:v1` (structure + generator contracts + bounds) co-located with the existing `track` marker; **create no step content and no child** — materialization owns that. else → delegate to the canonical default policy (§13).

## 6. `workflow_materialize_next` — sole result-driven materializer

Level-triggered over leased non-terminal instances (discovery = leased origins bearing a blueprint; §10). For one instance, under the origin lease: compute the **frontier** (blueprint + materialization ledger + `child_result_status`), then perform exactly one derived action:
- **materialize-next** (predecessor merged, next slot unmaterialized): read the predecessor's merged result by `source_ref`/`content_fetch`; run the slot's generator contract — **static** = emit the literal intent (no codex); **generated** = spawn codex which fetches full prior issue/PR/code context by `source_ref` and produces the next issue's title/body; **latch** the generated spec into the materialization CAS (keyed by origin, blueprint digest, slot id, predecessor-result digest, generated-spec digest); then create exactly ONE ordinary devloop issue via github-proxy's durable create (§7) with a lineage header.
- **wait** (predecessor still running).
- **terminal**: child irreversibly failed → `workflow-terminal{blocked, why}`; all slots merged → `workflow-terminal{done}` + release lease.
It NEVER "advances a state machine"; it materializes one next issue from ground truth or writes one terminal. The name says exactly that.

## 7. Materialization CAS — the single at-most-once + provenance primitive

Replace v6's create-intent-reservation + duplicate-fail-closed scar with ONE durable per-slot materialization record (the authority): `{ slot_id, predecessor_result_digest, generator_contract_digest, generated_spec_digest, child_dedup_key, child_ref }`, states `pending → generated → created`. The generated spec is latched **before or atomically with** child creation, so a replay/regeneration for the same (slot, predecessor-result) is idempotent and cannot commit two different bodies for one slot. Child creation is the durable **`github-proxy.ensure_child_issue(parent, dedup_key, body)`** create-CAS (returns existing-or-created), making two children for one slot unrepresentable. (Verified need: github-proxy's current create ledger is intent+search, not a durable CAS across runtimes — `issue_create.lua:436-488`; this seam is the honest home of at-most-once, replacing runtime-local `with_lock` as the *design* authority. A local lock may remain a same-runtime optimization only.)

## 8. Generator contracts & template format

- `~/.fkst/workflow/**/*.toml` via the exposed sorted-recursive `file.list` (`sdk_fs.rs:25-52`; package bounds/normalizes/symlink-rejects). Needs a schema-constrained **`toml.decode` + canonicalizer** (only upstream dependency; JSON via `json.decode` would remove it — TOML is a UX choice).
- Each step slot declares `content.kind`: **`static`** (literal `intent`, compiled to a constant generator — no codex) or **`generated`** (a bounded `generator` contract: a prompt/instruction that receives the predecessor result by `source_ref` and produces the next issue spec). A single always-literal `intent` field is disallowed once dynamic exists (it would lie for generated steps).
- **v1 = linear + fixed slot count** (bounds: `MAX_WORKFLOW_STEPS`). Dynamic affects **content only, not continuation**: a step's result cannot decide to add/remove slots in v1 (that is out of scope — a bounded agent-loop is a separate future spec). Content-not-in-payload: generation receives source refs + digests, codex fetches full context from source, the generated spec is a durable latched artifact — never a payload blob.

## 9. Public child outcome predicate (no peeking into the atom)

The materializer consumes a public **`child_result_status(child) → { result_ready | fatal | recoverable | running | unknown }`** derived from trusted merged facts + irreversible-terminal facts (PR closed-unmerged etc.), NOT an enumeration of github-devloop's internal restart/PR lifecycle states. The workflow layer must treat each step as an atomic issue→PR unit and never hand-maintain a table of the atom's private states (round-6/7 leaked-abstraction fix). `recoverable` holds (impl-failed retryable, recovery blocks) are waited on with a progress-aware bound, not treated as immediate failure.

## 10. State authority — blueprint + materialization ledger + lease + terminal

Bot-authored (trust `FKST_GITHUB_BOT_LOGIN`), via the published comment seam: `workflow-blueprint:v1` (origin, immutable structure); the per-slot **materialization ledger** (§7, authority for generated content + child identity + provenance); `workflow-terminal:v1{done|blocked|error, why}` (origin, the durable inactive latch removing a finished instance from discovery); a child body **lineage header** (origin ref, blueprint digest, slot id) for the transitive recursion guard. The **origin materialization lease** = the existing assignee-claim doctrine used ONLY as an optimistic execution lease + discovery projection; it is named `origin_materialization_lease` in the design and must not masquerade as the domain concept (assignee is the implementation carrier, not "workflow ownership"). Progress/ordering are derived from the ledger + `child_result_status`; no runtime workflow state machine.

## 11. Idempotency & replay

Immutable blueprint + materialization CAS make replay clean: recompute frontier from blueprint + ledger + child outcomes; the CAS latches generated content per (slot, predecessor-result digest) so regeneration is idempotent and can't fork content; `ensure_child_issue` dedups creation. Crash between latch and create → replay sees `generated` and finishes create; crash after create → ledger `created`. `FKST_GITHUB_WRITE` the only posture switch.

## 12. Bounds & liveness

Finite slots, `MAX_WORKFLOW_STEPS`, bounded generator (single next spec, no loops/branching/dynamic continuation in v1). `workflow_materialize_next` declares a bounded reconciler contract terminating in `workflow-terminal:{done|blocked|error}`; missing blueprint = no-op; corrupt bot-authored blueprint = terminal error. Bounded, not an autonomous agent loop.

## 13. Intake-policy replacement & canonical default seam (additive, now legitimate)

Additive is retained and, under dynamic, is no longer a "tax": materialization is a genuine workflow-layer domain function, not an emulation of a core invariant. A topology-scoped mutually-exclusive **policy-set** ratchet keeps exactly one active `devloop_intake_candidate` consumer. The default policy (embedded in `intake_judge:104-344`) is extracted into a **canonical default-intake executor** `default_intake.act(core, event, opts)` — an event-level idempotent saga over `source_ref`/GitHub truth, NOT a pure `classify(candidate) → effects` (which would leak the executor's lock / re-read / decision-dedup-CAS / replay / reintake concurrency semantics into adapters and make drift *easier*). It lives in `libraries/devloop/intake/*` (the default engine + class-carrier `intake_class` + `intake_service_class` + intake prompt move out of the `github-devloop-intake-default` package into the shared devloop library; no peer cross-package require). Both `intake_judge` and `workflow_select` are **thin sibling adapters** over the one executor; workflow selection is expressed as an optional pre-codex hook `opts.before_codex(ctx) → handled` (ctx built AFTER the locked read/claim/reintake/idempotency gate): a workflow-matched origin writes the co-located blueprint+track decision (handled=true, no codex spawned); otherwise the unchanged default codepath runs. So the non-workflow behavior has ONE source of truth and cannot drift by construction — workflow is a prefilter, not the owner of default dispatch. (Cross-model note: an aesthetic critic preferred a no-extraction pre-marker seam with two candidate consumers; rejected because its zero-drift depends on a wrong-writer-wins first-refusal invariant that cannot be made crisp under two same-topology consumers, whereas the single-consumer-delegates executor is race-free and golden-master-guarded.) A core pre-consensus dependency gate is **out of scope / optional defense**, since dynamic ordering is intrinsic to materialization.

## 14. Validation & harness

Per-class fail-closed disposition (hooks at TDD): bad TOML/unsupported → reject template; missing generator for a `generated` slot, or literal `intent` on a `generated` slot → reject; non-linear/over-MAX → reject; blueprint corrupt → terminal:error; generator produces invalid/oversized spec → terminal:error with why (fail-closed, never create a garbage issue); predecessor result unavailable → wait; two children for one slot → unrepresentable via CAS; descendant candidate → default. Harness: production-shaped namespaced events; a **multi-runtime create-CAS test** (in-process passes falsely on same-runtime locks); a **regeneration-idempotency test** (same predecessor-result digest → same latched spec → one child); a **static-degenerate test** (constant generator, no codex spawned); descendant-recursion test. Golden-master for the §13 default extraction: the existing `github-devloop-intake-default` `*_test.lua` (pins raised-queue order + payload fields for enable/decline/track/escalate/reintake/skips + class-carrier/replay/idempotency/injection) must stay unchanged and green across the behavior-preserving move — but it is real, not complete (it does not pin log lines, cross-runtime races, or byte-for-byte comment/create bodies), so a **`workflow_select` non-workflow equivalence test** must byte-compare its raised queues/payloads against `intake_judge` for at least enable/track/decline/escalate to close the drift gap the move alone cannot.

## 15. Out of scope (v1)

Dynamic **continuation** (result deciding slot count / branching / loops), arbitrary DAG, nested workflows (guarded), unbounded agent loops, manual gates, payload result-blobs, progress DB, multi-repo graphs, decompose replacement, origin as an implementation issue, auto umbrella closure (terminal{done} suffices), a core pre-consensus gate (optional defense only), posture flags beyond `FKST_GITHUB_WRITE`.

## 16. Decisions

1. **Dynamic materialization** is the model; static = constant-generator degenerate case (subsumed). 2. Workflow layer owns **materialization**, not execution. 3. **Three authorities**: blueprint (structure) / materialization ledger-CAS (content+provenance) / frontier (derived ordering). 4. **`workflow_materialize_next`** (not `advance`); select authors blueprint only. 5. **Materialization CAS + `github-proxy.ensure_child_issue`** replace create-intent + with_lock-as-authority. 6. **Public `child_result_status`** predicate; no peeking into the atom. 7. **Generator contract** per slot (static|generated); `content.kind` schema. 8. **v1 linear, fixed slots, content-dynamic only** (no dynamic continuation). 9. **origin_materialization_lease** (assignee as carrier, not domain concept). 10. Additive retained, now legitimate (materialization is a domain function); core gate optional defense. 11. Upstream: `toml.decode` (+ ideally a github-proxy durable create-CAS). 12. Terminal = one `workflow-terminal{done|blocked|error}`.

## 17. Cumulative review audit

- R1–R2: additive shape; ordering insight; async-marker non-atomic-handoff → desired-state reconcilers.
- R3: additive scheduler over-built/unsound → user chose just-in-time.
- R4: 7 just-in-time contract gaps → `smallest_sound_jit_design` (v5).
- R5: cross-runtime/eventual-consistency contracts → assignee-lease + create-intent + terminal-table (v6); oracle declared the model soundness-converged.
- R6 (aesthetic): under the **static** assumption, additive v6 is a sound workaround, not beautiful; ordering-by-non-existence judged a proxy; non-additive small core gate judged most beautiful.
- **R7 (aesthetic, dynamic dimension surfaced, 4/4 revise)**: the static assumption was the blind spot. Under **dynamic step content**, ordering-by-non-existence is *ontological truth* (a generated step cannot exist before its input), `workflow_advance` becomes a *legitimate result-driven materializer* (a domain function the core lifecycle cannot do), the lazy-creation additive tax *dissolves* (lazy is feature-required), and dynamic is the *more general* essence (static = constant generator). Round-6's "non-additive most beautiful" is **narrowed** to the static case. Resolution (this v7): rewrite around dynamic materialization — three-authority split (blueprint / materialization CAS / frontier), rename to `workflow_materialize_next`, replace create-intent + with_lock-authority with a materialization CAS + `ensure_child_issue`, consume a public `child_result_status` instead of peeking into the atom, generator contracts (static|generated), `origin_materialization_lease` honesty, bounded v1 (content-dynamic, not continuation-dynamic). Retained: ordinary step issues, origin→child ledger, source_ref rehydration, no payload passing, immutable structure. Correlated blindness named: static-DAG bias, scheduler-disgust overcorrection, additive-purity fixation, soundness-scar normalization.

⟦AI:FKST⟧
