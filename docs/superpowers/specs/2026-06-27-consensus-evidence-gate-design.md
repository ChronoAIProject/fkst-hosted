# SPEC: Evidence-Adequacy Decision Gate (redesign of `packages/consensus`)

Status: DRAFT v2 (post-review). Derived via sshx: 2 high-adversarial design rounds + 1 review round
(architecture COMMENT, quality REJECT, tests REJECT — both rejects fixed here), cross-family
ChatGPT-Pro each round, caller synthesis. See §12 Provenance.
Scope: the source-agnostic judgment engine reused by autochrono (issue->reply) and github-devloop
(PR review + merge authorization). Defines the mechanism AND its mechanical enforcement; does not
implement it.

## 1. Problem and the category fix
The current engine spawns 3 fixed codex "angles" (minimal/structural/delete) over the SAME proposal,
peer-invisible, and approves on unanimity (else a meta-judge). This is deliberative VOTING, sharing
narrative + model-family + objective across angles, so its agreement is a CORRELATED signal, not
evidence of correctness. The math (§2) forces a category fix:

> This is not a "consensus" problem. Consensus measures agreement; we want correctness. When a truth
> exists and is checkable, agreement is noise and the right act is to MEASURE the truth (acquire
> trusted evidence); when no truth exists (taste/direction), agreement only measures shared bias. So
> the engine is a **bounded, loss-aware, evidence-adequacy DECISION GATE** — never a consensus
> protocol, never a fake truth machine where no truth exists.

Agreement may be logged; it is NEVER the gate.

## 2. Mathematical foundation (robust theorems; >=2 independent frameworks; Round-2 revised)
- **T1 Data-processing ceiling.** `I(theta; M(X,E)) <= I(theta; X,E)`: no aggregation/debate/vote
  creates truth-information. Nuance: post-processing can move a bad estimator toward the ceiling, so
  debate is a legitimate test-planner / decomposer / triage tool — NEVER evidence, NEVER the gate.
- **T2 Correctness != agreement.** Agreement is a statistic of the shared latent C; perfect
  agreement can carry zero theta-info; reward-for-agreement admits unanimous-false equilibria.
- **T3 Common-mode ceiling (QUALIFIED).** Holds only for the passive/shared-bias component
  (`theta->C->X_i`); a judge that reads theta-coupled EXECUTED evidence carries info beyond C. This
  strengthens grounding: judges must ACQUIRE, not opine.
- **T4 Identifiability is the master constraint.** If theta is not identified by the channels, no
  mechanism is correct. Tested at runtime (§3.1), never assumed.
- **T5 Grounding is the first-class lever (SCOPED).** Only a TRUSTED, reproducible, policy-controlled
  evidence channel raises the ceiling. The line is "trusted vs attacker-controlled", not
  "executed vs not": a proof checker / static analyzer counts; an author claim does not.
- **T6 Irreversible => asymmetric-loss gate (needs declared policy).** Optimal approve/reject is a
  likelihood-ratio / posterior-risk threshold set by the loss ratio, not a symmetric agreement
  threshold. Losses MUST be declared or the gate is a disguised agreement threshold.
- **T7 Adversary = bounded evidence-acquiring falsifier.** Helps only by acquiring trusted
  counterevidence (failing test / counterexample), bounded by relevance / EVOI / cost / budget (no
  unbounded "challenge until satisfied"). Debate alone has a proven blind spot (obfuscated args).
- **T8 Decorrelation must be measured, not assumed.** Cross-family shares C (corpus, RLHF fashion,
  prompt, the SAME proposal text) and can be confidently, identically wrong.
- **T9 Pre-elicitation independence.** Sequential/visible judging causes herding/cascade; same-round
  peer-invisible elicitation is information-preserving and required.
- **T10 Combine by likelihood pooling + audit trail, not headcount.** Linear pooling is not
  externally Bayesian; product pooling double-counts shared evidence; preserve raw evidence by ref.
- **T11 Calibration (DOWNGRADED).** Realized outcomes (reverts/incidents/AVM) are the FINAL anchor +
  necessary audit, but sparse/delayed/censored/confounded — NOT timely identification. Hot-path
  calibration is fast proxy labels whose link to outcomes is itself audited (§3.7).
- **T12 Threat model is part of the math.** Proposal + retrieved text is attacker-controllable;
  evidence must come from trusted execution the system controls; author content is read as data.
- **T13 proposer-challenger is a COROLLARY of T6+T7, not a primitive.**
Impossibility: no mechanism exceeds `I(theta; observations)`; if theta is unidentified, correctness
is impossible; agreement alone never certifies correctness.

## 3. Core design

### 3.1 Decision typing FIRST — TOTAL and fail-closed (fixes qual REJECT)
`decision_type` is a CLOSED enum; routing is total (every proposal reaches a terminal):
- **A factual/verifiable** (a trusted executable channel exists whose output depends on the claim)
  -> evidence-adequacy gate (§3.2).
- **B preference/design** (multiple acceptable outcomes under repo values; no theta) -> requires a
  declared `decision_basis` (owner / rubric / constraints / options / reversibility). Missing
  decision_basis/owner => terminal ESCALATE_NONIDENTIFIABLE (never approve).
- **C constitutive/product-direction** -> governance (maintainer / ADR / human). Always escalates;
  the engine never "identifies" it.
- **D mixed** -> SPLIT into A/B/C subclaims (closed subclaim-graph schema, §4); only A-subclaims are
  gated by evidence; terminal is SPLIT_PROPOSAL carrying the per-subclaim routing.
Totality rule (mechanically checked, §4): the four arms are exhaustive; **when unsure whether theta
exists, classify NON-A and escalate** (fail-closed; never fabricate objectivity). No Type-A approval
is possible while the controlling subclaim is non-identified within budget (§3.5) — this is what
makes autochrono-style no-theta replies TERMINATE (ESCALATE/DEFER with WHY) instead of looping.

### 3.2 Type-A evidence-adequacy gate (consensus core, SOURCE-AGNOSTIC)
consensus does NOT execute evidence (flat/source-agnostic, empty scratch, no git — enforced by §4
egress ratchet). The PRODUCER (source-specific, e.g. github-devloop-pr) / trusted CI / branch-
protection executes evidence and supplies a **typed evidence capsule** the consensus side treats as
OPAQUE-to-schema (arch fix): consensus reads only the typed fields it is allowed to (origin_class,
claim_id, likelihood-ratio inputs, artifact_ref, limitations), never source-specific schema bodies.
Steps: risk_framing (action A, bad event B, prior/prior_ref, loss model, evidence plan,
identifiability) -> read evidence capsule -> bounded falsifier seeks a verified refutation (may find
a counterexample OUTSIDE the decomposition, so decomposition need not be perfect) -> peer-invisible
likelihood judges read the FROZEN evidence ledger (not peer outputs), emitting bounded
likelihood/gap CLAIMS that are planner/explainer inputs, NOT observations (§3.3) -> deterministic
pooler (§3.5).

### 3.3 Typed evidence ledger — taint made UNREPRESENTABLE (fixes test REJECT, highest-impact)
Evidence is a typed ledger of `evidence_item`s; the pooler is REFERENCE-ONLY: every likelihood ratio
MUST cite one or more `evidence_item_id`s; an item with no id contributes nothing. Each item declares
`origin_class in {protected_runner, protected_policy, trusted_baseline, author_controlled}`.
Construction rule (illegal-states-unrepresentable): `author_controlled` items carry likelihood
weight 0 BY TYPE — the pooler cannot read a weight field on them; a verdict whose support set is
empty of non-author items is structurally a `no_evidence` terminal, not an approve. LLM worker
outputs (angle/judge/falsifier prose, claimed citations) are `author_controlled`-equivalent
PLANNER claims: they may propose what evidence to acquire, but are never `evidence_item`s. The
neutralizer (strip marker/verdict/instruction lines from any fetched content) is a hard contract.
EVIDENCE (origin_class protected_*): protected-runner CI / build / typecheck / lint / static
analysis / proof checking / symbolic exec / differential tests / fuzz / mutation / canary telemetry
/ protected benchmarks; trusted_baseline = git/forge facts the system re-derives. The patch must not
silently weaken the harness (an author-modified workflow/test is `author_controlled`).
**LR-value + origin_class provenance (post-v2 cross-family re-review fix, §13 — the one near-blocking
hole):** citing a non-author EvidenceItem prevents EMPTY support but does NOT make the numeric
`LikelihoodFact.lr` trusted — an LLM judge could mint a strong `lr` while citing a protected item, re-
introducing model opinion as the gate. So `LikelihoodFact.lr` MUST itself be capability-generated (a
deterministic / calibrated LR factory or an explicitly approved statistical model, carrying
`generator_id` / `model_version` / `calibration_ref` / cited `evidence_item_id`s); LLM workers may
propose LR hypotheses or evidence gaps but MUST NOT mint `LikelihoodFact.lr`. Likewise `origin_class`
MUST be capability-DERIVED from runner identity / provenance, never trusted because it appears in a
schema field — a forged `origin_class=protected_runner` capsule must be impossible by construction,
not merely invalid by convention.

### 3.4 Evidence ladder by risk/reversibility (feasibility: don't ground everything)
Ground every risk-bearing claim that affects the gate with the cheapest trusted evidence that can
change the decision: Tier0 mechanical (docs/format: parser/build) / Tier1 ordinary reversible (unit/
typecheck/lint/targeted regression) / Tier2 broad reversible (integration/benchmark/property/fuzz/
compat, staged) / Tier3 irreversible/high-blast (full protected CI/migration dry-run/rollback proof/
invariant+security/owner approval/canary; budget-exhaust => fail closed).

### 3.5 Min-expected-loss gate (no magic numbers)
The gate CHOOSES the min-expected-loss action over {accept, accept_staged, reject, defer,
acquire_more_evidence, escalate}. Binary harmful/safe:
`approve iff posterior_odds(B | evidence) < L_reject_good / L_approve_bad`,
posterior_odds = prior_odds * product of TRUSTED likelihood ratios with dependence-group discounts
(conservative no-double-count before calibration exists). Thresholds are COMPUTED from the declared
loss model / repo policy — never hard-coded percentages or vote counts. A bare risk_tier with no
auditable loss expansion is invalid -> terminal fail-closed. Acquire more evidence only when
`EVOI > evidence_cost AND liveness budget permits`.

### 3.6 Termination taxonomy with WHY + DEFINED terminal payload (fixes test REJECT)
Terminals (closed enum): ACCEPT / ACCEPT_STAGED / REJECT_WITH_COUNTEREVIDENCE /
ESCALATE_NONIDENTIFIABLE / DEFER_BUDGET_EXHAUSTED / DEFER_MISSING_EVIDENCE / SPLIT_PROPOSAL — each
carries WHY. The evidence-unavailable case MUST emit a defined terminal payload (never a silent
return / ACK-drop): `consensus_reached{ decision=DEFER_MISSING_EVIDENCE, terminal_code=no_evidence,
source_ref, dedup_key, effect_version, budget_key, why }`. Irreversible + budget-exhaust => fail
closed. Reversible low-risk + exhaust => ACCEPT_STAGED only if rollback+monitoring strong. Budgets/
round-counts derive from a STABLE key; cannot be reset by changing question text or a derived digest.

### 3.7 Calibration split (downgrades T11; SRP)
FAST proxy loop (same-day, calibrates the judge now): mutation tests / historical bug + bad-patch
replay / seeded failures / golden regressions / canary tasks / shadow review / prequential scoring
(log the risk estimate BEFORE outcome known). SLOW outcome loop (AVM: reverts/incidents/SLO/security/
maintainer overrides): a LAGGING audit that calibrates the PROXY system later; lives in a SIBLING
package, never inside consensus. discover->harness-ify: every escaped error becomes a NEW fast-proxy
harness (latency irrelevant; the battery grows monotonically) — this is why AVM is not vanity.

### 3.8 Proxy gap as a first-class risk (the hard, unsolved part)
"Trusted local evidence predicts real harm and is cheap enough to dominate" is NOT guaranteed (CI
green != true). No solution; only governance: track which evidence types actually predicted outcomes,
retire weak proxies, require canaries for high-risk, never let "green CI" mean "true".

## 4. Mechanical Conformance Contract (NEW — fixes test REJECT; the harness layer)
Every load-bearing invariant is enforced by a mechanical check (CI red on violation), per CLAUDE.md
"one canonical way, bypass forbidden", at the STRONGEST feasible tier (capability > schema > scan).

v2 typed schemas (declarative data; conformance reads them):
- `Proposal{ schema, source_ref, dedup_key, effect_version, decision_type, decision_basis?,
  loss_model | risk_tier_policy_ref, evidence_budget, evidence_capsule_ref }`.
- `DecisionType = enum{A,B,C,D}` (closed).
- `LossModel{ L_approve_bad>0, L_reject_good>0 }` (auditable expansion; bare label invalid).
- `EvidenceItem{ id, claim_id, origin_class, method, runner_identity, baseline_commit,
  candidate_commit, artifact_ref, result, scope, known_limitations, dependence_group }`.
- `EvidenceLedger{ items[], frozen:bool }`; `LikelihoodFact{ claim_id, lr, cites:[item_id..] }`.
- `Verdict{ decision (closed terminal enum), why, source_ref, effect_version, posterior?, evidence_ledger_digest }`.

Invariants and their enforcement tier:
1. **No agreement gate** — schema: `Verdict` has no field derivable from inter-judge agreement;
   conformance asserts the pooler reads only `LikelihoodFact.cites -> EvidenceItem`, never judge
   concurrence. (schema + scan)
2. **No magic-number threshold** — capability: the threshold function takes `LossModel` as its only
   numeric input; conformance forbids numeric literals in the gate path. (capability + scan)
3. **Taint / evidence-weighting** — capability: `author_controlled` EvidenceItems have no weight
   field; pooler input is `LikelihoodFact` that MUST cite >=1 non-author item or the verdict is
   `no_evidence`. Illegal to weight author bytes — unrepresentable, not checked. (capability)
4. **Consensus never executes source-specific evidence** — capability/ratchet: a
   `consensus`-scoped egress ratchet (sibling of G-ADAPTER) forbids any gh/git/CI/network head in
   `packages/consensus`; evidence enters only as `evidence_capsule_ref` resolved by approved
   content_fetch readers. Allowlist shrink-only to 0. (capability + ratchet)
5. **Every non-terminal state has a saga row** (budget + watchdog + guaranteed terminal-with-WHY) —
   schema: restart_transition_table conformance (existing engine contract). (schema)
6. **Evidence-unavailable emits a terminal, never silent return** — scan: forbid bare `return`
   on the evidence-missing path; require emitting the §3.6 `DEFER_MISSING_EVIDENCE` payload.
   (scan; target: schema once the saga table models it)
7. **decision_type routing is total + fail-closed** — schema: exhaustive dispatch over the closed
   enum; conformance tests assert A-missing-evidence->DEFER_MISSING_EVIDENCE, B/C-without-
   decision_basis->ESCALATE_NONIDENTIFIABLE, D->SPLIT. (schema + tests)
8. **LLM output is never an observation** — capability: only `protected_*`/`trusted_baseline`
   EvidenceItems can back a `LikelihoodFact`; LLM workers can write planner claims but cannot mint
   EvidenceItems. (capability)
9. **Budget cannot be reset by text/digest change** — schema: budget keyed by stable
   `(source_ref, effect_version, budget_key)`; conformance forbids deriving the key from
   question/digest. (schema + scan)
Items that remain reviewer-only (honest gap): the *correctness* of risk-framing (mapping a proposal
to the right B and loss model) and the proxy gap (§3.8) are not mechanically decidable; they are
governed by calibration (§3.7) and human/owner authority, not by conformance.

## 5. State machine (saga) inside `consensus`
Each non-terminal state declares receiver, liveness class, per-worker timeout, max rounds/cost from
`evidence_budget`, stable round key, on_timeout terminal-with-WHY.
```
proposal_received -> decision_typed
  [A] -> risk_framed -> evidence_adequacy_open -> falsification_open -> evidence_pooled
        -> independent_elicitation_open -> final_pooling -> consensus_reached|consensus_converge
  [B] -> consensus_reached(authorized-by-decision_basis | ESCALATE_NONIDENTIFIABLE)
  [C] -> consensus_reached(ESCALATE_NONIDENTIFIABLE/governance)
  [D] -> consensus_reached(SPLIT_PROPOSAL: per-subclaim routing)
  any -> blocked_with_why (invalid / non_identifiable_irreversible / stale_context /
         evidence_budget_exhausted / evidence_channel_failed / malformed_worker_output /
         missing_loss_model)
```
Sequential stop after each evidence batch. Downstream bounded converge loop keeps source_ref/
effect_version/version-CAS and true-stall -> reconcile -> blocked.

## 6. Mapping to engine primitives
`source_ref` = stable truth pointer (re-derive current truth; never trust stale payload).
`content_fetch` = approved reader to a TRUSTED capsule, read as data. EvidenceItem raw bulk stays in
artifacts addressed from the slim payload (content-not-in-payload). marker-as-fact + version-CAS:
durable decision facts (`review-result:v1` / future `consensus-evidence:v1`) bound to proposal_id/
dedup/effect_version/head/source_ref. peer-invisible spawn/await_all = independence hygiene. Saga
restart row per non-terminal state under engine conformance.

## 7. Delete / Keep
DELETE: unanimity-as-correctness; agreement rewards; minimal/structural/delete lenses as authority;
headcount/percentage/symmetric thresholds; free-form meta-judge as final authority; debate
transcripts as evidence; claimed citations as evidence; author body/context/tests as evidence;
assumed cross-family independence; payload bulk as evidence transport; approval from local transient
probes alone for irreversible actions.
KEEP: proposal -> consensus_reached/consensus_converge seam; source_ref; content_fetch (tightened to
approved readers); peer-invisible same-round spawn; bounded converge loop with narrowed_question +
stable digests; dedup_key/effect_version/version-CAS; fail-closed parsing + bounded schemas.

## 8. Migration (v1 -> v2, NO compatibility shim)
v2 Proposal schema adds decision_type / loss_model|risk_tier_policy_ref / evidence_budget /
evidence_capsule_ref / decision_basis. Consumers (autochrono, github-devloop) move to v2 in one cut;
old approve/reject-only bodies removed (CLAUDE.md: no deprecated shim / dual-mode). github-devloop-pr
becomes the evidence producer (supplies the capsule); AVM moves to / stays in the calibration sibling.

## 9. Open problems and acknowledged first-class risks
proxy gap (§3.8, fundamental, managed not solved); exact capsule field set per source; cold-start
conservative priors + no-double-count discounts before calibration data; Type-B/C authority when the
owner is undefined (autochrono autonomous replies) -> declared policy, default fail-closed to
escalate; who falsifies claim-decomposition (mitigated: falsifier may attack outside it); the
reviewer-only correctness of risk-framing (§4 honest gap).

## 10. How this obeys CLAUDE.md doctrine
harness-first (decision theory / Neyman-Pearson / Blackwell-DPI / Tetlock calibration / Temporal
saga); seek-truth-from-facts (trusted evidence only; author bytes tainted by TYPE); BEAUTY GATE
(deletes the voting mechanism; make-illegal-states-unrepresentable: author bytes cannot carry
weight, an irreversible non-identified proposal cannot ACCEPT); saga/liveness (bounded + terminal-
with-WHY per state, §5); one canonical way + bypass forbidden (§4 conformance contract); package
boundary (consensus source-agnostic via egress ratchet; producer/calibration siblings via published
seam); content-not-in-payload; no magic numbers (threshold computed from loss model); competence axis
(fast-proxy + AVM calibration is the mechanical metric).

## 11. Review record (this draft)
arch=COMMENT (seam pinned via §3.2 opaque capsule + §4 egress ratchet); qual=REJECT FIXED (§3.1 total
+ fail-closed routing guarantees non-identifiable termination); tests=REJECT FIXED (§4 Mechanical
Conformance Contract: typed schemas + per-invariant enforcement tier; author EvidenceItems made
unweightable by type; terminal payload defined; closed enums). **A post-v2 cross-family re-review
(§13) found the "taint made unrepresentable" claim OVERSTATED: author *items* cannot carry weight, but
`LikelihoodFact.lr` value-provenance and `origin_class` authenticity are NOT yet unrepresentable — they
are open capability requirements before implementation (§3.3, §13).** Residual reviewer-only gaps named
in §4 / §9 / §13.

## 12. Provenance
sshx inline consensus: Round 1 (5 independent formalizations: probability / information-theory /
game-theory / isomorphism + ChatGPT-Pro cross-family + caller) converged on the anti-consensus
foundation (§2). Round 2 high-adversarial (map / open-problems / red-team + ChatGPT-Pro + caller)
revised it: T3 qualified, T11 downgraded, "consensus executes evidence" rejected on package
boundary, decision-typing added as keystone. Review round (architecture / quality / tests +
ChatGPT-Pro) produced 2 rejects, fixed in v2 (§4, §3.1, §3.3, §3.6). No reject-and-reshape survived.
Post-v2 cross-family re-review (ChatGPT-Pro, independent, closing the "v2 not yet re-reviewed" honest
gap): verdict MERGE_WITH_NOTED_CAVEATS — the direction is validated; two near-blocking caveats are
recorded in §13 and must be fixed before v2 is treated as implementation-ready.
Artifacts: meta_r1.md, meta_r2.md, r1_*/r2_*/rev_* worker logs.

## 13. Cross-family re-review caveats (post-v2, independent ChatGPT-Pro)
An independent cross-model-family re-review (closing §11's "v2 not yet re-reviewed" gap) returned
**MERGE_WITH_NOTED_CAVEATS**: merge-worthy as a docs-only DESIGN RECORD because the central move —
delete voting-as-correctness, replace it with a bounded, loss-aware, trusted-evidence adequacy gate —
is internally coherent and aligned with §2 / §3.1 / §3.5 / §4; NOT merge-worthy as
"implementation-ready". The implementable-detail gaps (exact capsule fields per producer; exact
dependence-discount formula; EVOI approximation; calibration package interface; saga naming cleanup)
are acceptable draft gaps. Two caveats are near-blocking for IMPLEMENTATION and must be fixed first:

- **C1 — LikelihoodFact.lr provenance hole (the one near-category error; §3.3, §4 invariant 3).** v2
  taints author EvidenceItems but NOT likelihood-ratio GENERATION: a citation to a non-author item
  blocks empty support, but the `lr` number can still be LLM-minted (model opinion re-enters as the
  gate). Fix: `LikelihoodFact.lr` must be capability-generated / calibrated (generator_id /
  model_version / calibration_ref); LLM workers propose hypotheses, never mint `lr`. And `origin_class`
  must be capability-derived from runner identity, not a trusted schema field (forged
  `protected_runner` impossible by construction).
- **C2 — LossModel / risk-framing provenance (§3.5, §4 invariant 2).** Because proposal text is
  attacker-controllable (T12), `loss_model` / `risk_tier_policy_ref` must be policy- or owner-derived
  and capability-bound, never proposal-carried; otherwise an attacker lowers the approval threshold by
  manipulating `L_approve_bad` / `L_reject_good` / priors / risk framing. Invariant 2 should bind the
  threshold to a protected policy/loss-model input, not merely "forbid numeric literals" (some
  constants — 0, 1, caps, tolerances — are structurally unavoidable).

Also-noted (not blocking, sharpen during implementation): "no agreement gate" (§4 inv.1) needs
capability-level isolation so the gate path cannot reconstruct agreement from judge IDs / counts /
concurrence, not just a missing field + scan; the §3.3 neutralizer should be framed as "hostile text
is always quoted data, never affects weight or capability" rather than "strip hostile lines"; T4
identifiability is a runtime *witness* check, not a runtime *proof* of semantic identifiability; §3.1
totality is syntactic (exhaustive dispatch), not semantic (correct A/B/C/D classification) — the
fail-closed "when unsure, classify NON-A" doctrine carries that gap; the §3.8 proxy gap (CI-green !=
true) remains fundamental, managed not solved.
[AI:FKST]
